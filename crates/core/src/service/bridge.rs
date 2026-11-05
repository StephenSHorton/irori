//! The bridge from a board log into this device's inbox (v0.5 §7.1), and
//! the Core side of the tools' shared boards (§7.3).
//!
//! A row another person's device wrote on a board becomes a private
//! message.sent from a `board:` sender, through [`inbox::Inbox::bridge`],
//! which keeps only the agents this device hosts, applies the person's
//! receive setting (§7.2, B2) and never bridges one row to one agent
//! twice. What crosses:
//! - a comment, an assignment or a blocker change, to the card's holders,
//!   watchers and assignees (the assignee alone for an assignment);
//! - a board ask (§9): the asking device's `board.ask.state` telling free
//!   agents, to those of them this device hosts, once per round;
//! - an open card handed out (card #174): the device that made it hands it
//!   to one free agent at a time, and the one it names hears it here;
//! - a review request (§10), to the agent it names or, without one, to
//!   this person's dedicated reviewer for the card's repo or the board;
//! - a lease release request (§5.4), to the lease's holder only;
//! - a post in a space's channel naming one of this person's agents (card
//!   #461), woken with `ref` `talk:<conv>` so a reply goes in the channel.
//!
//! The person's own rows don't cross here: the tool that wrote one already
//! routed its notice. Rows met on a resync aren't bridged either, so a
//! re-fold doesn't replay old talk.

use std::sync::Arc;

use coord::{Coord, RELEASE_REQUEST, Task};
use inbox::{BoardFrom, BridgeRow, row_kind};
use serde::Deserialize;
use store::Event;
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};

use super::Services;
use super::boards::BoardLog;
use crate::logf;

/// The fields of a board row the bridge reads; each kind fills some.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RowFields {
    task_id: String,
    comment_id: String,
    text: String,
    /// A comment's mentions, in board form (card #177).
    mentions: Vec<RowMention>,
    /// A task.created's ask: a board thread's mentions (card #187).
    ask: Option<RowAsk>,
    wake: bool,
    to: String,
    answer: String,
    note: String,
    pr: String,
    blocker: Option<serde_json::Value>,
    // board.ask.state
    taker: String,
    was: String,
    reason: String,
    heard: Vec<String>,
    since: String,
}

/// A task.created row's ask field, for its mentions.
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct RowAsk {
    mentions: Vec<RowMention>,
}

/// One mention on a board row: the board key it names (`agent:<cardId>`
/// or `person:<personId>`) and the name it was written with.
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
struct RowMention {
    key: String,
    name: String,
}

/// The message text of a plan, given how the sender shows.
type TextFn = Box<dyn FnOnce(&str) -> String>;

/// What one board row says to this device's agents, before the inbox's
/// per-agent checks.
struct Plan {
    kind: &'static str,
    /// The writing agent's card; empty when the person wrote it.
    card_id: String,
    /// Board keys of the agents it is for.
    to: Vec<String>,
    /// The row the inbox dedupes on: `<deviceId>/<seq>`, or an ask's round.
    row: String,
    wake: bool,
    text: TextFn,
}

/// The take reasons that tell an ask's free agents (coord's asks): posted,
/// asked again, given back, its taker gone, or still unclaimed.
const ASK_ROUND: &[&str] = &["posted", "reopened", "released", "gone", "unclaimed"];

