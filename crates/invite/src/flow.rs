//! The invite conversation on one `irori/invite/1` connection (design §4.1
//! steps 3-6): the inviter side is a [`Host`], the invitee a [`Guest`].
//!
//! The net layer (card #43, `irori_net::invite`) hands both sides a
//! [`Conn`] whose open proof already showed the invite room key. Each frame
//! is one JSON message on it, tagged by `t`, and the session binding is its
//! `fingerprints()` (the TLS exporter, as pairing's words):
//!
//! 1. B → A `hello` (device id and key, person id, key and epoch, name,
//!    board id); A → B `hello`. B refuses an A whose person key doesn't
//!    match the link's fingerprint, and either side refuses a person whose
//!    key differs from the one another board already pins (§4.1 step 4).
//! 2. Both: `proof` = the code proof and the person proof over the info
//!    ([`crate::code::info`]: both devices, both people, the board, the
//!    session binding). A wrong proof ends the invite.
//! 3. Both screens show both names and person fingerprints; B's also shows
//!    the five words. The person at A types them ([`Host::confirm`]): a
//!    miss counts, the second ends the invite.
//! 4. A → B `bundle` ([`crate::bundle::admit`]) without its rows, the rows
//!    in `rows` batches under the message cap, then `end`; B joins the board
//!    ([`crate::bundle::join`]) and answers `done` with the rows it wrote,
//!    which A inserts. Either side may send `fail` with the reason instead.
//!
//! The host is one-use: the first guest that proves the code holds it, and
//! a second guest ([`Host::run`] called again) ends the invite for both, as
//! pairing does (v0.4 §3 step 5), since someone else has the code.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use irori_identity::{Identity, MemberPin};
use irori_meshseam::{Conn, MAX_MESSAGE};
use irori_pair::wire::WireRow;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};

use crate::bundle::{Bundle, InviteBoards};
use crate::code::{self, SIDE_A, SIDE_B, Side};
use crate::ticket::Ticket;

const VERSION: u32 = 1;
/// Rows per `rows` frame (a board pull's batch, mesh frames).
const ROWS_PER_FRAME: usize = mesh::frames::MAX_BATCH_ROWS;
/// The most rows a bundle may carry.
pub const MAX_BUNDLE_ROWS: usize = 200_000;
/// How long either side waits for the other's next frame before the words
/// (the words themselves wait for the person, up to the window).
pub const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest an invite stays open (design §4.1 step 1: it fits a call);
/// the net layer's invite window has the same cap.
pub const INVITE_TTL: Duration = Duration::from_secs(60 * 60);
/// Word misses that end the invite.
pub const MAX_MISSES: u32 = 2;

pub const STATE_WAITING: &str = "waiting";
pub const STATE_WORDS: &str = "words";
pub const STATE_JOINING: &str = "joining";
pub const STATE_DONE: &str = "done";
pub const STATE_FAILED: &str = "failed";

/// What the screen shows, either side.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: String,
    pub board_id: String,
    pub board_name: String,
    /// Invitee only: the words to read out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
    pub peer_name: String,
    pub peer_fingerprint: String,
    pub my_fingerprint: String,
    pub misses: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t")]
enum Frame {
    #[serde(rename = "hello")]
    Hello(Hello),
    #[serde(rename = "proof")]
    Proof {
        proof: String,
        #[serde(rename = "personSig")]
        person_sig: String,
    },
    #[serde(rename = "bundle")]
    Bundle { bundle: Box<Bundle> },
    #[serde(rename = "rows")]
    Rows { rows: Vec<WireRow> },
    #[serde(rename = "end")]
    End,
    #[serde(rename = "done")]
    Done { rows: Vec<WireRow> },
    #[serde(rename = "fail")]
    Fail { why: String },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hello {
    v: u32,
    device_id: String,
    device_pub: String,
    person_id: String,
    person_pub: String,
    person_epoch: i64,
    name: String,
    board_id: String,
}

impl Hello {
    fn mine(ident: &Identity, board_id: &str) -> Hello {
        let p = ident.current();
        Hello {
            v: VERSION,
            device_id: ident.device_id(),
            device_pub: hex::encode(ident.device_pub()),
            person_id: p.id,
            person_pub: p.pub_hex,
            person_epoch: p.epoch,
            name: p.name,
            board_id: board_id.into(),
        }
    }

