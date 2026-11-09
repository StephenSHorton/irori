//! Person-to-person DMs (card #538, docs/design/talk-dms.md): a DM between
//! two people rides on a hidden two-person space, made on the first
//! message.
//!
//! - **The space** is an ordinary space log with a random id, its tag
//!   `~dm`, and a private marker (`boards_own::DmSpaces`) that says what it
//!   is. Only the marker is trusted, never the tag.
//! - **The hand-off** runs over a space the two already share (S), on the
//!   point-to-point `dm` frame: `ask`, `pin`, `offer`, `done`. S's log
//!   carries nothing, and S's word is never trusted for who someone is:
//!   each side proves its key back to the person id ([`check_pin`],
//!   [`check_sender`]).
//! - **Pending** is derived, not queued: the other person has no counted
//!   cert. The hand-off runs when the space is made, on a hello from one
//!   of their devices on S, and on each S vector tick while pending.
//! - **The fold rule** ([`dm_conv_of`]) is the one door from a space's rows
//!   into a DM.
//!
//! The mesh's hooks run on a connection's own task, so they only queue
//! work ([`Dms`]); one thread takes it, which also keeps two steps of one
//! hand-off from running at once.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use gocompat::sub::Queue;
use irori_identity::membership::{MEMBER_LEFT, Membership, valid_pin};
use irori_identity::objects::{ROLE_DESKTOP, verify_person};
use irori_identity::{KeyProof, MemberDeviceAdded, MemberPin, verify_key_proof};
use irori_invite::bundle::{self, Bundle, OfferFold};
use irori_pair::wire::WireRow;
use mesh::boardmesh::{DMS_PROTOCOL, DmFrame, DmHooks, LivePeer};
use serde_json::{Value, json};
use store::kinds::{self, PERSON_ACTOR_PREFIX};
use store::{Event, Filter};

use super::Services;
use super::boards::{
    BoardLog, BoardMember, Error, Result, each_row, lock, new_board_id, valid_board_id,
};
use super::boards_own::{BoardForgotRow, DM_SPACE_NAME, DM_SPACE_TAG, DmSpace};
use super::invites::CoreBoards;
use crate::logf;

/// `dm` frame steps.
const ASK: &str = "ask";
const PIN: &str = "pin";
const OFFER: &str = "offer";
const DONE: &str = "done";

/// A person holds at most this many live DM spaces with one other person,
/// so the members of a shared space can't fill a disk (talk-dms.md §2).
pub(crate) const MAX_DM_SPACES: usize = 4;

/// What an offer can carry: half of a frame, as the bundle grows as JSON.
pub(crate) const MAX_PENDING_LOG: usize = irori_meshseam::MAX_MESSAGE / 2;

/// How many hand-off steps wait at once. Past it a frame is dropped: the
/// next tick asks again. An offer can be a whole frame, so this is what
/// bounds the memory a member of S can make this take.
const MAX_JOBS: usize = 32;

/// One step for the DM thread.
enum Job {
    /// Run the hand-off with `peer`, if anything is pending with them.
    Try { via: String, peer: LivePeer },
    Frame {
        via: String,
        peer: LivePeer,
        frame: DmFrame,
    },
}

/// The DM thread's work, and the lock that keeps one founding at a time.
pub(crate) struct Dms {
    jobs: Arc<Queue<Job>>,
    /// The (via, device) tries already queued: a tick comes per connection,
    /// so a device on two epochs ticks twice.
    tries: Mutex<HashSet<(String, String)>>,
    /// Two posts at once must not found two logs.
    found: Mutex<()>,
    /// Offers sent, for tests: an offer carries the space's secret.
    offers: std::sync::atomic::AtomicUsize,
}

impl Default for Dms {
    fn default() -> Dms {
        Dms {
            jobs: Arc::new(Queue::new(MAX_JOBS)),
            tries: Mutex::new(HashSet::new()),
            found: Mutex::new(()),
            offers: Default::default(),
        }
    }
}

impl Dms {
    fn queue_try(&self, via: &str, peer: &LivePeer) {
        let key = (via.to_string(), peer.device_id.clone());
        if !lock(&self.tries).insert(key.clone()) {
            return;
        }
        let job = Job::Try {
            via: via.to_string(),
            peer: peer.clone(),
        };
        if !self.jobs.push(job) {
            lock(&self.tries).remove(&key);
        }
    }

    /// Ends the DM thread.
    pub(crate) fn close(&self) {
        self.jobs.close();
    }
}

/// The mesh's hooks, on every space's connections.
pub(crate) struct DmBridge(pub(crate) std::sync::Weak<Services>);

impl DmBridge {
    /// Whether anything can be pending with `peer` on `via`. Runs on the
    /// connection's task, so it reads the cache and nothing else.
    fn on(&self, via: &str, peer: &LivePeer) {
        let Some(s) = self.0.upgrade() else { return };
        if !s.is_dm_space(via) && !s.dm_spaces.of(&peer.person_id).is_empty() {
            s.dms.queue_try(via, peer);
        }
    }
}

impl DmHooks for DmBridge {
    fn hello(&self, via: &str, peer: &LivePeer) {
        self.on(via, peer);
        // A device too old for a hand-off: the window says so (§6). It
        // hears no other news of one, since no hand-off runs.
        if peer.dms < DMS_PROTOCOL
            && let Some(s) = self.0.upgrade()
            && !s.is_dm_space(via)
            && !s.dm_spaces.of(&peer.person_id).is_empty()
        {
            s.hub.publish(crate::events::CoreEvent::Talk(
                crate::events::TalkChange::all(),
            ));
        }
    }

    fn tick(&self, via: &str, peer: &LivePeer) {
        self.on(via, peer);
    }

