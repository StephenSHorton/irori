//! Talk (card #389; docs/design/talk-channels.md): the person's channels,
//! DMs and threads, read and written through the Core's Talk calls. A
//! thread's posts are its comments, so posting to one is the comment box's
//! call ([`Window::comment`]).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::ts;

/// One side of a conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkMember {
    /// `person` or `agent`.
    pub kind: String,
    pub key: String,
    /// What it goes by ("you" for the person). Never a key (card #478):
    /// "BillingEng1 · Spencer" for another member's agent, "unknown agent"
    /// when nothing names it.
    #[serde(default)]
    pub name: String,
    /// Another member's agent's board status (`busy`, `idle`, `gone`); ""
    /// when this device knows none, which isn't gone.
    #[serde(default)]
    pub status: String,
    /// Whose agent it is when it is another member's ("Spencer"), else "".
    #[serde(default)]
    pub owner: String,
}

/// Who to add to or take out of a conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkMemberRef {
    /// `person` or `agent`.
    pub kind: String,
    pub key: String,
}

/// One conversation in Talk's list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkConversation {
    pub conv_id: String,
    /// `thread`, `channel` or `dm`.
    pub kind: String,
    pub name: String,
    #[specta(optional)]
    pub topic: Option<String>,
    pub unread: i64,
    pub mentions: i64,
    pub members: Vec<TalkMember>,
    /// The newest post's time (the zero time when it has none).
    #[specta(type = ts::Time)]
    pub last_at: gocompat::time::GoTime,
    /// The newest post's text on one line, clipped (card #486): the list's
    /// subtitle. "" when it has none.
    pub last_text: String,
    /// Who wrote the newest post: a member's key, "" when none.
    pub last_by: String,
    /// A DM between two agents: the person reads it only.
    pub read_only: bool,
    /// A thread's "Agents answer now" (card #391): its toggle. False for a
    /// channel or DM, whose composer starts from the default for what the
    /// text names.
    pub agents_now: bool,
    /// The shared space a channel or thread is in (card #461, #558); absent
    /// for Personal and DMs. Matches the switcher's space id (its first
    /// board's).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub space_id: String,
    /// A DM with another person that hasn't reached them yet (card #538):
    /// `them` ("Waiting for Bob's Irori") or `update` ("Bob's Irori needs
    /// updating"). Absent otherwise. `name` is who they are.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub waiting: String,
}

/// Someone the person can DM (card #538): the new-DM picker's people.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkPerson {
    /// `person:<id>`, as talk_dm takes it with kind `person`.
    pub key: String,
    pub name: String,
    /// Their own colour pick, "" for none.
    pub color: String,
    /// Their avatar, "" for none.
    pub avatar: String,
}

/// One post.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TalkPost {
    pub post_id: String,
    pub conv_id: String,
    pub by: String,
    pub text: String,
    pub mentions: Vec<String>,
    pub agents_now: bool,
    #[specta(optional)]
    pub thread_from: Option<String>,
    /// The card made from it (always null in Talk 1).
    #[specta(optional)]
    pub card: Option<String>,
    #[specta(type = ts::Time)]
    pub at: gocompat::time::GoTime,
}

/// The most posts one page returns, and the default.
const MAX_LIMIT: i64 = coord::talk::MAX_PAGE as i64;
const DEFAULT_LIMIT: i64 = 50;

fn is_thread_id(conv: &str) -> bool {
    !coord::talk::is_channel(conv) && !coord::talk::is_dm(conv)
}

impl Window {
    fn me(&self) -> String {
        self.core.user_sender().key
    }

    /// Talk's list: every open channel, every DM (agents' own read-only)
    /// and every open thread, newest first.
    pub fn talk_conversations(&self) -> Result<Vec<TalkConversation>, String> {
        let cs = self.core.talk_conversations(&self.me())?;
        Ok(cs
            .into_iter()
            .map(|c| TalkConversation {
                conv_id: c.conv_id,
                kind: c.kind,
                name: c.name,
                topic: (!c.topic.is_empty()).then_some(c.topic),
                unread: c.unread as i64,
                mentions: c.mentions as i64,
                members: c
                    .members
                    .into_iter()
                    .map(|m| TalkMember {
                        kind: m.kind,
                        key: m.key,
                        name: m.name,
                        status: m.status,
                        owner: m.owner,
                    })
                    .collect(),
                last_at: c.last_at,
                last_text: c.last_text,
                last_by: c.last_by,
                read_only: c.read_only,
                agents_now: c.agents_now,
                space_id: c.space_id,
                waiting: c.waiting,
            })
            .collect())
    }

