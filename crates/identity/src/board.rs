//! A board log's Verifier (I:internal/identity/board.go; identity.md §4.5):
//! one chain per pinned member, run by v0.4's rules inside the board log,
//! the membership fold to a fixed point with them, and the binding of every
//! row's actor to its writing device's person.
//!
//! Go ranges over maps in several places whose result depends on the order
//! when three or more members certify one device (trap 9); this port walks
//! person ids sorted, so it is deterministic and agrees with Go everywhere
//! else.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use gocompat::json::{self, Out, Schema};
use serde::Serialize;

use crate::chain::{certify_rows, floor_of};
use crate::kinds;
use crate::membership::{
    Membership, MembershipKeys, MembershipRow, RowAt, fold_membership, valid_pin,
};
use crate::objects::{
    MemberCert, MemberDeviceAdded, MemberDeviceRevoked, MemberKeyRotated, MemberPin, MemberProfile,
    MemberRevoke, canonical, decode, ed25519_verify, valid_pub, verify_person, verify_person_with,
};
use crate::row::{CutClaim, RowPos, RowRef, Verifier, VerifyError, signing_bytes};

#[derive(Clone, Debug)]
struct MCertRow {
    pos: RowPos,
    cert: MemberCert,
    sig: String,
}

#[derive(Clone, Debug)]
struct MRevokeRow {
    pos: RowPos,
    rev: MemberRevoke,
    sig: String,
}

#[derive(Clone, Debug)]
struct MRotRow {
    pos: RowPos,
    rot: MemberKeyRotated,
}

#[derive(Clone, Debug)]
struct MProfileRow {
    pos: RowPos,
    prof: MemberProfile,
}

#[derive(Clone, Debug)]
struct MCardRow {
    pos: RowPos,
    card_id: String,
    person_id: String,
}

/// One accepted epoch of a member's key; `end` is the HLC of what ended it
/// (0 while it is the newest).
#[derive(Clone, Debug, PartialEq, Eq)]
struct BoardEpoch {
    epoch: i64,
    pub_hex: String,
    end: u64,
}

/// One member's folded chain.
#[derive(Clone, Debug)]
struct MemberFold {
    pin: MemberPin,
    keys: Vec<BoardEpoch>,
    ever: BTreeMap<String, MCertRow>,
    valid: BTreeMap<String, MCertRow>,
    cuts: BTreeMap<String, i64>,
    claims: Vec<CutClaim>,
    name: String,
    profile: bool,
}

/// Person-signature checks remembered by (row, key): a row's bytes never
/// change once observed.
#[derive(Default)]
struct SigMemo(RefCell<HashMap<(String, i64, String), bool>>);

impl SigMemo {
    fn verify(
        &self,
        pos: &RowPos,
        pub_hex: &str,
        sig: &str,
        msg: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> bool {
        let k = (pos.writer.clone(), pos.seq, pub_hex.to_string());
        if let Some(ok) = self.0.borrow().get(&k) {
            return *ok;
        }
        let ok = verify_person_with(pub_hex, sig, msg).is_ok();
        self.0.borrow_mut().insert(k, ok);
        ok
    }
}

fn canon_of<T: Serialize>(v: &T) -> impl FnOnce() -> Result<Vec<u8>, String> + '_ {
    move || canonical(v).map_err(|e| e.to_string())
}

/// Go `signedIn`: `sig` is `epoch`'s key over the body and the row at
/// `pos` may still be signed by it (the epoch is the newest, or the row's
/// HLC is below what ended it).
fn signed_in(
    memo: &SigMemo,
    keys: &[BoardEpoch],
    epoch: i64,
    pos: &RowPos,
    sig: &str,
    msg: impl FnOnce() -> Result<Vec<u8>, String>,
) -> bool {
    for k in keys {
        if k.epoch != epoch {
            continue;
        }
        if k.end != 0 && pos.hlc >= k.end {
            return false;
        }
        return memo.verify(pos, &k.pub_hex, sig, msg);
    }
    false
}

fn certify_member(
    root: &str,
    rows: &[MCertRow],
    cuts: Option<&BTreeMap<String, i64>>,
) -> BTreeMap<String, MCertRow> {
    certify_rows(root, rows, |a| (&a.pos, a.cert.device_id.as_str()), cuts)
}