    fn frame(&self, via: &str, peer: &LivePeer, frame: DmFrame) {
        let Some(s) = self.0.upgrade() else { return };
        // A hand-off rides on a shared space, never on a hidden one.
        if !s.is_dm_space(via) {
            s.dms.jobs.push(Job::Frame {
                via: via.to_string(),
                peer: peer.clone(),
                frame,
            });
        }
    }
}

/// What a pending DM waits on, given the other person's `peers` connected
/// now: an update only when every one of them is too old, since a hand-off
/// needs just one, and nobody connected says nothing of their build.
fn waiting_on(peers: &[LivePeer]) -> &'static str {
    if !peers.is_empty() && peers.iter().all(|p| p.dms < DMS_PROTOCOL) {
        tools::talk::WAITING_FOR_UPDATE
    } else {
        tools::talk::WAITING_FOR_THEM
    }
}

fn person_key(id: &str) -> String {
    format!("{PERSON_ACTOR_PREFIX}{id}")
}

/// §4's rules on hidden space `d` itself: `me` is this person's id.
fn dm_sound(me: &str, d: &DmSpace, m: &Membership) -> bool {
    // 1. A marker, whose creator is board.created's.
    m.created
        && m.creator == d.creator.person_id
        && m.pins.get(&m.creator) == Some(&d.creator)
        // 2. Nobody ever on it but the two of them.
        && d.with != me
        && m.members
            .iter()
            .all(|p| p.person_id == me || p.person_id == d.with)
}

/// §4's rules on one row of hidden space `d`, as written in the log (keys
/// in board form): the DM it may fold into, or `None` for a row that folds
/// to nothing. `me` is this person's id.
pub(crate) fn dm_conv_of(
    me: &str,
    d: &DmSpace,
    m: &Membership,
    actor: &str,
    kind: &str,
    row: &Value,
) -> Option<String> {
    if !dm_sound(me, d, m) {
        return None;
    }
    // 3. The pair's own conversation.
    let (mine, theirs) = (person_key(me), person_key(&d.with));
    let conv = coord::talk::pair_dm_id(&mine, &theirs);
    if row.get("convId").and_then(Value::as_str) != Some(conv.as_str()) {
        return None;
    }
    // 4. Written by one of them, under their own person key.
    let pair = |k: &str| k == mine || k == theirs;
    if !pair(actor) || row.get("by").and_then(Value::as_str) != Some(actor) {
        return None;
    }
    // 5. A post or a huddle's line, title or canvas item (cards #656, #1043, #1051), or a join of one of them.
    let ok = match kind {
        kinds::TALK_POST
        | kinds::CALL_SEGMENT
        | kinds::CALL_TITLE
        | kinds::CALL_ASSIST
        | kinds::CALL_ITEM
        | kinds::CALL_ITEM_SET => true,
        kinds::TALK_MEMBER => {
            let key = row.pointer("/member/key").and_then(Value::as_str);
            row.get("op").and_then(Value::as_str) == Some(coord::talk::JOIN)
                && row.pointer("/member/kind").and_then(Value::as_str) == Some(coord::talk::PERSON)
                && key.is_some_and(pair)
        }
        _ => false,
    };
    ok.then_some(conv)
}

/// The founder's checks on a `pin` (talk-dms.md §2): the proof proves the
/// pin's key for `with`, the pin is rooted at this connection's device, and
/// the cert verifies under the proven key and names this connection's key,
/// so a replay of the person's real pin and chain fails. `known` is the
/// newest epoch S knows for them: an older key can still be proven.
pub(crate) fn check_pin(
    with: &str,
    peer: &LivePeer,
    pin: &MemberPin,
    proof: &KeyProof,
    cert: &MemberDeviceAdded,
    known: i64,
) -> std::result::Result<(), String> {
    if !valid_pin(pin) || pin.person_id != with || peer.person_id != with {
        return Err("the pin isn't this person's".into());
    }
    if verify_key_proof(with, proof) != Some((pin.person_epoch, pin.person_pub.clone())) {
        return Err("the pin's key doesn't prove back to this person".into());
    }
    if pin.person_epoch < known {
        return Err("the pin's key is older than this person's newest".into());
    }
    if pin.root_device_id != peer.device_id {
        return Err("the pin isn't rooted at this device".into());
    }
    let c = &cert.cert;
    if c.person_id != with
        || c.person_epoch != pin.person_epoch
        || c.device_id != peer.device_id
        || c.device_pub != peer.device_pub
    {
        return Err("the cert doesn't name this connection".into());
    }
    verify_person(&pin.person_pub, &cert.person_sig, c)
        .map_err(|_| "the cert doesn't verify".into())
}

/// The joiner's checks on an offer's sender, after the bundle's own
/// (`bundle::check_offer`): the bundle's creator proves back to its person
/// id, S counts the sender as that person, and the proven key's own log
/// certifies the very key this connection proved. S alone wouldn't do: an
/// owner of S can list a device of theirs under anyone's name. `known` is
/// the newest epoch S knows for the creator.
pub(crate) fn check_sender(
    peer: &LivePeer,
    fold: &OfferFold,
    proof: &KeyProof,
    known: i64,
) -> std::result::Result<(), String> {
    let c = &fold.creator;
    if verify_key_proof(&c.person_id, proof) != Some((c.person_epoch, c.person_pub.clone())) {
        return Err("the creator's key doesn't prove back to their person id".into());
    }
    if peer.person_id != c.person_id {
        return Err("the sender isn't the space's creator".into());
    }
    if fold.verifier.epoch(&c.person_id) < known {
        return Err("the creator's key is older than this person's newest".into());
    }
    let certified = fold.verifier.person_of(&peer.device_id).as_deref() == Some(&c.person_id)
        && fold
            .verifier
            .cert(&peer.device_id)
            .is_some_and(|d| d.device_pub == peer.device_pub);
    if !certified {
        return Err("the space doesn't certify the sender".into());
    }
    Ok(())
}

