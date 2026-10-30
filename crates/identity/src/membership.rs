//! A board's membership fold (I:internal/identity/membership.go; identity.md
//! §4.4): its board.* rows, each signed by the acting person's key, folded
//! into who is on the board, who owns it, who was removed or left, and with
//! which cuts. Pure: the board Verifier hands it the rows and what its
//! member chains say ([`MembershipKeys`]).
//!
//! Two layers, because a cut can't be undone:
//! - authority (only grows): who has been an owner since when, which
//!   removals and leaves count, and so each gone person's cuts;
//! - state, the ordered fold in (hlc, device, seq) order: who is a member
//!   now and with which role. Every ignored row is a [`Conflict`].

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::kinds;
use crate::objects::{
    BoardCreated, BoardMemberAdded, BoardMemberLeft, BoardMemberRemoved, BoardMemberRole,
    BoardRenamed, MemberPin, decode, valid_pub,
};
use crate::row::RowRef;

pub const BOARD_OWNER: &str = "owner";
pub const BOARD_MEMBER: &str = "member";
pub const MEMBER_ACTIVE: &str = "active";
pub const MEMBER_REMOVED: &str = "removed";
pub const MEMBER_LEFT: &str = "left";

/// A board's name cap, in runes.
pub const MAX_BOARD_NAME: usize = 80;

/// A row's place in fold order (GUI JSON `{hlc, device, seq}`). The derived
/// order is Go's `RowAt.less`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct RowAt {
    pub hlc: u64,
    pub device: String,
    pub seq: i64,
}

/// One board.* row as the fold reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipRow {
    pub at: RowAt,
    pub kind: String,
    pub payload: Vec<u8>,
}

impl MembershipRow {
    /// Go `MembershipRowOf(ev)`.
    pub fn of(ev: RowRef<'_>) -> MembershipRow {
        MembershipRow {
            at: RowAt {
                hlc: ev.hlc,
                device: ev.device_id.to_string(),
                seq: ev.seq,
            },
            kind: ev.kind.to_string(),
            payload: ev.payload.to_vec(),
        }
    }
}

/// What the fold asks of the board's identity chains.
pub trait MembershipKeys {
    /// The member whose chain ever certified `device`, cuts ignored (`""`
    /// for none).
    fn person_of(&self, device: &str) -> String;
    /// Whether `sig` is the member's key of `epoch` over Canonical(body),
    /// and a row at `at` may still be signed by it. `body` is Go
    /// `json.Marshal` of the signed value (`None` when it doesn't marshal,
    /// which never verifies).
    fn signed(
        &self,
        person_id: &str,
        epoch: i64,
        at: &RowAt,
        sig: &str,
        body: Option<&[u8]>,
    ) -> bool;
    /// Whether the member's chain holds `pub_hex` as `epoch`'s key.
    fn has_key(&self, person_id: &str, epoch: i64, pub_hex: &str) -> bool;
    /// Every device ever certified to the member, sorted.
    fn devices(&self, person_id: &str) -> Vec<String>;
    /// A device's revocation cut (member.device.revoked).
    fn cut(&self, device: &str) -> Option<i64>;
}

fn is_empty_str(s: &str) -> bool {
    s.is_empty()
}

/// One person as the fold sees them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardPerson {
    pub person_id: String,
    #[serde(skip_serializing_if = "is_empty_str")]
    pub name: String,
    pub role: String,
    pub state: String,
    pub pin: MemberPin,
    pub since: RowAt,
}

/// A board.* row the fold ignored, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub at: RowAt,
    pub kind: String,
    #[serde(skip_serializing_if = "is_empty_str")]
    pub person_id: String,
    #[serde(skip_serializing_if = "is_empty_str")]
    pub by: String,
    pub reason: String,
}

fn nil_if_empty<S: serde::Serializer, T: Serialize>(v: &[T], s: S) -> Result<S::Ok, S::Error> {
    if v.is_empty() {
        s.serialize_none()
    } else {
        v.serialize(s)
    }
}

