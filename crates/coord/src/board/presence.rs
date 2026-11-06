//! Telling agents when an agent they work with drops off (card #372): a
//! fellow holder of an open card, or an agent one of their cards waits on
//! (a blocker on it, a review asked of it). Once that agent's device has
//! read `offline` for [`PRESENCE_GRACE`], each such agent this device
//! hosts hears it once; when the device is back it hears that once too.
//! Nothing is written: the state lives in memory, and a restart starts
//! it again.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use gocompat::time::GoTime;

use super::fold::Fold;
use super::types::{Notice, is_terminal};
use super::util::SWEEPER;
use crate::gotime;
use crate::roster::{AgentCard, OFFLINE, device_phrase};

/// How long an agent's device reads offline before the agents working
/// with it hear so.
pub const PRESENCE_GRACE: Duration = Duration::from_secs(120);

/// The notice kind for both the offline and the back notice.
pub const NOTICE_PRESENCE: &str = "presence";

/// One watched agent whose device reads offline.
#[derive(Debug, Clone, Default)]
pub(crate) struct Away {
    /// When this device first saw it offline.
    since: GoTime,
    /// The agents told it went offline: they hear when it's back.
    told: BTreeSet<String>,
    /// They heard it's reachable through another device (card #544). Such a
    /// device reads offline whenever it writes nothing for a while, so it
    /// stays here: they aren't told or woken on every quiet spell, and hear
    /// once more when it has a link of its own.
    relayed: bool,
}

/// Why `watcher` cares about `watched`, per card.
#[derive(Default)]
struct Ties {
    /// Cards they both hold.
    shared: BTreeSet<String>,
    /// Cards of the watcher's that wait on the watched one.
    waits: BTreeSet<String>,
}

impl Fold {
    /// Every (watcher, watched) pair on open cards: fellow holders, and a
    /// holder with the agent a blocker (`on agent …`) or a review waits on.
    fn presence_ties(&self, agents: &[AgentCard]) -> BTreeMap<(String, String), Ties> {
        let by_name = |r: &str| -> Option<String> {
            let r = r.trim();
            agents
                .iter()
                .find(|a| a.key == r)
                .or_else(|| agents.iter().find(|a| !a.name.is_empty() && a.name == r))
                .map(|a| a.key.clone())
        };
        let mut out: BTreeMap<(String, String), Ties> = BTreeMap::new();
        for t in self.tasks.values() {
            if is_terminal(&t.state) || t.ask.is_some() {
                continue;
            }
            let mut holders: Vec<&String> = t.holders.iter().chain(&t.co_holders).collect();
            holders.sort();
            holders.dedup();
            let r = t.short_ref();
            for w in &holders {
                for x in &holders {
                    if w != x {
                        out.entry(((*w).clone(), (*x).clone()))
                            .or_default()
                            .shared
                            .insert(r.clone());
                    }
                }
            }
            let mut waited: Vec<String> = Vec::new();
            for b in &t.blockers {
                if b.resolved.is_some() {
                    continue;
                }
                if let Some(on) = &b.on
                    && on.kind == "agent"
                    && let Some(k) = by_name(&on.reference)
                {
                    waited.push(k);
                }
            }
            for rv in self.reviews_of(&t.id) {
                if rv.state == super::review::REVIEW_REQUESTED {
                    waited.push(rv.reviewer.clone());
                }
            }
            for x in waited {
                for w in &holders {
                    if **w != x {
                        out.entry(((*w).clone(), x.clone()))
                            .or_default()
                            .waits
                            .insert(r.clone());
                    }
                }
            }
        }
        out
    }