impl Services {
    /// Bridges one board row, live from `b`'s follower.
    pub(crate) fn bridge_row(&self, b: &BoardLog, ev: &Event) {
        if ev.seq == 0 {
            return;
        }
        // A thread (or ask) naming this person: a notification on each of
        // their devices that opens it (card #187). Their agents it names
        // hear it from the round's board.ask.state.
        if ev.kind == kinds::TASK_CREATED {
            self.ask_mention(b, ev);
            return;
        }
        if ev.kind == kinds::TALK_POST {
            self.talk_post_bridge(b, ev);
            return;
        }
        let bridged = matches!(
            ev.kind.as_str(),
            kinds::TASK_COMMENT
                | kinds::TASK_ASSIGNED
                | kinds::BLOCKER_CLOSED
                | kinds::BLOCKER_OPTION
                | kinds::BLOCKER_RESOLVED
                | kinds::TASK_STATUS
                | kinds::REVIEW_REQUESTED
                | kinds::BOARD_ASK_STATE
                | kinds::TASK_OFFERED
        );
        if !bridged {
            return;
        }
        let Ok(f) = serde_json::from_slice::<RowFields>(&ev.payload) else {
            return;
        };
        if f.task_id.is_empty() {
            return;
        }
        let me = self.person_id();
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        if person.is_empty() || person == me {
            return;
        }
        let co = b.coord();
        // The coord folds on its own thread: a card that arrived with this
        // row (an ask's task.created and its first board.ask.state) may not
        // have folded yet, so fold what the log holds before giving up.
        // The coord thread then skips those rows (apply_remote folds a
        // device's rows past its last only).
        let Some(t) = co
            .get_task(&f.task_id)
            .or_else(|| co.catch_up().ok().and_then(|()| co.get_task(&f.task_id)))
        else {
            return;
        };
        let who = self.board_person_name(b, &person);
        // A comment (a blocker reply too) naming this person: a desktop
        // notification on each of their devices that opens the card at the
        // comment (card #177). Their agents named in it hear below.
        let me_key = format!("{PERSON_ACTOR_PREFIX}{me}");
        if ev.kind == kinds::TASK_COMMENT && f.mentions.iter().any(|m| m.key == me_key) {
            self.mention_note(b, &t, &who, &person, &f);
        }
        let mut plans = Vec::new();
        if ev.kind == kinds::BOARD_ASK_STATE {
            plans.extend(self.ask_plan(b, ev, f, &t));
        } else if ev.kind == kinds::TASK_OFFERED {
            plans.extend(offer_plan(co, f, &t));
        } else {
            let mentioned: Vec<String> = if ev.kind == kinds::TASK_COMMENT {
                f.mentions
                    .iter()
                    .filter(|m| m.key.starts_with(AGENT_ACTOR_PREFIX))
                    .map(|m| m.key.clone())
                    .collect()
            } else {
                Vec::new()
            };
            let mention = (!mentioned.is_empty())
                .then(|| mention_plan(ev, &f, &t, mentioned.clone()))
                .flatten();
            if let Some(mut p) = row_plan(co, &me, ev, f, &t) {
                // One notice each: a mentioned agent hears the mention only.
                p.to.retain(|k| !mentioned.contains(k));
                plans.push(p);
            }
            plans.extend(mention);
        }
        for p in plans {
            self.send_plan(b, &t, &person, &who, ev, p);
        }
    }

    /// Hands one plan to the inbox's bridge, for the agents this device
    /// hosts among its `to`.
    fn send_plan(&self, b: &BoardLog, t: &Task, person: &str, who: &str, ev: &Event, p: Plan) {
        self.send_ref(b, person, who, ev, p, format!("task:{}", t.id));
    }