    fn side(&self) -> Option<Side> {
        let k = |s: &str| hex::decode(s).ok().filter(|b| b.len() == 32);
        Some(Side {
            device_id: self.device_id.clone(),
            device_pub: k(&self.device_pub)?,
            person_pub: k(&self.person_pub)?,
        })
    }

    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.person_id.clone(),
            person_pub: self.person_pub.clone(),
            root_device_id: self.device_id.clone(),
            person_epoch: self.person_epoch,
        }
    }
}

/// Why an invite ended; the text is what the screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed(pub String);

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Failed {}

fn fail(s: impl Into<String>) -> Failed {
    Failed(s.into())
}

const LOST: &str = "The connection to the other person's Irori was lost.";
const BAD_PROOF: &str = "The other side doesn't hold this invite's code.";
const TAKEN: &str = "Someone else tried to use this invite. Make a new one.";
const MISSED: &str = "Those weren't the words twice. Make a new invite.";
const EXPIRED: &str = "The invite expired. Make a new one.";
const OTHER_KEY: &str = "This person has a different key on another of your boards. Don't go on; check with them first.";

async fn write_frame(c: &dyn Conn, f: &Frame) -> Result<(), Failed> {
    let body = serde_json::to_vec(f).map_err(|e| fail(e.to_string()))?;
    if body.len() > MAX_MESSAGE {
        return Err(fail("A row of this board is too large to send."));
    }
    c.send(&body).await.map_err(|_| fail(LOST))
}

async fn read_frame(c: &dyn Conn) -> Result<Frame, Failed> {
    let body = c.recv().await.map_err(|_| fail(LOST))?;
    serde_json::from_slice(&body).map_err(|_| fail(LOST))
}

async fn read_step(c: &dyn Conn, within: Duration) -> Result<Frame, Failed> {
    match tokio::time::timeout(within, read_frame(c)).await {
        Ok(Ok(Frame::Fail { why })) => Err(fail(why)),
        Ok(r) => r,
        Err(_) => Err(fail("The other side stopped answering.")),
    }
}

/// Both sides after the hellos: send our proof, check theirs.
#[allow(clippy::too_many_arguments)]
async fn prove(
    c: &dyn Conn,
    ident: &Identity,
    room_key: &[u8; 32],
    info: &[u8],
    mine: u8,
    theirs: u8,
    their_person: &[u8],
    within: Duration,
) -> Result<(), Failed> {
    let epoch = ident.current().epoch;
    let person_sig = ident
        .sign_epoch(epoch, &code::PersonProof::new(mine, info))
        .map_err(|e| fail(e.to_string()))?;
    write_frame(
        c,
        &Frame::Proof {
            proof: code::proof(room_key, mine, info),
            person_sig,
        },
    )
    .await?;
    match read_step(c, within).await? {
        Frame::Proof { proof, person_sig }
            if code::proof_ok(room_key, theirs, info, &proof)
                && code::person_proof_ok(their_person, theirs, info, &person_sig) =>
        {
            Ok(())
        }
        _ => Err(fail(BAD_PROOF)),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Knobs; the defaults are the shipping values.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// How long one step waits for the other side.
    pub step: Duration,
    /// How long the inviter waits for the person to type the words.
    pub words: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            step: STEP_TIMEOUT,
            words: INVITE_TTL,
        }
    }
}

struct HostState {
    st: Status,
    words: Vec<String>,
    /// A guest is (or was) on the invite.
    taken: bool,
    /// The person's verdict on the words: Some(true) matched.
    verdict: Option<bool>,
}

/// The inviter's side of one invite: one code, one board, one guest.
pub struct Host {
    ident: Arc<Identity>,
    boards: Arc<dyn InviteBoards>,
    board_id: String,
    code: String,
    room: irori_meshseam::Secret,
    timing: Timing,
    state: Mutex<HostState>,
    changed: watch::Sender<Status>,
    decided: Notify,
}