    /// The offline and back notices due at `now`, for the agents `hosted`
    /// says this device hosts (so each is told once across the person's
    /// devices), from `agents`' device states.
    pub(crate) fn sweep_presence(
        &mut self,
        agents: &[AgentCard],
        now: &GoTime,
        hosted: &dyn Fn(&str) -> bool,
    ) -> Vec<Notice> {
        let by_key: HashMap<&str, &AgentCard> =
            agents.iter().map(|a| (a.key.as_str(), a)).collect();
        let off = |k: &str| {
            by_key
                .get(k)
                .is_some_and(|a| a.device_state == OFFLINE && a.status != "gone")
        };
        let mut out = Vec::new();
        // Back: told agents hear it once their device no longer reads
        // offline (or it went: the card comes back by itself then).
        let back: Vec<String> = self.away.keys().filter(|k| !off(k)).cloned().collect();
        for k in back {
            let gone = by_key.get(k.as_str()).is_none_or(|c| c.status == "gone");
            let via = by_key.get(k.as_str()).map_or("", |c| c.device_via.as_str());
            let name = self.named(&k);
            let back_notice = |to: BTreeSet<String>, how: &str| Notice {
                kind: NOTICE_PRESENCE.into(),
                reference: agent_ref(&k),
                title: format!("{name} is back"),
                by: SWEEPER.into(),
                to: to.into_iter().collect(),
                text: format!("{name}'s device is {how}."),
                ..Notice::default()
            };
            // Rows through another device aren't a link of its own.
            if !gone
                && !via.is_empty()
                && let Some(a) = self.away.get_mut(&k)
                && !a.told.is_empty()
            {
                a.since = *now;
                if !a.relayed {
                    a.relayed = true;
                    let how = format!("reachable again, through {via}");
                    out.push(back_notice(a.told.clone(), &how));
                }
                continue;
            }
            let Some(a) = self.away.remove(&k) else {
                continue;
            };
            if !gone && !a.told.is_empty() {
                out.push(back_notice(a.told, "back online"));
            }
        }
        for a in agents {
            if off(&a.key) {
                self.away.entry(a.key.clone()).or_insert_with(|| Away {
                    since: *now,
                    ..Away::default()
                });
            }
        }
        if self.away.is_empty() {
            return out;
        }
        let ties = self.presence_ties(agents);
        let due: Vec<String> = self
            .away
            .iter()
            .filter(|(k, a)| off(k) && gotime::at_least(now, &a.since, PRESENCE_GRACE))
            .map(|(k, _)| k.clone())
            .collect();
        for x in due {
            let card = by_key[x.as_str()];
            let phrase =
                device_phrase(&card.device_state, &card.device_seen, &card.device_via, now);
            for ((w, wx), tie) in &ties {
                if *wx != x
                    || !hosted(w)
                    || !by_key.get(w.as_str()).is_some_and(|c| c.status != "gone")
                    || self.away[&x].told.contains(w)
                {
                    continue;
                }
                let mut why = Vec::new();
                if !tie.shared.is_empty() {
                    why.push(format!(
                        "You hold {} with it.",
                        tie.shared.iter().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                if !tie.waits.is_empty() {
                    why.push(format!(
                        "Your {} waits on it.",
                        tie.waits.iter().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                let first = tie.waits.iter().chain(&tie.shared).next().cloned();
                let task_id = first
                    .and_then(|r| self.tasks.values().find(|t| t.short_ref() == r))
                    .map(|t| t.id.clone())
                    .unwrap_or_default();
                out.push(Notice {
                    kind: NOTICE_PRESENCE.into(),
                    task_id: task_id.clone(),
                    reference: if task_id.is_empty() {
                        agent_ref(&x)
                    } else {
                        format!("task:{task_id}")
                    },
                    title: format!("{}'s device is offline", self.named(&x)),
                    by: SWEEPER.into(),
                    to: vec![w.clone()],
                    text: format!(
                        "{}'s device is {phrase}. {} Messages to it wait until it's heard from again; you'll hear when it is.",
                        self.named(&x),
                        why.join(" ")
                    ),
                    wake: !tie.waits.is_empty(),
                    ..Notice::default()
                });
                if let Some(a) = self.away.get_mut(&x) {
                    a.told.insert(w.clone());
                }
            }
        }
        out
    }
}

/// An agent's ref for `show`: `agent:<key>`; a board key is one already.
fn agent_ref(k: &str) -> String {
    if k.starts_with(store::kinds::AGENT_ACTOR_PREFIX) {
        k.to_string()
    } else {
        format!("agent:{k}")
    }
}