    /// [`Services::send_plan`] for a row whose message refers to `ref_`
    /// (a card's `task:<id>`, a channel's `talk:<conv>`).
    fn send_ref(&self, b: &BoardLog, person: &str, who: &str, ev: &Event, p: Plan, ref_: String) {
        let to: Vec<String> =
            p.to.iter()
                .filter(|k| k.starts_with(AGENT_ACTOR_PREFIX))
                .map(|k| self.private_key(k))
                .collect();
        if to.is_empty() {
            return;
        }
        let who = who.to_string();
        let name = if p.card_id.is_empty() {
            who.clone()
        } else {
            let card = super::boards::lock(&b.maps)
                .cards
                .get(&p.card_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            if card.is_empty() {
                format!("{who}'s agent")
            } else {
                format!("{who}'s agent {card}")
            }
        };
        let text = (p.text)(&name);
        let tag = b.verifier.membership().tag;
        let r = BridgeRow {
            from: BoardFrom {
                board_id: b.id.clone(),
                tag,
                person_id: person.to_string(),
                card_id: p.card_id,
                row: p.row,
                kind: p.kind.into(),
                wake: p.wake,
            },
            name,
            to,
            text,
            ref_,
            hop: coord::row_hop(&ev.payload),
            attachments: vec![],
        };
        if let Err(e) = self.inbox.bridge(&r) {
            logf(format_args!("board {}: bridge {}: {e}", b.id, r.from.row));
        }
    }

    /// A post in a space's channel that names this person's agents (card
    /// #461): each is woken as a comment mention is, its message referring
    /// to the channel (`talk:<conv>`), so the receive setting, holds, rate
    /// limit and the bridged-once set apply. Read from the raw row, whose
    /// mentions are still board keys. With "Agents answer now" off nobody
    /// is woken, as on the poster's side.
    fn talk_post_bridge(&self, b: &BoardLog, ev: &Event) {
        let Ok(r) = serde_json::from_slice::<coord::talk::PostRow>(&ev.payload) else {
            return;
        };
        if !r.agents_now || r.conv_id.is_empty() {
            return;
        }
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        if person.is_empty() || person == self.person_id() {
            return;
        }
        let to: Vec<String> = r
            .mentions
            .iter()
            .filter(|k| k.starts_with(AGENT_ACTOR_PREFIX))
            .cloned()
            .collect();
        if to.is_empty() {
            return;
        }
        // Folded just before; none when the space is leaving Talk here. A
        // post names only a live channel of the space whose log holds it:
        // a row naming another space's channel wakes nobody (card #461).
        let Some(ch) = self
            .talk
            .channel(&r.conv_id)
            .filter(|c| c.space_id == b.id && !c.archived)
        else {
            return;
        };
        let card_id = ev
            .actor
            .strip_prefix(AGENT_ACTOR_PREFIX)
            .unwrap_or_default()
            .to_string();
        if card_id.is_empty() && !ev.actor.starts_with(PERSON_ACTOR_PREFIX) {
            return;
        }
        let who = self.board_person_name(b, &person);
        let (conv, text, chan) = (r.conv_id.clone(), r.text, ch.name);
        let p = Plan {
            kind: row_kind::COMMENT,
            card_id,
            to,
            row: format!("{}/{}", ev.device_id, ev.seq),
            wake: true,
            text: Box::new(move |name| {
                format!(
                    "{name} mentioned you in #{chan} (Talk, conv={conv}):
{text}

Answer there with talk_post conv={conv} (or reply to this message) if you have something to add; talk_read conv={conv} shows the rest."
                )
            }),
        };
        self.send_ref(b, &person, &who, ev, p, format!("talk:{}", r.conv_id));
    }

    /// A board thread or ask that names this person (card #187): the same
    /// notification a comment mention posts, opening the thread's card.
    fn ask_mention(&self, b: &BoardLog, ev: &Event) {
        let Ok(mut f) = serde_json::from_slice::<RowFields>(&ev.payload) else {
            return;
        };
        let me = self.person_id();
        let me_key = format!("{PERSON_ACTOR_PREFIX}{me}");
        let named = f
            .ask
            .as_ref()
            .is_some_and(|a| a.mentions.iter().any(|m| m.key == me_key));
        if f.task_id.is_empty() || !named {
            return;
        }
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        if person.is_empty() || person == me {
            return;
        }
        let co = b.coord();
        let Some(t) = co
            .get_task(&f.task_id)
            .or_else(|| co.catch_up().ok().and_then(|()| co.get_task(&f.task_id)))
        else {
            return;
        };
        let who = self.board_person_name(b, &person);
        if f.text.is_empty() {
            f.text = t.objective.clone();
        }
        self.mention_note(b, &t, &who, &person, &f);
    }

    /// The desktop notification for a comment that mentions this person:
    /// "<who> mentioned you on <title>", opening the card at the comment.
    /// Every device of the person posts its own, so it shows wherever they
    /// are; a resync never replays it (bridge_row runs on live rows only).
    fn mention_note(&self, b: &BoardLog, t: &Task, who: &str, person: &str, f: &RowFields) {
        let n = &self.notifier.0;
        if n.is_headless() {
            return;
        }
        let short = if t.short.is_empty() {
            String::new()
        } else {
            format!(" ({})", t.short_ref())
        };
        let target = notify::Target {
            tab: notify::TAB_BOARD.into(),
            ref_: if f.comment_id.is_empty() {
                format!("task:{}", t.id)
            } else {
                format!("task:{}/comment:{}", t.id, f.comment_id)
            },
        };
        let opener = n.clone();
        n.post(notify::Note {
            id: format!("mention:{}:{}", t.id, f.comment_id),
            title: format!(
                "{who} mentioned you on “{}”{short}",
                notify::preview(&t.title)
            ),
            body: notify::preview(&f.text),
            level: notify::Level::Info,
            about: Some(coord::notifications::PostedRow {
                kind: coord::notifications::KIND_MENTION.into(),
                board_id: b.id.clone(),
                task_id: t.id.clone(),
                comment_id: f.comment_id.clone(),
                from: person.to_string(),
                ..Default::default()
            }),
            on_click: Some(Arc::new(move || opener.open(target.clone()))),
            ..notify::Note::default()
        });
    }

    /// A board ask's round of telling (§9): the asking device's
    /// board.ask.state naming the free agents it told. A round (its
    /// `since`) reaches an agent once, however often a sweep widens it. A
    /// take, a give-back or a gone taker tells nobody here: the claimer's
    /// own device answers its claim.
    fn ask_plan(&self, b: &BoardLog, ev: &Event, f: RowFields, t: &Task) -> Option<Plan> {
        let a = t.ask.as_ref()?;
        if f.reason == coord::TAKE_HANDED {
            return self.handed_plan(b, ev, f, t);
        }
        if ev.device_id != a.owner
            || a.outcome.is_some()
            || !f.taker.is_empty()
            || !f.was.is_empty()
            || f.heard.is_empty()
            || f.since.is_empty()
            || !ASK_ROUND.contains(&f.reason.as_str())
        {
            return None;
        }
        // As the asker's own sweep wakes: only while the round is young.
        let young = gocompat::time::GoTime::parse_rfc3339(&f.since)
            .and_then(|since| {
                (self.now)()
                    .to_system()
                    .duration_since(since.to_system())
                    .ok()
            })
            .is_none_or(|age| age < coord::ASK_UNCLAIMED);
        let wake = young && (f.reason == "reopened" || f.reason == "posted" && a.wake);
        let why = match f.reason.as_str() {
            "reopened" => ", again: the person wants it picked up",
            "released" => ", again: the agent that took it gave it back",
            "gone" => ", again: the agent that took it went away",
            _ => "",
        };
        let ask_me = self.inbox.autonomy(&b.id) == inbox::autonomy::ASK;
        let (refs, short, objective) = (t.refs(), t.short_ref(), t.objective.clone());
        if a.thread {
            // A board thread (card #134): the agents on the cards it names
            // hear it first, woken, while it is young. With "Agents answer
            // now" off (card #391) they read it at their next turn.
            let agents_now = a.wake;
            return Some(Plan {
                kind: row_kind::ASK,
                card_id: String::new(),
                to: f.heard,
                row: format!("ask:{}@{}", t.id, f.since),
                wake: young && agents_now,
                text: Box::new(move |name| {
                    if !agents_now {
                        return format!(
                            "{name} started a thread in this space ({refs}). It's a conversation, not a card:\n{objective}\nYou see it because it names your card or you. \"Agents answer now\" is off for it, so no agent was woken or handed it. A reply is optional: answer in the thread with board_comment {short} only if you have something to add or a correction. {}",
                            coord::THREAD_NOT_YOURS
                        );
                    }
                    format!(
                        "{name} started a thread in this space ({refs}). It's a question or a conversation, not a card:\n{objective}\nYou hear it because it names your card or you. If you can answer, answer in the thread with board_comment {short}; if it isn't about your work, leave it. Nobody waits on you: Irori gives it to another agent now too, and you can still add to it later. {}",
                        coord::THREAD_NOT_YOURS
                    )
                }),
            });
        }
        Some(Plan {
            kind: row_kind::ASK,
            card_id: String::new(),
            to: f.heard,
            row: format!("ask:{}@{}", t.id, f.since),
            wake,
            text: Box::new(move |name| {
                let mut s = format!(
                    "{name} asks the room ({refs}){why}:\n{objective}\nIf you can take it on now, ask_claim {short}: one agent takes it, and you hear if someone else got there first. If you're busy, leave it."
                );
                if ask_me {
                    s.push_str(" Your autonomy in this space is Ask me: a new card from it is a request to your person first.");
                }
                s
            }),
        })
    }

    /// A board ask the asking device handed to one of this person's agents
    /// (#91): that agent alone hears it, woken, and holds it.
    fn handed_plan(&self, b: &BoardLog, ev: &Event, f: RowFields, t: &Task) -> Option<Plan> {
        let a = t.ask.as_ref()?;
        if ev.device_id != a.owner
            || a.outcome.is_some()
            || f.taker.is_empty()
            || f.since.is_empty()
        {
            return None;
        }
        let ask_me = self.inbox.autonomy(&b.id) == inbox::autonomy::ASK;
        let (refs, short, objective) = (t.refs(), t.short_ref(), t.objective.clone());
        if a.thread {
            return Some(Plan {
                kind: row_kind::ASK,
                card_id: String::new(),
                row: format!("ask:{}@{}:{}", t.id, f.since, f.taker),
                to: vec![f.taker],
                wake: true,
                text: Box::new(move |name| {
                    format!(
                        "{name} started a thread in this space ({refs}), and Irori handed it to you. It's a question or a conversation, not a card:\n{objective}\nAnswer it in the thread with board_comment {short}: read it first, in case an agent already answered. {} If you can't answer it now, board_leave {short} and it goes to the next idle agent; ending your turn without answering does the same.",
                        coord::THREAD_NOT_YOURS
                    )
                }),
            });
        }
        Some(Plan {
            kind: row_kind::ASK,
            card_id: String::new(),
            row: format!("ask:{}@{}:{}", t.id, f.since, f.taker),
            to: vec![f.taker],
            wake: true,
            text: Box::new(move |name| {
                let mut s = format!(
                    "{name} asks the room ({refs}), and Irori handed it to you:\n{objective}\nYou hold it: no ask_claim needed. Take it on, or board_leave {short} and it goes to the next idle agent; ending your turn without touching it does the same."
                );
                if ask_me {
                    s.push_str(" Your autonomy in this space is Ask me: a new card from it is a request to your person first.");
                }
                s
            }),
        })
    }

    /// The hop a board row an agent writes now carries (coord's `hop_of`):
    /// its running turn's, by the agent's private key.
    pub(crate) fn board_hop(&self, actor: &str) -> i64 {
        if !actor.starts_with(AGENT_ACTOR_PREFIX) {
            return 0;
        }
        self.inbox.current_hop(&self.private_key(actor))
    }

    fn board_person_name(&self, b: &BoardLog, person: &str) -> String {
        let mut name = b.verifier.profile(person).unwrap_or_default();
        if name.is_empty()
            && let Some(p) = b.verifier.membership().person(person)
        {
            name = p.name.clone();
        }
        if name.is_empty() {
            "another person".into()
        } else {
            name
        }
    }

    /// The tools' view of a space's first board: its own name, which a
    /// space not ready yet doesn't have.
    fn tools_board(&self, s: super::boards::BoardSummary) -> tools::BoardInfo {
        let first = s.boards.first().map(|b| b.name.clone());
        tools::BoardInfo {
            board_id: s.board_id,
            tag: s.tag,
            name: first
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| s.name.clone()),
            people: s.people,
            space_board: String::new(),
            space_name: s.name,
        }
    }
}