impl Host {
    /// Opens an invite to `board_id`: a fresh code and the room it derives.
    /// Only an owner may invite (checked again when the bundle is built).
    pub fn new(
        ident: Arc<Identity>,
        boards: Arc<dyn InviteBoards>,
        board_id: &str,
        timing: Timing,
    ) -> Result<Host, Failed> {
        let ver = boards
            .board_verifier(board_id)
            .ok_or_else(|| fail("This device doesn't hold that board."))?;
        let m = ver.membership();
        if !m.owner(&ident.current().id) {
            return Err(fail("Only an owner of the board can invite."));
        }
        let code = irori_pair::new_code();
        let room = code::room_secret(&code).map_err(fail)?;
        let st = Status {
            state: STATE_WAITING.into(),
            board_id: board_id.into(),
            board_name: m.name,
            my_fingerprint: my_fingerprint(&ident),
            ..Default::default()
        };
        let (changed, _) = watch::channel(st.clone());
        Ok(Host {
            ident,
            boards,
            board_id: board_id.into(),
            code,
            room,
            timing,
            state: Mutex::new(HostState {
                st,
                words: Vec::new(),
                taken: false,
                verdict: None,
            }),
            changed,
            decided: Notify::new(),
        })
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    /// The invite room (its key is what the net layer's open proof and
    /// guest window use).
    pub fn room(&self) -> &irori_meshseam::Secret {
        &self.room
    }

    /// The ticket for this invite, given where this device can be reached
    /// (`irori_net::invite::InviteAddr`'s fields).
    pub fn ticket(
        &self,
        endpoint: &str,
        relay: &str,
        relay_key: &str,
        relay_quic_port: u16,
        addrs: Vec<String>,
    ) -> Ticket {
        Ticket {
            code: self.code.clone(),
            endpoint: endpoint.into(),
            relay: relay.into(),
            relay_key: relay_key.into(),
            relay_quic_port,
            addrs,
            fingerprint: my_fingerprint(&self.ident),
            board_id: self.board_id.clone(),
            board_name: lock(&self.state).st.board_name.clone(),
        }
    }

    pub fn status(&self) -> Status {
        lock(&self.state).st.clone()
    }

    /// Told on every change.
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.changed.subscribe()
    }

    fn set(&self, g: &mut HostState, f: impl FnOnce(&mut Status)) {
        f(&mut g.st);
        self.changed.send_replace(g.st.clone());
    }

    fn end(&self, why: &str) -> Failed {
        let mut g = lock(&self.state);
        if g.st.state != STATE_DONE && g.st.state != STATE_FAILED {
            self.set(&mut g, |s| {
                s.state = STATE_FAILED.into();
                s.error = why.into();
            });
        }
        g.verdict.get_or_insert(false);
        drop(g);
        self.decided.notify_waiters();
        fail(why)
    }

    /// The person typing the guest's words. A miss counts; the second ends
    /// the invite. Ok(true) on a match.
    pub fn confirm(&self, typed: &str) -> Result<bool, Failed> {
        let mut g = lock(&self.state);
        if g.st.state != STATE_WORDS || g.verdict.is_some() {
            return Err(fail("No invite is waiting for words."));
        }
        if irori_pair::words_match(&g.words, typed) {
            g.verdict = Some(true);
            self.set(&mut g, |s| s.state = STATE_JOINING.into());
            drop(g);
            self.decided.notify_waiters();
            return Ok(true);
        }
        let misses = g.st.misses + 1;
        self.set(&mut g, |s| s.misses = misses);
        drop(g);
        if misses >= MAX_MISSES {
            return Err(self.end(MISSED));
        }
        Ok(false)
    }

    /// Ends the invite (the person closed it).
    pub fn cancel(&self) {
        self.end("Canceled.");
    }

    /// Ends the invite when its hour is up.
    pub fn expire(&self) {
        self.end(EXPIRED);
    }

    /// Serves one guest on a connection whose open proof showed the code
    /// (the net layer's invite handler). A second call ends the invite for
    /// both, since someone else has the code.
    pub async fn run(&self, c: Arc<dyn Conn>) -> Result<(), Failed> {
        let taken = std::mem::replace(&mut lock(&self.state).taken, true);
        if taken {
            let e = self.end(TAKEN);
            let _ = write_frame(&*c, &Frame::Fail { why: TAKEN.into() }).await;
            c.close(TAKEN);
            return Err(e);
        }
        let r = self.serve(&*c).await;
        if let Err(e) = &r {
            let e = self.end(&e.0);
            let _ = write_frame(&*c, &Frame::Fail { why: e.0.clone() }).await;
        }
        r
    }

