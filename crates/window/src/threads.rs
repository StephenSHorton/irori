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

/// One thread in the panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub short: String,
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
    /// and in a channel named after a repo here it is that repo's thread,
    /// so the agent Irori starts for it starts in that repo.
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
            self.private_board_thread(text, mentions, files, board_id, quiet)?
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
                    "files go with a thread on your own board only; on a shared board, attach them on a card"
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
                "your threads on this board are private, so only your own agents hear them: {} is another member's. Switch \"My threads\" to shared in the board's settings to talk with them.",
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
        let about = |t: &Task| t.ask.as_ref().map(|a| a.board.clone()).unwrap_or_default();
        let mut found: Vec<(std::sync::Arc<coord::Coord>, Task, bool)> = Vec::new();
        for t in own.list_tasks(&coord::TaskFilter::default()) {
            if is_thread(&t) && about(&t) == board_id {
                found.push((own.clone(), t, !board_id.is_empty()));
            }
        }
        if !board_id.is_empty() {
            let Ok(bc) = self.core.board_coord(board_id) else {
                return Vec::new();
            };
            for t in bc.list_tasks(&coord::TaskFilter::default()) {
                found.push((bc.clone(), t, false));
            }
        }
        let p = self.people();
        let mut open = Vec::new();
        let mut done = Vec::new();
        for (co, t, private) in found {
            if !is_thread(&t) || t.withdrawn {
                continue;
            }
            let raw = co.comments(&t.id).unwrap_or_default();
            // People, not agents: on a board, any member (card #134).
            let mut answered: Vec<String> = Vec::new();
            for c in &raw {
                let k = self.core.private_key(&c.by);
                if c.msg_id.is_empty() && !co.is_any_person(&c.by) && !answered.contains(&k) {
                    answered.push(k);
                }
            }
            let cs = self.priv_comments(raw);
            let t = self.priv_task(t);
            let a = t.ask.clone().unwrap_or_default();
            let resolved = a.outcome.is_some() || coord::is_terminal(&t.state);
            let state = if resolved {
                THREAD_RESOLVED
            } else if !answered.is_empty() || a.reason == coord::TAKE_ANSWERED {
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
            let last = cs.iter().filter(|c| c.msg_id.is_empty()).max_by(|x, y| {
                x.at.unix_nano()
                    .cmp(&y.at.unix_nano())
                    .then_with(|| x.id.cmp(&y.id))
            });
            let s = ThreadSummary {
                id: t.id.clone(),
                short: t.short.clone(),
                title: t.title.clone(),
                text: t.objective.clone(),
                state: state.into(),
                told: p.refs(&told),
                answered_by: p.refs(&answered),
                replies: cs.iter().filter(|c| c.msg_id.is_empty()).count() as i64,
                last_by: last.map(|c| p.ref_(&c.by)),
                last_text: last.map(|c| c.text.clone()).unwrap_or_default(),
                cards: t
                    .made
                    .iter()
                    .filter_map(|id| co.get_task(id))
                    .map(|c| ThreadCard {
                        id: c.id.clone(),
                        short: c.short.clone(),
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
                    .filter(|sp| sp.op == coord::SPAWN_FAILED && state == THREAD_WAITING)
                    .map(|sp| crate::cards::cant_start_text(&t, sp, &p))
                    .unwrap_or_default(),
                // Any open thread, not only waiting ones: a follow-up to an
                // answered thread waits on an agent too (card #541).
                no_agent: if !resolved {
                    p.no_agent_for(&t.id)
                } else {
                    String::new()
                },
            };
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

    /// BoardService.ResolveThread: the person is done with it.
    pub fn resolve_thread(&self, id: &str) -> Result<(), String> {
        let id = crate::trim(id);
        let (co, keys) = self.on(id, &[])?;
        co.resolve_thread(&keys[0], id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
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