    /// Up to `limit` posts of `conv` before post `before` (none: the
    /// newest), oldest first.
    pub fn talk_posts(
        &self,
        conv: &str,
        before: Option<&str>,
        limit: i64,
    ) -> Result<Vec<TalkPost>, String> {
        let limit = if limit <= 0 {
            DEFAULT_LIMIT
        } else {
            limit.min(MAX_LIMIT)
        };
        let ps = self
            .core
            .talk_posts(&self.me(), conv, before, limit as usize)?;
        Ok(ps
            .into_iter()
            .map(|p| TalkPost {
                post_id: p.post_id,
                conv_id: p.conv_id,
                by: p.by,
                text: p.text,
                mentions: p.mentions,
                agents_now: p.agents_now,
                thread_from: p.thread_from,
                card: p.card,
                at: p.at,
            })
            .collect())
    }

    /// Posts as the person; returns the post's id. A thread's post is a
    /// comment on it (agents woken when `agents_now`, else told at their
    /// next turn); a channel's or DM's is a `talk.post` (a DM with an agent
    /// wakes it, an @name in the text wakes that agent). `mentions` are
    /// the keys the composer's picker recorded: one names exactly the agent
    /// picked, another person's in a space too (card #461).
    pub fn talk_post(
        &self,
        conv: &str,
        text: &str,
        mentions: &[String],
        agents_now: bool,
        thread_from: Option<String>,
    ) -> Result<String, String> {
        let conv = crate::trim(conv);
        if conv.is_empty() {
            return Err("pick a conversation".into());
        }
        if is_thread_id(conv) {
            let tell = if agents_now { "wake" } else { "next_turn" };
            let r = self.comment(conv, text, tell, &[], &[])?;
            return Ok(r.comment.id);
        }
        let p = self.core.talk_post(
            &self.core.user_sender(),
            conv,
            text,
            mentions,
            agents_now,
            thread_from,
        )?;
        Ok(p.post.post_id)
    }

    /// The person asks about channel `conv`'s post `post_id` in a new
    /// thread (design b2, card #454): the thread records the channel (and,
    /// in your own log, its repo, [`Window::channel_repo`]), and the channel
    /// gets a one-line pointer to it. `text` is the thread's whole ask.
    ///
    /// A space's channel starts it in the space's log, so its members and
    /// their agents get it, and the pointer carries the tag, as a bare #N
    /// reads across every log (card #558). With your threads in that space
    /// set to private it stays in your own log, and the channel gets no
    /// pointer: nobody else could open it.
    pub fn talk_thread_from(
        &self,
        conv: &str,
        post_id: &str,
        text: &str,
    ) -> Result<crate::board::TaskResult, String> {
        let (conv, post_id) = (crate::trim(conv), crate::trim(post_id));
        let channel = coord::talk::is_channel(conv)
            .then(|| self.core.talk().channel(conv))
            .flatten()
            .ok_or_else(|| format!("no channel {conv}"))?;
        if !self
            .core
            .talk()
            .posts(conv)
            .iter()
            .any(|p| p.post_id == post_id)
        {
            return Err(format!("no post {post_id} in that channel"));
        }
        let space = channel.space_id.as_str();
        let private = !space.is_empty()
            && self.core.inbox().threads_mode(space) == inbox::threads_mode::PRIVATE;
        // The pointer is refused while a device can't read the space's rows:
        // refuse first, or the thread would start with nothing to show for it.
        if !space.is_empty() && !private {
            self.core.space_gate(space)?;
        }
        let r = self.start_thread_in(text, &[], &[], space, None, conv)?;
        if !private {
            let pointer = r.task.short_ref();
            self.talk_post(conv, &pointer, &[], false, Some(post_id.to_string()))?;
        }
        Ok(r)
    }

