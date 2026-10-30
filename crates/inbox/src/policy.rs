//! The wake policy (I:internal/inbox/policy.go): block and allow lists,
//! bypass_receiver with the own-agent exemption (decision 34), Full auto
//! (decision 36), the hop limit and Pause all.

use std::collections::BTreeMap;

use gocompat::text;
use irori_identity::owner::device_of;

use crate::state::{DeliveryRow, Inner, PolicyRow, lower_trim, norm_policy, trim};
use crate::{
    AgentCard, BYPASS_MODE, Error, Inbox, MAX_HOP, Message, Policy, Result, Sender, hold, kinds,
    policy_mode, st,
};

pub(crate) type Cards = BTreeMap<String, AgentCard>;

/// Whether key runs with bypassed permissions (Grok and people never do).
pub(crate) fn bypass(key: &str, cards: &Cards) -> bool {
    cards
        .get(key)
        .is_some_and(|c| c.agent != "grok" && c.permission_mode == BYPASS_MODE)
}

/// m's sender key names the device that wrote it.
pub(crate) fn from_writer(m: &Message) -> bool {
    let dev = device_of(&m.from.key);
    !dev.is_empty() && dev == m.writer
}

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

    pub(crate) fn revoked(&self, dev: &str) -> bool {
        self.revoked_fn.as_ref().is_some_and(|f| f(dev))
    }

    /// One of the person's own Claude agents (decision 34).
    pub(crate) fn own_agent(&self, m: &Message, cards: &Cards) -> bool {
        let s = &m.from;
        if s.kind != "claude" || s.key.starts_with("user:") {
            return false;
        }
        if cards.get(&s.key).is_some_and(|c| c.agent == "grok") {
            return false;
        }
        let dev = device_of(&s.key);
        if !from_writer(m) {
            return false;
        }
        if dev == self.dev {
            return true;
        }
        self.is_owner(&format!("user:{dev}")) && !self.revoked(dev)
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
        } else if !self.is_person(&m.from) {
            let allowed = self.listed(&self.pol.allow, &m.from, cards, false);
            if self.listed(&self.pol.block, &m.from, cards, true) {
                return hold::BLOCKED;
            }
            if self.pol.mode == policy_mode::ALLOWLIST && !allowed {
                return hold::NOT_ALLOWED;
            }
            if self.pol.mode != policy_mode::OPEN
                && bypass(recipient, cards)
                && !allowed
                && !self.own_agent(m, cards)
                && (m.from.kind == "grok" || !from_writer(m) || !bypass(&m.from.key, cards))
            {
                return hold::BYPASS_RECEIVER;
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
