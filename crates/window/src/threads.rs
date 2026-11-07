//! Threads (card #116): the header box starts a conversation with the
//! room, shown in the Threads panel beside the Board, not in the grid.
//! The agents on the cards it names (#12) and the agents it @mentions hear
//! it, and at once one idle agent or its own spawned agent gets it (card
//! #512; coord's threads).
//! The person replies with the card comment box (BoardService.Comment) and
//! resolves it here. A shared board's page starts its threads on that
//! board (card #134): they live in its log, only its members' agents hear
//! them, and its page lists them in its own Threads panel.

use coord::{NewAsk, Task};
use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::board::{TaskResult, Told, UploadFile};
use crate::cards::AgentRef;
use crate::ts;

/// Where a thread stands, for the panel.
pub const THREAD_WAITING: &str = "waiting";
pub const THREAD_HANDED: &str = "handed";
pub const THREAD_ANSWERED: &str = "answered";
pub const THREAD_RESOLVED: &str = "resolved";

/// How many resolved threads the panel lists.
const RESOLVED_SHOWN: usize = 20;

/// Whether `t` is the person's thread.
pub fn is_thread(t: &Task) -> bool {
    t.ask.as_ref().is_some_and(|a| a.thread)
}

/// The shared board a private thread is about ("" for none).
fn about(t: &Task) -> String {
    t.ask.as_ref().map(|a| a.board.clone()).unwrap_or_default()
}

/// A thread to list: the log that holds it, and whether it is a private
/// one about a shared board.
type Found = (std::sync::Arc<coord::Coord>, Task, bool);

/// One thread in the panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub short: String,
    /// Its board's tag: numbers are per board (card #558), and the channel
    /// pointer says `mlg#5`. "": a private thread.
    pub tag: String,
    pub title: String,
    /// What the person asked, in full.
    pub text: String,
    /// waiting (for an answer), handed (to one idle agent), answered or
    /// resolved.
    pub state: String,
    /// Who heard it first, and who it was handed to.
    pub told: Vec<AgentRef>,
    /// The agents that answered.
    pub answered_by: Vec<AgentRef>,
    pub replies: i64,
    /// The newest reply, by anyone.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub last_by: Option<AgentRef>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub last_text: String,
    /// The cards made or changed from it (card #155), first first.
    pub cards: Vec<ThreadCard>,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    #[specta(type = ts::Time)]
    pub updated_at: GoTime,
    /// A private thread about a shared board (card #185): in your own log,
    /// heard only by your agents, never on other members' devices.
    pub private: bool,
    /// Its "Agents answer now" (card #391): off, no agent is woken or
    /// handed it, and its agents read it at their next turn.
    pub agents_now: bool,
    /// An agent Irori starts for it (card #470): where, while it starts.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub starting: String,
    /// Why one couldn't start (the failed request's reason).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub cant_start: String,
    /// Why no agent can start now, by device, and what happens next (card
    /// #541): it needs the person to know, not to answer.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub no_agent: String,

    /// The shared space it is in, or a private thread is about (card #558);
    /// absent for Personal. Matches the switcher's space id.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub space_id: String,
    /// The device whose cap it waits on, with its place in that device's
    /// spawn queue (card #450): "queued: 2nd for the PC".
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub at_cap: String,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub queue_place: i64,
    /// While a Claude usage limit holds spawns, when they start again.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Time>)]
    pub usage_limit: Option<GoTime>,
    /// The agent that opened it (card #631); absent for the person's own.
    /// Until the person writes in it, it waits on them and no agent is
    /// started for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub opened_by: Option<AgentRef>,
    /// An agent wrote in it after the person last opened or replied in it
    /// (card #719): Talk's count counts it and its list marks it. An
    /// agent's own thread is unread until the person first opens it.
    pub unread: bool,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// What a #ref in text names (card #144): a card or a thread, live or
/// done, so the window can link it without loading every done card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefTarget {
    pub id: String,
    pub short: String,
    pub title: String,
    /// A thread opens in the Threads panel, not on a card's page.
    pub thread: bool,
    /// Done, failed or canceled (a thread: resolved).
    pub done: bool,
    /// The card's state as its page shows it (card #343): the ref's chip
    /// takes its dot from it. A thread's is its task's, so read done.
    pub state: String,
    /// Open blockers that wait on the person (or input required): the
    /// chip says blocked, as the card's red pill does.
    pub blocked: bool,
    /// An agent says it's done and only your Done waits (card #112): the
    /// chip says Ready, as the card's green pill does.
    pub ready: bool,
    /// Done steps over all (a parent's, its subtasks' rollup).
    pub progress: crate::cards::CardProgress,
}

