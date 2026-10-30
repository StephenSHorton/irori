//! Deliveries and wakes across devices (I:internal/inbox/mesh.go,
//! owner.go): the owner table, folding other devices' rows, the host's
//! hold decision and delivery.decided requests.

use std::collections::{BTreeMap, HashMap, HashSet};

use irori_identity::owner::device_of;
use store::{Event, Filter, MAX_LIST_LIMIT, Subscription as StoreSub};

use crate::messages::{bad_state, dismissed, not_found_delivery};
use crate::policy::placed;
use crate::state::{DecidedRow, Drec, Inner, REPLAY_KINDS, decode_decided, decode_delivery};
use crate::{ChangeKind, Error, ErrorKind, Inbox, Policy, Result, choice, hold, kinds, st};

impl Inner {
    /// agent is hosted here: its key names this device, or none.
    pub(crate) fn local(&self, agent: &str) -> bool {
        let dev = device_of(agent);
        dev.is_empty() || dev == self.dev
    }

    /// This device owns d (§6.0).
    pub(crate) fn owns(&self, d: &Drec) -> bool {
        if !d.d.to.is_empty() {
            return self.local(&d.d.to);
        }
        d.first_writer.is_empty() || d.first_writer == self.dev
    }

    /// A device whose rows the log now holds fewer of than were folded.
    fn cut_since(&self, vec: &store::Vector) -> bool {
        self.last
            .iter()
            .any(|(dev, seq)| vec.get(dev).copied().unwrap_or(0) < *seq)
    }

    /// Rebuilds everything folded from rows; memory-only state stays.
    fn refold(&mut self) {
        self.msgs = HashMap::new();
        self.dels = HashMap::new();
        self.order = vec![];
        self.open.clear();
        self.counted = HashSet::new();
        self.wakes = HashMap::new();
        self.tokens = HashMap::new();
        self.aliases = BTreeMap::new();
        self.known = BTreeMap::new();
        self.pol = Policy::default();
        self.running = BTreeMap::new();
        self.pairs = HashMap::new();
        self.visible_at = HashMap::new();
        self.held_first = HashSet::new();
        // handled stays: a request already applied isn't applied again.
        self.last = BTreeMap::new();
        self.requests = vec![];
        self.ignored = 0;
        self.ignored_by = BTreeMap::new();
        if let Err(e) = self.replay() {
            eprintln!("inbox: re-fold after a revoke's cut: {e}");
        }
        self.changes.push(crate::Change {
            kind: ChangeKind::Updated,
            to: String::new(),
            delivery_id: String::new(),
        });
    }

    /// Folds one row of another device and does the owner's part.
    pub(crate) fn apply_remote(&mut self, ev: &Event) {
        if ev.seq <= self.last.get(&ev.device_id).copied().unwrap_or(0) {
            return;
        }
        self.last.insert(ev.device_id.clone(), ev.seq);
        if let Err(e) = self.apply(ev) {
            eprintln!("inbox: fold {} {}/{}: {e}", ev.kind, ev.device_id, ev.seq);
            return;
        }
        match ev.kind.as_str() {
            kinds::DELIVERY_STATE => {
                if let Ok(r) = decode_delivery(&ev.payload)
                    && self.dels.contains_key(&r.delivery_id)
                    && let Err(e) = self.host_hold(&r.delivery_id)
                {
                    eprintln!("inbox: hold {}: {e}", r.delivery_id);
                }
            }
            kinds::DELIVERY_DECIDED => {
                if let Ok(r) = decode_decided(&ev.payload)
                    && ev.actor == "user"
                    && self.is_owner(&r.by)
                {
                    let id = r.delivery_id.clone();
                    if let Err(e) = self.apply_decided(r) {
                        eprintln!("inbox: decide {id}: {e}");
                    }
                }
            }
            _ => {}
        }
    }