    async fn serve(&self, c: &dyn Conn) -> Result<(), Failed> {
        let t = self.timing;
        let binding = c.fingerprints().0;
        let Frame::Hello(guest) = read_step(c, t.step).await? else {
            return Err(fail(BAD_PROOF));
        };
        let Some(b_side) = guest.side() else {
            return Err(fail(BAD_PROOF));
        };
        if guest.board_id != self.board_id {
            return Err(fail("That invite is for another board."));
        }
        if guest.person_id == self.ident.current().id {
            return Err(fail(
                "That's you. Add your own devices in Settings → Devices.",
            ));
        }
        if self
            .boards
            .pinned_key(&guest.person_id)
            .is_some_and(|k| k != guest.person_pub)
        {
            return Err(fail(OTHER_KEY));
        }
        let me = Hello::mine(&self.ident, &self.board_id);
        write_frame(c, &Frame::Hello(me.clone())).await?;
        let a_side = me
            .side()
            .ok_or_else(|| fail("this device's keys are damaged"))?;
        let info = code::info(&a_side, &b_side, &binding, &self.board_id);
        prove(
            c,
            &self.ident,
            &self.room.key,
            &info,
            SIDE_A,
            SIDE_B,
            &b_side.person_pub,
            t.step,
        )
        .await?;
        let notified = self.decided.notified();
        {
            let mut g = lock(&self.state);
            if g.verdict.is_some() {
                return Err(fail(g.st.error.clone()));
            }
            g.words = code::words(&self.room.key, &info);
            let fp = code::person_fingerprint(&b_side.person_pub);
            self.set(&mut g, |s| {
                s.state = STATE_WORDS.into();
                s.peer_name = guest.name.clone();
                s.peer_fingerprint = fp;
            });
        }
        if tokio::time::timeout(t.words, notified).await.is_err() {
            return Err(fail("Nobody typed the words in time. Make a new invite."));
        }
        if lock(&self.state).verdict != Some(true) {
            let why = lock(&self.state).st.error.clone();
            return Err(fail(why));
        }
        let mut bundle = crate::bundle::admit(
            &*self.boards,
            &self.ident,
            &self.board_id,
            &guest.pin(),
            &guest.name,
        )
        .map_err(fail)?;
        let rows = std::mem::take(&mut bundle.rows);
        write_frame(
            c,
            &Frame::Bundle {
                bundle: Box::new(bundle),
            },
        )
        .await?;
        for batch in rows.chunks(ROWS_PER_FRAME) {
            write_frame(
                c,
                &Frame::Rows {
                    rows: batch.to_vec(),
                },
            )
            .await?;
        }
        write_frame(c, &Frame::End).await?;
        let Frame::Done { rows } = read_step(c, t.step).await? else {
            return Err(fail(LOST));
        };
        if let Some(st) = self.boards.board_store(&self.board_id) {
            let evs = rows.iter().map(WireRow::event).collect();
            let _ = mesh::boards::insert_board_rows(&st, evs);
        }
        let mut g = lock(&self.state);
        self.set(&mut g, |s| s.state = STATE_DONE.into());
        Ok(())
    }
}

fn my_fingerprint(ident: &Identity) -> String {
    hex::decode(ident.current().pub_hex)
        .map(|k| code::person_fingerprint(&k))
        .unwrap_or_default()
}

/// The invitee's side: one link, one attempt.
pub struct Guest {
    ident: Arc<Identity>,
    boards: Arc<dyn InviteBoards>,
    ticket: Ticket,
    room: irori_meshseam::Secret,
    timing: Timing,
    state: Mutex<Status>,
    changed: watch::Sender<Status>,
}