/// A card made or changed from a thread, as its chip shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCard {
    pub id: String,
    pub short: String,
    /// As [`ThreadSummary::tag`].
    pub tag: String,
    pub title: String,
    pub state: String,
}

impl Window {
    /// BoardService.CardRef: what `#12`, `12b` or `mlg#12` names across every
    /// board on this device, or None when nothing (or more than one card,
    /// or a deleted one) does.
    pub fn card_ref(&self, r: &str) -> Option<RefTarget> {
        let id = self.core.task_ref(crate::trim(r)).ok()?;
        let t = self.read(&id).get_task(&id)?;
        let s = crate::cards::card_summary(&t, &self.people());
        Some(RefTarget {
            id: t.id.clone(),
            short: t.short.clone(),
            title: t.title.clone(),
            thread: is_thread(&t),
            done: coord::is_terminal(&t.state)
                || (is_thread(&t) && t.ask.as_ref().is_some_and(|a| a.outcome.is_some())),
            state: s.state,
            blocked: s.open_blockers > 0 && !(s.ready && s.open_blockers == 1),
            ready: s.ready && s.open_blockers == 1,
            progress: s.progress,
        })
    }

    /// BoardService.StartThread: the header box's message, as a thread.
    /// Its files go on the thread like an ask's (card #124). With
    /// `board_id`, on that shared board (card #134): no mentions or files
    /// there.
    pub fn start_thread(
        &self,
        text: &str,
        mentions: &[ts::Mention],
        files: &[UploadFile],
        board_id: &str,
    ) -> Result<TaskResult, String> {
        self.start_thread_with(text, mentions, files, board_id, None)
    }

    /// [`Self::start_thread`] with the composer's "Agents answer now" (card
    /// #391); none: the default for its mentions.
    pub fn start_thread_with(
        &self,
        text: &str,
        mentions: &[ts::Mention],
        files: &[UploadFile],
        board_id: &str,
        agents_now: Option<bool>,
    ) -> Result<TaskResult, String> {
        self.start_thread_in(text, mentions, files, board_id, agents_now, "")
    }

