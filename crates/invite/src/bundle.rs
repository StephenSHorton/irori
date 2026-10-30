//! The board bundle (v0.5-shared-boards §4.1 step 6): what the inviter
//! hands the invitee once the words match, and what the invitee does with
//! it.
//!
//! The inviter writes `board.member.added` for the invitee (role member,
//! signed with its person key) and sends the board's whole log, the board
//! secret for its epoch and that epoch's maker. The invitee opens
//! `boards/<boardId>.db` pinned to the creator in the log's `board.created`,
//! inserts the rows the way a board pull does, and only then checks what
//! the design's step 4 can't yet: that the inviter is an owner of the board
//! and the invitee is now its member under the key it proved. Then it
//! writes its own `member.device.added` (its board root's self-cert) and
//! `member.profile`, and keeps the secret. Those two rows go back to the
//! inviter in `done`, so both logs hold both people before the board
//! connection (#43) ever runs.

use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use gocompat::time::GoTime;
use irori_identity::membership::BOARD_MEMBER;
use irori_identity::{
    BoardCreated, BoardSecrets, BoardVerifier, Identity, MemberPin, ROLE_DESKTOP, decode, kinds,
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

/// The inviter: writes `board.member.added` for `pin` (role member) and
/// builds the bundle.
pub fn admit(
    boards: &dyn InviteBoards,
    ident: &Identity,
    board_id: &str,
    pin: &MemberPin,
    name: &str,
) -> Result<Bundle, String> {
    let st = boards
        .board_store(board_id)
        .ok_or("this device doesn't hold that board")?;
    let ver = boards
        .board_verifier(board_id)
        .ok_or("this device doesn't hold that board")?;
    let secrets = boards
        .board_secrets(board_id)
        .ok_or("this device has no secret for that board")?;
    let me = ident.current();
    let m = ver.membership();
    if !m.owner(&me.id) {
        return Err("only an owner of the board can invite".into());
    }
    if m.active(&pin.person_id) {
        return Err(format!("{name} is already on this board"));
    }
    let epoch = ver.epoch(&me.id);
    let row = ident
        .board_member_added_row(epoch, pin, name, BOARD_MEMBER, now())
        .map_err(|e| e.to_string())?;
    st.append(&person_actor(&me.id), kinds::BOARD_MEMBER_ADDED, &row)
        .map_err(|e| e.to_string())?;
    let board_epoch = secrets.epoch();
    let (secret, maker) = secrets
        .secret(board_epoch)
        .ok_or("this device has no secret for the board's epoch")?;
    let rows = all_rows(&st)?.iter().map(WireRow::of).collect();
    Ok(Bundle {
        board_id: board_id.into(),
        name: m.name,
        tag: m.tag,
        board_epoch,
        maker,
        secret: B64.encode(&*secret),
        rows,
    })
}

/// The invitee: opens the board from the bundle and writes its own
/// `member.*` rows. `inviter` is the person id that proved itself on the
/// invite. Returns the rows it wrote, for the inviter.
pub fn join(
    boards: &dyn InviteBoards,
    ident: &Identity,
    want_board: &str,
    inviter: &str,
    b: &Bundle,
) -> Result<Vec<Event>, String> {
    if b.board_id != want_board {
        return Err("the inviter sent a different board than the link names".into());
    }
    let evs: Vec<Event> = b.rows.iter().map(WireRow::event).collect();
    let created = evs
        .iter()
        .filter(|e| e.kind == kinds::BOARD_CREATED)
        .find_map(|e| decode::<BoardCreated>(&e.payload))
        .filter(|c| c.board_id == b.board_id)
        .ok_or("the bundle has no board.created for this board")?;
    let secret = Zeroizing::new(
        B64.decode(&b.secret)
            .map_err(|_| "the board secret is damaged")?,
    );
    boards.open_board(&b.board_id, Some(created.creator.clone()))?;
    let st = boards
        .board_store(&b.board_id)
        .ok_or("the board didn't open")?;
    let ver = boards
        .board_verifier(&b.board_id)
        .ok_or("the board didn't open")?;
    // A row the store refuses is recorded there; what matters is whether
    // the membership below came out right.
    let inserted = mesh::boards::insert_board_rows(&st, evs);
    let me = ident.current();
    let m = ver.membership();
    if !m.created {
        return Err(format!(
            "the board's log doesn't verify ({})",
            inserted.err().unwrap_or_default()
        ));
    }
    if !m.owner(inviter) {
        return Err("the person who invited you isn't an owner of this board".into());
    }
    let mine = m.person(&me.id).filter(|_| m.active(&me.id));
    match mine {
        Some(p) if p.pin.person_pub == me.pub_hex && p.pin.root_device_id == ident.device_id() => {}
        _ => return Err("the board doesn't list you under the key you proved".into()),
    }
    let epoch = me.epoch;
    let cert = ident
        .member_cert_for(
            epoch,
            &ident.device_id(),
            &hex::encode(ident.device_pub()),
            ROLE_DESKTOP,
            now(),
        )
        .map_err(|e| e.to_string())?;
    let actor = person_actor(&me.id);
    let a = st
        .append(&actor, kinds::MEMBER_DEVICE_ADDED, &cert)
        .map_err(|e| e.to_string())?;
    let prof = ident.member_profile_row(epoch).map_err(|e| e.to_string())?;
    let p = st
        .append(&actor, kinds::MEMBER_PROFILE, &prof)
        .map_err(|e| e.to_string())?;
    let secrets = boards
        .board_secrets(&b.board_id)
        .ok_or("the board's secrets didn't open")?;
    secrets
        .add(b.board_epoch, &b.maker, &secret)
        .map_err(|e| e.to_string())?;
    Ok(vec![a, p])
}