    /// Decides every owned delivery another device queued that nobody here
    /// decided, then the requests that waited for this device.
    fn owner_pass(&mut self) {
        for id in self.order.clone() {
            if !self.open.contains(&id) {
                continue;
            }
            if let Err(e) = self.host_hold(&id) {
                eprintln!("inbox: hold {id}: {e}");
            }
            if let Err(e) = self.lift_retired_hold(&id) {
                eprintln!("inbox: lift {id}: {e}");
            }
        }
        for r in std::mem::take(&mut self.requests) {
            let id = r.delivery_id.clone();
            if let Err(e) = self.apply_decided(r) {
                eprintln!("inbox: decide {id}: {e}");
            }
        }
    }

    /// The host deciding a hold on a delivery another device queued.
    fn host_hold(&mut self, did: &str) -> Result<()> {
        let d = &self.dels[did];
        if d.d.state != st::QUEUED
            || !d.d.hold.is_empty()
            || !d.d.wake_id.is_empty()
            || d.d.to.is_empty()
            || !self.local(&d.d.to)
            || d.last_writer.is_empty()
            || d.last_writer == self.dev
            || d.host_checked
        {
            return Ok(());
        }
        let (mid, to) = (d.d.message_id.clone(), d.d.to.clone());
        self.dels.get_mut(did).expect("delivery").host_checked = true;
        if !self.msgs.contains_key(&mid) {
            return Ok(());
        }
        let cards = self.live_cards()?;
        let h = self.hold_id(&mid, &to, &cards);
        if h.is_empty() {
            self.touch(&[&to]);
            self.passed(did);
            return Ok(());
        }
        let r = placed(self.dels[did].row(), h);
        self.put_delivery(r, "irori")
    }

    /// A delivery to one of this device's sessions still held for
    /// bypass_receiver, a hold that no longer exists (card #71): the
    /// policy decides it again, which queues it unless the person's own
    /// settings hold it.
    fn lift_retired_hold(&mut self, did: &str) -> Result<()> {
        let d = &self.dels[did];
        if d.d.state != st::HELD || d.d.hold != hold::BYPASS_RECEIVER || !self.local(&d.d.to) {
            return Ok(());
        }
        let (mid, to) = (d.d.message_id.clone(), d.d.to.clone());
        if !self.msgs.contains_key(&mid) {
            return Ok(());
        }
        let cards = self.live_cards()?;
        let h = self.hold_id(&mid, &to, &cards);
        let r = placed(self.dels[did].row(), h);
        self.put_delivery(r, "irori")
    }