/// An open card the device that made it handed to an agent (card #174):
/// that agent alone hears it, woken, once per hand-out. The board fold
/// counts a hand-out only from that device, so one it ignored (or that is
/// already stale: taken, passed on) tells nobody.
fn offer_plan(co: &Coord, f: RowFields, t: &Task) -> Option<Plan> {
    if f.reason != coord::OFFER_HANDED || f.to.is_empty() || f.since.is_empty() {
        return None;
    }
    let live = |t: &Task| t.live_offer().is_some_and(|o| o.to == f.to);
    let t = if live(t) {
        t.clone()
    } else {
        co.catch_up().ok()?;
        co.get_task(&t.id).filter(live)?
    };
    let (title, refs, short) = (t.title.clone(), t.refs(), t.short_ref());
    let (objective, done) = (t.objective.clone(), t.done_criteria.clone());
    Some(Plan {
        kind: row_kind::ASK,
        card_id: String::new(),
        row: format!("offer:{}@{}:{}", t.id, f.since, f.to),
        to: vec![f.to],
        wake: true,
        text: Box::new(move |name| {
            let mut s = format!(
                "{name} put up a card, and Irori handed it to you: {} ({refs})",
                gocompat::quote::quote(&title)
            );
            if objective.is_empty() {
                s.push('.');
            } else {
                s.push_str(&format!(":\n{objective}"));
            }
            if !done.is_empty() {
                s.push_str(&format!("\nDone when: {done}"));
            }
            s.push_str(&format!(
                "\nIt is yours to take: board_join {short} and start. If you can't take it on now, board_leave {short} and it goes to the next free agent."
            ));
            s
        }),
    })
}