/// Go `epochsFrom`: the epoch chain from the pin; at each epoch the
/// rotation to the next with a prevSig by this epoch's key, written by its
/// By device, which is in `ever` and not skipped; the lowest By (then
/// position) wins.
fn epochs_from(
    memo: &SigMemo,
    pin: &MemberPin,
    rots: &[MRotRow],
    ever: &BTreeMap<String, MCertRow>,
    skip: &dyn Fn(&str) -> bool,
    freeze: bool,
) -> Vec<BoardEpoch> {
    let mut keys = vec![BoardEpoch {
        epoch: pin.person_epoch,
        pub_hex: pin.person_pub.clone(),
        end: 0,
    }];
    for _ in 0..rots.len() {
        let cur = keys.len() - 1;
        let mut best: Option<&MRotRow> = None;
        let mut frozen: u64 = 0;
        for r in rots {
            if r.rot.person_epoch != keys[cur].epoch + 1 || r.rot.by != r.pos.writer {
                continue;
            }
            let body = r.rot.body();
            if !memo.verify(&r.pos, &keys[cur].pub_hex, &r.rot.prev_sig, canon_of(&body)) {
                continue;
            }
            if !ever.contains_key(&r.rot.by) {
                continue;
            }
            if skip(&r.rot.by) {
                if frozen == 0 || r.pos.hlc < frozen {
                    frozen = r.pos.hlc;
                }
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => r.rot.by < b.rot.by || (r.rot.by == b.rot.by && r.pos < b.pos),
            };
            if better {
                best = Some(r);
            }
        }
        let Some(best) = best else {
            if freeze && frozen > 0 {
                keys[cur].end = frozen;
            }
            break;
        };
        keys[cur].end = best.pos.hlc;
        keys.push(BoardEpoch {
            epoch: best.rot.person_epoch,
            pub_hex: best.rot.person_pub.clone(),
            end: 0,
        });
    }
    keys
}

/// Go `memberChain`: a member's accepted epochs and the devices ever
/// certified under them (cuts ignored), iterated at most `len(rots)+2`
/// times.
fn member_chain(
    memo: &SigMemo,
    pin: &MemberPin,
    certs: &[MCertRow],
    rots: &[MRotRow],
    skip: &dyn Fn(&str) -> bool,
    freeze: bool,
) -> (Vec<BoardEpoch>, BTreeMap<String, MCertRow>) {
    let mut keys = vec![BoardEpoch {
        epoch: pin.person_epoch,
        pub_hex: pin.person_pub.clone(),
        end: 0,
    }];
    let mut ever = BTreeMap::new();
    for _ in 0..rots.len() + 2 {
        let signed: Vec<MCertRow> = certs
            .iter()
            .filter(|r| {
                signed_in(
                    memo,
                    &keys,
                    r.cert.person_epoch,
                    &r.pos,
                    &r.sig,
                    canon_of(&r.cert),
                )
            })
            .cloned()
            .collect();
        ever = certify_member(&pin.root_device_id, &signed, None);
        let next = epochs_from(memo, pin, rots, &ever, skip, freeze);
        if next == keys {
            break;
        }
        keys = next;
    }
    (keys, ever)
}

/// The rows a board verifier holds, by kind and sorted by position.
#[derive(Default)]
struct Rows {
    certs: Vec<MCertRow>,
    revokes: Vec<MRevokeRow>,
    rotations: Vec<MRotRow>,
    profiles: Vec<MProfileRow>,
}