/// A board's folded membership.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Membership {
    /// A board.created matching an anchored creator counted; nothing else
    /// counts without one.
    pub created: bool,
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub creator: String,
    /// Everyone the board ever added, in the order they joined. Go writes a
    /// nil slice (no board) as `null`.
    #[serde(serialize_with = "nil_if_empty")]
    pub members: Vec<BoardPerson>,
    /// The anchors and each pinned person's pin (`json:"-"`).
    #[serde(skip)]
    pub pins: BTreeMap<String, MemberPin>,
    /// Each removed or left person's devices' cuts.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub cuts: BTreeMap<String, i64>,
    /// The persons removed or left by a counted row, sorted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gone: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<Conflict>,
}

impl Membership {
    pub fn person(&self, person_id: &str) -> Option<&BoardPerson> {
        self.members.iter().find(|p| p.person_id == person_id)
    }
    pub fn active(&self, person_id: &str) -> bool {
        self.person(person_id)
            .is_some_and(|p| p.state == MEMBER_ACTIVE)
    }
    pub fn owner(&self, person_id: &str) -> bool {
        self.person(person_id)
            .is_some_and(|p| p.state == MEMBER_ACTIVE && p.role == BOARD_OWNER)
    }
}

/// One decoded membership row.
#[derive(Clone, Debug, Default)]
struct MRow {
    at: RowAt,
    kind: String,
    by: String,
    who: String,
    epoch: i64,
    sig: String,
    /// `json.Marshal` of what `sig` covers.
    body: Option<Vec<u8>>,
    name: String,
    tag: String,
    board: String,
    role: String,
    pin: MemberPin,
    cut: Option<BTreeMap<String, i64>>,
}

fn body_of<T: Serialize>(v: &T) -> Option<Vec<u8>> {
    gocompat::json::marshal(v).ok()
}

fn decode_membership(r: &MembershipRow) -> Option<MRow> {
    let mut d = MRow {
        at: r.at.clone(),
        kind: r.kind.clone(),
        ..MRow::default()
    };
    match r.kind.as_str() {
        kinds::BOARD_CREATED => {
            let mut p: BoardCreated = decode(&r.payload)?;
            d.sig = std::mem::take(&mut p.person_sig);
            d.by = p.creator.person_id.clone();
            d.who = p.creator.person_id.clone();
            d.epoch = p.creator.person_epoch;
            d.body = body_of(&p);
            d.name = p.name;
            d.tag = p.tag;
            d.board = p.board_id;
            d.pin = p.creator;
        }
        kinds::BOARD_RENAMED => {
            let mut p: BoardRenamed = decode(&r.payload)?;
            d.sig = std::mem::take(&mut p.person_sig);
            d.by = p.by.clone();
            d.epoch = p.person_epoch;
            d.body = body_of(&p);
            d.name = p.name;
        }
        kinds::BOARD_MEMBER_ADDED => {
            let mut p: BoardMemberAdded = decode(&r.payload)?;
            d.sig = std::mem::take(&mut p.person_sig);
            d.by = p.by.clone();
            d.who = p.person_id.clone();
            d.epoch = p.by_epoch;
            d.body = body_of(&p);
            d.pin = p.pin();
            d.name = p.name;
            d.role = p.role;
        }
        kinds::BOARD_MEMBER_ROLE => {
            let mut p: BoardMemberRole = decode(&r.payload)?;
            d.sig = std::mem::take(&mut p.person_sig);
            d.by = p.by.clone();
            d.who = p.person_id.clone();
            d.epoch = p.person_epoch;
            d.body = body_of(&p);
            d.role = p.role;
        }
        kinds::BOARD_MEMBER_REMOVED => {
            let mut p: BoardMemberRemoved = decode(&r.payload)?;
            d.sig = std::mem::take(&mut p.person_sig);
            d.by = p.by.clone();
            d.who = p.person_id.clone();
            d.epoch = p.person_epoch;
            d.body = body_of(&p);
            d.cut = p.cut;
        }
        kinds::BOARD_MEMBER_LEFT => {
            let mut p: BoardMemberLeft = decode(&r.payload)?;
            d.sig = std::mem::take(&mut p.person_sig);
            d.by = p.person_id.clone();
            d.who = p.person_id.clone();
            d.epoch = p.person_epoch;
            d.body = body_of(&p);
            d.cut = p.cut;
        }
        _ => return None,
    }
    (!d.by.is_empty()).then_some(d)
}