/// The plan for the agents a comment mentions (card #177): each woken,
/// as "a request to wake one of your agents" in the receive table (§7.2),
/// so the person's receive setting, holds and rate limits decide.
fn mention_plan(ev: &Event, f: &RowFields, t: &Task, to: Vec<String>) -> Option<Plan> {
    let card_id = ev
        .actor
        .strip_prefix(AGENT_ACTOR_PREFIX)
        .unwrap_or_default()
        .to_string();
    if card_id.is_empty() && !ev.actor.starts_with(PERSON_ACTOR_PREFIX) {
        return None;
    }
    let title = gocompat::quote::quote(&t.title);
    let (refs, short) = (t.refs(), t.short_ref());
    let held = t.holders.iter().any(|h| to.contains(h));
    let ending = if t.holders.is_empty() {
        format!(
            "Nobody holds it. If the work is yours, board_join {short}; else answer on the card with board_comment if it asks you."
        )
    } else if held {
        "Answer on the card with board_comment if it asks you.".to_string()
    } else {
        "Its holders answer; answer on the card with board_comment if it asks you.".to_string()
    };
    let text = f.text.clone();
    Some(Plan {
        kind: row_kind::COMMENT,
        card_id,
        to,
        row: format!("{}/{}", ev.device_id, ev.seq),
        wake: true,
        text: Box::new(move |name| {
            format!("{name} mentioned you on {title} ({refs}):\n{text}\n{ending}")
        }),
    })
}