    /// [`Self::start_thread_with`] from a message in Talk channel `channel`
    /// (its `ch_` id; "" none, card #454): the thread records the channel,
    /// and in your own log, in a channel named after a repo here it is that
    /// repo's thread, so the agent Irori starts for it starts in that repo.
    pub(crate) fn start_thread_in(
        &self,
        text: &str,
        mentions: &[ts::Mention],
        files: &[UploadFile],
        board_id: &str,
        agents_now: Option<bool>,
        channel: &str,
    ) -> Result<TaskResult, String> {
        let quiet = !agents_now.unwrap_or_else(|| agents_now_default(mentions));
        let board_id = crate::trim(board_id);
        let private = !board_id.is_empty()
            && self.core.inbox().threads_mode(board_id) == inbox::threads_mode::PRIVATE;
        let (t, n) = if private {
            self.private_board_thread(text, mentions, files, board_id, quiet, channel)?
        } else if board_id.is_empty() {
            let plan = self.plan_mentions("", mentions);
            let refs = self.store_uploads(files)?;
            self.core
                .coord()
                .post_ask(
                    &self.user_key(),
                    NewAsk {
                        text: text.into(),
                        mentions: plan.log.clone(),
                        files: refs.iter().map(crate::coord_ref).collect(),
                        thread: true,
                        quiet,
                        channel: channel.into(),
                        repo_key: self.channel_repo(channel),
                        ..Default::default()
                    },
                    &self.core.ask_agents(),
                )
                .map_err(|e| e.to_string())?
        } else {
            if !files.is_empty() {
                return Err(
                    "files go with a thread in Personal only; in a shared space, attach them on a card"
                        .into(),
                );
            }
            self.core
                .board_ask(
                    board_id,
                    NewAsk {
                        text: text.into(),
                        // Board keys (card #187): another person, their
                        // agents, or this person's own agents on the board.
                        mentions: self.plan_mentions_on(board_id, mentions).log,
                        thread: true,
                        quiet,
                        // No repo: another member's device has its own
                        // checkouts, and a board ask names none.
                        channel: channel.into(),
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?
        };
        let mut told = Told::default();
        if !n.to.is_empty() {
            told.to = n.to.iter().map(|k| self.core.private_key(k)).collect();
            told.mode = if n.wake {
                inbox::mode::WAKE
            } else {
                inbox::mode::NEXT_TURN
            }
            .into();
            let from = if n.person {
                self.core.user_sender()
            } else {
                inbox::system_sender()
            };
            if let Err(e) = self.core.route_notices(&from, std::slice::from_ref(&n)) {
                told.error = e;
            }
        }
        Ok(TaskResult {
            task: t,
            told,
            ..TaskResult::default()
        })
    }

    /// A private thread about shared board `board_id` (card #185): it goes
    /// in your own log, tagged with the board, and only your own agents
    /// hear it: yours among the holders of the board cards it names, and
    /// the ones you mention. Naming another member's agent is refused, so
    /// its text never reaches their devices.
    fn private_board_thread(
        &self,
        text: &str,
        mentions: &[ts::Mention],
        files: &[UploadFile],
        board_id: &str,
        quiet: bool,
        channel: &str,
    ) -> Result<(Task, coord::Notice), String> {
        let bc = self.core.board_coord(board_id).map_err(|e| e.to_string())?;
        let theirs = |k: &str| {
            k.starts_with(store::kinds::AGENT_ACTOR_PREFIX)
                || k.starts_with(store::kinds::PERSON_ACTOR_PREFIX)
        };
        // Checked before the mentions are resolved: another member's agent
        // isn't one of yours, so resolving would drop it without a word.
        if let Some(m) = mentions
            .iter()
            .find(|m| theirs(&self.core.private_key(&m.key)))
        {
            return Err(format!(
                "your threads in this space are private, so only your own agents hear them: {} is another member's. Switch \"My threads\" to shared in Space settings to talk with them.",
                if m.name.is_empty() { &m.key } else { &m.name }
            ));
        }
        let plan = self.plan_mentions("", mentions);
        let mut log: Vec<coord::Mention> = plan
            .log
            .iter()
            .map(|m| coord::Mention {
                key: self.core.private_key(&m.key),
                ..m.clone()
            })
            .collect();
        // Your agents on the board cards it names hear it first, as on your
        // own board; other members' holders don't.
        for h in bc.thread_hearers(text, &[]) {
            let k = self.core.private_key(&h);
            if theirs(&k) || log.iter().any(|m| m.key == k) {
                continue;
            }
            let name = self.people().ref_(&k).name;
            log.push(coord::Mention {
                key: k,
                name,
                ..coord::Mention::default()
            });
        }
        let refs = self.store_uploads(files)?;
        self.core
            .coord()
            .post_ask(
                &self.user_key(),
                NewAsk {
                    text: text.into(),
                    mentions: log,
                    files: refs.iter().map(crate::coord_ref).collect(),
                    thread: true,
                    board: board_id.into(),
                    quiet,
                    channel: channel.into(),
                    repo_key: self.channel_repo(channel),
                    ..Default::default()
                },
                &self.core.ask_agents(),
            )
            .map_err(|e| e.to_string())
    }

    /// BoardService.Threads: open threads, newest first, then the newest
    /// resolved ones: your own board's, or with `board_id` that shared
    /// board's.
    pub fn threads(&self, board_id: &str) -> Vec<ThreadSummary> {
        let board_id = crate::trim(board_id);
        // Where the threads come from: your own board's are your log's,
        // minus the private ones about a shared board; a shared board's are
        // its log's plus your private ones about it (card #185).
        let own = self.core.coord().clone();
        let mut found: Vec<Found> = Vec::new();
        for t in own.list_tasks(&coord::TaskFilter::default()) {
            if is_thread(&t) && about(&t) == board_id {
                found.push((own.clone(), t, !board_id.is_empty()));
            }
        }
        // A space you left keeps your private threads about it (card #558).
        if !board_id.is_empty()
            && let Ok(bc) = self.core.board_coord(board_id)
        {
            for t in bc.list_tasks(&coord::TaskFilter::default()) {
                found.push((bc.clone(), t, false));
            }
        }
        self.thread_rows(found, board_id)
    }

    /// BoardService.ThreadsAll: every thread on this device, for All spaces
    /// and for finding one the picked space doesn't list (card #558): your
    /// own log's, then each shared space's, each carrying its space, with
    /// each log's newest resolved ones, so a busy space can't push a quiet
    /// one's out. A private thread about a space you've left is Personal's.
    pub fn threads_all(&self) -> Vec<ThreadSummary> {
        let mut all = self.threads("");
        let spaces: Vec<String> = self
            .core
            .board_list()
            .into_iter()
            .map(|b| b.board_id)
            .collect();
        for id in &spaces {
            all.extend(self.threads(id));
        }
        let own = self.core.coord().clone();
        let left: Vec<Found> = own
            .list_tasks(&coord::TaskFilter::default())
            .into_iter()
            .filter(|t| is_thread(t) && !about(t).is_empty() && !spaces.contains(&about(t)))
            .map(|t| (own.clone(), t, true))
            .collect();
        all.extend(self.thread_rows(left, ""));
        let resolved = |t: &ThreadSummary| t.state == THREAD_RESOLVED;
        all.sort_by(|a, b| {
            resolved(a)
                .cmp(&resolved(b))
                .then(b.updated_at.unix_nano().cmp(&a.updated_at.unix_nano()))
        });
        all
    }

    /// `found` as the panel lists it, in space `space` ("": Personal): open
    /// ones newest first, then the newest resolved ones.
    fn thread_rows(&self, found: Vec<Found>, space: &str) -> Vec<ThreadSummary> {
        let p = self.people();
        let me = self.core.user_sender().key;
        let mut open = Vec::new();
        let mut done = Vec::new();
        for (co, t, private) in found {
            if !is_thread(&t) || t.withdrawn {
                continue;
            }
            let raw = co.comments(&t.id).unwrap_or_default();
            // People, not agents: on a board, any member (card #134).
            let mut answered: Vec<String> = Vec::new();
            let mut said: Vec<Said> = Vec::new();
            for c in &raw {
                if c.msg_id.is_empty() {
                    said.push(Said {
                        at: c.at.unix_nano(),
                        id: c.id.clone(),
                        agent: !co.is_any_person(&c.by),
                    });
                }
                let k = self.core.private_key(&c.by);
                if c.msg_id.is_empty() && !co.is_any_person(&c.by) && !answered.contains(&k) {
                    answered.push(k);
                }
            }
            // An agent's own thread (card #631) waits on the person until
            // they write in it: no agent is handed or started for it.
            let rests = co.agent_thread_rests(&t);

            let cs = self.priv_comments(raw);
            let t = self.priv_task(t);
            let a = t.ask.clone().unwrap_or_default();
            let opener = self.core.private_key(&a.opener);
            let resolved = a.outcome.is_some() || coord::is_terminal(&t.state);
            let cursor = self.core.talk().cursor(&me, &read_conv(&t.id));
            let unread = !resolved && thread_unread(&said, cursor.as_deref(), !opener.is_empty());
            let state = if resolved {
                THREAD_RESOLVED
            } else if !answered.is_empty() || a.reason == coord::TAKE_ANSWERED || rests {
                THREAD_ANSWERED
            } else if !a.taker.is_empty() {
                THREAD_HANDED
            } else {
                THREAD_WAITING
            };
            let mut told: Vec<String> = a.heard.iter().map(|k| self.core.private_key(k)).collect();
            let taker = self.core.private_key(&a.taker);
            if !taker.is_empty() && !told.contains(&taker) {
                told.push(taker);
            }
            told.retain(|k| *k != opener);
            let last = cs.iter().filter(|c| c.msg_id.is_empty()).max_by(|x, y| {
                x.at.unix_nano()
                    .cmp(&y.at.unix_nano())
                    .then_with(|| x.id.cmp(&y.id))
            });
            // Nothing written yet: its newest word is its opener's.
            let (last_by, last_text) = match last {
                Some(c) => (Some(p.ref_(&c.by)), c.text.clone()),
                None if !opener.is_empty() => (Some(p.ref_(&opener)), t.objective.clone()),
                None => (None, String::new()),
            };
            let s = ThreadSummary {
                id: t.id.clone(),
                short: t.short.clone(),
                tag: t.tag.clone(),
                title: t.title.clone(),
                text: t.objective.clone(),
                state: state.into(),
                told: p.refs(&told),
                answered_by: p.refs(&answered),
                replies: cs.iter().filter(|c| c.msg_id.is_empty()).count() as i64,
                last_by,
                last_text,
                opened_by: (!opener.is_empty()).then(|| p.ref_(&opener)),
                cards: t
                    .made
                    .iter()
                    .filter_map(|id| co.get_task(id))
                    .map(|c| ThreadCard {
                        id: c.id.clone(),
                        short: c.short.clone(),
                        tag: c.tag.clone(),
                        title: c.title.clone(),
                        state: c.state.to_string(),
                    })
                    .collect(),
                at: t.created_at,
                updated_at: t.updated_at,
                private,
                agents_now: t.ask.as_ref().is_some_and(|a| a.wake),
                starting: t
                    .spawn_request()
                    .filter(|sp| sp.pending() && !sp.stale(&p.now) && state == THREAD_WAITING)
                    .map(|sp| p.where_(&sp.device))
                    .unwrap_or_default(),
                cant_start: t
                    .spawn_request()
                    .filter(|sp| {
                        sp.op == coord::SPAWN_FAILED
                            && !coord::usage_limited(sp)
                            && state == THREAD_WAITING
                            && crate::cards::failed_wait_applies(&t, &p)
                    })
                    .map(|sp| crate::cards::cant_start_text(&t, sp, &p))
                    .unwrap_or_default(),
                // A waiting thread, or an answered one the person followed
                // up on: that waits on an agent too (card #541).
                no_agent: if state == THREAD_WAITING
                    || (state == THREAD_ANSWERED && a.reason == coord::TAKE_REOPENED)
                {
                    p.no_agent_for(&t.id)
                } else {
                    String::new()
                },

                space_id: space.to_string(),
                unread,
                ..ThreadSummary::default()
            };
            let mut s = s;
            if state == THREAD_WAITING {
                let w = crate::cards::queue_words(&t.id, &p);
                s.at_cap = w.at_cap;
                s.queue_place = w.queue_place;
                s.usage_limit = w.usage_limit;
            }
            if resolved {
                done.push(s);
            } else {
                open.push(s);
            }
        }
        let newest = |a: &ThreadSummary, b: &ThreadSummary| {
            b.updated_at.unix_nano().cmp(&a.updated_at.unix_nano())
        };
        open.sort_by(newest);
        done.sort_by(newest);
        done.truncate(RESOLVED_SHOWN);
        open.extend(done);
        open
    }

    /// BoardService.ReopenThread: a resolved thread is live again (card
    /// #169); the person's next reply goes to the agents that answered.
    pub fn reopen_thread(&self, id: &str) -> Result<(), String> {
        let (co, keys) = self.on(crate::trim(id), &[])?;
        co.reopen_thread(&keys[0], crate::trim(id))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// A person turns an open thread's "Agents answer now" on or off (card
    /// #391).
    pub fn set_agents_now(&self, id: &str, on: bool) -> Result<(), String> {
        let id = crate::trim(id);
        let (co, keys) = self.on(id, &[])?;
        co.set_agents_now(&keys[0], id, on)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// BoardService.ReadThread: the person has the thread open (card
    /// #719), so what is in it now is read: its newest reply, or its
    /// opening words when nobody has replied yet. Kept with DMs' read
    /// cursors, so the person's other devices see it read too.
    pub fn read_thread(&self, id: &str) -> Result<(), String> {
        let id = crate::trim(id);
        let co = self.read(id);
        let t = co
            .get_task(id)
            .filter(is_thread)
            .ok_or_else(|| format!("no thread {id}"))?;
        let newest = co
            .comments(&t.id)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.msg_id.is_empty())
            .max_by(|x, y| {
                x.at.unix_nano()
                    .cmp(&y.at.unix_nano())
                    .then_with(|| x.id.cmp(&y.id))
            })
            .map(|c| c.id)
            .unwrap_or_else(|| t.id.clone());
        self.core
            .talk_mark_read(&read_conv(&t.id), &newest, &self.core.user_sender().key)
    }

    /// BoardService.ResolveThread: the person is done with it.
    pub fn resolve_thread(&self, id: &str) -> Result<(), String> {
        let id = crate::trim(id);
        let (co, keys) = self.on(id, &[])?;
        co.resolve_thread(&keys[0], id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// The read cursor that keeps how far the person has read thread `id`
/// (card #719): Talk's, beside the DMs' and channels'.
fn read_conv(id: &str) -> String {
    format!("thread:{id}")
}

/// One reply in a thread, for [`thread_unread`].
#[derive(Debug, Clone)]
struct Said {
    at: i64,
    id: String,
    agent: bool,
}

/// Whether a thread waits unread on the person (card #719): an agent
/// replied after the newest of where they last read it (`cursor`: a reply's
/// id, or the thread's own when it had none) and their own newest reply.
/// An agent's own thread (`agent_opened`) is unread until the person first
/// reads or replies in it.
fn thread_unread(said: &[Said], cursor: Option<&str>, agent_opened: bool) -> bool {
    let pos = |s: &Said| (s.at, s.id.clone());
    let read = said.iter().find(|s| Some(s.id.as_str()) == cursor).map(pos);
    let replied = said.iter().filter(|s| !s.agent).map(pos).max();
    let seen = read.max(replied);
    if said.iter().any(|s| s.agent && Some(pos(s)) > seen) {
        return true;
    }
    agent_opened && cursor.is_none() && !said.iter().any(|s| !s.agent)
}

/// "Agents answer now"'s default for a composer's mentions (card #391):
/// off when they name only people (another member, `person:<id>`), on when
/// they name nobody or an agent.
pub fn agents_now_default(mentions: &[ts::Mention]) -> bool {
    let ms: Vec<coord::Mention> = mentions
        .iter()
        .map(|m| coord::Mention {
            key: crate::trim(&m.key).to_string(),
            ..coord::Mention::default()
        })
        .filter(|m| !m.key.is_empty() && m.key != "irori")
        .collect();
    coord::agents_now_default(&ms)
}

#[cfg(test)]
mod tests {
    use super::{Said, thread_unread};

    fn agent(at: i64, id: &str) -> Said {
        Said {
            at,
            id: id.into(),
            agent: true,
        }
    }
    fn person(at: i64, id: &str) -> Said {
        Said {
            at,
            id: id.into(),
            agent: false,
        }
    }

    /// Card #719: a thread is unread when an agent wrote after the person
    /// last read or replied in it.
    #[test]
    fn unread_is_an_agent_reply_after_the_person_looked() {
        // Nobody answered yet: nothing to read.
        assert!(!thread_unread(&[], None, false));
        // An agent answered, never opened: unread.
        let one = [agent(1, "c1")];
        assert!(thread_unread(&one, None, false));
        // Opened since the answer: read.
        assert!(!thread_unread(&one, Some("c1"), false));
        // Opened before any answer (the cursor is the thread's own id),
        // then an agent answered: unread.
        assert!(thread_unread(&one, Some("t"), false));
        // A new agent reply after the read one brings it back.
        let two = [agent(1, "c1"), agent(2, "c2")];
        assert!(thread_unread(&two, Some("c1"), false));
        assert!(!thread_unread(&two, Some("c2"), false));
        // The person's own reply reads what came before it, cursor or not.
        let replied = [agent(1, "c1"), person(2, "c2")];
        assert!(!thread_unread(&replied, None, false));
        let again = [agent(1, "c1"), person(2, "c2"), agent(3, "c3")];
        assert!(thread_unread(&again, None, false));
        assert!(thread_unread(&again, Some("c1"), false));
        // Same instant: the id breaks the tie.
        assert!(thread_unread(&[agent(5, "b"), person(5, "a")], None, false));
        assert!(!thread_unread(
            &[agent(5, "a"), person(5, "b")],
            None,
            false
        ));
    }

    /// An agent's own thread waits unread until the person first opens or
    /// replies in it.
    #[test]
    fn an_agents_own_thread_is_unread_until_opened() {
        assert!(thread_unread(&[], None, true));
        assert!(!thread_unread(&[], Some("t"), true));
        assert!(!thread_unread(&[person(1, "c1")], None, true));
        assert!(thread_unread(&[agent(1, "c1")], Some("t"), true));
    }
}