impl Guest {
    /// Reads the link (and derives the room, which is slow on purpose:
    /// scrypt).
    pub fn new(
        ident: Arc<Identity>,
        boards: Arc<dyn InviteBoards>,
        link: &str,
        timing: Timing,
    ) -> Result<Guest, Failed> {
        let ticket = Ticket::parse(link).map_err(|e| fail(e.0))?;
        if boards.board_verifier(&ticket.board_id).is_some() {
            return Err(fail("This device already holds that board."));
        }
        let room = code::room_secret(&ticket.code).map_err(fail)?;
        let st = Status {
            state: STATE_WAITING.into(),
            board_id: ticket.board_id.clone(),
            board_name: ticket.board_name.clone(),
            peer_fingerprint: ticket.fingerprint.clone(),
            my_fingerprint: my_fingerprint(&ident),
            ..Default::default()
        };
        let (changed, _) = watch::channel(st.clone());
        Ok(Guest {
            ident,
            boards,
            ticket,
            room,
            timing,
            state: Mutex::new(st),
            changed,
        })
    }

    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn room(&self) -> &irori_meshseam::Secret {
        &self.room
    }

    pub fn status(&self) -> Status {
        lock(&self.state).clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.changed.subscribe()
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        let mut g = lock(&self.state);
        f(&mut g);
        self.changed.send_replace(g.clone());
    }

    /// Ends the attempt before [`Guest::run`] (the inviter couldn't be
    /// reached, or the person canceled), unless it already ended.
    pub fn fail(&self, why: &str) {
        self.set(|s| {
            if s.state != STATE_DONE && s.state != STATE_FAILED {
                s.state = STATE_FAILED.into();
                s.error = why.into();
            }
        });
    }

    /// Joins over the connection the net layer dialed from the ticket
    /// (`irori_net::invite::IrohNet::dial_invite`).
    pub async fn run(&self, c: Arc<dyn Conn>) -> Result<(), Failed> {
        let r = self.join(&*c).await;
        if let Err(e) = &r {
            self.set(|s| {
                s.state = STATE_FAILED.into();
                s.error = e.0.clone();
            });
            let _ = write_frame(&*c, &Frame::Fail { why: e.0.clone() }).await;
        }
        r
    }

    async fn join(&self, c: &dyn Conn) -> Result<(), Failed> {
        let t = self.timing;
        let binding = c.fingerprints().0;
        let me = Hello::mine(&self.ident, &self.ticket.board_id);
        write_frame(c, &Frame::Hello(me.clone())).await?;
        let Frame::Hello(host) = read_step(c, t.step).await? else {
            return Err(fail(BAD_PROOF));
        };
        let Some(a_side) = host.side() else {
            return Err(fail(BAD_PROOF));
        };
        if code::person_fingerprint(&a_side.person_pub) != self.ticket.fingerprint {
            return Err(fail(
                "The other side isn't the person the link names. Don't go on; ask for a new link on a call.",
            ));
        }
        if self
            .boards
            .pinned_key(&host.person_id)
            .is_some_and(|k| k != host.person_pub)
        {
            return Err(fail(OTHER_KEY));
        }
        let b_side = me
            .side()
            .ok_or_else(|| fail("this device's keys are damaged"))?;
        let info = code::info(&a_side, &b_side, &binding, &self.ticket.board_id);
        prove(
            c,
            &self.ident,
            &self.room.key,
            &info,
            SIDE_B,
            SIDE_A,
            &a_side.person_pub,
            t.step,
        )
        .await?;
        let words = code::words(&self.room.key, &info);
        self.set(|s| {
            s.state = STATE_WORDS.into();
            s.words = words;
            s.peer_name = host.name.clone();
        });
        let Frame::Bundle { mut bundle } = read_step(c, t.words + t.step).await? else {
            return Err(fail(LOST));
        };
        self.set(|s| s.state = STATE_JOINING.into());
        loop {
            match read_step(c, t.step).await? {
                Frame::Rows { rows } if bundle.rows.len() + rows.len() <= MAX_BUNDLE_ROWS => {
                    bundle.rows.extend(rows)
                }
                Frame::End => break,
                _ => return Err(fail(LOST)),
            }
        }
        let rows = crate::bundle::join(
            &*self.boards,
            &self.ident,
            &self.ticket.board_id,
            &host.person_id,
            &bundle,
        )
        .map_err(fail)?;
        write_frame(
            c,
            &Frame::Done {
                rows: rows.iter().map(WireRow::of).collect(),
            },
        )
        .await?;
        self.set(|s| {
            s.state = STATE_DONE.into();
            s.board_name = bundle.name.clone();
        });
        Ok(())
    }
}
