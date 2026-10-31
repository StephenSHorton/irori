//! The wake policy (I:internal/inbox/policy.go): the person's own block
//! and allow lists and Pause all, and the hop limit as a runaway-loop
//! guard. Nothing else holds a message: Go's bypass_receiver hold (a
//! bypass-mode session held other agents' messages) is gone (card #71,
//! Stephen 2026-10-30: people run Irori privately with their friends), so
//! the old Full auto mode `open` now equals `everyone`.

use std::collections::BTreeMap;

use gocompat::text;

use crate::state::{DeliveryRow, Inner, PolicyRow, lower_trim, norm_policy, trim};
use crate::{
    AgentCard, Error, Inbox, MAX_HOP, Message, Policy, Result, Sender, hold, kinds, policy_mode, st,
};

pub(crate) type Cards = BTreeMap<String, AgentCard>;

/// Sets r's state from a policy verdict.
pub(crate) fn placed(mut r: DeliveryRow, h: &str) -> DeliveryRow {
    r.wake_id.clear();
    if h.is_empty() {
        r.state = st::QUEUED.into();
        r.hold.clear();
    } else {
        r.state = st::HELD.into();
        r.hold = h.into();
    }
    r
}

impl Inner {
    /// The list names sender by key or name. Block (strict) also matches
    /// the names it sent under; Allow never does.
    pub(crate) fn listed(
        &self,
        list: &[String],
        from: &Sender,
        cards: &Cards,
        strict: bool,
    ) -> bool {
        let mut names: Vec<String> = vec![];
        if let Some(c) = cards.get(&from.key) {
            names.push(lower_trim(&c.name));
        } else if let Some(n) = self.known.get(&from.key)
            && !n.is_empty()
        {
            names.push(n.clone());
        }
        if strict {
            names.push(lower_trim(&from.name));
            if let Some(a) = self.aliases.get(&from.key) {
                names.extend(a.iter().cloned());
            }
        }
        for e in list {
            let e = trim(e);
            if e.is_empty() {
                continue;
            }
            if text::equal_fold(e, &from.key) {
                return true;
            }
            let le = text::to_lower_str(e);
            if names.iter().any(|n| !n.is_empty() && *n == le) {
                return true;
            }
        }
        false
    }

    /// The hold reason for m going to recipient, or `""` to queue it.
    pub(crate) fn hold(&self, m: &Message, recipient: &str, cards: &Cards) -> &'static str {
        if recipient.is_empty() {
            return hold::UNOWNED;
        }
        if m.from.is_system() {
            if self.listed(&self.pol.block, &m.from, cards, true) {
                return hold::BLOCKED;
            }
        } else if m.from.kind == crate::BOARD_KIND {
            // Another person or their agent (v0.5 §7.1): the receive
            // setting stands in for the bypass hold (B2).
            let allowed = self.listed(&self.pol.allow, &m.from, cards, false);
            if self.listed(&self.pol.block, &m.from, cards, true) {
                return hold::BLOCKED;
            }
            if self.pol.mode == policy_mode::ALLOWLIST && !allowed {
                return hold::NOT_ALLOWED;
            }
            let h = self.receive_hold(m, recipient, cards);
            if !h.is_empty() {
                return h;
            }
            if m.hop > MAX_HOP {
                return hold::HOP_LIMIT;
            }
        } else if !self.is_person(&m.from) {
            let allowed = self.listed(&self.pol.allow, &m.from, cards, false);
            if self.listed(&self.pol.block, &m.from, cards, true) {
                return hold::BLOCKED;
            }
            if self.pol.mode == policy_mode::ALLOWLIST && !allowed {
                return hold::NOT_ALLOWED;
            }
            if m.hop > MAX_HOP {
                return hold::HOP_LIMIT;
            }
        }
        if self.pol.paused {
            return hold::PAUSED;
        }
        ""
    }

    pub(crate) fn hold_id(&self, msg_id: &str, recipient: &str, cards: &Cards) -> &'static str {
        match self.msgs.get(msg_id) {
            Some(m) => self.hold(m, recipient, cards),
            None => "",
        }
    }

    pub(crate) fn set_policy(&mut self, mut p: Policy, by: &str) -> Result<()> {
        let was_paused = self.pol.paused;
        let row = PolicyRow {
            mode: std::mem::take(&mut p.mode),
            block: std::mem::take(&mut p.block),
            allow: std::mem::take(&mut p.allow),
            paused: p.paused,
            by: by.into(),
            at: self.now(),
        };
        self.emit_local(by, kinds::POLICY_SET, &row)?;
        if !was_paused || row.paused {
            return Ok(());
        }
        // Unpaused: run what's left of the policy over every paused hold.
        let cards = self.live_cards()?;
        for id in self.order.clone() {
            let d = &self.dels[&id];
            if d.d.state != st::HELD || d.d.hold != hold::PAUSED {
                continue;
            }
            if !self.msgs.contains_key(&d.d.message_id) {
                continue;
            }
            let h = self.hold_id(&d.d.message_id, &d.d.to, &cards);
            let r = placed(d.row(), h);
            self.put_delivery(r, by)?;
        }
        Ok(())
    }
}

impl Inbox {
    /// The current policy.
    pub fn policy(&self) -> Policy {
        self.read(|b| norm_policy(b.pol.clone()))
    }

    /// Replaces the policy (the person only; callers enforce that).
    /// Unpausing re-queues paused holds.
    pub fn set_policy(&self, mut p: Policy, by: &str) -> Result<()> {
        match p.mode.as_str() {
            "" => p.mode = policy_mode::EVERYONE.into(),
            policy_mode::EVERYONE | policy_mode::ALLOWLIST | policy_mode::OPEN => {}
            other => {
                return Err(Error::other(format!(
                    "inbox: unknown policy mode {} (everyone, allowlist or open)",
                    gocompat::quote::quote(other)
                )));
            }
        }
        let clean = |v: &[String]| {
            let mut out: Vec<String> = vec![];
            for s in v {
                let s = trim(s);
                if !s.is_empty() && !out.iter().any(|o| o == s) {
                    out.push(s.to_string());
                }
            }
            out
        };
        p.block = clean(&p.block);
        p.allow = clean(&p.allow);
        let r = self.mutate(|b| b.set_policy(p, by));
        self.fill_cards();
        r
    }
}
