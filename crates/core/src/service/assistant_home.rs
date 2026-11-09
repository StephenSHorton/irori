//! The Office Assistant's computer (one-assistant.md §3, card 1): one
//! computer per person runs the assistant, recorded in a private row so it
//! stays the same while that computer is off.
//!
//! Rows are `assistant.home`: `{device, how: "auto" | "person", by, at}`.
//! [`fold`] picks the answer from them, by the store's own order
//! `(hlc, device_id, seq)` and never by `at`, so no clock decides:
//!
//! 1. A person's pick beats any automatic claim; the newest pick wins.
//! 2. Among automatic claims the first wins; a late one never takes over.
//! 3. A row naming a removed device never counts once the device is gone.
//!    A winner naming a removed device ends every row older than that
//!    device's removal, and the fold runs again on what is left.
//! 4. No row left: no assistant's computer yet.
//!
//! The fold never looks at `spawn.host`: a computer whose Claude Code
//! goes missing, or that is offline, stays the assistant's computer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use gocompat::time::GoTime;
use serde::{Deserialize, Serialize};
use store::{Filter, kinds};

use super::Services;
use crate::events::CoreEvent;
use crate::logf;

/// `how` of a claim made by a computer for itself.
pub const HOW_AUTO: &str = "auto";
/// `how` of the person's pick.
pub const HOW_PERSON: &str = "person";

/// An `assistant.home` row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeRow {
    pub device: String,
    #[serde(default)]
    pub how: String,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub at: GoTime,
}

/// A row's place in the log's total order: `(hlc, device_id, seq)`.
pub type Pos = (u64, String, i64);

/// A row as the fold sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub pos: Pos,
    pub device: String,
    pub person: bool,
}

/// The answer of the four rules. `revoked` maps a removed device to the
/// place of its `person.device.revoked` row.
pub fn fold(claims: &[Claim], revoked: &HashMap<String, Pos>) -> Option<String> {
    let mut cutoff: Option<Pos> = None;
    loop {
        let live = || {
            claims.iter().filter(|c| {
                if cutoff.as_ref().is_some_and(|k| c.pos <= *k) {
                    return false;
                }
                // Written after its device was removed: never counts.
                !revoked.get(&c.device).is_some_and(|r| c.pos > *r)
            })
        };
        let winner = live()
            .filter(|c| c.person)
            .max_by(|a, b| a.pos.cmp(&b.pos))
            .or_else(|| live().min_by(|a, b| a.pos.cmp(&b.pos)))?;
        match revoked.get(&winner.device) {
            None => return Some(winner.device.clone()),
            // Removed after it was written: everything older goes too.
            Some(r) => cutoff = Some(r.clone()),
        }
    }
}

/// What the person is told about the recorded computer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomeStatus {
    /// The assistant's computer, "" when there is none yet.
    pub device: String,
    pub name: String,
    /// This device is it.
    pub here: bool,
    /// `connected`, `recent` or `offline`; "" with no computer.
    pub state: String,
    pub last_seen: GoTime,
    /// Its newest `spawn.host` says `assistantTurn`.
    pub claude: bool,
    /// Its newest `spawn.host` says `assistantChat`.
    pub chat: bool,
    /// With no computer: `setting_up`, `needs_update` or `no_claude`.
    pub none: String,
}

/// Called with the new answer when it changes.
pub type HomeListener = Arc<dyn Fn(Option<&str>) + Send + Sync>;

/// The folded answer, cached.
#[derive(Default)]
pub(crate) struct HomeState {
    cached: RwLock<Option<String>>,
    listener: Mutex<Option<HomeListener>>,
    /// One claim or pick is written and folded at a time.
    write: Mutex<()>,
    /// One fold is read and stored at a time, so an older read never
    /// overwrites a newer answer (the kind loop, the sweep and the chat
    /// worker all fold).
    fold: Mutex<()>,
}