fn decode<T: irori_identity::objects::GoDecode>(body: &Value, field: &str) -> Option<T> {
    irori_identity::decode::<T>(body.get(field)?.to_string().as_bytes())
}

/// As many of this device's rows, from seq 1, as a `done` can carry. A
/// store refuses a row past a gap, so a prefix is all that ever helps.
fn done_rows(evs: &[Event]) -> Vec<WireRow> {
    let mut size = 0;
    let mut out = Vec::new();
    for ev in evs {
        // Base64 of the payload and the signature, and the row's fields.
        size += (ev.payload.len() + ev.sig.len()) * 4 / 3 + 256;
        if size > MAX_PENDING_LOG && !out.is_empty() {
            break;
        }
        out.push(WireRow::of(ev));
    }
    out
}

impl Services {
    /// Takes the DM thread's work until the Core stops.
    pub(crate) fn run_dm_jobs(self: &Arc<Self>) {
        while let Some(job) = self.dms.jobs.recv() {
            match job {
                Job::Try { via, peer } => {
                    lock(&self.dms.tries).remove(&(via.clone(), peer.device_id.clone()));
                    self.dm_try(&via, &peer);
                }
                Job::Frame { via, peer, frame } => self.dm_frame(&via, &peer, frame),
            }
        }
    }

    /// `b` is a space both this person and `other` are active in, and not
    /// a hidden one: the only kind of space a hand-off rides on.
    fn dm_rides_on(&self, b: &BoardLog, other: &str) -> bool {
        let m = b.verifier.membership();
        !self.is_dm_space(&b.id)
            && !self.board_leaving(b)
            && m.active(&self.person_id())
            && m.active(other)
    }

    /// Every space this person shares with `other`, hidden ones aside.
    pub(crate) fn dm_shared(&self, other: &str) -> Vec<Arc<BoardLog>> {
        self.shown_boards()
            .into_iter()
            .filter(|b| self.dm_rides_on(b, other))
            .collect()
    }

    /// Space `via` when a hand-off with `other` may ride on it.
    fn dm_via(&self, via: &str, other: &str) -> std::result::Result<Arc<BoardLog>, String> {
        match self.board(via) {
            Ok(b) if self.dm_rides_on(&b, other) => Ok(b),
            _ => Err("this connection's space isn't one the two of you share".into()),
        }
    }

    /// Whether `other` has a cert that counts in `b`: the hand-off is over.
    fn dm_counted(b: &BoardLog, other: &str) -> bool {
        b.verifier.membership().active(other)
            && b.verifier
                .devices(other)
                .iter()
                .any(|d| !d.revoked && !d.cert.device_id.is_empty())
    }

    /// `other`'s name as `b` holds it, for the add.
    fn dm_name(b: &BoardLog, other: &str) -> String {
        b.verifier
            .profile(other)
            .or_else(|| {
                b.verifier
                    .membership()
                    .person(other)
                    .map(|p| p.name.clone())
            })
            .filter(|n| irori_identity::membership::valid_board_name(n))
            .unwrap_or_else(|| "someone".into())
    }

    /// The live DM spaces with `other`, lowest id first.
    fn dm_live(&self, other: &str) -> Vec<(Arc<BoardLog>, DmSpace)> {
        self.dm_spaces
            .of(other)
            .into_iter()
            .filter_map(|(id, d)| Some((self.board(&id).ok()?, d)))
            .collect()
    }

    /// The space a DM with `other` is written to now (§1): the lowest whose
    /// other person has a counted cert, otherwise this person's own lowest
    /// pending log. A log `other` left isn't one, nor one that broke §4's
    /// rules: a later DM offers a new space, which is how that person hears
    /// from this one again.
    pub(crate) fn dm_writer(&self, other: &str) -> Option<String> {
        let (me, dev) = (self.person_id(), self.device.id.clone());
        let can: Vec<(Arc<BoardLog>, DmSpace)> = self
            .dm_live(other)
            .into_iter()
            .filter(|(b, d)| {
                let m = b.verifier.membership();
                dm_sound(&me, d, &m)
                    && m.active(&me)
                    && b.verifier.cert(&dev).is_some()
                    && b.verifier.cut(&dev).is_none()
            })
            .collect();
        if let Some((b, _)) = can.iter().find(|(b, _)| Self::dm_counted(b, other)) {
            return Some(b.id.clone());
        }
        can.iter()
            .find(|(b, d)| {
                let m = b.verifier.membership();
                d.creator.person_id == me && m.person(other).is_none_or(|_| m.active(other))
            })
            .map(|(b, _)| b.id.clone())
    }

    /// The space a DM with `other` (`person:<id>`) goes in: the one in use,
    /// or a new one, its hand-off started. Refused with no space shared.
    pub(crate) fn dm_for(self: &Arc<Self>, other: &str) -> std::result::Result<String, String> {
        let other = other
            .strip_prefix(PERSON_ACTOR_PREFIX)
            .filter(|o| !o.is_empty())
            .ok_or("a DM's other side is a person")?;
        if other == self.person_id() {
            return Err("a DM needs two different people".into());
        }
        let _one = lock(&self.dms.found);
        if let Some(id) = self.dm_writer(other) {
            return Ok(id);
        }
        let shared = self.dm_shared(other);
        if shared.is_empty() {
            return Err("you don't share a space with this person".into());
        }
        let id = self.found_dm_space(other).map_err(|e| e.to_string())?;
        // They may be connected already, with their hello long gone.
        for b in &shared {
            if let Some(m) = self.mesh.board(&b.id) {
                for p in m.live_peers().iter().filter(|p| p.person_id == other) {
                    self.dms.queue_try(&b.id, p);
                }
            }
        }
        Ok(id)
    }