    /// The repo a Talk channel is about (card #454): the one repo with a
    /// checkout on this device whose name is the channel's ("irori" for
    /// `remote:github.com/a/irori`), any case. "" when none or several
    /// match, or `conv` isn't a channel: its thread then starts where an
    /// ask naming no repo does.
    pub fn channel_repo(&self, conv: &str) -> String {
        let conv = crate::trim(conv);
        if conv.is_empty() {
            return String::new();
        }
        let Some(c) = self.core.talk().channel(conv) else {
            return String::new();
        };
        let name = c.name.trim().trim_start_matches('#').to_lowercase();
        if name.is_empty() {
            return String::new();
        }
        let hits: Vec<String> = self
            .core
            .capture()
            .repos()
            .into_iter()
            .filter(|(k, roots)| {
                k.starts_with("remote:")
                    // A root on this device: roots can name another
                    // device's checkout (a Mac path on the PC).
                    && roots.iter().any(|r| std::path::Path::new(r).is_dir())
                    && ledger::repo_name(k).to_lowercase() == name
            })
            .map(|(k, _)| ledger::canonical_repo_key(&k))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        match hits.as_slice() {
            [k] => k.clone(),
            _ => String::new(),
        }
    }

    /// The person made card `task_id` from post `post_id` (card #390,
    /// "Make a card"): the post shows the card's chip.
    pub fn talk_link_card(&self, post_id: &str, task_id: &str) -> Result<(), String> {
        self.core
            .talk_link_card(post_id, task_id, &self.me())
            .map(|_| ())
    }

    /// The person has read `conv` up to post `up_to`.
    pub fn talk_read(&self, conv: &str, up_to: &str) -> Result<(), String> {
        self.core.talk_mark_read(conv, up_to, &self.me())
    }

    /// Makes a channel the person is in, in space `space` (None or empty:
    /// Personal); returns its id. A space's channel reaches its members
    /// (card #392).
    pub fn talk_channel_create(
        &self,
        name: &str,
        topic: Option<&str>,
        space: Option<&str>,
    ) -> Result<String, String> {
        let c = self.core.talk_channel_create_in(
            &self.me(),
            name,
            topic.unwrap_or(""),
            space.unwrap_or(""),
        )?;
        Ok(c.id)
    }

    /// The person's DM with `m`, opened if it's new; returns its id for
    /// talk_posts and talk_post. `m` is one of their agents, or another
    /// person by `person:<id>` (card #538): that opens a draft, which
    /// writes nothing until the first post. Another person's agent is
    /// refused.
    pub fn talk_dm(&self, m: &TalkMemberRef) -> Result<String, String> {
        self.core.talk_dm(&self.me(), &m.key)
    }

    /// Everyone the person can DM (card #538): whoever shares a space with
    /// them, by name.
    pub fn talk_dm_people(&self) -> Vec<TalkPerson> {
        self.core
            .talk_dm_people()
            .into_iter()
            .map(|p| TalkPerson {
                key: p.key,
                name: p.name,
                color: p.color,
                avatar: p.avatar,
            })
            .collect()
    }

    /// Leaves DM `conv` with another person (card #538): what's in it goes
    /// from all the person's devices. A later DM from either side starts a
    /// new one.
    pub fn talk_dm_leave(&self, conv: &str) -> Result<(), String> {
        self.core.talk_dm_leave(conv)
    }

    /// Adds `m` to a channel or DM, as the person.
    pub fn talk_join(&self, conv: &str, m: &TalkMemberRef) -> Result<(), String> {
        self.core
            .talk_join(conv, &member(m), &self.me())
            .map(|_| ())
    }

    /// Takes `m` out of a channel or DM, as the person.
    pub fn talk_leave(&self, conv: &str, m: &TalkMemberRef) -> Result<(), String> {
        self.core
            .talk_leave(conv, &member(m), &self.me())
            .map(|_| ())
    }
}

fn member(m: &TalkMemberRef) -> coord::talk::Member {
    coord::talk::Member {
        kind: m.kind.clone(),
        key: m.key.clone(),
    }
}