impl HomeState {
    pub(crate) fn current(&self) -> Option<String> {
        self.cached
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

fn pos_of(ev: &store::Event) -> Pos {
    (ev.hlc.0, ev.device_id.clone(), ev.seq)
}

impl Services {
    /// The assistant's computer: the fold's answer, cached.
    pub(crate) fn assistant_home(&self) -> Option<String> {
        self.assistant_home.current()
    }

    /// Whether this device is the recorded assistant's computer. It reads
    /// the fold, not the ranking: it never moves for a device that is
    /// offline or a Claude Code that is missing.
    pub(crate) fn is_assistant_home(&self) -> bool {
        self.assistant_home().as_deref() == Some(self.device.id.as_str())
    }

    pub(crate) fn set_assistant_home_listener(&self, l: Option<HomeListener>) {
        *self
            .assistant_home
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = l;
    }

    /// Reads every `assistant.home` and `person.device.revoked` row and
    /// folds them.
    fn fold_assistant_home(&self) -> Option<String> {
        let mut claims: Vec<Claim> = Vec::new();
        let mut f = Filter {
            kinds: vec![kinds::ASSISTANT_HOME.into()],
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        };
        loop {
            let Ok(page) = self.store.list(&f) else {
                // Keep the answer we have: a read that failed says nothing.
                return self.assistant_home();
            };
            for ev in &page {
                // The person's own devices write it as `user`.
                if ev.actor != "user" || !self.chain.is_owner(&format!("user:{}", ev.device_id)) {
                    continue;
                }
                let Ok(r) = serde_json::from_slice::<HomeRow>(&ev.payload) else {
                    continue;
                };
                if r.device.is_empty() || (r.how != HOW_AUTO && r.how != HOW_PERSON) {
                    continue;
                }
                claims.push(Claim {
                    pos: pos_of(ev),
                    device: r.device,
                    person: r.how == HOW_PERSON,
                });
            }
            match page.last() {
                Some(last) if page.len() as i64 >= f.limit => f.after = last.cursor(),
                _ => break,
            }
        }
        if claims.is_empty() {
            return None;
        }
        // Removed devices, by the chain, with the place of the row that
        // removed each (the first, should two exist).
        let gone: std::collections::HashSet<String> = self
            .chain
            .devices()
            .into_iter()
            .filter(|d| d.revoked)
            .map(|d| d.cert.device_id)
            .collect();
        let mut revoked: HashMap<String, Pos> = HashMap::new();
        if !gone.is_empty() {
            let mut f = Filter {
                kinds: vec![kinds::PERSON_DEVICE_REVOKED.into()],
                limit: store::MAX_LIST_LIMIT,
                ..Filter::default()
            };
            loop {
                let Ok(page) = self.store.list(&f) else {
                    return self.assistant_home();
                };
                for ev in &page {
                    let dev = serde_json::from_slice::<serde_json::Value>(&ev.payload)
                        .ok()
                        .and_then(|v| v["revoke"]["deviceId"].as_str().map(str::to_string));
                    let Some(dev) = dev.filter(|d| gone.contains(d)) else {
                        continue;
                    };
                    let p = pos_of(ev);
                    revoked
                        .entry(dev)
                        .and_modify(|cur| {
                            if p < *cur {
                                *cur = p.clone();
                            }
                        })
                        .or_insert(p);
                }
                match page.last() {
                    Some(last) if page.len() as i64 >= f.limit => f.after = last.cursor(),
                    _ => break,
                }
            }
            // A device the chain removed whose row isn't in this log yet
            // is removed from the start: nothing it is named in counts.
            for d in gone {
                revoked.entry(d).or_insert((0, String::new(), 0));
            }
        }
        fold(&claims, &revoked)
    }

    /// Folds again. When the answer changed, tells the listener and the
    /// window (`assistant:home`).
    pub(crate) fn follow_assistant_home(&self) {
        let guard = self
            .assistant_home
            .fold
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = self.fold_assistant_home();
        let changed = {
            let mut cur = self
                .assistant_home
                .cached
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let changed = *cur != now;
            *cur = now.clone();
            changed
        };
        drop(guard);
        if !changed {
            return;
        }
        // Another computer may have answered meanwhile: the next question
        // here starts a new held session (card 8).
        self.chat_session_reset();
        let l = self
            .assistant_home
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(l) = l {
            l(now.as_deref());
        }
        self.hub.publish(CoreEvent::AssistantHome);
        // Questions that waited while another computer was recorded
        // (card 3).
        if now.as_deref() == Some(self.device.id.as_str()) {
            self.chat_scan(super::assistant_turns::Why::Home);
        }
    }

    /// The person picks `device` (Settings, or the away line's one tap),
    /// from any of their devices.
    pub(crate) fn set_assistant_home(&self, device: &str) -> Result<(), String> {
        let revoked = self.chain.cut(device).is_some();
        if device.is_empty() || revoked || self.chain.cert(device).is_none() {
            return Err("that isn't one of your devices".into());
        }
        let _w = self
            .assistant_home
            .write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let row = HomeRow {
            device: device.to_string(),
            how: HOW_PERSON.into(),
            by: format!("user:{}", self.device.id),
            at: (self.now)(),
        };
        self.store
            .append("user", kinds::ASSISTANT_HOME, &row)
            .map_err(|e| e.to_string())?;
        self.follow_assistant_home();
        Ok(())
    }

    /// A computer that can run the assistant claims itself when nothing is
    /// recorded: it says `assistantTurn` and `assistantChat` (card 3). It
    /// only ever names itself, and writes nothing while there is a
    /// recorded computer, even one that has been offline for days.
    pub(crate) fn claim_assistant_home(&self) {
        if self.assistant_home().is_some() {
            return;
        }
        let hosts = self.hosts();
        let me = self.device.id.as_str();
        // Every field the claim needs.
        let able = |h: &super::spawn_remote::SpawnHost| h.assistant_turn && h.assistant_chat;
        if !hosts.get(me).is_some_and(able) {
            return;
        }
        // Today's ranking, over the computers that say what the claim
        // needs (`assistant_home_of` skips the rest), so two connected
        // desktops agree on which of them claims and one left on an older
        // build never blocks an updated one.
        let homes = self
            .spawn
            .agents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .homes
            .clone();
        let hosts: std::collections::HashMap<_, _> =
            hosts.into_iter().filter(|(_, h)| able(h)).collect();
        if super::spawn_capable::assistant_home_of(&homes, &hosts).as_deref() != Some(me) {
            return;
        }
        // Not before the existing claim could have reached this device.
        if !self.mesh.node.caught_up() {
            return;
        }
        let _w = self
            .assistant_home
            .write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // The fold again under the lock: a row may have arrived.
        self.follow_assistant_home();
        if self.assistant_home().is_some() {
            return;
        }
        let row = HomeRow {
            device: me.to_string(),
            how: HOW_AUTO.into(),
            by: format!("user:{me}"),
            at: (self.now)(),
        };
        match self.store.append("user", kinds::ASSISTANT_HOME, &row) {
            Ok(_) => self.follow_assistant_home(),
            Err(e) => logf(format_args!("assistant home: claim: {e}")),
        }
    }

    /// Device `dev`'s newest `spawn.host` row, read for that device alone:
    /// `hosts()` reads only the newest 1,000 rows.
    fn host_of(&self, dev: &str) -> Option<super::spawn_remote::SpawnHost> {
        let evs = self
            .store
            .list(&Filter {
                kinds: vec![kinds::SPAWN_HOST.into()],
                device_id: dev.to_string(),
                desc: true,
                limit: 1,
                ..Filter::default()
            })
            .ok()?;
        let ev = evs.first()?;
        if ev.actor != "irori" {
            return None;
        }
        let h = serde_json::from_slice::<super::spawn_remote::SpawnHost>(&ev.payload).ok()?;
        (h.device == ev.device_id && self.chain.is_owner(&format!("user:{}", h.device)))
            .then_some(h)
    }

    /// What the chat and Settings say about the assistant's computer.
    pub(crate) fn assistant_home_status(&self) -> HomeStatus {
        let labels = self.device_labels();
        let name = |id: &str| {
            labels
                .get(id)
                .filter(|n| !n.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "a computer".to_string())
        };
        let Some(dev) = self.assistant_home() else {
            // What the newest `spawn.host` of each computer says. A
            // computer that says `assistantTurn` and `assistantChat` will
            // claim; one that says only `assistantTurn` runs an Irori that
            // can't answer other devices yet.
            let (mut turn, mut chat) = (false, false);
            for (id, h) in &self.hosts() {
                if self.chain.cut(id).is_some() {
                    continue;
                }
                turn |= h.assistant_turn;
                chat |= h.assistant_turn && h.assistant_chat;
            }
            let none = if chat {
                "setting_up"
            } else if turn {
                "needs_update"
            } else {
                "no_claude"
            };
            return HomeStatus {
                none: none.into(),
                ..HomeStatus::default()
            };
        };
        let p = self.device_presence(&dev);
        let host = self.host_of(&dev);
        HomeStatus {
            name: name(&dev),
            here: dev == self.device.id,
            state: p.state,
            last_seen: p.seen,
            claude: host.as_ref().is_some_and(|h| h.assistant_turn),
            chat: host.as_ref().is_some_and(|h| h.assistant_chat),
            device: dev,
            none: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(h: u64, dev: &str, person: bool, at: i64) -> Claim {
        Claim {
            pos: (h, format!("w{at}"), at),
            device: dev.into(),
            person,
        }
    }

    fn rev(dev: &str, h: u64) -> (String, Pos) {
        (dev.into(), (h, "w".into(), 0))
    }

    #[test]
    fn no_row_means_no_computer() {
        assert_eq!(fold(&[], &HashMap::new()), None);
    }

    #[test]
    fn the_first_automatic_claim_wins() {
        let rows = [c(20, "pc", false, 2), c(10, "mac", false, 1)];
        assert_eq!(fold(&rows, &HashMap::new()).as_deref(), Some("mac"));
    }

    #[test]
    fn a_persons_pick_beats_claims_and_the_newest_pick_wins() {
        let rows = [
            c(10, "mac", false, 1),
            c(20, "pc", true, 2),
            c(30, "mac2", true, 3),
            c(40, "pc", false, 4),
        ];
        assert_eq!(fold(&rows, &HashMap::new()).as_deref(), Some("mac2"));
        // Store order, not `at`: a later row wins whatever it says.
        let rows = [c(30, "pc", true, 1), c(20, "mac", true, 9)];
        assert_eq!(fold(&rows, &HashMap::new()).as_deref(), Some("pc"));
    }

    #[test]
    fn a_row_naming_a_removed_device_never_counts_after_its_removal() {
        let revoked = HashMap::from([rev("pc", 50)]);
        let rows = [c(10, "mac", false, 1), c(60, "pc", true, 2)];
        assert_eq!(fold(&rows, &revoked).as_deref(), Some("mac"));
    }

    #[test]
    fn a_removed_winner_ends_the_rows_older_than_its_removal() {
        // Mac claimed, the person picked the PC, the PC was removed: the
        // Mac's old claim doesn't come back.
        let revoked = HashMap::from([rev("pc", 50)]);
        let rows = [c(10, "mac", false, 1), c(20, "pc", true, 2)];
        assert_eq!(fold(&rows, &revoked), None);
        // Only a claim written after the removal can win.
        let rows = [
            c(10, "mac", false, 1),
            c(20, "pc", true, 2),
            c(70, "pc2", false, 3),
        ];
        assert_eq!(fold(&rows, &revoked).as_deref(), Some("pc2"));
    }
}
