//! Offering a new space to friends, with no invite (card #517,
//! docs/design/talk-channels.md §10.3).
//!
//! New board's "Who's on it?" makes a new space and offers it to each
//! picked friend: a person this person already shares a space with (the
//! pair's friend space, or an older shared board). The offer travels as
//! `space.offer` frames on that shared space's connection, between two
//! devices its verifier already admits, so it needs no words:
//!
//! 1. `offer {space, name, tag}`, maker → a device of the friend.
//! 2. `accept {space, pin, name}`: the friend's device answers with a
//!    fresh pin, its person key and epoch now with itself as the root
//!    device. A pin copied from another space would name the device that
//!    joined that one, which may be off or gone, and no device of the
//!    friend could then certify into the new space.
//! 3. `bundle {space, bundle}`: the maker adds that pin as a member and
//!    sends the space's rows and secret, as an invite's bundle.
//! 4. `joined {space, rows}`: the friend's device opens the space from it
//!    (as an invitee does) and sends back its device cert and profile.
//!
//! The maker's private log keeps a `space.offered` row per friend, so the
//! offer waits for a friend whose devices are all offline, and goes out
//! from any of the maker's devices once one of the friend's says hello.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use gocompat::time::GoTime;
use irori_identity::MemberPin;
use irori_pair::wire::WireRow;
use serde::{Deserialize, Serialize};
use store::Filter;
use store::kinds;

use super::Services;
use super::boards::each_row;

/// How often pending offers are tried.
const PUMP_EVERY: Duration = Duration::from_secs(2);
/// The least time between two sends of one offer to one person.
const RESEND_AFTER: Duration = Duration::from_secs(10);

/// The private `space.offered` row: `space` was offered to `person`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfferedRow {
    pub space_id: String,
    pub person_id: String,
    pub at: GoTime,
}

/// One `space.offer` frame's body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    step: String,
    space: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pin: Option<PinWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundle: Option<irori_invite::bundle::Bundle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rows: Vec<WireRow>,
}

const STEP_OFFER: &str = "offer";
const STEP_ACCEPT: &str = "accept";
const STEP_BUNDLE: &str = "bundle";
const STEP_JOINED: &str = "joined";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinWire {
    person_id: String,
    person_pub: String,
    root_device_id: String,
    person_epoch: i64,
}

impl PinWire {
    fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.person_id.clone(),
            person_pub: self.person_pub.clone(),
            root_device_id: self.root_device_id.clone(),
            person_epoch: self.person_epoch,
        }
    }
}