/// Go `validPin`.
pub fn valid_pin(p: &MemberPin) -> bool {
    !p.person_id.is_empty()
        && !p.root_device_id.is_empty()
        && p.person_epoch >= 1
        && valid_pub(&p.person_pub)
}

/// Fold a board's membership rows. `anchors` are the pins this device
/// trusts from outside the log; a board.created counts only for an
/// anchored creator, written by its board root, for `board_id` (`""` skips
/// that check). Independent of row order.
pub fn fold_membership(
    board_id: &str,
    anchors: &BTreeMap<String, MemberPin>,
    rows: &[MembershipRow],
    k: &dyn MembershipKeys,
) -> Membership {
    let mut ds: Vec<MRow> = rows.iter().filter_map(decode_membership).collect();
    ds.sort_by(|a, b| a.at.cmp(&b.at));

    let mut m = Membership {
        board_id: board_id.to_string(),
        pins: anchors.clone(),
        ..Membership::default()
    };
    let authored = |d: &MRow| {
        !d.by.is_empty()
            && k.person_of(&d.at.device) == d.by
            && k.signed(&d.by, d.epoch, &d.at, &d.sig, d.body.as_deref())
    };

    // The one board.created: the first that matches an anchor.
    let mut created: Option<usize> = None;
    for (i, d) in ds.iter().enumerate() {
        if d.kind != kinds::BOARD_CREATED {
            continue;
        }
        let ok = anchors.get(&d.pin.person_id).is_some_and(|a| *a == d.pin);
        if ok
            && d.at.device == d.pin.root_device_id
            && (board_id.is_empty() || d.board == board_id)
            && authored(d)
        {
            created = Some(i);
            break;
        }
    }
    let Some(created) = created else {
        return m;
    };
    let c = ds[created].clone();
    m.created = true;
    m.name = c.name.clone();
    m.tag = c.tag.clone();
    m.creator = c.by.clone();

    // Authority: owner since when, to a fixed point.
    let mut since: BTreeMap<String, RowAt> = BTreeMap::new();
    since.insert(c.by.clone(), c.at.clone());
    let owner_before =
        |since: &BTreeMap<String, RowAt>, p: &str, at: &RowAt| since.get(p).is_some_and(|s| s < at);
    let mut changed = true;
    while changed {
        changed = false;
        for d in &ds {
            let grant = (d.kind == kinds::BOARD_MEMBER_ADDED || d.kind == kinds::BOARD_MEMBER_ROLE)
                && d.role == BOARD_OWNER;
            if !grant || d.who.is_empty() || !owner_before(&since, &d.by, &d.at) || !authored(d) {
                continue;
            }
            match since.get(&d.who) {
                Some(s) if *s <= d.at => {}
                _ => {
                    since.insert(d.who.clone(), d.at.clone());
                    changed = true;
                }
            }
        }
    }
    for d in &ds {
        if d.kind != kinds::BOARD_MEMBER_ADDED || d.who.is_empty() || !valid_pin(&d.pin) {
            continue;
        }
        if m.pins.contains_key(&d.who) || !owner_before(&since, &d.by, &d.at) || !authored(d) {
            continue;
        }
        m.pins.insert(d.who.clone(), d.pin.clone()); // the earliest counted add pins
    }
    let mut honoured: BTreeSet<RowAt> = BTreeSet::new();
    let mut gone: BTreeMap<String, Vec<&MRow>> = BTreeMap::new();
    for d in &ds {
        if !m.pins.contains_key(&d.who) {
            continue;
        }
        let counts = (d.kind == kinds::BOARD_MEMBER_REMOVED
            && d.who != d.by
            && owner_before(&since, &d.by, &d.at)
            && authored(d))
            || (d.kind == kinds::BOARD_MEMBER_LEFT && authored(d));
        if counts {
            honoured.insert(d.at.clone());
            gone.entry(d.who.clone()).or_default().push(d);
        }
    }
    for (who, rs) in &gone {
        m.gone.push(who.clone());
        for dev in k.devices(who) {
            let mut cut: i64 = -1;
            for r in rs {
                let mut v = r
                    .cut
                    .as_ref()
                    .and_then(|c| c.get(&dev))
                    .copied()
                    .unwrap_or(0);
                if r.kind == kinds::BOARD_MEMBER_LEFT && r.at.device == dev && r.at.seq > v {
                    v = r.at.seq; // the row that cuts can't cut itself
                }
                if cut < 0 || v < cut {
                    cut = v;
                }
            }
            // No cut reaches below board.created.
            if dev == c.at.device && cut < c.at.seq {
                cut = c.at.seq;
            }
            m.cuts.insert(dev, cut);
        }
    }
    m.gone.sort();

    // State: the ordered fold.
    let mut people: BTreeMap<String, BoardPerson> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut conflicts: Vec<Conflict> = Vec::new();
    fn put(people: &mut BTreeMap<String, BoardPerson>, order: &mut Vec<String>, p: BoardPerson) {
        if !people.contains_key(&p.person_id) {
            order.push(p.person_id.clone());
        }
        people.insert(p.person_id.clone(), p);
    }
    let is_owner = |people: &BTreeMap<String, BoardPerson>, p: &str| {
        people
            .get(p)
            .is_some_and(|e| e.state == MEMBER_ACTIVE && e.role == BOARD_OWNER)
    };
    let owners = |people: &BTreeMap<String, BoardPerson>| {
        people
            .values()
            .filter(|e| e.state == MEMBER_ACTIVE && e.role == BOARD_OWNER)
            .count()
    };
    let mcuts = m.cuts.clone();
    let pins = m.pins.clone();
    let mut name = m.name.clone();
    let past = |at: &RowAt| {
        if let Some(cut) = mcuts.get(&at.device)
            && at.seq > *cut
        {
            return true;
        }
        matches!(k.cut(&at.device), Some(cut) if at.seq > cut)
    };
    let conflict = |conflicts: &mut Vec<Conflict>, d: &MRow, reason: &str| {
        conflicts.push(Conflict {
            at: d.at.clone(),
            kind: d.kind.clone(),
            person_id: d.who.clone(),
            by: d.by.clone(),
            reason: reason.to_string(),
        });
    };
    for (i, d) in ds.iter().enumerate() {
        let gate = d.kind == kinds::BOARD_MEMBER_REMOVED || d.kind == kinds::BOARD_MEMBER_LEFT;
        if !gate && past(&d.at) {
            continue; // the store refuses it; nothing to show
        }
        if i < created {
            conflict(&mut conflicts, d, "it comes before the board was created");
            continue;
        }
        match d.kind.as_str() {
            kinds::BOARD_CREATED => {
                if i == created {
                    put(
                        &mut people,
                        &mut order,
                        BoardPerson {
                            person_id: c.by.clone(),
                            name: String::new(),
                            role: BOARD_OWNER.into(),
                            state: MEMBER_ACTIVE.into(),
                            pin: c.pin.clone(),
                            since: c.at.clone(),
                        },
                    );
                } else {
                    conflict(&mut conflicts, d, "the board was already created");
                }
            }
            kinds::BOARD_RENAMED => {
                if !authored(d) {
                    conflict(&mut conflicts, d, "its signature doesn't count");
                } else if !is_owner(&people, &d.by) {
                    conflict(&mut conflicts, d, "only an owner renames the board");
                } else if !valid_board_name(&d.name) {
                    conflict(&mut conflicts, d, "the name is not a board name");
                } else {
                    name = d.name.clone();
                }
            }
            kinds::BOARD_MEMBER_ADDED => {
                let pin = pins.get(&d.who).cloned();
                let active = people.get(&d.who).is_some_and(|e| e.state == MEMBER_ACTIVE);
                if !authored(d) {
                    conflict(&mut conflicts, d, "its signature doesn't count");
                } else if !is_owner(&people, &d.by) {
                    conflict(&mut conflicts, d, "only an owner adds members");
                } else if d.role != BOARD_OWNER && d.role != BOARD_MEMBER {
                    conflict(&mut conflicts, d, "the role is not owner or member");
                } else if pin.as_ref().is_none_or(|p| {
                    *p != d.pin && !k.has_key(&d.who, d.pin.person_epoch, &d.pin.person_pub)
                }) {
                    conflict(
                        &mut conflicts,
                        d,
                        "it pins another key than the one this board holds for them",
                    );
                } else if active {
                    conflict(&mut conflicts, d, "they are already a member");
                } else {
                    put(
                        &mut people,
                        &mut order,
                        BoardPerson {
                            person_id: d.who.clone(),
                            name: d.name.clone(),
                            role: d.role.clone(),
                            state: MEMBER_ACTIVE.into(),
                            pin: pin.unwrap_or_default(),
                            since: d.at.clone(),
                        },
                    );
                }
            }
            kinds::BOARD_MEMBER_ROLE => {
                let e = people.get(&d.who);
                if !authored(d) {
                    conflict(&mut conflicts, d, "its signature doesn't count");
                } else if !is_owner(&people, &d.by) {
                    conflict(&mut conflicts, d, "only an owner changes roles");
                } else if d.role != BOARD_OWNER && d.role != BOARD_MEMBER {
                    conflict(&mut conflicts, d, "the role is not owner or member");
                } else if e.is_none() {
                    conflict(&mut conflicts, d, "they are not on the board");
                } else if e.is_some_and(|e| e.state != MEMBER_ACTIVE) {
                    conflict(
                        &mut conflicts,
                        d,
                        "they were removed or left before it; the removal stands",
                    );
                } else if e.is_some_and(|e| e.role == BOARD_OWNER)
                    && d.role == BOARD_MEMBER
                    && owners(&people) == 1
                {
                    conflict(&mut conflicts, d, "the last owner can't stop being one");
                } else if let Some(e) = people.get_mut(&d.who) {
                    e.role = d.role.clone();
                }
            }
            kinds::BOARD_MEMBER_REMOVED | kinds::BOARD_MEMBER_LEFT => {
                let left = d.kind == kinds::BOARD_MEMBER_LEFT;
                let state = if left { MEMBER_LEFT } else { MEMBER_REMOVED };
                if !honoured.contains(&d.at) {
                    if left {
                        conflict(&mut conflicts, d, "it isn't signed by the person leaving");
                    } else {
                        conflict(&mut conflicts, d, "only an owner removes members");
                    }
                    continue;
                }
                if !left && !is_owner(&people, &d.by) {
                    conflict(
                        &mut conflicts,
                        d,
                        "its remover was no longer an owner, but a removal always stands",
                    );
                }
                match people.get_mut(&d.who) {
                    None => {
                        let pin = pins.get(&d.who).cloned().unwrap_or_default();
                        put(
                            &mut people,
                            &mut order,
                            BoardPerson {
                                person_id: d.who.clone(),
                                name: String::new(),
                                role: BOARD_MEMBER.into(),
                                state: state.into(),
                                pin,
                                since: d.at.clone(),
                            },
                        );
                    }
                    Some(e) => {
                        if e.state == MEMBER_ACTIVE {
                            e.state = state.into();
                        }
                    }
                }
            }
            _ => {}
        }
    }
    m.name = name;
    m.conflicts = conflicts;
    m.members = order
        .iter()
        .filter_map(|id| people.get(id).cloned())
        .collect();
    m
}

/// A board name: 1 to 80 runes, no surrounding space, no control
/// characters (Go `ValidBoardName`).
pub fn valid_board_name(name: &str) -> bool {
    if name.is_empty() || go_trim_space(name) != name || name.chars().count() > MAX_BOARD_NAME {
        return false;
    }
    !name.chars().any(gocompat::text::is_control)
}

/// Go `strings.TrimSpace`.
fn go_trim_space(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}

/// A board's short handle: a lowercase letter, then up to 15 lowercase
/// letters, digits or `-` (Go `ValidBoardTag`).
pub fn valid_board_tag(tag: &str) -> bool {
    let b = tag.as_bytes();
    if b.is_empty() || b.len() > 16 || !b[0].is_ascii_lowercase() {
        return false;
    }
    b[1..]
        .iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
