//! The board bundle (v0.5-shared-boards §4.1 step 6): what the inviter
//! hands the invitee once the words match, and what the invitee does with
//! it.
//!
//! The inviter writes `board.member.added` for the invitee (role member,
//! signed with its person key) and sends the board's whole log, the board
//! secret for its epoch and that epoch's maker. The invitee folds the
//! bundle on its own first, then opens `boards/<boardId>.db` pinned to the
//! creator in the log's `board.created`, keeps the secret, inserts the rows
//! the way a board pull does, and checks what the design's step 4 can't
//! yet: that the inviter is an owner of the board and the invitee is now
//! its member under the key it proved. Then it writes its own
//! `member.device.added` (its board root's self-cert) and `member.profile`.
//! Those two rows go back to the inviter in `done`, so both logs hold both
//! people before the board connection (#43) ever runs.
//!
//! A person DM (card #538) hands a hidden two-person space over with the
//! same bundle: [`admit`] on the founder, [`rebuild`] for a retry after the
//! add, [`check_offer`] and then [`join`] on the other side.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use gocompat::time::GoTime;
use irori_identity::membership::{BOARD_MEMBER, Membership};
use irori_identity::{
    BOARD_SECRET_SIZE, BoardCreated, BoardSecrets, BoardVerifier, Identity, MemberPin, RowRef,
    VerifyError, decode, kinds,
};
use irori_pair::wire::WireRow;
use serde::{Deserialize, Serialize};
use store::{Event, MAX_MISSING_LIMIT, Store, Vector};
use zeroize::Zeroizing;

/// The Core's boards, as the invite needs them. Each method is the Core
/// call of the same name (bramble's `irori_core::Core` boards API, #49),
/// so the Core implements this one-to-one.
pub trait InviteBoards: Send + Sync {
    /// Opens or creates `boards/<id>.db`, pinned to `creator`.
    fn open_board(&self, board_id: &str, creator: Option<MemberPin>) -> Result<(), String>;
    fn board_store(&self, board_id: &str) -> Option<Arc<Store>>;
    fn board_verifier(&self, board_id: &str) -> Option<Arc<BoardVerifier>>;
    fn board_secrets(&self, board_id: &str) -> Option<Arc<BoardSecrets>>;
    /// The person key (hex) another board this device holds pins for
    /// `person_id`, if any (design §4.1 step 4: a different key there stops
    /// the invite).
    fn pinned_key(&self, _person_id: &str) -> Option<String> {
        None
    }
}

/// What the inviter sends once the words match.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub board_epoch: i64,
    /// The device that made this epoch's secret (two makers of one epoch
    /// settle on the lower id, §6.1).
    pub maker: String,
    /// The board secret for `board_epoch`, base64.
    pub secret: String,
    /// The board's whole log, identity rows first.
    pub rows: Vec<WireRow>,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("board_id", &self.board_id)
            .field("board_epoch", &self.board_epoch)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

/// The person actor a person-signed board row is written under.
pub fn person_actor(person_id: &str) -> String {
    format!("{}{person_id}", store::kinds::PERSON_ACTOR_PREFIX)
}

fn now() -> GoTime {
    GoTime::from_system(SystemTime::now())
}

/// Every row of `st`, `board.*` and `member.*` first (§6.4), each device's
/// rows in seq order.
pub fn all_rows(st: &Store) -> Result<Vec<Event>, String> {
    let mut v = Vector::new();
    let mut out: Vec<Event> = Vec::new();
    loop {
        let page = st
            .missing(&v, MAX_MISSING_LIMIT)
            .map_err(|e| e.to_string())?;
        if page.is_empty() {
            break;
        }
        for ev in &page {
            let seq = v.entry(ev.device_id.clone()).or_insert(0);
            *seq = (*seq).max(ev.seq);
        }
        out.extend(page);
    }
    out.sort_by(|a, b| {
        let ident = |e: &Event| !store::is_board_identity_kind(&e.kind);
        (ident(a), &a.device_id, a.seq).cmp(&(ident(b), &b.device_id, b.seq))
    });
    Ok(out)
}

