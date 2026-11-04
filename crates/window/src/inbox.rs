//! The person's view of the inbox (I:inbox_service.go). Every action acts
//! as the person at this device (`Core::user_sender`), never as an agent.

use inbox::{Delivery, Policy, st};
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::mentions::MentionOutcome;
use crate::ts;

/// Send's answer: the message, and what each mention came to.
#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct SendResult {
    #[specta(type = ts::Message)]
    pub message: inbox::Message,
    pub mentioned: Vec<MentionOutcome>,
}

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

/// A repo key as a person reads it (card #488): for a local repo
/// (root:<sha>) its checkout folder's name when `root` gives one, else
/// "local repo 5a3c1e0"; for a hosted one its last part without .git
/// ("remote:github.com/o/ww-multiplayer.git" is "ww-multiplayer").
pub use inbox::watch::repo_short_name;

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
    pub fn inbox_send(
        &self,
        to: &str,
        repo: &str,
        text: &str,
        mentions: &[ts::Mention],
    ) -> Result<SendResult, String> {
        let (to, repo) = (trim(to), trim(repo));
        let addr = if !repo.is_empty() {
            inbox::Address::new(inbox::addr::REPO, repo)
        } else if to.is_empty() {
            return Err("pick an agent or a repo to send to".into());
        } else {
            inbox::Address::new(inbox::addr::AGENT, to)
        };
        let plan = self.plan_mentions("", mentions);
        let me = self.core.user_sender();
        let ib = self.core.inbox();
        let (m, d) = ib
            .send_with(
                &me,
                &addr,
                text,
                inbox::SendOptions {
                    mode: inbox::mode::WAKE.into(),
                    mentions: plan.inbox(),
                    ..Default::default()
                },
            )
            .map_err(err)?;
        let rs = ib.send_mention_copies(&me, &m, &d.to, &self.core.agent_name(&d.to));
        Ok(SendResult {
            mentioned: plan.copy_outcomes(&rs, &d.to),
            message: m,
        })
    }

    /// InboxService.Reply: answers a message sent to the person. A reply
    /// about a card also goes on that card's thread.
    pub fn inbox_reply(
        &self,
        id: &str,
        text: &str,
        mentions: &[ts::Mention],
    ) -> Result<Vec<MentionOutcome>, String> {
        self.inbox_reply_files(id, text, mentions, &[])
    }

    /// [`Window::inbox_reply`] with files (card #195), stored here first
    /// as a message's are.
    pub fn inbox_reply_files(
        &self,
        id: &str,
        text: &str,
        mentions: &[ts::Mention],
        files: &[crate::board::UploadFile],
    ) -> Result<Vec<MentionOutcome>, String> {
        self.inbox_reply_mode(id, text, mentions, files, ::inbox::mode::WAKE)
    }

    /// [`Self::inbox_reply_files`] with the reply's mode: `wake`, or `stop`
    /// (card #207): the sender's running turn is cut and this is its next
    /// prompt.
    pub fn inbox_reply_mode(
        &self,
        id: &str,
        text: &str,
        mentions: &[ts::Mention],
        files: &[crate::board::UploadFile],
        md: &str,
    ) -> Result<Vec<MentionOutcome>, String> {
        let ib = self.core.inbox();
        let plan = self.plan_mentions("", mentions);
        let me = self.core.user_sender();
        let refs: Vec<::inbox::FileRef> = self
            .store_uploads(files)?
            .iter()
            .map(crate::inbox_ref)
            .collect();
        let r = ib.reply_files_mode(&me, trim(id), text, &plan.inbox(), &refs, md);
        let mut out = Vec::new();
        if let Ok((m, nd)) = &r
            && !m.mentions.is_empty()
        {
            let rs = ib.send_mention_copies(&me, m, &nd.to, &self.core.agent_name(&nd.to));
            out = plan.copy_outcomes(&rs, &nd.to);
            // On a card the mentioned agents watch it.
            // On a card the mentioned agents watch it, on whichever log
            // holds it (a shared board's too).
            if let Some(tid) = tools::notice::card_for(self.core.coord(), &m.ref_) {
                let mut keys = vec![me.key.clone()];
                keys.extend(m.mentions.iter().filter(|x| !x.gone).map(|x| x.key.clone()));
                if keys.len() > 1
                    && let Err(e) = self
                        .core
                        .task_coord(&tid, &keys)
                        .and_then(|(co, ks)| co.watch_for(&ks[0], &tid, &ks[1..]))
                {
                    irori_core::log(&format!("irori: mentioned agents watch {tid}: {e}"));
                }
            }
        }
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
        self.acted(r.map(|_| ()).map_err(err))?;
        Ok(out)
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

    /// InboxService.StartAgent (card #488): "Start an agent in <repo>" on
    /// the person's own message that no agent in its repo can take yet.
    /// Release can't deliver it (there is nobody to deliver to), so the
    /// message becomes a card in that repo, with its text as the
    /// objective, and an agent is started for it (spawning on), else the
    /// card is handed to a free agent as New card does. The held message
    /// is then dismissed: the card carries it. The answer says which card
    /// ("#12") and which of those happened.
    pub fn inbox_start_agent(&self, id: &str) -> Result<ts::StartedAgent, String> {
        let ib = self.core.inbox();
        let Some(d) = ib.get(trim(id)) else {
            return Err(inbox::ErrorKind::NotFound.text().into());
        };
        if d.state != st::HELD || d.hold != inbox::hold::UNOWNED || d.repo_key.is_empty() {
            return Err(format!(
                "{}: only a message waiting for an agent in its repo starts one",
                inbox::ErrorKind::BadState.text()
            ));
        }
        let me = self.core.user_sender();
        if d.message.from.key != me.key && !d.message.from.key.starts_with("user:") {
            return Err(format!(
                "{}: only your own message starts an agent",
                inbox::ErrorKind::BadState.text()
            ));
        }
        let name = self.repo_name(&d.repo_key);
        let co = self.core.coord().clone();
        let t = co
            .create_task(
                &me.key,
                coord::NewTask {
                    title: format!("Your message for {name}"),
                    objective: d.message.text.clone(),
                    repo_key: d.repo_key.clone(),
                    ..coord::NewTask::default()
                },
            )
            .map_err(|e| e.to_string())?;
        let mut out = ts::StartedAgent {
            repo: name,
            ..ts::StartedAgent::default()
        };
        if co.spawning() {
            self.core.start_agent(&t.id, "")?;
            out.started = true;
        } else {
            let n = co
                .offer_card(&me.key, &t.id, &self.core.ask_agents())
                .map_err(|e| e.to_string())?;
            out.offered = !n.to.is_empty();
            self.tell_from(&inbox::system_sender(), &n);
        }
        let r = ib.dismiss(&d.id, &self.by()).map_err(err);
        self.acted(r)?;
        out.card_ref = co.get_task(&t.id).unwrap_or(t).short_ref();
        Ok(out)
    }

    /// InboxService.RepoNames (card #488): every repo this device has a
    /// checkout of, by its short name: the checkout folder's name for a
    /// local repo (root:<sha>), the repo's own name for a hosted one.
    pub fn inbox_repo_names(&self) -> Vec<ts::RepoName> {
        self.core
            .capture()
            .repos()
            .into_iter()
            .map(|(key, roots)| ts::RepoName {
                name: repo_short_name(&key, roots.first().map(String::as_str)),
                repo_key: key,
            })
            .collect()
    }

    /// The short name of repo `key` here (see [`repo_short_name`]).
    pub(crate) fn repo_name(&self, key: &str) -> String {
        let roots = self.core.capture().repo_roots(key);
        repo_short_name(key, roots.first().map(String::as_str))
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
                st::HELD if inbox::editing(&d) => {}
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

#[cfg(test)]
mod tests {
    use super::repo_short_name;

    #[test]
    fn repo_short_names() {
        let k = "root:5a3c1e0b9d7f2a4c6e8b";
        assert_eq!(
            repo_short_name(k, Some("/Users/s/projects/ww-multiplayer")),
            "ww-multiplayer"
        );
        assert_eq!(
            repo_short_name(k, Some("C:\\Users\\s\\ww-multiplayer\\")),
            "ww-multiplayer"
        );
        assert_eq!(repo_short_name(k, None), "local repo 5a3c1e0");
        assert_eq!(
            repo_short_name("remote:github.com/o/irori.git", Some("/x/fork")),
            "irori"
        );
    }
}
