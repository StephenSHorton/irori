//! Routing (I:internal/inbox/route.go): the live roster, resolving an
//! agent address, reachability and the repo front desk.

use std::collections::BTreeSet;

use gocompat::quote::quote;
use gocompat::time::GoTime;

use crate::policy::Cards;
use crate::state::{Inner, NameRow, ReachRec, lower_trim};
use crate::{ChangeKind, Error, ErrorKind, Inbox, MIN_KEY_PREFIX_LEN, Result, kinds, path, tm};

/// The session id of `<device>/<session>`.
pub(crate) fn session_part(key: &str) -> &str {
    match key.rfind('/') {
        Some(i) => &key[i + 1..],
        None => key,
    }
}

impl Inner {
    /// The roster minus gone cards; a card name not yet recorded for its key
    /// is logged (agent.name) so it stays addressable after the agent goes.
    pub(crate) fn live_cards(&mut self) -> Result<Cards> {
        let out = self.roster_cards();
        for (k, c) in &out {
            let name = lower_trim(&c.name);
            if self.known.get(k).is_some_and(|n| *n == name) {
                continue;
            }
            let row = NameRow {
                key: k.clone(),
                name,
                at: self.now(),
            };
            self.emit("irori", kinds::AGENT_NAME, &row)?;
        }
        Ok(out)
    }

    /// `name (key)` for error text.
    pub(crate) fn describe(&self, key: &str, cards: &Cards) -> String {
        let name = if let Some(c) = cards.get(key) {
            c.name.clone()
        } else if let Some(n) = self.known.get(key).filter(|n| !n.is_empty()) {
            n.clone()
        } else if let Some(a) = self.aliases.get(key).and_then(|a| a.last()) {
            a.clone()
        } else {
            String::new()
        };
        if name.is_empty() {
            key.to_string()
        } else {
            format!("{name} ({key})")
        }
    }

    /// A key, key prefix, name or alias as one agent key.
    pub(crate) fn resolve_agent(&self, me: &str, value: &str, cards: &Cards) -> Result<String> {
        let wanted = lower_trim(value);
        if wanted.is_empty() {
            return Err(Error::wrap(ErrorKind::UnknownAgent, "say which agent (to)"));
        }
        let mut all: BTreeSet<&str> = BTreeSet::new();
        all.extend(cards.keys().map(String::as_str));
        all.extend(self.known.keys().map(String::as_str));
        for k in self.aliases.keys() {
            if !self.is_owner(k) {
                all.insert(k);
            }
        }
        let pick = |mut matches: Vec<&str>| -> Result<String> {
            matches.sort();
            if matches.len() == 1 && matches[0] == me {
                return Err(Error::sentinel(ErrorKind::SelfAddress));
            }
            matches.retain(|k| *k != me);
            match matches.len() {
                0 => Ok(String::new()),
                1 => Ok(matches[0].to_string()),
                n => {
                    let names: Vec<String> =
                        matches.iter().map(|k| self.describe(k, cards)).collect();
                    Err(Error::wrap(
                        ErrorKind::Ambiguous,
                        format!(
                            "{} matches {n} agents: {}; use a key",
                            quote(value),
                            names.join(", ")
                        ),
                    ))
                }
            }
        };
        let lower = gocompat::text::to_lower_str;

        // 1. Exact key.
        for k in &all {
            if lower(k) == wanted {
                return pick(vec![k]);
            }
        }
        // 2. Unique key prefix.
        if wanted.len() >= MIN_KEY_PREFIX_LEN {
            let m: Vec<&str> = all
                .iter()
                .copied()
                .filter(|k| {
                    lower(k).starts_with(&wanted) || lower(session_part(k)).starts_with(&wanted)
                })
                .collect();
            let key = pick(m)?;
            if !key.is_empty() {
                return Ok(key);
            }
        }
        // 3. Card name, then alias; live before gone within each.
        let card_named = |k: &str| match cards.get(k) {
            Some(c) => lower_trim(&c.name) == wanted,
            None => self.known.get(k).is_some_and(|n| *n == wanted),
        };
        let aliased = |k: &str| self.aliases.get(k).is_some_and(|a| a.contains(&wanted));
        let mut saw_self = false;
        for named in [&card_named as &dyn Fn(&str) -> bool, &aliased] {
            let (mut live, mut gone) = (vec![], vec![]);
            for k in &all {
                if !named(k) {
                } else if *k == me {
                    saw_self = true;
                } else if cards.get(*k).is_some_and(|c| !c.key.is_empty()) {
                    live.push(*k);
                } else {
                    gone.push(*k);
                }
            }
            for m in [live, gone] {
                let key = pick(m)?;
                if !key.is_empty() {
                    return Ok(key);
                }
            }
        }
        if saw_self {
            return Err(Error::sentinel(ErrorKind::SelfAddress));
        }
        let mut here: Vec<&str> = cards
            .iter()
            .filter(|(k, _)| *k != me)
            .map(|(_, c)| c.name.as_str())
            .collect();
        here.sort();
        let list = if here.is_empty() {
            "none".to_string()
        } else {
            here.join(", ")
        };
        Err(Error::wrap(
            ErrorKind::UnknownAgent,
            format!("no agent called {}; agents here now: {list}", quote(value)),
        ))
    }