/// Go `foldMember`: one member's chain, with the cuts and claims of its
/// honoured revokes; `extra` cuts (a removal's or leave's) also stop certs
/// above them from counting.
fn fold_member(
    memo: &SigMemo,
    rows: &Rows,
    pin: &MemberPin,
    extra: Option<&BTreeMap<String, i64>>,
) -> MemberFold {
    let pid = &pin.person_id;
    let certs: Vec<MCertRow> = rows
        .certs
        .iter()
        .filter(|r| &r.cert.person_id == pid)
        .cloned()
        .collect();
    let revs: Vec<&MRevokeRow> = rows
        .revokes
        .iter()
        .filter(|r| &r.rev.person_id == pid)
        .collect();
    let rots: Vec<MRotRow> = rows
        .rotations
        .iter()
        .filter(|r| &r.rot.person_id == pid)
        .cloned()
        .collect();

    // Revokes first, each judged on the chain without the revoked device's
    // own rotations.
    let mut revoked: BTreeSet<String> = BTreeSet::new();
    let mut honoured: Vec<&MRevokeRow> = Vec::new();
    for r in &revs {
        let target = r.rev.device_id.clone();
        let (keys, ever) = member_chain(memo, pin, &certs, &rots, &|d| d == target, false);
        if !ever.contains_key(&r.pos.writer)
            || !signed_in(
                memo,
                &keys,
                r.rev.person_epoch,
                &r.pos,
                &r.sig,
                canon_of(&r.rev),
            )
        {
            continue;
        }
        if !ever.contains_key(&target) {
            continue; // a member revokes only its own devices
        }
        revoked.insert(target);
        honoured.push(r);
    }
    let (keys, ever) = member_chain(memo, pin, &certs, &rots, &|d| revoked.contains(d), true);
    let signed: Vec<MCertRow> = certs
        .iter()
        .filter(|r| {
            signed_in(
                memo,
                &keys,
                r.cert.person_epoch,
                &r.pos,
                &r.sig,
                canon_of(&r.cert),
            )
        })
        .cloned()
        .collect();
    let mut mine: BTreeMap<String, i64> = BTreeMap::new();
    let mut claims = Vec::new();
    for r in &honoured {
        let dev = &r.rev.device_id;
        let mut cut = r.rev.board_cut;
        if let Some(f) = floor_of(&pin.root_device_id, &ever, |a| &a.pos, &r.pos.writer, dev)
            && f > cut
        {
            cut = f;
        }
        match mine.get(dev) {
            Some(cur) if *cur <= cut => {}
            _ => {
                mine.insert(dev.clone(), cut);
            }
        }
        if !r.rev.board_cut_hash.is_empty() && r.rev.board_cut > 0 {
            claims.push(CutClaim {
                device_id: dev.clone(),
                seq: r.rev.board_cut,
                hash: r.rev.board_cut_hash.clone(),
            });
        }
    }
    let mut within = mine.clone();
    if let Some(extra) = extra.filter(|e| !e.is_empty()) {
        for (dev, cut) in extra {
            if !ever.contains_key(dev) {
                continue;
            }
            match within.get(dev) {
                Some(cur) if *cur <= *cut => {}
                _ => {
                    within.insert(dev.clone(), *cut);
                }
            }
        }
    }
    let valid = certify_member(&pin.root_device_id, &signed, Some(&within));
    let mut m = MemberFold {
        pin: pin.clone(),
        keys,
        ever,
        valid,
        cuts: mine,
        claims,
        name: String::new(),
        profile: false,
    };
    for p in &rows.profiles {
        if &p.prof.person_id != pid || !m.ever.contains_key(&p.pos.writer) {
            continue;
        }
        let b = p.prof.body();
        if signed_in(
            memo,
            &m.keys,
            b.person_epoch,
            &p.pos,
            &p.prof.person_sig,
            canon_of(&b),
        ) {
            m.name = p.prof.name.clone(); // the latest counted one
            m.profile = true;
        }
    }
    m
}

/// The membership fold's view of the board's chains.
struct Keys<'a> {
    memo: &'a SigMemo,
    members: &'a BTreeMap<String, MemberFold>,
}