/// One board as this device holds it.
struct Held {
    st: Arc<Store>,
    ver: Arc<BoardVerifier>,
    secrets: Arc<BoardSecrets>,
}

fn held(boards: &dyn InviteBoards, board_id: &str) -> Result<Held, String> {
    Ok(Held {
        st: boards
            .board_store(board_id)
            .ok_or("this device doesn't hold that board")?,
        ver: boards
            .board_verifier(board_id)
            .ok_or("this device doesn't hold that board")?,
        secrets: boards
            .board_secrets(board_id)
            .ok_or("this device has no secret for that board")?,
    })
}

/// The bundle without its rows. It fails on a device with no secret for
/// the board's epoch, so [`admit`] calls it before it writes (card #538):
/// such a device must not burn the add.
fn bundle_head(board_id: &str, m: Membership, secrets: &BoardSecrets) -> Result<Bundle, String> {
    let board_epoch = secrets.epoch();
    let (secret, maker) = secrets
        .secret(board_epoch)
        .ok_or("this device has no secret for the board's epoch")?;
    Ok(Bundle {
        board_id: board_id.into(),
        name: m.name,
        tag: m.tag,
        board_epoch,
        maker,
        secret: B64.encode(&*secret),
        rows: Vec::new(),
    })
}

fn wire_rows(st: &Store) -> Result<Vec<WireRow>, String> {
    Ok(all_rows(st)?.iter().map(WireRow::of).collect())
}

/// The inviter: writes `board.member.added` for `pin` (role member) and
/// builds the bundle.
pub fn admit(
    boards: &dyn InviteBoards,
    ident: &Identity,
    board_id: &str,
    pin: &MemberPin,
    name: &str,
) -> Result<Bundle, String> {
    let h = held(boards, board_id)?;
    let me = ident.current();
    let m = h.ver.membership();
    if !m.owner(&me.id) {
        return Err("only an owner of the board can invite".into());
    }
    if m.active(&pin.person_id) {
        return Err(format!("{name} is already on this board"));
    }
    let epoch = h.ver.epoch(&me.id);
    let mut bundle = bundle_head(board_id, m, &h.secrets)?;
    let row = ident
        .board_member_added_row(epoch, pin, name, BOARD_MEMBER, now())
        .map_err(|e| e.to_string())?;
    h.st.append(&person_actor(&me.id), kinds::BOARD_MEMBER_ADDED, &row)
        .map_err(|e| e.to_string())?;
    bundle.rows = wire_rows(&h.st)?;
    Ok(bundle)
}

/// The bundle again, for a person [`admit`] already added, with nothing
/// written: a DM hand-off's retry after the add (card #538).
pub fn rebuild(
    boards: &dyn InviteBoards,
    ident: &Identity,
    board_id: &str,
    person_id: &str,
) -> Result<Bundle, String> {
    let h = held(boards, board_id)?;
    let m = h.ver.membership();
    if !m.owner(&ident.current().id) {
        return Err("only an owner of the board can invite".into());
    }
    if !m.active(person_id) {
        return Err("that person isn't on this board".into());
    }
    let mut bundle = bundle_head(board_id, m, &h.secrets)?;
    bundle.rows = wire_rows(&h.st)?;
    Ok(bundle)
}

/// What a bundle folds to on its own, before anything touches disk.
pub struct OfferFold {
    /// The creator its counted `board.created` names.
    pub creator: MemberPin,
    pub membership: Membership,
    /// The throwaway Verifier it folded in.
    pub verifier: BoardVerifier,
}