    /// A wake path, or a mod path dropped less than the reattach grace ago.
    pub(crate) fn reachable(&self, agent: &str, now: &GoTime) -> bool {
        let Some(r) = self.reach.get(agent) else {
            return false;
        };
        r.channel
            || r.md
            || (!r.mod_lost.is_zero() && tm::sub(now, &r.mod_lost) < self.lim.reattach)
    }

    pub(crate) fn set_reachable(&mut self, agent: &str, p: &str, on: bool) {
        if agent.is_empty() {
            return;
        }
        let now = self.now();
        let was = self.reachable(agent, &now);
        let r = self.reach.entry(agent.to_string()).or_default();
        match p {
            path::MOD => {
                if r.md && !on {
                    r.mod_lost = now;
                }
                r.md = on;
                if on {
                    r.mod_lost = GoTime::ZERO;
                }
            }
            path::CHANNEL => r.channel = on,
            _ => return,
        }
        if on && !was {
            r.first_at = now;
        }
        self.touch(&[agent]);
        self.note(ChangeKind::Updated, agent, "");
    }

    /// Front-desk candidates for repo, best first.
    pub(crate) fn desks(&self, repo: &str, cards: &Cards, now: &GoTime) -> Vec<String> {
        if repo.is_empty() {
            return vec![];
        }
        let mut out: Vec<String> = cards
            .iter()
            .filter(|(k, c)| c.repo_key == repo && self.reachable(k, now))
            .map(|(k, _)| k.clone())
            .collect();
        let first = |k: &str| self.reach.get(k).map(|r| r.first_at).unwrap_or_default();
        out.sort_by(|x, y| tm::cmp(&first(x), &first(y)).then_with(|| x.cmp(y)));
        out
    }

    /// The first desk candidate not in exclude, or `""`.
    pub(crate) fn desk(&self, repo: &str, cards: &Cards, now: &GoTime, exclude: &[&str]) -> String {
        self.desks(repo, cards, now)
            .into_iter()
            .find(|k| !exclude.contains(&k.as_str()))
            .unwrap_or_default()
    }
}

impl Inbox {
    /// Records that agent can (on) or can no longer be woken by path.
    pub fn set_reachable(&self, agent: &str, p: &str, on: bool) {
        self.mutate(|b| b.set_reachable(agent, p, on));
    }

    /// The wake paths agent has open now: (mod, channel).
    pub fn reachable(&self, agent: &str) -> (bool, bool) {
        self.read(|b| {
            b.reach
                .get(agent)
                .map(|r: &ReachRec| (r.md, r.channel))
                .unwrap_or((false, false))
        })
    }

    /// Logs the live roster's names the way every send does.
    pub fn record_roster(&self) -> Result<()> {
        self.mutate(|b| b.live_cards().map(|_| ()))
    }

    /// The agent repo work for repo_key goes to now, or `""`.
    pub fn front_desk(&self, repo_key: &str) -> String {
        self.read(|b| b.desk(repo_key, &b.roster_cards(), &b.now(), &[]))
    }

    /// How long agent has been out of presence, as the sweeper last saw it.
    pub fn ended_for(&self, agent: &str) -> std::time::Duration {
        self.read(|b| {
            let n = b.ended_for(agent, &b.now());
            std::time::Duration::from_nanos(n.max(0) as u64)
        })
    }
}
