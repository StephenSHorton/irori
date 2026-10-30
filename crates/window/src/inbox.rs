//! The person's view of the inbox (I:inbox_service.go). Every action acts
//! as the person at this device (`Core::user_sender`), never as an agent.

use inbox::{Delivery, Message, Policy, st};
use serde::Serialize;
use specta::Type;

use crate::Window;

/// List's and HeldHistory's cap.
pub const MAX_LIST: usize = 500;
/// The List limit for a pass over every delivery.
const SCAN_ALL: usize = 1 << 30;

/// The inbox:changed payload and the tray's numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
pub struct InboxCounts {
    pub unread: i64,
    pub held: i64,
    pub paused: bool,
}

/// One pass over every delivery: the counts and the held deliveries.
#[derive(Debug, Clone, Default)]
pub(crate) struct Snapshot {
    pub counts: InboxCounts,
    pub held: Vec<Delivery>,
}

fn trim(s: &str) -> &str {
    crate::trim(s)
}

fn err(e: inbox::Error) -> String {
    e.to_string()
}

impl Window {
    /// InboxService.List: recent deliveries, newest first.
    pub fn inbox_list(&self, limit: i64) -> Vec<Delivery> {
        let limit = if limit <= 0 {
            inbox::DEFAULT_INBOX_LIST
        } else {
            limit as usize
        };
        self.core.inbox().list(limit.min(MAX_LIST))
    }

    /// InboxService.Held: every delivery waiting on the person.
    pub fn inbox_held(&self) -> Vec<Delivery> {
        self.snapshot().held
    }

    /// InboxService.HeldHistory.
    pub fn inbox_held_history(&self, limit: i64) -> Vec<Delivery> {
        let limit = if limit <= 0 {
            inbox::DEFAULT_INBOX_LIST
        } else {
            limit as usize
        };
        self.core.inbox().held_history(limit.min(MAX_LIST))
    }

    /// InboxService.Counts.
    pub fn inbox_counts(&self) -> InboxCounts {
        self.snapshot().counts
    }

    /// InboxService.Get: one delivery, for the window's message dialog.
    pub fn inbox_get(&self, id: &str) -> Result<Delivery, String> {
        let id = trim(id);
        self.core.inbox().get(id).ok_or_else(|| {
            format!(
                "{}: delivery {}",
                inbox::ErrorKind::NotFound.text(),
                gocompat::quote::quote(id)
            )
        })
    }

    /// InboxService.Send: text from the person to an agent or a repo.
    pub fn inbox_send(&self, to: &str, repo: &str, text: &str) -> Result<Message, String> {
        let (to, repo) = (trim(to), trim(repo));
        let addr = if !repo.is_empty() {
            inbox::Address::new(inbox::addr::REPO, repo)
        } else if to.is_empty() {
            return Err("pick an agent or a repo to send to".into());
        } else {
            inbox::Address::new(inbox::addr::AGENT, to)
        };
        let (m, _) = self
            .core
            .inbox()
            .send(&self.core.user_sender(), &addr, text, "")
            .map_err(err)?;
        Ok(m)
    }

    /// InboxService.Reply: answers a message sent to the person. A reply
    /// about a card also goes on that card's thread.
    pub fn inbox_reply(&self, id: &str, text: &str) -> Result<(), String> {
        let ib = self.core.inbox();
        let r = ib.reply(&self.core.user_sender(), trim(id), text);
        if let Ok((m, nd)) = &r
            && !m.ref_.is_empty()
        {
            let rec = tools::notice::Record {
                held: tools::notice::held_on_card(ib, nd),
                ..Default::default()
            };
            match tools::notice::record_on_card(self.core.coord(), m, &nd.to, &rec) {
                Ok(_) | Err(tools::notice::RecordError::NoCard) => {}
                Err(tools::notice::RecordError::Coord(e)) if e.is(coord::ErrorKind::NotFound) => {}
                Err(e) => irori_core::log(&format!("irori: put the reply on {}: {e}", m.ref_)),
            }
        }
        self.acted(r.map(|_| ()).map_err(err))
    }