/// A card row's plan: a comment, an assignment, a blocker change, a review
/// request or a release request.
fn row_plan(co: &Coord, me: &str, ev: &Event, f: RowFields, t: &Task) -> Option<Plan> {
    let release =
        ev.kind == kinds::TASK_COMMENT && !f.to.is_empty() && f.text.starts_with(RELEASE_REQUEST);
    let kind = match ev.kind.as_str() {
        kinds::TASK_COMMENT if release => row_kind::LEASE,
        kinds::TASK_COMMENT => row_kind::COMMENT,
        kinds::TASK_ASSIGNED | kinds::REVIEW_REQUESTED => row_kind::ASSIGN,
        kinds::TASK_STATUS if f.blocker.is_none() => return None,
        _ => row_kind::BLOCKER,
    };
    let card_id = ev
        .actor
        .strip_prefix(AGENT_ACTOR_PREFIX)
        .unwrap_or_default()
        .to_string();
    if card_id.is_empty() && !ev.actor.starts_with(PERSON_ACTOR_PREFIX) {
        return None;
    }
    let to: Vec<String> = if release || ev.kind == kinds::TASK_ASSIGNED {
        vec![f.to.clone()]
    } else if ev.kind == kinds::REVIEW_REQUESTED {
        // Without `to` it goes to this person's dedicated reviewer, for
        // the card's repo or else the board (§10).
        if f.to.is_empty() {
            vec![co.reviewer(me, &t.repo_key)]
        } else {
            vec![f.to.clone()]
        }
    } else if ev.kind == kinds::TASK_COMMENT
        && card_id.is_empty()
        && t.open_ask().is_some_and(|a| a.thread)
    {
        // A person's follow-up in a board thread (card #134) goes to the
        // agents that answered, as on the private board; nobody holds it.
        co.thread_to(&t.id)
    } else {
        let mut v = t.holders.clone();
        v.extend(t.watchers.iter().cloned());
        v.extend(t.assigned.iter().cloned());
        v
    };
    let title = gocompat::quote::quote(&t.title);
    let short = t.short_ref();
    let k = ev.kind.clone();
    Some(Plan {
        kind,
        card_id,
        to,
        row: format!("{}/{}", ev.device_id, ev.seq),
        wake: f.wake,
        text: Box::new(move |name| match k.as_str() {
            kinds::TASK_COMMENT => f.text,
            kinds::TASK_ASSIGNED => format!("{name} assigned you {title}."),
            kinds::REVIEW_REQUESTED => format!(
                "{name} asks you to review {} for {title} ({short}). Review it with your own gh; Irori never posts reviews or merges.",
                f.pr
            ),
            kinds::BLOCKER_OPTION => {
                format!("{name} recommends an answer on {title}: {}", f.answer)
            }
            kinds::BLOCKER_RESOLVED => {
                format!("{name} answered a blocker on {title}: {}", f.answer)
            }
            kinds::BLOCKER_CLOSED => {
                let mut s = format!("{name} closed the blockers on {title}.");
                if !f.note.is_empty() {
                    s += &format!(" {}", f.note);
                }
                s
            }
            _ => {
                let q = f
                    .blocker
                    .as_ref()
                    .and_then(|v| v.get("question").or_else(|| v.get("reason")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("{name} raised a blocker on {title}: {q}")
            }
        }),
    })
}

impl tools::SharedBoards for super::ToolsView {
    fn boards(&self) -> Vec<tools::BoardInfo> {
        self.0
            .board_list()
            .into_iter()
            .filter(|s| s.ready && !self.0.is_hidden_space(&s.board_id))
            .flat_map(|s| {
                // Each board inside a space is its own entry on the
                // space's log (card #392).
                let inside = self.0.space_boards(&s.board_id).unwrap_or_default();
                let own = self.0.tools_board(s);
                let mut out = Vec::with_capacity(inside.len().max(1));
                for b in inside.into_iter().skip(1) {
                    out.push(tools::BoardInfo {
                        tag: b.tag,
                        name: b.name,
                        space_board: b.board_id,
                        ..own.clone()
                    });
                }
                out.insert(0, own);
                out
            })
            .collect()
    }

    fn board_of(&self, task_id: &str) -> Option<tools::BoardInfo> {
        self.0.board_of(task_id).map(|s| self.0.tools_board(s))
    }

    fn board_coord(
        &self,
        board_id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        let co = self
            .0
            .board(board_id)
            .map_err(super::boards::Error::into_coord)?
            .coord()
            .clone();
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            if k.is_empty() {
                out.push(String::new());
                continue;
            }
            out.push(
                self.0
                    .board_key(board_id, k)
                    .map_err(super::boards::Error::into_coord)?,
            );
        }
        Ok((co, out))
    }

    fn share(&self, task_id: &str, board_id: &str, _by: &str) -> Result<coord::Task, String> {
        let p = self
            .0
            .preview_share(task_id, board_id)
            .map_err(|e| e.to_string())?;
        self.0
            .share_to_board(task_id, board_id, &p.hash)
            .map_err(|e| e.to_string())
    }

    fn withdraw(&self, task_id: &str) -> Result<(), String> {
        self.0
            .withdraw_card(task_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn board_mentionables(&self, board_id: &str) -> Vec<coord::roster::AgentCard> {
        self.0.board_mentionables(board_id)
    }
}

impl Services {
    /// The other members of board `board_id` and their agents' cards, as
    /// an agent's @name may name them (card #186): `person:<id>` by the
    /// member's profile name, `agent:<cardId>` by the card's name. This
    /// person and the agents its devices host are left out: the roster
    /// has those, by their private keys.
    pub(crate) fn board_mentionables(&self, board_id: &str) -> Vec<coord::roster::AgentCard> {
        let Ok(b) = self.board(board_id) else {
            return Vec::new();
        };
        let me = self.person_id();
        let m = b.verifier.membership();
        let active: Vec<String> = m
            .members
            .iter()
            .filter(|p| p.state == irori_identity::membership::MEMBER_ACTIVE && p.person_id != me)
            .map(|p| p.person_id.clone())
            .collect();
        let mut out: Vec<coord::roster::AgentCard> = active
            .iter()
            .map(|id| coord::roster::AgentCard {
                key: format!("{PERSON_ACTOR_PREFIX}{id}"),
                name: b.verifier.profile(id).unwrap_or_default(),
                status: "idle".into(),
                ..Default::default()
            })
            .collect();
        let maps = super::boards::lock(&b.maps);
        let mut cards: Vec<_> = maps
            .cards
            .values()
            .filter(|c| !maps.by_id.contains_key(&c.card_id) && active.contains(&c.person_id))
            .collect();
        cards.sort_by(|a, b| a.card_id.cmp(&b.card_id));
        out.extend(cards.into_iter().map(|c| coord::roster::AgentCard {
            key: format!("{AGENT_ACTOR_PREFIX}{}", c.card_id),
            name: c.name.clone(),
            status: if c.status.is_empty() {
                "idle".into()
            } else {
                c.status.clone()
            },
            ..Default::default()
        }));
        out
    }
}