/// The offers in flight on this device.
#[derive(Default)]
pub(crate) struct Offers {
    /// Maker: when each (space, person) offer last went out.
    sent: Mutex<HashMap<(String, String), Instant>>,
    /// Friend: the spaces this device accepted, and who offered each.
    accepted: Mutex<HashMap<String, String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Services {
    /// Records that space `space` is offered to `person` and tries now.
    pub(crate) fn offer_space(
        self: &Arc<Self>,
        space: &str,
        person: &str,
    ) -> super::boards::Result<()> {
        self.store.append(
            "irori",
            kinds::SPACE_OFFERED,
            &OfferedRow {
                space_id: space.into(),
                person_id: person.into(),
                at: (self.now)(),
            },
        )?;
        self.pump_offers();
        Ok(())
    }

    /// The people this person shares a shown space with (card #517 b3:
    /// anyone you share a space with is a friend), by person id.
    pub(crate) fn friend_ids(&self) -> std::collections::BTreeSet<String> {
        let me = self.person_id();
        let mut out = std::collections::BTreeSet::new();
        for b in self.shown_boards() {
            let m = b.verifier.membership();
            if !m.active(&me) {
                continue;
            }
            for p in &m.members {
                if p.person_id != me && m.active(&p.person_id) {
                    out.insert(p.person_id.clone());
                }
            }
        }
        out
    }

    /// The offers still waiting: (space, person) with a `space.offered`
    /// row, where the space is open here, this person still owns it and
    /// the person isn't on it yet.
    pub(crate) fn pending_offers(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = Vec::new();
        let _ = each_row(
            &self.store,
            Filter {
                kinds: vec![kinds::SPACE_OFFERED.into()],
                ..Default::default()
            },
            |ev| {
                if let Ok(r) = serde_json::from_slice::<OfferedRow>(&ev.payload)
                    && !rows
                        .iter()
                        .any(|(s, p)| *s == r.space_id && *p == r.person_id)
                {
                    rows.push((r.space_id, r.person_id));
                }
            },
        );
        let me = self.person_id();
        rows.retain(|(space, person)| {
            self.board(space).is_ok_and(|b| {
                let m = b.verifier.membership();
                m.created && m.owner(&me) && !m.active(person) && !self.board_leaving(&b)
            })
        });
        rows
    }

    /// Sends each pending offer to one connected device of its person, on
    /// a space both share, at most once per [`RESEND_AFTER`].
    pub(crate) fn pump_offers(self: &Arc<Self>) {
        let pending = self.pending_offers();
        if pending.is_empty() {
            return;
        }
        let me = self.person_id();
        let shown = self.shown_boards();
        for (space, person) in pending {
            {
                let sent = lock(&self.offers.sent);
                if sent
                    .get(&(space.clone(), person.clone()))
                    .is_some_and(|t| t.elapsed() < RESEND_AFTER)
                {
                    continue;
                }
            }
            let Ok(sb) = self.board(&space) else { continue };
            let sm = sb.verifier.membership();
            // A device of `person` connected on a space both are on.
            let target = shown.iter().filter(|b| b.id != space).find_map(|b| {
                let m = b.verifier.membership();
                if !m.active(&me) || !m.active(&person) {
                    return None;
                }
                let mesh = self.mesh.board(&b.id)?;
                let up: Vec<String> = mesh.peers().into_iter().map(|(_, d)| d).collect();
                b.verifier
                    .devices(&person)
                    .into_iter()
                    .filter(|d| !d.revoked)
                    .find(|d| up.contains(&d.cert.device_id))
                    .map(|d| (mesh, d.cert.device_id))
            });
            let Some((mesh, dev)) = target else { continue };
            lock(&self.offers.sent).insert((space.clone(), person.clone()), Instant::now());
            let body = Step {
                step: STEP_OFFER.into(),
                space: space.clone(),
                name: sm.name.clone(),
                tag: sm.tag.clone(),
                ..Default::default()
            };
            self.send_step(mesh, dev, body);
        }
    }

    fn send_step(&self, mesh: Arc<mesh::boardmesh::BoardMesh>, dev: String, body: Step) {
        let Ok(v) = serde_json::to_value(&body) else {
            return;
        };
        self.pairing.handle.spawn(async move {
            if let Err(e) = mesh.send_offer(&dev, v).await {
                crate::log(&format!("space.offer {} to {dev}: {e}", body.step));
            }
        });
    }

    /// A `space.offer` frame from device `dev` on space `carrier`'s
    /// connection. The sender's person is the one the carrier's verifier
    /// certifies `dev` to, never what its hello said.
    pub(crate) fn take_offer_step(
        self: &Arc<Self>,
        carrier: &str,
        dev: &str,
        body: serde_json::Value,
    ) {
        let Ok(step) = serde_json::from_value::<Step>(body) else {
            return;
        };
        let Ok(cb) = self.board(carrier) else { return };
        let Some(from) = cb.verifier.person_of(dev) else {
            crate::log(&format!("space.offer: {dev} isn't certified on {carrier}"));
            return;
        };
        let me = self.person_id();
        let m = cb.verifier.membership();
        if from == me || !m.active(&from) || !m.active(&me) {
            return;
        }
        if cb
            .verifier
            .devices(&from)
            .iter()
            .any(|d| d.cert.device_id == dev && d.revoked)
        {
            return;
        }
        let Some(mesh) = self.mesh.board(carrier) else {
            return;
        };
        let r = match step.step.as_str() {
            STEP_OFFER => self.offered(&step, &from, mesh, dev),
            STEP_ACCEPT => self.accepted(&step, &from, carrier, mesh, dev),
            STEP_BUNDLE => self.bundled(&step, &from, mesh, dev),
            STEP_JOINED => self.joined(&step, &from, dev),
            _ => Ok(()),
        };
        if let Err(e) = r {
            crate::log(&format!(
                "space.offer {} for {} from {dev}: {e}",
                step.step, step.space
            ));
        }
    }

    /// The friend: a space offered by `from`. Friends are trusted (b3),
    /// so this device accepts with a fresh pin of its own.
    fn offered(
        self: &Arc<Self>,
        step: &Step,
        from: &str,
        mesh: Arc<mesh::boardmesh::BoardMesh>,
        dev: &str,
    ) -> Result<(), String> {
        if !super::boards::valid_board_id(&step.space) {
            return Err("not a space id".into());
        }
        if self.board(&step.space).is_ok() {
            return Ok(()); // already here
        }
        lock(&self.offers.accepted).insert(step.space.clone(), from.to_string());
        let p = self.ident.current();
        let body = Step {
            step: STEP_ACCEPT.into(),
            space: step.space.clone(),
            name: p.name.clone(),
            pin: Some(PinWire {
                person_id: p.id,
                person_pub: p.pub_hex,
                root_device_id: self.ident.device_id(),
                person_epoch: p.epoch,
            }),
            ..Default::default()
        };
        self.send_step(mesh, dev.to_string(), body);
        Ok(())
    }

    /// The maker: friend `from` accepted with a pin from its device
    /// `dev`. It joins as a member, and gets the space's rows and secret.
    fn accepted(
        self: &Arc<Self>,
        step: &Step,
        from: &str,
        carrier: &str,
        mesh: Arc<mesh::boardmesh::BoardMesh>,
        dev: &str,
    ) -> Result<(), String> {
        if !self
            .pending_offers()
            .iter()
            .any(|(s, p)| *s == step.space && p == from)
        {
            return Err("no offer of that space to them is waiting".into());
        }
        let pin = step.pin.as_ref().ok_or("no pin")?.pin();
        if pin.person_id != from || pin.root_device_id != dev {
            return Err("the pin isn't the sending device's person".into());
        }
        // The key must be the one the carrier pins for them, or a newer
        // epoch of it.
        let cb = self.board(carrier).map_err(|e| e.to_string())?;
        let known = cb.verifier.pinned(from).ok_or("they aren't pinned there")?;
        if pin.person_epoch < known.person_epoch
            || (pin.person_epoch == known.person_epoch && pin.person_pub != known.person_pub)
        {
            return Err("the pin's key isn't theirs".into());
        }
        let name = if step.name.trim().is_empty() {
            cb.verifier.profile(from).unwrap_or_default()
        } else {
            step.name.clone()
        };
        let boards = super::invites::core_boards(Arc::downgrade(self));
        let bundle = irori_invite::bundle::admit(&*boards, &self.ident, &step.space, &pin, &name)?;
        let body = Step {
            step: STEP_BUNDLE.into(),
            space: step.space.clone(),
            bundle: Some(bundle),
            ..Default::default()
        };
        self.send_step(mesh, dev.to_string(), body);
        Ok(())
    }

    /// The friend: the space's bundle. It opens here as an invitee's
    /// would, and its own rows go back.
    fn bundled(
        self: &Arc<Self>,
        step: &Step,
        from: &str,
        mesh: Arc<mesh::boardmesh::BoardMesh>,
        dev: &str,
    ) -> Result<(), String> {
        let offered_by = lock(&self.offers.accepted).get(&step.space).cloned();
        if offered_by.as_deref() != Some(from) {
            return Err("this device didn't accept that space from them".into());
        }
        let bundle = step.bundle.as_ref().ok_or("no bundle")?;
        let boards = super::invites::core_boards(Arc::downgrade(self));
        let evs = irori_invite::bundle::join(&*boards, &self.ident, &step.space, from, bundle)?;
        lock(&self.offers.accepted).remove(&step.space);
        let body = Step {
            step: STEP_JOINED.into(),
            space: step.space.clone(),
            rows: evs.iter().map(WireRow::of).collect(),
            ..Default::default()
        };
        self.send_step(mesh, dev.to_string(), body);
        let (s, space, dev) = (self.clone(), step.space.clone(), dev.to_string());
        self.pairing.handle.spawn(async move {
            s.invite_joined(&space, &dev, Vec::new(), true, false).await;
        });
        Ok(())
    }

    /// The maker: the friend's device cert and profile in the new space.
    fn joined(self: &Arc<Self>, step: &Step, from: &str, dev: &str) -> Result<(), String> {
        let b = self.board(&step.space).map_err(|e| e.to_string())?;
        let evs: Vec<store::Event> = step.rows.iter().map(WireRow::event).collect();
        if evs.iter().any(|e| {
            e.device_id != dev || e.actor != format!("{}{from}", kinds::PERSON_ACTOR_PREFIX)
        }) {
            return Err("rows from someone else".into());
        }
        mesh::boards::insert_board_rows(&b.store, evs)?;
        let (s, space, dev) = (self.clone(), step.space.clone(), dev.to_string());
        self.pairing.handle.spawn(async move {
            s.invite_joined(&space, &dev, Vec::new(), false, false)
                .await;
        });
        Ok(())
    }
}

/// Tries pending offers every [`PUMP_EVERY`] until the Core goes.
pub(crate) fn start_pump(s: &Arc<Services>) {
    let w: Weak<Services> = Arc::downgrade(s);
    s.pairing.handle.spawn(async move {
        loop {
            tokio::time::sleep(PUMP_EVERY).await;
            let Some(s) = w.upgrade() else { return };
            if s.boards.stop.get().is_some_and(|c| c.is_canceled()) {
                return;
            }
            s.pump_offers();
        }
    });
}