    /// Everyone this person can DM (§6): whoever is active in a space they
    /// are too, hidden ones aside, once each, by name.
    pub(crate) fn dm_people(&self) -> Vec<BoardMember> {
        let me = self.person_id();
        let mut out: BTreeMap<String, BoardMember> = BTreeMap::new();
        for b in self.shown_boards() {
            if self.is_dm_space(&b.id) || !b.verifier.membership().active(&me) {
                continue;
            }
            let m = b.verifier.membership();
            for p in m.members.iter().filter(|p| !p.person_id.is_empty()) {
                let id = &p.person_id;
                if *id == me || !m.active(id) {
                    continue;
                }
                out.entry(person_key(id)).or_insert_with(|| BoardMember {
                    key: person_key(id),
                    name: Self::dm_name(&b, id),
                    board_id: b.id.clone(),
                    color: b.verifier.profile_color(id),
                    avatar: b.verifier.profile_avatar(id),
                });
            }
        }
        let mut people: Vec<BoardMember> = out.into_values().collect();
        people.sort_by_cached_key(|p| (p.name.to_lowercase(), p.key.clone()));
        people
    }

    /// What the DM with `other` (`person:<id>`) is waiting on (§6). Only the
    /// log it is written to counts: a pending one nobody writes to waits on
    /// nothing. A device of theirs too old for a hand-off says so only while
    /// no newer one is here.
    pub(crate) fn dm_waiting(&self, other: &str) -> &'static str {
        let Some(other) = other.strip_prefix(PERSON_ACTOR_PREFIX) else {
            return "";
        };
        let Some(id) = self.dm_writer(other) else {
            return "";
        };
        let pending = self
            .dm_live(other)
            .iter()
            .any(|(b, d)| b.id == id && self.dm_pending(b, d));
        if !pending {
            return "";
        }
        let peers: Vec<LivePeer> = self
            .dm_shared(other)
            .iter()
            .filter_map(|b| self.mesh.board(&b.id))
            .flat_map(|m| m.live_peers())
            .filter(|p| p.person_id == other)
            .collect();
        waiting_on(&peers)
    }

    /// Leaves every DM space with `other` (`person:<id>`; §7). `leave_board`
    /// and not Talk's leave: a hidden log takes no `leave` row (§4), and
    /// the `left` row is what stops the other side writing to it. A later
    /// post from either side founds a new space.
    pub(crate) fn dm_leave(self: &Arc<Self>, other: &str) -> Result<()> {
        let other = other.strip_prefix(PERSON_ACTOR_PREFIX).unwrap_or(other);
        let mut out = Ok(());
        for (b, _) in self.dm_live(other) {
            if !self.board_leaving(&b)
                && let Err(e) = self.leave_board(&b.id)
            {
                logf(format_args!("board {}: leave a DM: {e}", b.id));
                out = Err(e);
            }
        }
        out
    }

    /// The other person of person DM `conv`, by the markers.
    pub(crate) fn dm_route(&self, conv: &str) -> Option<String> {
        let me = self.person_actor();
        self.dm_spaces
            .people()
            .iter()
            .map(|p| person_key(p))
            .find(|k| coord::talk::pair_dm_id(&me, k) == conv)
    }

    /// Makes a DM space with `other` and returns its id (§1): the marker
    /// first, so no row folds without it, then what `create_board` does,
    /// but with a tag nobody can type and no tag check. A failure after the
    /// marker forgets the space.
    fn found_dm_space(self: &Arc<Self>, other: &str) -> Result<String> {
        let id = new_board_id()?;
        let pin = self.ident.board_pin();
        // A rotation from before v0.5 can't be proven (§2): no offer would
        // ever be taken, so no space is made.
        if self
            .chain
            .key_proof(pin.person_epoch, &pin.person_pub)
            .is_none()
        {
            return Err(Error::other("this person's key can't be proven"));
        }
        self.write_dm_marker(&id, &pin, other)?;
        let made = (|| {
            let b = self.open_board(&id, true, Some(&pin))?;
            let row = self.ident.board_created_row(
                &id,
                DM_SPACE_NAME,
                DM_SPACE_TAG,
                &pin,
                (self.now)(),
            )?;
            b.store
                .append(&person_key(&pin.person_id), kinds::BOARD_CREATED, &row)?;
            if !b.verifier.membership().created {
                return Err(Error::other(
                    "core: the DM space's board.created did not count",
                ));
            }
            // A secret that fails here is made again when the space next
            // opens or a row moves its epoch.
            if let Err(e) = self.rotate_secret(&b) {
                logf(format_args!("board {id}: secret: {e}"));
            }
            Ok(())
        })();
        if let Err(e) = made {
            self.dm_forget(&id);
            return Err(e);
        }
        self.boards.moved();
        Ok(id)
    }

    /// Takes DM space `id` off every device of this person: what a failure
    /// after its marker does. The newer board.forgot covers the marker
    /// everywhere, and a later offer's newer marker lifts it.
    fn dm_forget(self: &Arc<Self>, id: &str) {
        let row = BoardForgotRow {
            board_id: id.into(),
            at: (self.now)(),
        };
        if let Err(e) = self.store.append("irori", kinds::BOARD_FORGOT, &row) {
            logf(format_args!("board {id}: forget a failed DM space: {e}"));
        }
        self.reconcile_boards();
        // One that never opened has no log for the reconcile to forget.
        if let Err(e) = self.remove_board(id) {
            logf(format_args!("board {id}: forget a failed DM space: {e}"));
        }
    }

    /// Sends a hand-off step to one device. The send is on the mesh's
    /// runtime: nothing here waits on it, and a lost frame is sent again.
    fn dm_send(&self, via: &str, to: &str, id: &str, step: &str, body: Value) {
        let Some(m) = self.mesh.board(via) else {
            return;
        };
        let (to, step) = (to.to_string(), step.to_string());
        let frame = DmFrame {
            board_id: id.to_string(),
            step: step.clone(),
            body,
        };
        self.pairing.handle.spawn(async move {
            if let Err(e) = m.send_to(&to, frame).await {
                logf(format_args!("dm {step}: {e}"));
            }
        });
    }

    /// Runs the hand-off of every space pending with `peer` (§2). Any
    /// device of the founder that holds the secret may. It asks; after the
    /// add it asks only the pin's root, and the offer waits for that device
    /// to prove its key again (`dm_on_pin`): a device id is only a claim,
    /// and another space's owner can certify a device of theirs under it.
    fn dm_try(self: &Arc<Self>, via: &str, peer: &LivePeer) {
        if peer.dms < DMS_PROTOCOL || self.dm_via(via, &peer.person_id).is_err() {
            return;
        }
        let me = self.person_id();
        for (b, d) in self.dm_live(&peer.person_id) {
            let m = b.verifier.membership();
            if d.creator.person_id != me
                || !m.created
                || b.secrets.epoch() < 1
                || self.board_leaving(&b)
                || Self::dm_counted(&b, &d.with)
            {
                continue;
            }
            match m.person(&d.with) {
                None => self.dm_send(via, &peer.device_id, &b.id, ASK, json!({})),
                Some(p) if m.active(&d.with) && p.pin.root_device_id == peer.device_id => {
                    self.dm_send(via, &peer.device_id, &b.id, ASK, json!({}))
                }
                // On another device's pin, or gone from it: nothing to do.
                Some(_) => {}
            }
        }
    }

    fn dm_send_offer(&self, via: &str, peer: &LivePeer, d: &DmSpace, offer: Bundle) {
        let id = offer.board_id.clone();
        let c = &d.creator;
        let Some(proof) = self.chain.key_proof(c.person_epoch, &c.person_pub) else {
            logf(format_args!("board {id}: dm offer: no proof of this key"));
            return;
        };
        let body = json!({"bundle": offer, "proof": proof});
        self.dms
            .offers
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.dm_send(via, &peer.device_id, &id, OFFER, body);
    }

    /// One `dm` frame, on the DM thread. A refused step is logged and
    /// answered with nothing.
    pub(crate) fn dm_frame(self: &Arc<Self>, via: &str, peer: &LivePeer, frame: DmFrame) {
        let id = frame.board_id.as_str();
        if !valid_board_id(id) {
            return;
        }
        let r = self.dm_via(via, &peer.person_id).and_then(|s| {
            match frame.step.as_str() {
                ASK => self.dm_on_ask(via, peer, id),
                PIN => self.dm_on_pin(&s, peer, id, &frame.body),
                OFFER => self.dm_on_offer(&s, peer, id, &frame.body),
                DONE => self.dm_on_done(peer, id, &frame.body),
                // A step of a newer build.
                _ => Ok(()),
            }
        });
        if let Err(e) = r {
            logf(format_args!("board {id}: dm {}: {e}", frame.step));
        }
    }

    /// Whether this person may hold space `id` with `other` (§2): an id it
    /// already holds must be this pair's, and at most [`MAX_DM_SPACES`] are
    /// live with one person.
    fn dm_may_hold(&self, id: &str, other: &str) -> std::result::Result<bool, String> {
        let held = self.dm_spaces.get(id);
        let live = self.board(id).is_ok();
        match &held {
            Some(d) if d.with != other => return Err("that id is another DM's".into()),
            Some(d) if d.creator.person_id != other => {
                return Err("that DM's space isn't the other side's to offer".into());
            }
            None if live => return Err("that id is a space that isn't a DM".into()),
            _ => {}
        }
        // A space `other` left doesn't count: it stays here with what's in
        // it, but they can't write to it, and counting it would shut out a
        // person who left four times.
        let open = self
            .dm_live(other)
            .iter()
            .filter(|(b, _)| {
                let m = b.verifier.membership();
                !m.person(other).is_some_and(|p| p.state == MEMBER_LEFT)
            })
            .count();
        if !live && open >= MAX_DM_SPACES {
            return Err(format!(
                "already {MAX_DM_SPACES} DM spaces with this person"
            ));
        }
        Ok(live)
    }

    /// The other side, asked: answers with this device's pin, the proof of
    /// its key, and a cert for the connection's key. A person with a
    /// rotation from before v0.5 can't prove theirs and gets no DM space.
    fn dm_on_ask(&self, via: &str, peer: &LivePeer, id: &str) -> std::result::Result<(), String> {
        self.dm_may_hold(id, &peer.person_id)?;
        let pin = self.ident.board_pin();
        let proof = self
            .chain
            .key_proof(pin.person_epoch, &pin.person_pub)
            .ok_or("this person's key can't be proven")?;
        let cert = self
            .ident
            .member_cert_for(
                pin.person_epoch,
                &self.device.id,
                &hex::encode(self.ident.device_pub()),
                &self.dm_role(),
                (self.now)(),
            )
            .map_err(|e| e.to_string())?;
        let body = json!({"pin": pin, "proof": proof, "cert": cert});
        self.dm_send(via, &peer.device_id, id, PIN, body);
        Ok(())
    }

    /// This device's role, from its own chain.
    fn dm_role(&self) -> String {
        self.chain
            .cert(&self.device.id)
            .map(|c| c.role)
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| ROLE_DESKTOP.into())
    }

    /// The founder, told a pin: checks it, writes `board.member.added` and
    /// sends the offer. After the add, only a pin the added pin's own root
    /// proves again, under the same key, gets the offer, rebuilt without
    /// writing; a device id alone proves nothing. Any other pin changes
    /// nothing.
    fn dm_on_pin(
        self: &Arc<Self>,
        s: &BoardLog,
        peer: &LivePeer,
        id: &str,
        body: &Value,
    ) -> std::result::Result<(), String> {
        let d = self.dm_spaces.get(id).ok_or("no such DM space here")?;
        if d.creator.person_id != self.person_id() || d.with != peer.person_id {
            return Err("this DM space isn't with this person".into());
        }
        let b = self.board(id).map_err(|e| e.to_string())?;
        let m = b.verifier.membership();
        let added = m.person(&d.with).map(|p| p.pin.clone());
        if added.is_some() && (!m.active(&d.with) || Self::dm_counted(&b, &d.with)) {
            return Ok(());
        }
        let bad = "a pin frame that doesn't read";
        let pin: MemberPin = decode(body, "pin").ok_or(bad)?;
        let proof: KeyProof = decode(body, "proof").ok_or(bad)?;
        let cert: MemberDeviceAdded = decode(body, "cert").ok_or(bad)?;
        check_pin(
            &d.with,
            peer,
            &pin,
            &proof,
            &cert,
            s.verifier.epoch(&d.with),
        )?;
        let boards = CoreBoards(Arc::downgrade(self));
        if let Some(was) = added {
            // The retry: the very pin the add wrote, proven again.
            if was.root_device_id != pin.root_device_id
                || was.person_epoch != pin.person_epoch
                || was.person_pub != pin.person_pub
            {
                return Ok(());
            }
            let offer = bundle::rebuild(&boards, &self.ident, id, &d.with)?;
            self.dm_send_offer(&s.id, peer, &d, offer);
            return Ok(());
        }
        let offer = bundle::admit(&boards, &self.ident, id, &pin, &Self::dm_name(s, &d.with))?;
        self.dm_send_offer(&s.id, peer, &d, offer);
        Ok(())
    }

    /// The other side, offered the space: every check runs before anything
    /// touches disk (`check_offer` in a throwaway verifier), then, in this
    /// order: marker, `join` (open, secret, rows, cert and profile),
    /// `keep_current`, `done`. A failure after the marker forgets the
    /// space, unless this offer found it already here.
    fn dm_on_offer(
        self: &Arc<Self>,
        s: &BoardLog,
        peer: &LivePeer,
        id: &str,
        body: &Value,
    ) -> std::result::Result<(), String> {
        // Before the fold, so a refused sender costs nothing.
        let held = self.dm_may_hold(id, &peer.person_id)?;
        let offer: Bundle = body
            .get("bundle")
            .and_then(|b| serde_json::from_value(b.clone()).ok())
            .ok_or("an offer that doesn't read")?;
        let proof: KeyProof = decode(body, "proof").ok_or("an offer with no proof")?;
        let fold = bundle::check_offer(&self.ident, id, &offer)?;
        check_sender(
            peer,
            &fold,
            &proof,
            s.verifier.epoch(&fold.creator.person_id),
        )?;
        let creator = fold.creator.clone();
        if self.dm_spaces.get(id).is_some_and(|d| d.creator != creator) {
            return Err("that DM space was founded under another key".into());
        }
        if !held {
            self.write_dm_marker(id, &creator, &creator.person_id)
                .map_err(|e| e.to_string())?;
        }
        let rows = self.dm_join(id, &creator, &offer);
        let rows = match rows {
            Ok(rows) => rows,
            Err(e) => {
                if !held {
                    self.dm_forget(id);
                }
                return Err(e);
            }
        };
        self.dm_send(
            &s.id,
            &peer.device_id,
            id,
            DONE,
            json!({"rows": done_rows(&rows)}),
        );
        self.dm_connect(id);
        self.boards.moved();
        Ok(())
    }

    /// `join` and `keep_current`: this device's rows, for `done`.
    fn dm_join(
        self: &Arc<Self>,
        id: &str,
        creator: &MemberPin,
        offer: &Bundle,
    ) -> std::result::Result<Vec<Event>, String> {
        let boards = CoreBoards(Arc::downgrade(self));
        let rows = bundle::join(
            &boards,
            &self.ident,
            id,
            &creator.person_id,
            &self.dm_role(),
            offer,
        )?;
        let b = self.board(id).map_err(|e| e.to_string())?;
        self.keep_current(&b).map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// The space's membership moved outside a pull: its mesh checks who
    /// counts and the address book feeds the new member's devices.
    fn dm_connect(&self, id: &str) {
        self.recheck_board_mesh(id);
        self.mesh.refeed();
    }

    /// The founder, told `done`: takes the joiner's rows, which are all
    /// its own. The space stops being pending when its cert counts.
    fn dm_on_done(
        &self,
        peer: &LivePeer,
        id: &str,
        body: &Value,
    ) -> std::result::Result<(), String> {
        let d = self.dm_spaces.get(id).ok_or("no such DM space here")?;
        if d.creator.person_id != self.person_id() || d.with != peer.person_id {
            return Err("this DM space isn't with this person".into());
        }
        let b = self.board(id).map_err(|e| e.to_string())?;
        let m = b.verifier.membership();
        if !m
            .person(&d.with)
            .is_some_and(|p| m.active(&d.with) && p.pin.root_device_id == peer.device_id)
        {
            return Err("this device isn't the pin's root".into());
        }
        let rows: Vec<WireRow> = body
            .get("rows")
            .and_then(|r| serde_json::from_value(r.clone()).ok())
            .ok_or("a done that doesn't read")?;
        let evs: Vec<Event> = rows
            .iter()
            .filter(|r| r.d == peer.device_id)
            .map(WireRow::event)
            .collect();
        if let Err(e) = mesh::boards::insert_board_rows(&b.store, evs) {
            logf(format_args!("board {id}: dm done: {e}"));
        }
        self.dm_connect(id);
        self.boards.moved();
        Ok(())
    }

    /// Hidden space `b`'s rows fold again: a late row can change what they
    /// fold to (§4), its board.created or a third member, which ends the
    /// DM. Only a space that broke its rules is dropped first: folding
    /// again adds what is missing, so a post just written stays in view.
    pub(crate) fn dm_refold(&self, b: &BoardLog) {
        let m = b.verifier.membership();
        let me = self.person_id();
        if !self
            .dm_spaces
            .get(&b.id)
            .is_some_and(|d| dm_sound(&me, &d, &m))
        {
            self.talk.drop_space(&b.id);
        }
        self.fold_space_talk(b);
        self.hub.publish(crate::events::CoreEvent::Talk(
            crate::events::TalkChange::all(),
        ));
    }

    /// Whether `b`'s other person has no counted cert yet: it is a hidden
    /// space still on its way to them.
    pub(crate) fn dm_pending(&self, b: &BoardLog, d: &DmSpace) -> bool {
        !Self::dm_counted(b, &d.with)
    }

    /// The bytes an offer of `b` would carry now.
    pub(crate) fn dm_offer_size(b: &BoardLog) -> std::result::Result<usize, String> {
        let rows: Vec<WireRow> = bundle::all_rows(&b.store)?
            .iter()
            .map(WireRow::of)
            .collect();
        serde_json::to_vec(&rows)
            .map(|v| v.len())
            .map_err(|e| e.to_string())
    }
}