    /// The owner applying a request: the first per delivery wins; an undo
    /// applies past that while the delivery is still dismissed.
    pub(crate) fn apply_decided(&mut self, r: DecidedRow) -> Result<()> {
        let Some(d) = self.dels.get(&r.delivery_id) else {
            return Ok(());
        };
        if !self.owns(d) {
            return Ok(());
        }
        let state = d.d.state.clone();
        if r.undo {
            self.handled.insert(r.delivery_id.clone());
            if r.choice == choice::RELEASE && state == st::DISMISSED {
                return self.release_locked(&r.delivery_id, &r.by);
            }
            return Ok(());
        }
        if self.handled.contains(&r.delivery_id) {
            return Ok(());
        }
        self.handled.insert(r.delivery_id.clone());
        match r.choice.as_str() {
            choice::RELEASE if state == st::HELD => self.release_locked(&r.delivery_id, &r.by),
            choice::DISMISS if state == st::HELD => {
                let row = dismissed(&self.dels[&r.delivery_id]);
                self.put_delivery(row, &r.by)
            }
            choice::MOVE => {
                let to = self.dels[&r.delivery_id].d.to.clone();
                if st::open(&state) && !r.to.is_empty() && r.to != to {
                    let cards = self.live_cards()?;
                    return self.move_to(&r.delivery_id, &r.to, &cards);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// A delivery.decided row for a delivery another device owns.
    pub(crate) fn request(&mut self, did: &str, ch: &str, to: &str, by: &str) -> Result<()> {
        if !self.is_owner(by) {
            return Err(Error::wrap(
                ErrorKind::BadState,
                "only the person decides a held delivery",
            ));
        }
        let undo = ch == choice::RELEASE && self.dels[did].d.state == st::DISMISSED;
        let row = DecidedRow {
            delivery_id: did.into(),
            choice: ch.into(),
            to: to.into(),
            by: by.into(),
            at_device: self.dev.clone(),
            at: self.now(),
            undo,
        };
        self.emit(by, kinds::DELIVERY_DECIDED, &row)
    }
}

impl Inbox {
    /// Folds other devices' rows as the store delivers them, until the
    /// subscription ends, doing this device's part as their owner. Core
    /// runs it once, on its own thread, with `store.subscribe()`.
    pub fn follow(&self, sub: &StoreSub) {
        self.catch_up();
        while let Some(ev) = sub.recv() {
            if ev.is_resync() {
                self.catch_up(); // the store's resync signal
                continue;
            }
            let dev = self.read(|b| b.dev.clone());
            if ev.device_id == dev || !REPLAY_KINDS.contains(&ev.kind.as_str()) {
                continue;
            }
            self.mutate(|b| b.apply_remote(&ev));
        }
    }

    /// Folds what other devices wrote and runs the owner pass once, now.
    pub fn catch_up(&self) {
        let (st, dev) = self.read(|b| (b.st.clone(), b.dev.clone()));
        let Ok(vec) = st.vector() else {
            return;
        };
        if self.read(|b| b.cut_since(&vec)) {
            self.mutate(|b| b.refold());
        }
        for (d, top) in &vec {
            if *d == dev {
                continue;
            }
            loop {
                let after = self.read(|b| b.last.get(d).copied().unwrap_or(0));
                if after >= *top {
                    break;
                }
                let evs = st.list(&Filter {
                    device_id: d.clone(),
                    after_seq: after,
                    kinds: REPLAY_KINDS.iter().map(|k| k.to_string()).collect(),
                    limit: MAX_LIST_LIMIT,
                    ..Filter::default()
                });
                let Ok(evs) = evs else { break };
                if evs.is_empty() {
                    break;
                }
                for ev in &evs {
                    self.mutate(|b| b.apply_remote(ev));
                }
                if (evs.len() as i64) < MAX_LIST_LIMIT {
                    break;
                }
            }
        }
        self.mutate(|b| b.owner_pass());
    }

    /// Hands an open delivery to another agent (by is the person). On
    /// another device's delivery it is a request its owner applies.
    pub fn move_delivery(&self, did: &str, to: &str, by: &str) -> Result<()> {
        self.mutate(|b| {
            let Some(d) = b.dels.get(did) else {
                return Err(not_found_delivery(did));
            };
            if !st::open(&d.d.state) {
                return Err(bad_state(d, "move"));
            }
            if !b.owns(d) {
                return b.request(did, choice::MOVE, to, by);
            }
            let cards = b.live_cards()?;
            b.move_to(did, to, &cards)
        })
    }

    /// This device owns the delivery (§6.0).
    pub fn owned(&self, did: &str) -> bool {
        self.read(|b| b.dels.get(did).is_some_and(|d| b.owns(d)))
    }

    /// Owner-only rows ignored because another device wrote them.
    pub fn ignored_rows(&self) -> i64 {
        self.read(|b| b.ignored)
    }

    /// Ignored rows per writing device.
    pub fn ignored_by(&self) -> BTreeMap<String, i64> {
        self.read(|b| b.ignored_by.clone())
    }

    /// Folds one other-device row as Follow would (for a Core that drives
    /// the store subscription itself).
    pub fn apply_remote(&self, ev: &Event) {
        let dev = self.read(|b| b.dev.clone());
        if ev.device_id == dev || !REPLAY_KINDS.contains(&ev.kind.as_str()) {
            return;
        }
        self.mutate(|b| b.apply_remote(ev));
    }
}