/// Folds `evs` the way `mesh::boards::insert_board_rows` stores them: each
/// device's rows in seq order, a run stopping at its first refused row or
/// seq gap, and a run with no cert yet trying again while another moves.
/// Returns the last refusal.
///
/// The store also refuses rows for reasons that need a log (HLC order, a
/// row ahead of the clock), so this can count a row the store won't.
/// [`join`] checks the real log again after it inserts.
fn fold_rows(ver: &BoardVerifier, evs: &[Event]) -> Option<String> {
    let mut runs: BTreeMap<&str, (i64, VecDeque<&Event>)> = BTreeMap::new();
    for ev in evs {
        runs.entry(&ev.device_id)
            .or_insert((1, VecDeque::new()))
            .1
            .push_back(ev);
    }
    for (_, run) in runs.values_mut() {
        run.make_contiguous().sort_by_key(|e| e.seq);
    }
    let mut refused = None;
    let mut moved = true;
    while moved {
        moved = false;
        for (dev, (next, run)) in runs.iter_mut() {
            while let Some(ev) = run.front().copied() {
                if ev.seq != *next {
                    refused = Some(format!("{dev} holds seq {}, got {}", *next - 1, ev.seq));
                    run.clear();
                    break;
                }
                match ver.verify_row(RowRef::from(ev)) {
                    Ok(()) => {
                        ver.observe(RowRef::from(ev));
                        run.pop_front();
                        *next += 1;
                        moved = true;
                    }
                    Err(VerifyError::NoCert(_)) => break,
                    Err(e) => {
                        refused = Some(e.to_string());
                        run.clear();
                    }
                }
            }
        }
    }
    refused
}

/// `b` folded in a throwaway Verifier, pinned to the creator it names.
/// Nothing touches disk. It fails when the fold's `board.created` doesn't
/// count.
fn fold_offer(want_board: &str, b: &Bundle) -> Result<OfferFold, String> {
    if b.board_id != want_board {
        return Err("the inviter sent a different board than the link names".into());
    }
    let evs: Vec<Event> = b.rows.iter().map(WireRow::event).collect();
    let creator = evs
        .iter()
        .filter(|e| e.kind == kinds::BOARD_CREATED)
        .find_map(|e| decode::<BoardCreated>(&e.payload))
        .filter(|c| c.board_id == b.board_id)
        .ok_or("the bundle has no board.created for this board")?
        .creator;
    let verifier = BoardVerifier::new_for(&b.board_id, std::slice::from_ref(&creator));
    let refused = fold_rows(&verifier, &evs);
    let membership = verifier.membership();
    if !membership.created {
        return Err(format!(
            "the board's log doesn't verify ({})",
            refused.unwrap_or_default()
        ));
    }
    Ok(OfferFold {
        creator,
        membership,
        verifier,
    })
}

/// The secret [`BoardSecrets::add`] will take. Refused here, it refuses
/// before anything is opened.
fn secret_of(b: &Bundle) -> Result<Zeroizing<Vec<u8>>, String> {
    let secret = Zeroizing::new(
        B64.decode(&b.secret)
            .map_err(|_| "the board secret is damaged")?,
    );
    if secret.len() != BOARD_SECRET_SIZE || b.board_epoch < 1 || b.maker.is_empty() {
        return Err("the board secret is damaged".into());
    }
    Ok(secret)
}

/// `inviter` owns the board, and it lists this device under the key this
/// person has now.
fn check_joined(m: &Membership, ident: &Identity, inviter: &str) -> Result<(), String> {
    if !m.owner(inviter) {
        return Err("the person who invited you isn't an owner of this board".into());
    }
    let me = ident.current();
    let mine = m.person(&me.id).filter(|_| m.active(&me.id));
    match mine {
        Some(p) if p.pin.person_pub == me.pub_hex && p.pin.root_device_id == ident.device_id() => {
            Ok(())
        }
        _ => Err("the board doesn't list you under the key you proved".into()),
    }
}