impl super::Core {
    /// Every DM space this device holds with person `other`, lowest id
    /// first (tests).
    #[doc(hidden)]
    pub fn dm_spaces_with(&self, other: &str) -> Vec<String> {
        self.s
            .dm_live(other)
            .into_iter()
            .map(|(b, _)| b.id.clone())
            .collect()
    }

    /// Whether DM space `id` is still on its way to its other person (the
    /// window's "Waiting for Bob's Irori").
    pub fn dm_pending(&self, id: &str) -> bool {
        let (Some(d), Ok(b)) = (self.s.dm_spaces.get(id), self.s.board(id)) else {
            return false;
        };
        self.s.dm_pending(&b, &d)
    }

    /// Runs a `dm` frame as if `peer` sent it on `via` (tests: what a
    /// hostile device could send).
    #[doc(hidden)]
    pub fn dm_take_frame(&self, via: &str, peer: &LivePeer, frame: DmFrame) {
        self.s.dm_frame(via, peer, frame);
    }

    /// How many DM offers this device has sent (tests: each carries a
    /// space's secret).
    #[doc(hidden)]
    pub fn dm_offers_sent(&self) -> usize {
        self.s.dms.offers.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The rows of kind `kind` in space `id` (tests).
    #[doc(hidden)]
    pub fn board_rows(&self, id: &str, kind: &str) -> Vec<Event> {
        let mut out = Vec::new();
        if let Ok(b) = self.s.board(id) {
            let f = Filter {
                kinds: vec![kind.to_string()],
                ..Default::default()
            };
            let _ = each_row(&b.store, f, |ev| out.push(ev.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_identity::membership::BoardPerson;

    const ME: &str = "me";
    const THEM: &str = "them";

    /// §6: one device new enough to take the hand-off means it's only a
    /// matter of waiting; nobody connected says nothing about their build.
    #[test]
    fn a_dm_waits_on_an_update_only_when_every_device_is_too_old() {
        use tools::talk::{WAITING_FOR_THEM, WAITING_FOR_UPDATE};
        let peer = |dms| LivePeer {
            dms,
            ..LivePeer::default()
        };
        assert_eq!(waiting_on(&[]), WAITING_FOR_THEM);
        assert_eq!(waiting_on(&[peer(0)]), WAITING_FOR_UPDATE);
        assert_eq!(waiting_on(&[peer(0), peer(0)]), WAITING_FOR_UPDATE);
        assert_eq!(waiting_on(&[peer(DMS_PROTOCOL)]), WAITING_FOR_THEM);
        assert_eq!(waiting_on(&[peer(0), peer(DMS_PROTOCOL)]), WAITING_FOR_THEM);
    }

    fn pin(person: &str) -> MemberPin {
        MemberPin {
            person_id: person.into(),
            person_pub: "ab".repeat(32),
            root_device_id: format!("dev-{person}"),
            person_epoch: 1,
        }
    }

    fn marker() -> DmSpace {
        DmSpace {
            creator: pin(THEM),
            with: THEM.into(),
        }
    }

    fn member(person: &str) -> BoardPerson {
        BoardPerson {
            person_id: person.into(),
            pin: pin(person),
            ..BoardPerson::default()
        }
    }

    /// A space founded by `them`, with this person on it.
    fn ms() -> Membership {
        Membership {
            created: true,
            creator: THEM.into(),
            members: vec![member(THEM), member(ME)],
            pins: [(THEM.to_string(), pin(THEM))].into(),
            ..Membership::default()
        }
    }

    fn conv() -> String {
        coord::talk::pair_dm_id("person:me", "person:them")
    }

    fn post(by: &str) -> Value {
        json!({"postId": "p1", "convId": conv(), "by": by, "text": "hi"})
    }

    fn stamp(m: &Membership, actor: &str, kind: &str, row: &Value) -> Option<String> {
        dm_conv_of(ME, &marker(), m, actor, kind, row)
    }

    #[test]
    fn a_post_of_either_person_folds_into_their_dm() {
        for who in ["person:me", "person:them"] {
            assert_eq!(
                stamp(&ms(), who, kinds::TALK_POST, &post(who)),
                Some(conv())
            );
        }
    }

    /// Rule 1: the marker's creator must be board.created's, with the same
    /// pin, and a log with no counted board.created folds nothing.
    #[test]
    fn rule_1_the_markers_creator_is_the_logs() {
        let row = post("person:them");
        let mut m = ms();
        m.created = false;
        assert_eq!(stamp(&m, "person:them", kinds::TALK_POST, &row), None);
        let mut m = ms();
        m.creator = "someone".into();
        assert_eq!(stamp(&m, "person:them", kinds::TALK_POST, &row), None);
        let mut m = ms();
        m.pins.get_mut(THEM).unwrap().person_pub = "cd".repeat(32);
        assert_eq!(stamp(&m, "person:them", kinds::TALK_POST, &row), None);
    }

    /// Rule 2: anyone else, in any state, ends the DM.
    #[test]
    fn rule_2_a_third_member_in_any_state_ends_it() {
        let row = post("person:them");
        let mut m = ms();
        m.members.push(member("third"));
        assert_eq!(stamp(&m, "person:them", kinds::TALK_POST, &row), None);
        m.members[2].state = irori_identity::membership::MEMBER_LEFT.into();
        assert_eq!(stamp(&m, "person:them", kinds::TALK_POST, &row), None);
        // The founder's side, before the other person is added.
        let m = Membership {
            members: vec![member(THEM)],
            ..ms()
        };
        assert_eq!(
            stamp(&m, "person:them", kinds::TALK_POST, &row),
            Some(conv())
        );
    }

    /// Rule 3: only the pair's own conversation.
    #[test]
    fn rule_3_only_the_pairs_conversation() {
        let mut row = post("person:them");
        row["convId"] = coord::talk::pair_dm_id("person:me", "person:other").into();
        assert_eq!(stamp(&ms(), "person:them", kinds::TALK_POST, &row), None);
        row["convId"] = "ch_general".into();
        assert_eq!(stamp(&ms(), "person:them", kinds::TALK_POST, &row), None);
        row.as_object_mut().unwrap().remove("convId");
        assert_eq!(stamp(&ms(), "person:them", kinds::TALK_POST, &row), None);
    }

    /// Rule 4: the actor is one of the two, as a person, and says so.
    #[test]
    fn rule_4_the_actor_is_one_of_the_pair() {
        for actor in ["person:other", "agent:c1", "irori", "user:dev-a"] {
            assert_eq!(stamp(&ms(), actor, kinds::TALK_POST, &post(actor)), None);
        }
        assert_eq!(
            stamp(&ms(), "person:them", kinds::TALK_POST, &post("person:me")),
            None
        );
        let mut no_by = post("person:them");
        no_by.as_object_mut().unwrap().remove("by");
        assert_eq!(stamp(&ms(), "person:them", kinds::TALK_POST, &no_by), None);
    }

    /// Rule 5: a post, or a join of one of the two; nothing else.
    #[test]
    fn rule_5_only_posts_and_joins_of_the_pair() {
        let join = |key: &str, kind: &str, op: &str| {
            json!({"convId": conv(), "by": "person:them", "op": op,
                   "member": {"kind": kind, "key": key}})
        };
        let row = |r: &Value| stamp(&ms(), "person:them", kinds::TALK_MEMBER, r);
        assert_eq!(row(&join("person:me", "person", "join")), Some(conv()));
        assert_eq!(row(&join("person:them", "person", "join")), Some(conv()));
        assert_eq!(row(&join("person:other", "person", "join")), None);
        assert_eq!(row(&join("agent:c1", "agent", "join")), None);
        assert_eq!(row(&join("person:me", "agent", "join")), None);
        assert_eq!(row(&join("person:me", "person", "leave")), None);
        for kind in [
            kinds::TALK_CHANNEL_CREATED,
            kinds::TALK_CHANNEL_EDITED,
            kinds::TALK_CARD,
        ] {
            assert_eq!(
                stamp(&ms(), "person:them", kind, &post("person:them")),
                None
            );
        }
    }

    /// A canvas item or its change (card #1051) folds into the DM only when
    /// one of the pair wrote it under their own person key: an agent's
    /// item, someone else's, or one for another conversation folds nothing.
    #[test]
    fn a_canvas_row_folds_only_when_one_of_the_pair_wrote_it() {
        let item = |by: &str| {
            json!({"itemId": "i1", "callId": "c1", "convId": conv(), "kind": "note",
                   "text": "x", "assistant": false, "by": by})
        };
        for kind in [kinds::CALL_ITEM, kinds::CALL_ITEM_SET] {
            for who in ["person:me", "person:them"] {
                assert_eq!(stamp(&ms(), who, kind, &item(who)), Some(conv()), "{kind}");
            }
            for who in ["agent:c1", "person:other", "irori"] {
                assert_eq!(stamp(&ms(), who, kind, &item(who)), None, "{kind} {who}");
            }
            assert_eq!(
                stamp(&ms(), "person:them", kind, &item("person:me")),
                None,
                "{kind}: by must be the actor"
            );
            let mut other = item("person:them");
            other["convId"] = "ch_general".into();
            assert_eq!(stamp(&ms(), "person:them", kind, &other), None);
        }
    }

    /// A marker that names this person as the other side folds nothing.
    #[test]
    fn a_marker_with_itself_folds_nothing() {
        let d = DmSpace {
            creator: pin(ME),
            with: ME.into(),
        };
        let m = Membership {
            creator: ME.into(),
            pins: [(ME.to_string(), pin(ME))].into(),
            members: vec![member(ME)],
            ..ms()
        };
        let row = json!({"convId": coord::talk::pair_dm_id("person:me", "person:me"),
                         "by": "person:me"});
        assert_eq!(
            dm_conv_of(ME, &d, &m, "person:me", kinds::TALK_POST, &row),
            None
        );
    }

    #[test]
    fn done_keeps_a_prefix_of_what_fits() {
        let ev = |seq, n: usize| Event {
            device_id: "d".into(),
            seq,
            payload: vec![b'x'; n],
            ..Event::default()
        };
        let big = MAX_PENDING_LOG / 2;
        let rows = done_rows(&[ev(1, 10), ev(2, big), ev(3, big), ev(4, 10)]);
        assert_eq!(rows.iter().map(|r| r.s).collect::<Vec<_>>(), [1, 2]);
        // One row always goes, or a big first row would stall for ever.
        assert_eq!(done_rows(&[ev(1, MAX_PENDING_LOG * 2)]).len(), 1);
    }
}
