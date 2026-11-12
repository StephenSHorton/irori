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
//!   #461), woken with `ref` `talk:<conv>` so a reply goes in the channel;
//! - the end of a huddle with a transcript in a space's channel (card
//!   #656), to this person's agents in the channel, once, woken;
//! - what GitHub said of a card's PR, heard on another person's device
//!   (card #548). This one isn't a row as it arrives: the PR poller's tick
//!   reads the fold ([`Services::tell_pr_news`]), so a row that came ahead
//!   of the card's holder still reaches them. It crosses as a comment
//!   does, under the receive setting: only that person's device vouches
//!   for it;
//! - a review ask this device missed live (merge desk §12.8), read from
//!   the fold on the same tick ([`Services::tell_missed_reviews`]).
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
use super::review_claim::clean_name;
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
    /// A review ping (merge desk §12.7): look at the PR, no review asked.
    look: bool,
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
        // A review ask is for a PR the card links: a row naming any other
        // is dropped (§12.12 B1b). And only a row the fold took is told:
        // its checks (the author's own agents, who may ask as a person)
        // are the fold's (§12.3, §12.7). The sender links the PR just
        // before it asks, so the link can come in the same sync and not
        // be folded yet: fold what the log holds before dropping it.
        let t = if ev.kind == kinds::REVIEW_REQUESTED {
            let pos = coord::RowPos::of(ev);
            let ok = |co: &Coord| {
                co.get_task(&f.task_id)
                    .filter(|t| links_pr(t, &f.pr) && co.review_row_folded(&t.id, &pos))
            };
            match ok(co).or_else(|| co.catch_up().ok().and_then(|()| ok(co))) {
                Some(t) => t,
                None => return,
            }
        } else {
            t
        };
        let who = self.board_person_name(b, &person);
        // A comment (a blocker reply too) naming this person: a desktop
        // notification on each of their devices that opens the card at the
        // comment (card #177). Their agents named in it hear below.
        let me_key = format!("{PERSON_ACTOR_PREFIX}{me}");
        if ev.kind == kinds::TASK_COMMENT && f.mentions.iter().any(|m| m.key == me_key) {
            self.mention_note(b, &t, &who, &person, &f);
        }
        // A card given to another person came back (decision 69): tell
        // this person, and their assistant what happened to the gift.
        if ev.kind == kinds::TASK_STATUS {
            self.blocker_note(b, &t, &who, &person, &f);
        }
        if matches!(
            ev.kind.as_str(),
            kinds::TASK_STATUS | kinds::BLOCKER_RESOLVED
        ) {
            self.given_follow_row(b, &t);
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
            let reviewer = if ev.kind == kinds::REVIEW_REQUESTED && f.look {
                reviewer_name(b, co, &t.id)
            } else {
                String::new()
            };
            if let Some(mut p) = row_plan(co, &me, ev, f, &t, reviewer) {
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
        let hop = coord::row_hop(&ev.payload);
        self.send_ref(b, person, who, hop, p, format!("task:{}", t.id));
    }

    /// Tells the agents this device hosts what GitHub said of their cards'
    /// PRs on another person's device (card #548). A device of this person
    /// that wrote the same told them as it wrote, whichever row the fold
    /// kept. The inbox bridges a mark to an agent once, so a tick with
    /// nothing new sends nothing, across restarts too.
    pub(crate) fn tell_pr_news(&self) {
        let me = self.person_id();
        for b in self.shown_boards() {
            for n in b.coord().pr_news() {
                let people: Vec<String> = n
                    .writers
                    .iter()
                    .filter_map(|d| b.verifier.person_of(d))
                    .filter(|p| !p.is_empty())
                    .collect();
                if people.contains(&me) {
                    continue;
                }
                let Some(person) = people.first() else {
                    continue;
                };
                let who = self.board_person_name(&b, person);
                let text = n.notice.text;
                let plan = Plan {
                    kind: row_kind::COMMENT,
                    card_id: String::new(),
                    to: n.notice.to,
                    row: n.mark,
                    wake: n.notice.wake,
                    text: Box::new(move |name| format!("Heard on {name}'s device: {text}")),
                };
                self.send_ref(&b, person, &who, 0, plan, n.notice.reference);
            }
        }
    }

    /// Delivers the review asks this device's agents may have missed live
    /// (merge desk §12.8): a resync skips rows, and an ask can come before
    /// the asked agent's card-of row. Each is the newest ask of a PR card
    /// still asked, sent under the live row's key, so the inbox's
    /// bridged-once set makes it arrive once whichever path runs first.
    /// Asks this person's devices wrote are theirs to have routed.
    pub(crate) fn tell_missed_reviews(&self) {
        let me = self.person_id();
        for b in self.shown_boards() {
            let co = b.coord();
            for m in co.missed_review_asks() {
                let Some(person) = b
                    .verifier
                    .person_of(&m.pos.dev)
                    .filter(|p| !p.is_empty() && *p != me)
                else {
                    continue;
                };
                let Some(t) = co.get_task(&m.card) else {
                    continue;
                };
                let card_id = m
                    .from
                    .strip_prefix(AGENT_ACTOR_PREFIX)
                    .unwrap_or_default()
                    .to_string();
                let by_person = m.from.starts_with(PERSON_ACTOR_PREFIX);
                let who = self.board_person_name(&b, &person);
                let (pr, title, short) = (m.pr, gocompat::quote::quote(&t.title), t.short_ref());
                // Only a PR card's ask is caught up, so review_claim works.
                let plan = Plan {
                    kind: row_kind::REVIEW,
                    card_id,
                    to: vec![m.to],
                    row: format!("{}/{}", m.pos.dev, m.pos.seq),
                    wake: m.wake,
                    text: Box::new(move |name| {
                        review_ask_text(name, by_person, false, true, &pr, &title, &short, "")
                    }),
                };
                self.send_ref(&b, &person, &who, 0, plan, format!("task:{}", t.id));
            }
        }
    }

    /// [`Services::send_plan`] for a row whose message refers to `ref_`
    /// (a card's `task:<id>`, a channel's `talk:<conv>`).
    fn send_ref(&self, b: &BoardLog, person: &str, who: &str, hop: i64, p: Plan, ref_: String) {
        let to: Vec<String> =
            p.to.iter()
                .filter(|k| k.starts_with(AGENT_ACTOR_PREFIX))
                .map(|k| self.private_key(k))
                .collect();
        self.send_to(b, person, who, hop, p, ref_, to);
    }

    /// [`Services::send_ref`] to agents by their keys on this device.
    #[allow(clippy::too_many_arguments)]
    fn send_to(
        &self,
        b: &BoardLog,
        person: &str,
        who: &str,
        hop: i64,
        p: Plan,
        ref_: String,
        to: Vec<String>,
    ) {
        if to.is_empty() {
            return;
        }
        // A review ask's text is fixed but for names, clipped (§12.3).
        let review = p.kind == row_kind::REVIEW;
        let clip = |n: &str| if review { clean_name(n) } else { n.to_string() };
        let who = clip(who);
        let name = if p.card_id.is_empty() {
            who.clone()
        } else {
            let card = super::boards::lock(&b.maps)
                .cards
                .get(&p.card_id)
                .map(|c| clip(&c.name))
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
            hop,
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
        // A huddle's end post never names anyone ("Agents answer now" is
        // off on it): its transcript is told apart.
        if r.call
            .as_ref()
            .is_some_and(|m| m.op == coord::talk::CALL_END && m.transcript)
        {
            self.call_end_bridge(b, ev, &r);
            return;
        }
        if r.conv_id.is_empty() {
            return;
        }
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        if person.is_empty() || person == self.person_id() {
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
        // Two copies of one post id from two devices (both of a person's
        // desktops answered): Talk kept one, and the other wakes nobody.
        if self
            .talk
            .posts(&r.conv_id)
            .iter()
            .any(|p| p.post_id == r.post_id && p.device != ev.device_id)
        {
            return;
        }
        // A question to this person's assistant (assistants-talk.md §5)
        // goes to the answering path whatever "Agents answer now" says.
        // Its card is no agent this device hosts, so nothing is woken.
        let mine = self.my_assistant_key(b);
        let to: Vec<String> = r
            .mentions
            .iter()
            .filter(|k| k.starts_with(AGENT_ACTOR_PREFIX) && Some(k.as_str()) != mine.as_deref())
            .cloned()
            .collect();
        if let Some(me) = &mine
            && r.mentions.contains(me)
            && coord::talk::is_assistants_channel(&ch.id, &b.id)
            && r.reply_to.as_deref().is_none_or(str::is_empty)
        {
            self.assistant_question(b, ev, &r, &person);
        }
        if !r.agents_now || to.is_empty() {
            return;
        }
        let card_id = ev
            .actor
            .strip_prefix(AGENT_ACTOR_PREFIX)
            .unwrap_or_default()
            .to_string();
        if card_id.is_empty() && !ev.actor.starts_with(PERSON_ACTOR_PREFIX) {
            return;
        }
        let who = self.board_person_name(b, &person);
        // Its files (card #643), named as tools' mention notice names
        // them: the agent can't otherwise see them.
        let mut text = r.text;
        for f in &r.attach {
            if !text.is_empty() {
                text.push('\n');
            }
            text += &format!("file: {} (attachment_get)", f.describe());
        }
        let (conv, chan) = (r.conv_id.clone(), ch.name);
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
        let hop = coord::row_hop(&ev.payload);
        self.send_ref(b, &person, &who, hop, p, format!("talk:{}", r.conv_id));
    }

    /// The board key of this person's assistant card on `b`, if it has one.
    fn my_assistant_key(&self, b: &BoardLog) -> Option<String> {
        let card = b.verifier.profile_assistant_card(&self.person_id());
        (!card.is_empty()).then(|| format!("{AGENT_ACTOR_PREFIX}{card}"))
    }

    /// Another person's end of a huddle with a transcript in a space's
    /// channel (card #656): this person's agents in the channel hear it once
    /// per huddle (however many end posts it has), woken, as the ending device's own agents hear it from
    /// [`tools::talk::tell_call_end`]. A person DM's is never told: it
    /// isn't a channel, and its hidden space never bridges anyway.
    fn call_end_bridge(&self, b: &BoardLog, ev: &Event, r: &coord::talk::PostRow) {
        let Some(m) = r.call.as_ref() else {
            return;
        };
        let Some(person) = b.verifier.person_of(&ev.device_id) else {
            return;
        };
        // Only a person ends a huddle.
        if person.is_empty()
            || person == self.person_id()
            || m.call_id.is_empty()
            || !ev.actor.starts_with(PERSON_ACTOR_PREFIX)
        {
            return;
        }
        if self.talk.pair_other(&r.conv_id).is_some() {
            return;
        }
        let Some(ch) = self
            .talk
            .channel(&r.conv_id)
            .filter(|c| c.space_id == b.id && !c.archived)
        else {
            return;
        };
        // This person's own end of it told them already (tell_call_end).
        let me = self.person_id();
        let mine = self.talk.posts(&r.conv_id).iter().any(|p| {
            p.call.as_ref().is_some_and(|c| {
                c.op == coord::talk::CALL_END && c.call_id == m.call_id && c.transcript
            }) && b.verifier.person_of(&p.device).is_some_and(|x| x == me)
        });
        if mine {
            return;
        }
        // This person's agents go by their own keys; another person's
        // (board keys) hear it on their own Core.
        let mut to: Vec<String> = Vec::new();
        for k in self.talk.members(&r.conv_id) {
            if k.kind == coord::talk::AGENT && !coord::board_form(&k.key) && !to.contains(&k.key) {
                to.push(k.key);
            }
        }
        if to.is_empty() {
            return;
        }
        let text = tools::talk::call_end_text(
            &ch.name,
            &m.call_id,
            self.talk.call_posts(&m.call_id).len(),
        );
        let who = self.board_person_name(b, &person);
        let p = Plan {
            kind: row_kind::COMMENT,
            card_id: String::new(),
            to: Vec::new(),
            row: tools::talk::call_end_row(&m.call_id),
            wake: true,
            text: Box::new(move |_| text),
        };
        let hop = coord::row_hop(&ev.payload);
        self.send_to(b, &person, &who, hop, p, format!("talk:{}", r.conv_id), to);
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

    /// The desktop notification for a card given back to this person
    /// (decision 69, plan §6.2): a `task.status` that raised a give-back
    /// item for them, on a card they own. Its id holds the blocker, so each
    /// device posts it once; opening it opens the card. It settles when
    /// they pick Keep it or Cancel it (`notifications.rs`).
    fn blocker_note(&self, b: &BoardLog, t: &Task, who: &str, person: &str, f: &RowFields) {
        let n = &self.notifier.0;
        if n.is_headless() {
            return;
        }
        let Some(id) = f
            .blocker
            .as_ref()
            .and_then(|v| v.get("id"))
            .and_then(serde_json::Value::as_str)
        else {
            return;
        };
        let me_key = format!("{PERSON_ACTOR_PREFIX}{}", self.person_id());
        let Some(bl) = t.blockers.iter().find(|bl| bl.id == id) else {
            return;
        };
        let to_me = bl.on.as_ref().is_some_and(|on| on.reference == me_key);
        if !bl.given_back || !bl.open() || !to_me || b.verifier.key_person(&bl.raised_by) != person
        {
            return;
        }
        let short = if t.short.is_empty() {
            String::new()
        } else {
            format!(" ({})", t.short_ref())
        };
        let target = notify::Target {
            tab: notify::TAB_BOARD.into(),
            ref_: format!("task:{}", t.id),
        };
        let opener = n.clone();
        n.post(notify::Note {
            id: format!("blocker:{}:{}", t.id, bl.id),
            title: format!("{who} gave back “{}”{short}", notify::preview(&t.title)),
            body: notify::preview(coord::given_back_reason(&bl.context)),
            level: notify::Level::Info,
            about: Some(coord::notifications::PostedRow {
                kind: coord::notifications::KIND_BLOCKER.into(),
                board_id: b.id.clone(),
                task_id: t.id.clone(),
                blocker_id: bl.id.clone(),
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

    pub(super) fn board_person_name(&self, b: &BoardLog, person: &str) -> String {
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
        let first = s.boards.first();
        let description = first.map(|b| b.description.clone()).unwrap_or_default();
        // A first board never closes; read it anyway (card #690).
        let closed = first.is_some_and(|b| b.closed);
        let first = first.map(|b| b.name.clone());
        tools::BoardInfo {
            board_id: s.board_id,
            tag: s.tag,
            name: first
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| s.name.clone()),
            people: s.people,
            space_board: String::new(),
            space_name: s.name,
            description,
            closed,
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
    let ending = if let Some(c) = &t.pr_card {
        // A PR card has no holders: the talk is on the PR (§12.4).
        format!(
            "This is the card for pull request {0}. Read it with `gh pr view {0} --comments` and answer there, on the PR by its URL; to review it, run review_claim pr={0} first.",
            c.pr
        )
    } else if t.holders.is_empty() {
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

/// What a review ask or ping says (merge desk §12.4, §12.7): fixed text,
/// the PR's URL (rebuilt from its parts, so the row's text never rides
/// along), the card's title and ref, and names clipped by the sender.
/// `by_person` for an ask from the PR author's person (their Irori picked
/// the agent); `claimable` when the PR has a PR card, so review_claim
/// works; `reviewer` is who has the review, for a ping. The live bridge
/// and the catch-up both word it here.
#[allow(clippy::too_many_arguments)]
fn review_ask_text(
    name: &str,
    by_person: bool,
    look: bool,
    claimable: bool,
    pr: &str,
    title: &str,
    short: &str,
    reviewer: &str,
) -> String {
    const CLAIM: &str = "if someone else holds it, comment on the PR instead.";
    let pr = &pr_url(pr);
    if look {
        let who = if by_person {
            format!("{name}'s Irori")
        } else {
            name.to_string()
        };
        return format!(
            "{who}: look at {pr} ({title}, {short}) and comment on the PR if your work bears on it (`gh pr view {pr} --comments`); {reviewer} has the review."
        );
    }
    let claim = if claimable {
        format!("Run review_claim pr={pr} first; {CLAIM} ")
    } else {
        String::new()
    };
    if by_person {
        format!(
            "{name}'s Irori picked you to review {pr} for {title} ({short}). {claim}Read it with `gh pr view {pr} --comments` and answer on the PR by its URL; Irori never posts reviews or merges."
        )
    } else {
        format!(
            "{name} asks you to review {pr} for {title} ({short}). {claim}Read it with `gh pr view {pr} --comments`, review it with your own gh and answer on the PR by its URL ({pr}); Irori never posts reviews or merges."
        )
    }
}

/// A PR's URL as `pr_link` reads it: `https://github.com/<repo>/pull/<n>`.
fn pr_url(pr: &str) -> String {
    let l = coord::Link {
        kind: coord::LINK_URL.into(),
        reference: pr.trim().into(),
        ..coord::Link::default()
    };
    match coord::pr_link(&l) {
        Some((repo, n)) => format!("https://github.com/{repo}/pull/{n}"),
        None => String::new(),
    }
}

/// Who has the review of PR card `card`, for a ping (§12.7): the live
/// claim's holder, else the newest ask's agent, by its card's clipped
/// name. The coord folds on its own thread, so the log is folded first
/// when it knows neither yet.
fn reviewer_name(b: &BoardLog, co: &Coord, card: &str) -> String {
    let who = |co: &Coord| {
        co.pick_facts(card).and_then(|f| {
            f.live_claim
                .map(|l| l.holder)
                .or_else(|| f.asks.last().map(|a| a.to.clone()))
        })
    };
    let key = who(co).or_else(|| co.catch_up().ok().and_then(|()| who(co)));
    key.and_then(|k| {
        let id = k.strip_prefix(AGENT_ACTOR_PREFIX)?.to_string();
        super::boards::lock(&b.maps)
            .cards
            .get(&id)
            .map(|c| clean_name(&c.name))
    })
    .filter(|n| !n.is_empty())
    .unwrap_or_else(|| "another agent".into())
}

/// Whether the card links the PR (as the claim resource reads both): a
/// review ask for any other is refused on receipt.
fn links_pr(t: &Task, pr: &str) -> bool {
    let Some(want) = coord::pr_claim_resource(pr) else {
        return false;
    };
    // A PR card always links its own PR, whatever links were removed.
    t.pr_card
        .as_ref()
        .is_some_and(|c| coord::pr_claim_resource(&c.pr).as_deref() == Some(&want))
        || t.links.iter().any(|l| {
            coord::pr_link(l).is_some()
                && coord::pr_claim_resource(&l.reference).as_deref() == Some(&want)
        })
}

/// A card row's plan: a comment, an assignment, a blocker change, a review
/// request or a release request. `reviewer` names who has the review, for
/// a ping.
fn row_plan(
    co: &Coord,
    me: &str,
    ev: &Event,
    f: RowFields,
    t: &Task,
    reviewer: String,
) -> Option<Plan> {
    let release =
        ev.kind == kinds::TASK_COMMENT && !f.to.is_empty() && f.text.starts_with(RELEASE_REQUEST);
    let kind = match ev.kind.as_str() {
        kinds::TASK_COMMENT if release => row_kind::LEASE,
        kinds::TASK_COMMENT => row_kind::COMMENT,
        kinds::TASK_ASSIGNED => row_kind::ASSIGN,
        kinds::REVIEW_REQUESTED => row_kind::REVIEW,
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
    // A ping never wakes, whatever the row says (§12.7).
    let look = ev.kind == kinds::REVIEW_REQUESTED && f.look;
    let by_person = card_id.is_empty();
    let claimable = ev.kind == kinds::REVIEW_REQUESTED && co.pr_card_for(&f.pr).is_some();
    Some(Plan {
        kind,
        card_id,
        to,
        row: format!("{}/{}", ev.device_id, ev.seq),
        wake: f.wake && !look,
        text: Box::new(move |name| match k.as_str() {
            kinds::TASK_COMMENT => f.text,
            kinds::TASK_ASSIGNED => format!("{name} assigned you {title}."),
            kinds::REVIEW_REQUESTED => review_ask_text(
                name, by_person, look, claimable, &f.pr, &title, &short, &reviewer,
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
                        description: b.description,
                        closed: b.closed,
                        ..own.clone()
                    });
                }
                out.insert(0, own);
                out
            })
            .collect()
    }

    fn board_of(&self, task_id: &str) -> Option<tools::BoardInfo> {
        let s = self.0.board_of(task_id)?;
        let inner = self
            .0
            .route(task_id)
            .ok()
            .and_then(|(_, co)| co.get_task(task_id))
            .and_then(|t| s.boards.iter().skip(1).find(|b| b.board_id == t.board))
            .cloned();
        let own = self.0.tools_board(s);
        Some(match inner {
            Some(b) => tools::BoardInfo {
                tag: b.tag,
                name: b.name,
                space_board: b.board_id,
                description: b.description,
                closed: b.closed,
                ..own
            },
            None => own,
        })
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

    fn share(
        &self,
        task_id: &str,
        board_id: &str,
        space_board: &str,
        _by: &str,
    ) -> Result<coord::Task, String> {
        let p = self
            .0
            .preview_share(task_id, board_id)
            .map_err(|e| e.to_string())?;
        self.0
            .share_to_space_board(task_id, board_id, space_board, &p.hash)
            .map_err(|e| e.to_string())
    }

    fn withdraw(&self, task_id: &str) -> Result<(), String> {
        self.0
            .withdraw_card(task_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn share_give(
        &self,
        task_ids: &[String],
        board_id: &str,
        to: &str,
    ) -> Result<Vec<coord::Task>, String> {
        self.0
            .share_give(task_ids, board_id, to)
            .map_err(|e| e.to_string())
    }

    fn board_mentionables(&self, board_id: &str) -> Vec<coord::roster::AgentCard> {
        self.0.board_mentionables(board_id)
    }

    fn dm_people(&self) -> Vec<tools::PersonInfo> {
        self.0
            .dm_people()
            .into_iter()
            .map(|p| tools::PersonInfo {
                key: p.key,
                name: p.name,
            })
            .collect()
    }

    fn dm_waiting(&self, other: &str) -> &'static str {
        self.0.dm_waiting(other)
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
        // Every active member's ways of being named as an assistant, so
        // that no other card can squat on one (assistants talk §3).
        let all_active: Vec<(String, String)> = m
            .members
            .iter()
            .filter(|p| p.state == irori_identity::membership::MEMBER_ACTIVE)
            .map(|p| {
                (
                    p.person_id.clone(),
                    b.verifier.profile(&p.person_id).unwrap_or_default(),
                )
            })
            .collect();
        let first_word = |n: &str| n.split_whitespace().next().unwrap_or("").to_lowercase();
        let alias_of = |owner: &str, name: &str| -> Option<String> {
            let w = first_word(name);
            let word = name.split_whitespace().next()?;
            if w.is_empty() || w == name.trim().to_lowercase() {
                return None; // one word: the default form is the alias
            }
            if all_active
                .iter()
                .any(|(id, n)| id != owner && first_word(n) == w)
            {
                return None;
            }
            Some(format!("{word}'s assistant"))
        };
        let mut squatted: Vec<String> = Vec::new();
        for (id, n) in &all_active {
            if n.trim().is_empty() {
                continue;
            }
            squatted.push(format!("{}'s assistant", n.trim()).to_lowercase());
            if let Some(a) = alias_of(id, n) {
                squatted.push(a.to_lowercase());
            }
        }
        let maps = super::boards::lock(&b.maps);
        let mut cards: Vec<_> = maps
            .cards
            .values()
            .filter(|c| !maps.by_id.contains_key(&c.card_id) && active.contains(&c.person_id))
            .collect();
        cards.sort_by(|a, b| a.card_id.cmp(&b.card_id));
        for c in cards {
            let status = if c.status.is_empty() {
                "idle".to_string()
            } else {
                c.status.clone()
            };
            let key = format!("{AGENT_ACTOR_PREFIX}{}", c.card_id);
            if c.kind == coord::roster::ASSISTANT_KIND {
                // An assistant counts only when its owner's profile points
                // at it; any other kind-assistant card is not listed.
                if b.verifier.profile_assistant_card(&c.person_id) != c.card_id {
                    continue;
                }
                let profile = b.verifier.profile(&c.person_id).unwrap_or_default();
                let chosen = b.verifier.profile_assistant_name(&c.person_id);
                let mut names: Vec<String> = Vec::new();
                if !chosen.trim().is_empty() {
                    names.push(chosen.trim().to_string());
                }
                if !profile.trim().is_empty() {
                    names.push(format!("{}'s assistant", profile.trim()));
                    names.extend(alias_of(&c.person_id, &profile));
                }
                if names.is_empty() {
                    names.push(c.name.clone());
                }
                names.dedup();
                for name in names {
                    out.push(coord::roster::AgentCard {
                        key: key.clone(),
                        name,
                        agent: coord::roster::ASSISTANT_KIND.into(),
                        status: status.clone(),
                        ..Default::default()
                    });
                }
                continue;
            }
            if squatted.contains(&c.name.trim().to_lowercase()) {
                continue;
            }
            out.push(coord::roster::AgentCard {
                key,
                name: c.name.clone(),
                status,
                ..Default::default()
            });
        }
        out
    }
}

#[cfg(test)]
mod links_pr_tests {
    use super::*;

    #[test]
    fn only_a_linked_pr_passes() {
        let pr = "https://github.com/o/r/pull/7";
        let mut t = Task::default();
        t.links.push(coord::Link {
            kind: "url".into(),
            reference: pr.into(),
            ..coord::Link::default()
        });
        assert!(links_pr(&t, pr));
        assert!(!links_pr(&t, "https://github.com/o/r/pull/8"));
        assert!(!links_pr(&t, ""));
        assert!(!links_pr(&Task::default(), pr));
    }
}