/// The joiner of a DM hand-off (card #538, talk-dms.md §2): whether a
/// founder's offer can be taken, with nothing written anywhere. It checks
/// what the bundle alone can show: the secret's shape, a `board.created`
/// that verifies, members that are exactly its creator and this person, and
/// this person's pin, a plain member on this device under the current key.
///
/// The returned fold is for what only the caller can check: the creator
/// against its key proof and the sender's cert.
pub fn check_offer(ident: &Identity, want_board: &str, b: &Bundle) -> Result<OfferFold, String> {
    let fold = fold_offer(want_board, b)?;
    secret_of(b)?;
    let m = &fold.membership;
    check_joined(m, ident, &fold.creator.person_id)?;
    let me = ident.current();
    if fold.creator.person_id == me.id || m.members.len() != 2 {
        return Err("a DM space holds exactly its two people".into());
    }
    match m.person(&me.id) {
        Some(p) if p.role == BOARD_MEMBER && p.pin.person_epoch == me.epoch => Ok(fold),
        _ => Err("the board doesn't list you under the key you proved".into()),
    }
}

/// This device's rows of `st`, in seq order.
fn rows_of(st: &Store, device: &str) -> Result<Vec<Event>, String> {
    let mut rows = all_rows(st)?;
    rows.retain(|e| e.device_id == device);
    rows.sort_by_key(|e| e.seq);
    Ok(rows)
}

/// The invitee: opens the board from the bundle and writes its own
/// `member.*` rows. `inviter` is the person id that proved itself on the
/// invite, and `role` goes on this device's cert. Returns this device's
/// rows, for the inviter.
///
/// The secret is kept before the rows go in (card #538), so whatever reads
/// a row already finds it. A refusal after that would leave the secret
/// behind, so the bundle is folded first and one the log would refuse
/// opens nothing.
///
/// Joining twice writes nothing new: a DM's retry after a lost `done` lands
/// here, and the same rows go back from seq 1, so the other store takes
/// them.
pub fn join(
    boards: &dyn InviteBoards,
    ident: &Identity,
    want_board: &str,
    inviter: &str,
    role: &str,
    b: &Bundle,
) -> Result<Vec<Event>, String> {
    let fold = fold_offer(want_board, b)?;
    let secret = secret_of(b)?;
    check_joined(&fold.membership, ident, inviter)?;
    boards.open_board(&b.board_id, Some(fold.creator))?;
    let st = boards
        .board_store(&b.board_id)
        .ok_or("the board didn't open")?;
    let ver = boards
        .board_verifier(&b.board_id)
        .ok_or("the board didn't open")?;
    let secrets = boards
        .board_secrets(&b.board_id)
        .ok_or("the board's secrets didn't open")?;
    secrets
        .add(b.board_epoch, &b.maker, &secret)
        .map_err(|e| e.to_string())?;
    // A row the store refuses is recorded there; what matters is whether
    // the membership below came out right.
    let evs = b.rows.iter().map(WireRow::event).collect();
    let inserted = mesh::boards::insert_board_rows(&st, evs);
    let m = ver.membership();
    if !m.created {
        return Err(format!(
            "the board's log doesn't verify ({})",
            inserted.err().unwrap_or_default()
        ));
    }
    check_joined(&m, ident, inviter)?;
    let me = ident.current();
    let epoch = me.epoch;
    let device = ident.device_id();
    let actor = person_actor(&me.id);
    let certified = ver
        .cert(&device)
        .is_some_and(|c| c.person_id == me.id && c.person_epoch == epoch);
    if !certified {
        let cert = ident
            .member_cert_for(
                epoch,
                &device,
                &hex::encode(ident.device_pub()),
                role,
                now(),
            )
            .map_err(|e| e.to_string())?;
        st.append(&actor, kinds::MEMBER_DEVICE_ADDED, &cert)
            .map_err(|e| e.to_string())?;
    }
    if ver.profile(&me.id).is_none() {
        let prof = ident.member_profile_row(epoch).map_err(|e| e.to_string())?;
        st.append(&actor, kinds::MEMBER_PROFILE, &prof)
            .map_err(|e| e.to_string())?;
    }
    rows_of(&st, &device)
}