impl MembershipKeys for Keys<'_> {
    fn person_of(&self, dev: &str) -> String {
        let mut found = "";
        for (pid, m) in self.members {
            if !m.ever.contains_key(dev) {
                continue;
            }
            if m.pin.root_device_id == dev {
                return pid.clone();
            }
            if !found.is_empty() {
                return String::new(); // certified by two members: neither's
            }
            found = pid;
        }
        found.to_string()
    }

    fn signed(&self, pid: &str, epoch: i64, at: &RowAt, sig: &str, body: Option<&[u8]>) -> bool {
        let Some(m) = self.members.get(pid) else {
            return false;
        };
        let pos = RowPos {
            hlc: at.hlc,
            writer: at.device.clone(),
            seq: at.seq,
        };
        signed_in(self.memo, &m.keys, epoch, &pos, sig, || match body {
            Some(b) => json::canonical_raw(b).map_err(|e| e.to_string()),
            None => Err("unmarshalable body".into()),
        })
    }

    fn has_key(&self, pid: &str, epoch: i64, pub_hex: &str) -> bool {
        self.members.get(pid).is_some_and(|m| {
            m.keys
                .iter()
                .any(|e| e.epoch == epoch && e.pub_hex == pub_hex)
        })
    }

    fn devices(&self, pid: &str) -> Vec<String> {
        self.members
            .get(pid)
            .map(|m| m.ever.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn cut(&self, dev: &str) -> Option<i64> {
        for m in self.members.values() {
            if !m.ever.contains_key(dev) {
                continue;
            }
            if let Some(c) = m.cuts.get(dev) {
                return Some(*c);
            }
        }
        None
    }
}

/// One device a member certified in this board (GUI JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardDevice {
    pub cert: MemberCert,
    pub revoked: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub cut: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[derive(Default)]
struct State {
    board_id: String,
    /// The anchors, kept across Reset.
    pins: BTreeMap<String, MemberPin>,
    /// Anchors and the fold's pins.
    eff: BTreeMap<String, MemberPin>,
    brows: Vec<MembershipRow>,
    ms: Membership,
    revoked_n: usize,
    rows: Rows,
    cards: Vec<MCardRow>,
    seen: HashSet<(String, i64)>,
    sigs: SigMemo,
    bad: i64,
    members: BTreeMap<String, MemberFold>,
    devs: BTreeMap<String, String>,
    valid: BTreeMap<String, MemberCert>,
    cuts: BTreeMap<String, i64>,
    rev_cuts: BTreeMap<String, i64>,
    claims: Vec<CutClaim>,
    card_of: Arc<BTreeMap<String, String>>,
    ambig: BTreeSet<String>,
}

impl State {
    fn reset(&mut self) {
        let board_id = std::mem::take(&mut self.board_id);
        let pins = std::mem::take(&mut self.pins);
        *self = State {
            eff: pins.clone(),
            board_id,
            pins,
            ..State::default()
        };
    }

    fn fold_members(
        &self,
        pins: &BTreeMap<String, MemberPin>,
        extra: Option<&BTreeMap<String, i64>>,
    ) -> BTreeMap<String, MemberFold> {
        pins.iter()
            .map(|(pid, pin)| (pid.clone(), fold_member(&self.sigs, &self.rows, pin, extra)))
            .collect()
    }

    fn refold(&mut self) {
        self.rows.certs.sort_by(|a, b| a.pos.cmp(&b.pos));
        self.rows.revokes.sort_by(|a, b| a.pos.cmp(&b.pos));
        self.rows.rotations.sort_by(|a, b| a.pos.cmp(&b.pos));
        self.rows.profiles.sort_by(|a, b| a.pos.cmp(&b.pos));

        let mut pins = self.pins.clone();
        let mut members = BTreeMap::new();
        let mut ms = Membership::default();
        for _ in 0..self.brows.len() + 2 {
            members = self.fold_members(&pins, None);
            ms = fold_membership(
                &self.board_id,
                &self.pins,
                &self.brows,
                &Keys {
                    memo: &self.sigs,
                    members: &members,
                },
            );
            if ms.pins == pins {
                break;
            }
            pins = ms.pins.clone();
        }
        if !ms.cuts.is_empty() {
            // A removed member's certs above their cut certify nothing.
            members = self.fold_members(&pins, Some(&ms.cuts));
        }
        for p in ms.members.iter_mut() {
            if let Some(m) = members.get(&p.person_id)
                && m.profile
            {
                p.name = m.name.clone();
            }
        }
        // Which person each device is: one counted by two members counts
        // for neither, unless it is one's pinned root.
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for pid in pins.keys() {
            let Some(m) = members.get(pid) else {
                continue;
            };
            for dev in m.valid.keys() {
                owners.entry(dev.clone()).or_default().push(pid.clone());
            }
        }
        let mut devs = BTreeMap::new();
        let mut valid = BTreeMap::new();
        let mut ambig = BTreeSet::new();
        for (dev, pids) in &owners {
            let mut pid = pids[0].clone();
            if pids.len() > 1 {
                pid = String::new();
                for p in pids {
                    if pins.get(p).is_some_and(|x| &x.root_device_id == dev) {
                        pid = p.clone();
                    }
                }
                ambig.insert(dev.clone());
            }
            if pid.is_empty() {
                continue;
            }
            if let Some(r) = members.get(&pid).and_then(|m| m.valid.get(dev)) {
                valid.insert(dev.clone(), r.cert.clone());
            }
            devs.insert(dev.clone(), pid);
        }
        // A member's revokes cut only devices that are that member's.
        let mut cuts = BTreeMap::new();
        let mut claims = Vec::new();
        for pid in pins.keys() {
            let Some(m) = members.get(pid) else {
                continue;
            };
            for (dev, cut) in &m.cuts {
                if devs.get(dev) == Some(pid) {
                    cuts.insert(dev.clone(), *cut);
                }
            }
            for cl in &m.claims {
                if devs.get(&cl.device_id) == Some(pid) {
                    claims.push(cl.clone());
                }
            }
        }
        self.revoked_n = cuts.len();
        let rev_cuts = cuts.clone();
        // A removed or departed member's devices are cut too; the lower
        // cut wins.
        let gone: BTreeSet<&String> = ms.gone.iter().collect();
        for (dev, cut) in &ms.cuts {
            match devs.get(dev) {
                Some(p) if gone.contains(p) => {}
                _ => continue,
            }
            match cuts.get(dev) {
                Some(cur) if *cur <= *cut => {}
                _ => {
                    cuts.insert(dev.clone(), *cut);
                }
            }
        }
        self.eff = pins;
        self.ms = ms;
        self.members = members;
        self.devs = devs;
        self.valid = valid;
        self.cuts = cuts;
        self.rev_cuts = rev_cuts;
        self.claims = claims;
        self.ambig = ambig;
        self.fold_cards();
    }

    /// Which person each card is: the first agent.card (in fold order) a
    /// device of that person wrote.
    fn fold_cards(&mut self) {
        self.cards.sort_by(|a, b| a.pos.cmp(&b.pos));
        let mut card_of = BTreeMap::new();
        for r in &self.cards {
            if card_of.contains_key(&r.card_id) {
                continue;
            }
            if let Some(p) = self.devs.get(&r.pos.writer)
                && !p.is_empty()
                && *p == r.person_id
            {
                card_of.insert(r.card_id.clone(), p.clone());
            }
        }
        self.card_of = Arc::new(card_of);
    }

    /// `ev`'s cert when it is a pinned board root's own self-cert, signed
    /// by the pinned key for the pinned epoch.
    fn root_self_cert(&self, ev: RowRef<'_>) -> Option<MemberCert> {
        if ev.kind != kinds::MEMBER_DEVICE_ADDED {
            return None;
        }
        let p: MemberDeviceAdded = decode(ev.payload)?;
        if p.cert.device_id != ev.device_id {
            return None;
        }
        let pin = self.eff.get(&p.cert.person_id)?;
        if pin.root_device_id != ev.device_id
            || p.cert.person_epoch != pin.person_epoch
            || verify_person(&pin.person_pub, &p.person_sig, &p.cert).is_err()
        {
            return None;
        }
        Some(p.cert)
    }
}

/// A board log's store Verifier.
pub struct BoardVerifier {
    st: Mutex<State>,
}

impl BoardVerifier {
    /// Go `NewBoardVerifier(pins...)`: no boardId check.
    pub fn new(pins: &[MemberPin]) -> BoardVerifier {
        BoardVerifier::new_for("", pins)
    }

    /// Go `NewBoardVerifierFor(boardID, pins...)`: a board.created counts
    /// only when it names `board_id`.
    pub fn new_for(board_id: &str, pins: &[MemberPin]) -> BoardVerifier {
        let mut st = State {
            board_id: board_id.to_string(),
            ..State::default()
        };
        for p in pins {
            st.pins.insert(p.person_id.clone(), p.clone());
        }
        st.reset();
        BoardVerifier { st: Mutex::new(st) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Anchor a member and refold. A second, different pin for a person, or
    /// one that disagrees with the fold's pin for them, is refused.
    pub fn pin(&self, p: &MemberPin) -> Result<(), String> {
        if !valid_pin(p) {
            return Err(format!(
                "identity: incomplete pin for {}",
                gocompat::quote::quote(&p.person_id)
            ));
        }
        let mut st = self.lock();
        let old = st
            .pins
            .get(&p.person_id)
            .or_else(|| st.eff.get(&p.person_id));
        if let Some(old) = old {
            if old != p {
                return Err(format!(
                    "identity: {} is already pinned on this board with another key or root",
                    p.person_id
                ));
            }
            if st.pins.contains_key(&p.person_id) {
                return Ok(());
            }
        }
        st.pins.insert(p.person_id.clone(), p.clone());
        st.refold();
        Ok(())
    }

    /// A person's pin: an anchor, or the fold's from an owner's add.
    pub fn pinned(&self, person_id: &str) -> Option<MemberPin> {
        self.lock().eff.get(person_id).cloned()
    }

    /// The folded membership, names from counted member.profile rows.
    pub fn membership(&self) -> Membership {
        self.lock().ms.clone()
    }

    /// The board secret's epoch: 0 before board.created, else 1 plus every
    /// gone member and every revoked member device.
    pub fn board_epoch(&self) -> i64 {
        let st = self.lock();
        if !st.ms.created {
            return 0;
        }
        1 + st.ms.gone.len() as i64 + st.revoked_n as i64
    }

    /// Fold one board identity row: member.*, board.*, and agent.card.
    pub fn observe(&self, ev: RowRef<'_>) {
        let mut st = self.lock();
        if !st.seen.insert((ev.device_id.to_string(), ev.seq)) {
            return;
        }
        let pos = RowPos::of(ev);
        match ev.kind {
            kinds::MEMBER_DEVICE_ADDED => match decode::<MemberDeviceAdded>(ev.payload) {
                Some(p) if !p.cert.device_id.is_empty() && valid_pub(&p.cert.device_pub) => {
                    st.rows.certs.push(MCertRow {
                        pos,
                        cert: p.cert,
                        sig: p.person_sig,
                    });
                }
                _ => {
                    st.bad += 1;
                    return;
                }
            },
            kinds::MEMBER_DEVICE_REVOKED => match decode::<MemberDeviceRevoked>(ev.payload) {
                Some(p) if !p.revoke.device_id.is_empty() && p.revoke.board_cut >= 0 => {
                    st.rows.revokes.push(MRevokeRow {
                        pos,
                        rev: p.revoke,
                        sig: p.person_sig,
                    });
                }
                _ => {
                    st.bad += 1;
                    return;
                }
            },
            kinds::MEMBER_KEY_ROTATED => match decode::<MemberKeyRotated>(ev.payload) {
                Some(p) if valid_pub(&p.person_pub) && p.by == ev.device_id => {
                    st.rows.rotations.push(MRotRow { pos, rot: p });
                }
                _ => {
                    st.bad += 1;
                    return;
                }
            },
            kinds::MEMBER_PROFILE => match decode::<MemberProfile>(ev.payload) {
                Some(p) => st.rows.profiles.push(MProfileRow { pos, prof: p }),
                None => {
                    st.bad += 1;
                    return;
                }
            },
            kinds::AGENT_CARD => {
                let schema = Schema::object(vec![
                    ("cardId", Schema::String),
                    ("personId", Schema::String),
                ]);
                match json::unmarshal(ev.payload, &schema) {
                    Ok(o) if !o.get_str("cardId").is_empty() => {
                        st.cards.push(MCardRow {
                            pos,
                            card_id: o.get_str("cardId").to_string(),
                            person_id: o.get_str("personId").to_string(),
                        });
                        st.fold_cards(); // a card changes no chain
                    }
                    _ => st.bad += 1,
                }
                return;
            }
            k if kinds::is_membership_kind(k) => st.brows.push(MembershipRow::of(ev)),
            _ => return,
        }
        st.refold();
    }

    /// Check a board row: its signature against its writer's counted cert
    /// (or a pinned root's self-cert), then its actor against that device's
    /// person.
    pub fn verify_row(&self, ev: RowRef<'_>) -> Result<(), VerifyError> {
        let (cert, person, cards, rev_cut) = {
            let st = self.lock();
            let (cert, person) = match st.valid.get(ev.device_id) {
                Some(c) => (
                    Some(c.clone()),
                    st.devs.get(ev.device_id).cloned().unwrap_or_default(),
                ),
                None => {
                    let c = st.root_self_cert(ev);
                    let p = c.as_ref().map(|c| c.person_id.clone()).unwrap_or_default();
                    (c, p)
                }
            };
            (
                cert,
                person,
                st.card_of.clone(),
                st.rev_cuts.get(ev.device_id).copied(),
            )
        };
        let Some(cert) = cert else {
            return Err(VerifyError::NoCert(ev.device_id.to_string()));
        };
        // Past its own person's revoke, a device has no say: only the
        // revoke itself is mutual.
        if let Some(rc) = rev_cut
            && ev.seq > rc
            && (ev.kind == kinds::BOARD_MEMBER_REMOVED || ev.kind == kinds::BOARD_MEMBER_LEFT)
        {
            return Err(VerifyError::PastCut(format!(
                "{}/{} was revoked at {}",
                ev.device_id, ev.seq, rc
            )));
        }
        let pk = match hex::decode(&cert.device_pub) {
            Ok(b) if b.len() == 32 => b,
            _ => {
                return Err(VerifyError::BadSig(format!(
                    "{} has a bad key",
                    ev.device_id
                )));
            }
        };
        if !ed25519_verify(&pk, &signing_bytes(ev), ev.sig) {
            return Err(VerifyError::BadSig(format!("{}/{}", ev.device_id, ev.seq)));
        }
        bind_actor(ev, &person, |card| cards.get(card).cloned())
    }

    /// A device's effective cut in this board.
    pub fn cut(&self, device_id: &str) -> Option<i64> {
        self.lock().cuts.get(device_id).copied()
    }

    /// Each honoured revoke's boardCutHash.
    pub fn cut_claims(&self) -> Vec<CutClaim> {
        self.lock().claims.clone()
    }

    /// Forget every observed row; the pins stay.
    pub fn reset(&self) {
        self.lock().reset();
    }

    /// The member a device is certified to in this board.
    pub fn person_of(&self, device_id: &str) -> Option<String> {
        self.lock().devs.get(device_id).cloned()
    }

    /// The member whose device wrote card `card_id`.
    pub fn card_person(&self, card_id: &str) -> Option<String> {
        self.lock().card_of.get(card_id).cloned()
    }

    /// The member a board key is: `person:<id>` itself when pinned,
    /// `agent:<card>` its card's person, `""` otherwise.
    pub fn key_person(&self, key: &str) -> String {
        if let Some(id) = key.strip_prefix(kinds::PERSON_ACTOR_PREFIX) {
            return if self.lock().eff.contains_key(id) {
                id.to_string()
            } else {
                String::new()
            };
        }
        if let Some(id) = key.strip_prefix(kinds::AGENT_ACTOR_PREFIX) {
            return self.card_person(id).unwrap_or_default();
        }
        String::new()
    }

    /// A device's counted cert in this board.
    pub fn cert(&self, device_id: &str) -> Option<MemberCert> {
        self.lock().valid.get(device_id).cloned()
    }

    /// A member's newest accepted epoch (0 when not pinned).
    pub fn epoch(&self, person_id: &str) -> i64 {
        let st = self.lock();
        match st.members.get(person_id) {
            Some(m) => m.keys.last().map(|k| k.epoch).unwrap_or(0),
            None => st.eff.get(person_id).map(|p| p.person_epoch).unwrap_or(0),
        }
    }

    /// Whether a member may still sign new objects: its newest epoch wasn't
    /// frozen by rotations that were all discarded.
    pub fn writable(&self, person_id: &str) -> bool {
        self.lock()
            .members
            .get(person_id)
            .is_some_and(|m| m.keys.last().is_some_and(|k| k.end == 0))
    }

    /// A member's counted member.profile name.
    pub fn profile(&self, person_id: &str) -> Option<String> {
        let st = self.lock();
        let m = st.members.get(person_id)?;
        m.profile.then(|| m.name.clone())
    }

    /// A member's certified devices in this board, by device id.
    pub fn devices(&self, person_id: &str) -> Vec<BoardDevice> {
        let st = self.lock();
        st.devs
            .iter()
            .filter(|(_, p)| p.as_str() == person_id)
            .map(|(dev, _)| {
                let cut = st.cuts.get(dev);
                BoardDevice {
                    cert: st.valid.get(dev).cloned().unwrap_or_default(),
                    revoked: cut.is_some(),
                    cut: cut.copied().unwrap_or(0),
                }
            })
            .collect()
    }

    /// Device ids two members both certified, which count for neither.
    pub fn ambiguous(&self) -> Vec<String> {
        self.lock().ambig.iter().cloned().collect()
    }

    /// Identity rows that failed to parse.
    pub fn bad(&self) -> i64 {
        self.lock().bad
    }
}

impl Verifier for BoardVerifier {
    fn observe(&self, ev: RowRef<'_>) {
        BoardVerifier::observe(self, ev)
    }
    fn verify_row(&self, ev: RowRef<'_>) -> Result<(), VerifyError> {
        BoardVerifier::verify_row(self, ev)
    }
    fn cut(&self, device_id: &str) -> Option<i64> {
        BoardVerifier::cut(self, device_id)
    }
    fn cut_claims(&self) -> Vec<CutClaim> {
        BoardVerifier::cut_claims(self)
    }
    fn reset(&self) {
        BoardVerifier::reset(self)
    }
}

/// Go `bindActor`: ties a row to its writing device's person.
///
/// Go ignores the payload's decode error and keeps whatever fields it
/// filled before it; this decoder fails whole, so a payload with a
/// wrong-typed `personId`/`by`/`deviceId`/`cert`/`revoke`/`creator` reads
/// as empty here. Only crafted rows differ, and both builds refuse them
/// whenever the empty field disagrees with the writer.
fn bind_actor(
    ev: RowRef<'_>,
    person: &str,
    card_person: impl Fn(&str) -> Option<String>,
) -> Result<(), VerifyError> {
    let who = || Schema::Ptr(Box::new(Schema::object(vec![("personId", Schema::String)])));
    let schema = Schema::object(vec![
        ("personId", Schema::String),
        ("deviceId", Schema::String),
        ("cert", who()),
        ("revoke", who()),
        ("by", Schema::String),
        ("creator", who()),
    ]);
    let p = json::unmarshal(ev.payload, &schema).unwrap_or_else(|_| Out::zero(&schema));
    let ptr = |name: &str| -> Option<String> {
        match p.get(name) {
            Some(Out::Ptr(Some(inner))) => Some(inner.get_str("personId").to_string()),
            _ => None,
        }
    };
    let refuse =
        |what: String| VerifyError::BoardActor(format!("{}/{} {}", ev.device_id, ev.seq, what));
    let q = gocompat::quote::quote;
    let by = p.get_str("by");
    if kinds::is_membership_kind(ev.kind) {
        let actor = match ev.kind {
            kinds::BOARD_CREATED => ptr("creator").unwrap_or_default(),
            kinds::BOARD_MEMBER_LEFT => p.get_str("personId").to_string(),
            _ => by.to_string(),
        };
        if actor != person {
            return Err(refuse(format!(
                "writes {} as person {}, not its own",
                ev.kind,
                q(&actor)
            )));
        }
    } else if ev.kind == kinds::AGENT_CARD {
        let pid = p.get_str("personId");
        if pid != person {
            return Err(refuse(format!(
                "writes an agent card for person {}, not its own",
                q(pid)
            )));
        }
    } else if ev.kind.starts_with("member.") {
        let mut about = p.get_str("personId").to_string();
        if let Some(c) = ptr("cert") {
            about = c;
        }
        if let Some(r) = ptr("revoke") {
            about = r;
        }
        if about != person {
            return Err(refuse(format!(
                "writes {} for person {}, not its own",
                ev.kind,
                q(&about)
            )));
        }
    } else if ev.kind == kinds::BOARD_SERVER {
        let dev = p.get_str("deviceId");
        if dev != ev.device_id {
            return Err(refuse(format!(
                "announces a server for device {}, not itself",
                q(dev)
            )));
        }
    } else if !by.is_empty() && by != ev.actor {
        return Err(refuse(format!(
            "credits {} to {} but is written as {}",
            ev.kind,
            q(by),
            ev.actor
        )));
    }
    if let Some(id) = ev.actor.strip_prefix(kinds::PERSON_ACTOR_PREFIX) {
        if id != person {
            return Err(refuse(format!("writes as {}", ev.actor)));
        }
    } else if let Some(card) = ev.actor.strip_prefix(kinds::AGENT_ACTOR_PREFIX) {
        if ev.kind == kinds::AGENT_CARD {
            return Ok(()); // the card itself: its person is checked above
        }
        let Some(owner) = card_person(card) else {
            return Err(VerifyError::NoCert(format!(
                "{}/{} names card {}, not held yet",
                ev.device_id, ev.seq, card
            )));
        };
        if owner != person {
            return Err(refuse(format!(
                "writes as another person's agent {}",
                ev.actor
            )));
        }
    }
    Ok(())
}