    /// InboxService.MarkRead: closes a message sent to the person.
    pub fn inbox_mark_read(&self, id: &str) -> Result<(), String> {
        let r = self
            .core
            .inbox()
            .skip(&self.by(), trim(id), "read in the Irori window")
            .map_err(err);
        self.acted(r)
    }

    /// InboxService.Release.
    pub fn inbox_release(&self, id: &str) -> Result<(), String> {
        let r = self.core.inbox().release(id, &self.by()).map_err(err);
        self.acted(r)
    }

    /// InboxService.Dismiss.
    pub fn inbox_dismiss(&self, id: &str) -> Result<(), String> {
        let r = self.core.inbox().dismiss(id, &self.by()).map_err(err);
        self.acted(r)
    }

    /// InboxService.AllowAndRelease: "Allow sender & deliver".
    pub fn inbox_allow_and_release(&self, id: &str) -> Result<(), String> {
        let ib = self.core.inbox();
        let Some(d) = ib.get(id) else {
            return Err(inbox::ErrorKind::NotFound.text().into());
        };
        if d.state != st::HELD {
            return Err(inbox::ErrorKind::BadState.text().into());
        }
        if d.hold != inbox::hold::NOT_ALLOWED && d.hold != inbox::hold::BYPASS_RECEIVER {
            return Err(format!(
                "{}: allowing the sender doesn't lift a {} hold",
                inbox::ErrorKind::BadState.text(),
                gocompat::quote::quote(&d.hold)
            ));
        }
        let key = d.message.from.key.clone();
        if key.is_empty() {
            return Err("that delivery has no sender key".into());
        }
        let mut p = ib.policy();
        if !p
            .allow
            .iter()
            .any(|e| gocompat::text::equal_fold(trim(e), &key))
        {
            p.allow.push(key);
            self.inbox_set_policy(p)?;
        }
        self.inbox_release(id)
    }

    /// InboxService.Policy.
    pub fn inbox_policy(&self) -> Policy {
        self.core.inbox().policy()
    }

    /// InboxService.SetPolicy: replaces the wake policy as the person.
    pub fn inbox_set_policy(&self, p: Policy) -> Result<(), String> {
        let r = self.core.inbox().set_policy(p, &self.by()).map_err(err);
        self.acted(r)?;
        self.nudge_inbox();
        Ok(())
    }

    /// InboxService.SetPaused: pause-all on or off.
    pub fn inbox_set_paused(&self, paused: bool) -> Result<(), String> {
        let mut p = self.core.inbox().policy();
        if p.paused == paused {
            return Ok(());
        }
        p.paused = paused;
        self.inbox_set_policy(p)
    }

    fn by(&self) -> String {
        self.core.user_sender().key
    }

    /// Drops the snapshot after the person's own change succeeded.
    fn acted(&self, r: Result<(), String>) -> Result<(), String> {
        if r.is_ok() {
            *self.inbox_snap.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        r
    }

    fn snapshot(&self) -> Snapshot {
        if let Some(s) = self
            .inbox_snap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return s;
        }
        self.refresh_inbox()
    }

    /// Scans every delivery once and keeps the result until the next
    /// change.
    pub(crate) fn refresh_inbox(&self) -> Snapshot {
        let ib = self.core.inbox();
        let mut snap = Snapshot {
            counts: InboxCounts {
                paused: ib.policy().paused,
                ..InboxCounts::default()
            },
            held: Vec::new(),
        };
        for d in ib.list(SCAN_ALL) {
            match d.state.as_str() {
                st::QUEUED | st::DELIVERED => snap.counts.unread += 1,
                st::HELD => {
                    snap.counts.held += 1;
                    snap.held.push(d);
                }
                _ => {}
            }
        }
        *self.inbox_snap.lock().unwrap_or_else(|e| e.into_inner()) = Some(snap.clone());
        snap
    }

    /// Asks the inbox pump to emit, for changes that may not reach the
    /// inbox's subscription (a policy write that re-queues nothing).
    fn nudge_inbox(&self) {
        self.inbox_kick.notify();
    }
}
