//! The cards window (I:cards_service.go): the grid, one opened card and
//! the person's answer to a blocker, as BoardService's reads. Every list is
//! always serialized, so the webview never gets null or a missing array.
//!
//! Cards of shared boards join the grid ([`Window::board_cards`]) and a
//! card's page reads the log it lives in (`boards.rs`, v0.5 §8).

use std::collections::{BTreeMap, HashMap};

use coord::FileRef;
use coord::{Activity, Comment, Link, Step, Task};
use gocompat::time::GoTime;
use inbox::{Delivery, st};
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::ts;

/// The private board's scope: the person's own devices.
pub const SCOPE_MINE: &str = "mine";
pub const SCOPE_MINE_LABEL: &str = "My devices";
/// How many activity lines Card returns.
const RECENT_LINES: usize = 8;
/// How many thread entries Card returns, the newest.
pub const MAX_THREAD_VIEW: usize = 300;
/// The blocker id of an input_required card's question.
pub const NOTE_ITEM: &str = "note";
const NOTE_QUESTION: &str = "The agent on it is waiting on your input.";
/// How recent agents' messages to each other on a card must be for its
/// tile to show them talking (card #22).
pub const TALK_WINDOW_MS: i64 = 5 * 60 * 1000;
const MAX_TALKING: usize = 4;
/// A working card with no activity this long, and nobody busy on it,
/// shows as stale (card #16).
pub const STALE_AFTER_MS: i64 = 2 * 3600 * 1000;

// ---- window types -------------------------------------------------------------

/// Someone on a card, as the window shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    pub key: String,
    /// The roster's name, "You" for the person, else the key.
    pub name: String,
    /// The roster's status, "gone" without a live card, or "person".
    pub status: String,
    pub live: bool,
    /// This device's own name with " (this one)" (e.g. "Macbook Pro (this
    /// one)"), or the other device's name (card #489).
    #[serde(rename = "where")]
    pub where_: String,
    /// It is on this device, so what only works here (Attach, Logs, End)
    /// shows; `where` is only for reading (card #489).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub here: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub assigned: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub waiting: bool,
    /// The open steps it owns, when it's on the card as a step owner
    /// rather than a holder (card #86).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub steps: Vec<String>,
    /// Irori started it for this card (card #448): the card shows End, not
    /// Revive, and the roster says "for #N".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub spawned: bool,
    /// The hand-off note it was offered the card with (card #85).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub handoff: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub color: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub offline: bool,
    /// The device its rows come through (card #544).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device_via: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub conflict: String,
    /// While it's idle: its last transcript activity, about when its turn
    /// ended (card #305: a held card with an idle holder and no blocker
    /// turns to "Idle, no reason given" after a while).
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    #[specta(type = Option<ts::Time>, optional)]
    pub idle_since: GoTime,
    /// The agent's own face, open-bot-avatar JSON (card #407); "" wears
    /// the default for its name.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub avatar: String,
}

/// Done steps over all steps; total 0 means no bar.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardProgress {
    pub done: i64,
    pub total: i64,
}

/// What a card's holder is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Holding {
    #[default]
    Nobody,
    Person,
    Waiting,
    Assigned,
    Active,
    Idle,
    /// Its session runs but leaves its wakes unread (card #110).
    Silent,
    Offline,
    Gone,
}

/// The holder line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardHolder {
    pub who: AgentRef,
    pub state: Holding,
    pub others: i64,
}

/// Another card: a subtask's parent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardRef {
    pub id: String,
    pub short: String,
    pub title: String,
}

/// Ask states, as the tile's line shows them.
pub const ASK_OPEN: &str = "open";
pub const ASK_WAITING: &str = "waiting";
pub const ASK_UNCLAIMED: &str = "unclaimed";
pub const ASK_TAKEN: &str = "taken";
/// Irori handed it to one idle agent, which hasn't acted on it yet (#91).
pub const ASK_HANDED: &str = "handed";
pub const ASK_CLOSED: &str = "closed";

/// An ask on its tile and page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardAsk {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub taker: Option<AgentRef>,
    pub told: i64,
    pub owner: String,
    pub wake: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Outcome>)]
    pub outcome: Option<coord::Outcome>,
    pub outcome_ref: String,
    /// The agents the ask went to; empty: the room.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub mentions: Vec<CardMention>,
}

/// A mention on a thread item or an ask, with the agent as the window
/// knows it now (card #77).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardMention {
    /// As written, for the @name in the text.
    pub name: String,
    pub who: AgentRef,
    /// Stored gone, or gone now: not delivered.
    pub gone: bool,
}

/// `ms` as the thread shows them.
pub(crate) fn card_mentions(ms: &[coord::Mention], p: &People) -> Vec<CardMention> {
    ms.iter()
        .map(|m| {
            let who = p.ref_(&m.key);
            CardMention {
                name: m.name.clone(),
                gone: m.gone || who.status == "gone",
                who,
            }
        })
        .collect()
}

/// One card in the grid.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardSummary {
    pub id: String,
    pub short: String,
    pub short_was: String,
    pub title: String,
    #[specta(type = ts::TaskState)]
    pub state: String,
    pub repo_key: String,
    pub scope: String,
    pub scope_label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub board: String,
    /// The work/personal side it's on (card #494); empty for any side.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub side: String,
    /// Why it waits on its side (card #495): "no device here runs Work".
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub side_wait: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub tag: String,
    pub progress: CardProgress,
    pub open_blockers: i64,
    pub blocker_question: String,
    /// An agent says it's done and it waits on the person's Done (card
    /// #112): the grid shows "Ready for you" for it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub ready: bool,
    pub waiting: i64,
    /// Open blockers that wait on something other than the person: another
    /// card, a lease, an agent, something external (card #158). They close
    /// by themselves, so they are not counted in open_blockers.
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub waiting_on: i64,
    /// What the first of those waits on, as the window names it: "#12" for
    /// a card, the agent's name, the resource or the outside thing.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub wait_on: String,
    /// The first wait in full where wait_on is the short form (card #553):
    /// "Irori 0.12.4 on Mac and PC" for wait_on's "Irori 0.12.4". The
    /// state pill's tooltip says it.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub wait_full: String,
    /// What kind of thing the first wait is on (card #487): "task",
    /// "lease", "agent", "version" or "external", so Home can name a lease
    /// plainly.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub wait_kind: String,
    /// The first wait is on an agent that is gone (card #487): nothing
    /// will end it by itself, so Home files the card under Stuck.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub wait_gone: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub wait_question: String,
    /// The card the first wait is on, by id (Home counts the cards that
    /// wait on a red main's card by it, card #436).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub wait_task: String,
    /// A "main is red" card's failing test (card #436): Home leads with
    /// it while it is open.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub main_red: String,
    /// Where the first wait's chain of cards ends at the person (card
    /// #347): "your answer on #12". A PR never waits on the person (card
    /// #471): agents merge it, or another member's agents approve it.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub wait_you: String,
    /// The cards this one waits on each other with (card #481), itself
    /// among them, in wait order: none of them can move until someone
    /// drops a wait. Every card of the loop carries the same list, so
    /// Home's Needs you shows it once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub wait_loop: Vec<CardRef>,
    /// The card this one's waits end at that nobody holds and nothing
    /// will hand out (card #481).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub wait_stuck: Option<CardRef>,
    /// The open PR an in-review card waits on (card #471): the card shows
    /// a small "waits on PR #N" chip with its link, never a Needs you row.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub wait_pr: Option<WaitPr>,
    /// The PR an agent's open external wait asks the person to merge (card
    /// #509): a Needs you row with the PR's link, until it is known merged
    /// or closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub merge_pr: Option<WaitPr>,
    /// A wait no agent will end (card #520): on a card nobody holds, an
    /// item waits on an agent that is gone, or an outside wait (not a PR's
    /// merge) was raised by one. A Needs you row with Hand to an agent,
    /// until it is handed on, the card is stopped or someone takes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub gone_wait: Option<GoneWait>,
    pub working: Vec<AgentRef>,
    pub watching: Vec<AgentRef>,
    pub holder: CardHolder,
    #[specta(type = ts::Time)]
    pub updated_at: GoTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub returned: Option<AgentRef>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub stale: bool,
    pub devices: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub accent: String,
    pub talking: Vec<AgentRef>,
    /// A shared board's card that one of the person's own agents holds or
    /// watches: Home lists it beside the private cards (card #199).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub ours: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub ask: Option<CardAsk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub parent: Option<CardRef>,
    /// The person's thread this card was made from (card #155).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub from_thread: Option<CardRef>,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub subtasks: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub subtask_blocked: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub pinned: bool,
    /// The person paused the card (card #207).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub paused: bool,
    #[serde(skip_serializing_if = "is_zero_f")]
    #[specta(optional, type = i64)]
    pub pin_rank: f64,
    /// Why the agent Irori tried to start for this card couldn't start (the
    /// failed spawn request's reason, card #446/#449); empty while one is
    /// requested or bound. A `not trusted: ` reason gets Trust and start.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub cant_start: String,
    /// The device an agent is starting on for this card ("<its own name>
    /// (this one)" or the other device's name, card #489), while Irori's
    /// spawn request waits for it (card #448).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub starting: String,
    /// The device whose cap holds this card back: it needs an agent, but
    /// that device already runs as many as it may (card #448).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub at_cap: String,
    /// Its place in that device's spawn queue, from 1 (card #450): "queued:
    /// 1st for the PC". 0 when it isn't queued.
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub queue_place: i64,
    /// While a Claude usage limit holds spawns, when they start again
    /// (card #450): "queued: usage limit".
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Time>)]
    pub usage_limit: Option<GoTime>,
    /// Where the card sits on the Board's canvas (card #93), once placed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub pos: Option<CanvasPos>,
    /// Where the holder's work is (card #137): its worktree, branch, last
    /// commit and whether it's pushed.
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub where_: Option<CardWhere>,
    /// The holder's last line on what's done and what's next (card #137).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub left_off: Option<CardLeftOff>,
    /// The card's words the grid doesn't show, for the Board's search box
    /// (card #171): its objective, done-when, step texts and its creator's
    /// name, one per line.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub search_text: String,
    /// A shared board's card: the member who made it (they or one of their
    /// agents), by person id (card #268); empty off a board or when the
    /// board can't say.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub owner: String,
}

/// A card's first wait whose agent is gone, on a card nobody holds (card
/// #520): "#73 waits on kestrel (gone)".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GoneWait {
    /// The item's id on the card.
    pub blocker_id: String,
    /// The agent that is gone: the one waited on, else the raiser.
    pub agent: AgentRef,
    /// The item waits on that agent; else it raised an outside wait.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub waited_on: bool,
    /// What an outside wait is on, as raised ("CI on the release").
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub on: String,
    pub question: String,
    /// More such items on the card, beyond this one.
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub more: i64,
    /// Why Hand to an agent can't move the card (a shared board's card,
    /// an open ask, subtasks): the row says it and offers only Open.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub no_hand_on: String,
}

/// The open PR an in-review card waits on (card #471).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WaitPr {
    pub repo: String,
    pub number: i64,
    pub url: String,
    /// GitHub says the PR needs an approval before it merges: it waits on
    /// another member's agents, never the person's or their own agents'.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub approval: bool,
}

/// Where a card's work is (card #137), as its holder's device last saw it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardWhere {
    /// The holder it describes, with its device.
    pub who: AgentRef,
    pub worktree: String,
    pub branch: String,
    /// The last commit's sha.
    pub head: String,
    pub pushed: bool,
    #[specta(type = ts::Time)]
    pub at: GoTime,
}

/// A holder's one line on what's done and what's next (card #137).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardLeftOff {
    pub text: String,
    pub by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
}

/// An activity line as the person reads it (card #490): a revision's
/// "revised blocker b2" (agents keep the id) says "revised it" under its
/// blocker, or quotes the question it asked in the card's own thread.
fn person_what(a: &Activity, under: bool) -> String {
    if !a.revised {
        return a.what.clone();
    }
    let head = format!("revised blocker {}", a.blocker_id);
    let Some(rest) = a.what.strip_prefix(&head) else {
        return a.what.clone();
    };
    let said = if under {
        "revised it".to_string()
    } else if a.blocker_gist.is_empty() {
        "revised a blocker".to_string()
    } else {
        format!("revised “{}”", a.blocker_gist)
    };
    format!("{said}{rest}")
}

/// A card's spot on the Board's canvas, in canvas pixels (card #93).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
pub struct CanvasPos {
    pub x: i64,
    pub y: i64,
}

fn pos_of(placed: &HashMap<String, (i64, i64)>, id: &str) -> Option<CanvasPos> {
    placed.get(id).map(|&(x, y)| CanvasPos { x, y })
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_zero_f(n: &f64) -> bool {
    *n == 0.0
}

/// One row of a parent's subtask list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardSubtask {
    pub id: String,
    pub short: String,
    pub title: String,
    #[specta(type = ts::TaskState)]
    pub state: String,
    pub holder: CardHolder,
    pub progress: CardProgress,
    pub blocked: bool,
}

/// One answer to pick.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardOption {
    pub answer: String,
    pub why: String,
    pub by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub recommended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Link>)]
    pub link: Option<Link>,
}

/// How a blocker was resolved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardResolution {
    pub answer: String,
    /// Index into options, or -1 for own words or a close.
    pub option: i64,
    pub by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub closed: bool,
    pub note: String,
}

/// Blocker states, CardBlocker.state.
pub const BLOCKER_OPEN: &str = "open";
pub const BLOCKER_ANSWERED: &str = "answered";
pub const BLOCKER_RESOLVED: &str = "resolved";
pub const BLOCKER_CLOSED: &str = "closed";

/// One blocker item, open or resolved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardBlocker {
    pub id: String,
    pub question: String,
    pub context: String,
    pub raised_by: AgentRef,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub options: Vec<CardOption>,
    /// null while open.
    pub resolved: Option<CardResolution>,
    pub state: String,
    #[specta(type = Vec<ts::Link>)]
    pub links: Vec<Link>,
    #[specta(type = Vec<ts::FileRef>)]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub sent_to: Option<AgentRef>,
    pub missing: Vec<String>,
    /// The person answered in their own words since the options were
    /// offered (card #90): the window folds them under "Earlier options"
    /// and shows a reply box until a revision offers new ones.
    pub options_aside: bool,
    /// A done check (card #112): the agent says it's done. Its one option,
    /// Done, closes the card; own words say why not and go back to them.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub done_check: bool,
    /// An ownership offer or request (card #432): Accept moves the card
    /// to the other member, Decline leaves it where it is.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub transfer: bool,
    /// On a shared board, the other member this question is for (card
    /// #264): it waits on them, not on this person, who can still answer.
    /// None when it's this person's.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub decider: Option<AgentRef>,
    /// What a wait on other work waits on (card #481): a card, a lease, an
    /// agent or something outside. It ends by itself or by whoever does
    /// that work, so it is never the person's to answer and never counts
    /// as a decision. None for a question for a person (one filed on
    /// other work with options is still the person's, card #347).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub waits_on: Option<CardWaitOn>,
    /// The agent this open item waits on, or that raised it and would hear
    /// the answer, when that agent is gone and nobody holds the card (card
    /// #485): drawn amber, "Waiting on kestrel (gone)", with "Hand to an
    /// agent".
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub gone_agent: Option<AgentRef>,
    pub thread: Vec<CardThreadItem>,
    /// A done check's previous round (card #476): the check the person
    /// sent back before this one, so this one shows their reason, the
    /// agent's answer and the proof it replaces.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub previous: Option<CardRound>,
}

/// The done check the person sent back before this one (card #476).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardRound {
    /// The earlier check's id.
    pub id: String,
    /// This check's round: 2 for the first one after a Not done.
    pub round: i64,
    /// The proof the person sent back.
    pub proof: String,
    /// Their reason when no comment on the thread carries it (a card from
    /// before #476, whose late Not done only resolved the check).
    pub answer: String,
    /// The earlier check's thread: their reason and the agent's answers.
    pub thread: Vec<CardThreadItem>,
    /// The new proof is the one they sent back, word for word.
    pub same_proof: bool,
}

/// What a wait item waits on (card #481), as the card page links it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardWaitOn {
    #[specta(type = ts::BlockerKind)]
    pub kind: String,
    /// How the window names it: "#392" for a card ("a card" when this
    /// device can't see it), the agent's name, the resource, or the
    /// outside thing as raised.
    pub label: String,
    /// The card it waits on, by id, for a task wait.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub task_id: String,
    /// That card's title, when this device can see it.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub title: String,
    /// A wait on an agent that is gone (card #485).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub gone: bool,
}

/// One reviewer's review of a private card (#88).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardReview {
    pub reviewer: AgentRef,
    pub asked_by: AgentRef,
    /// requested, approved or changes.
    pub state: String,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub pr: String,
    /// The verdict comment's text, once there is one.
    pub text: String,
}

/// A pull request linked from the card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardPR {
    pub repo: String,
    pub number: i64,
    pub url: String,
    pub label: String,
    pub opened_by: Vec<AgentRef>,
    pub reviewed_by: Vec<AgentRef>,
    pub touched_by: Vec<AgentRef>,
    /// How GitHub last said it stands (card #208): OPEN, MERGED or CLOSED;
    /// empty when nobody has asked yet or GitHub couldn't be asked.
    pub state: String,
    /// GitHub's review decision (APPROVED, CHANGES_REQUESTED, ...).
    pub review: String,
    /// Why GitHub couldn't be asked, said once.
    pub error: String,
    /// When it was last heard; zero when never.
    #[specta(type = ts::Time)]
    pub checked_at: GoTime,
}

/// One line of Recent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardActivity {
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub who: AgentRef,
    pub text: String,
}

/// Thread entry kinds.
pub const THREAD_COMMENT: &str = "comment";
pub const THREAD_MESSAGE: &str = "message";
pub const THREAD_EVENT: &str = "event";

/// One entry of a card's thread.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardThreadItem {
    pub id: String,
    pub kind: String,
    #[specta(type = ts::Time)]
    pub at: GoTime,
    pub who: AgentRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub to: Option<AgentRef>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub device: String,
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub blocker_id: String,
    /// The question of that blocker, cut short (card #490): what the
    /// person reads in place of its id.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub blocker_gist: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub missing: Vec<String>,
    /// An event that revised its blocker (card #50): what it asks now and
    /// its options changed. It sits in the blocker's thread.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub revised: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub gone: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub held: bool,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub files: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub dismissed: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub delivery_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional, type = Vec<ts::FileRef>)]
    pub attachments: Vec<FileRef>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub unheard: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub pending: Vec<String>,
    /// The person's comment: the agents (keys) that have read it (card
    /// #299: a thread says when one of them stopped without replying).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub heard_by: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub mentions: Vec<CardMention>,
    /// The person's comment no agent has read yet (card #105): "can" (Modify
    /// takes it back to edit) or "editing" (held back until Save or Cancel).
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub edit: String,
    /// The person edited it before any agent read it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub edited: bool,
    /// An event saying a card was made or changed from this thread (card
    /// #155): the Threads panel keeps it among the answers.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub made: bool,
}

/// What Modify can do with the person's comment (card #105).
pub const EDIT_CAN: &str = "can";
pub const EDIT_EDITING: &str = "editing";

/// A message on a card's thread in full, for the person.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct CardMessage {
    pub text: String,
    /// One line per attachment: name, size, type, hash.
    pub files: Vec<String>,
}

/// The shared board a card lives on (v0.5 §8).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardBoard {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub visible_to: Vec<String>,
    pub can_withdraw: bool,
    /// Card #432: Transfer ownership… is offered (the owner hands it to
    /// another member, anyone else asks for it).
    pub can_transfer: bool,
}

/// Where a private card was shared to (Go `core.SharedCopy`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedCopy {
    pub board_id: String,
    pub board_name: String,
    pub task_id: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub withdrawn: bool,
}

/// One card of [`Window::cards_by_id`]: the card, or why it couldn't be
/// read (gone, or not this device's to read).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardRead {
    /// The id as asked.
    pub id: String,
    pub card: Option<CardDetail>,
    /// "" when card is there.
    pub error: String,
}

/// One card opened beside the grid.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardDetail {
    #[serde(flatten)]
    pub summary: CardSummary,
    pub goal: String,
    pub done_when: String,
    pub note: String,
    #[specta(type = Vec<ts::Step>)]
    pub steps: Vec<Step>,
    pub blockers: Vec<CardBlocker>,
    pub reviews: Vec<CardReview>,
    pub prs: Vec<CardPR>,
    #[specta(type = Vec<ts::Link>)]
    pub links: Vec<Link>,
    pub recent: Vec<CardActivity>,
    #[specta(type = Vec<ts::Comment>)]
    pub comments: Vec<Comment>,
    pub thread: Vec<CardThreadItem>,
    pub thread_earlier: i64,
    pub history_trimmed: bool,
    pub subtask_list: Vec<CardSubtask>,
    /// Waits (card #87): agents waiting on this card, and what this card's
    /// holders wait on.
    pub waits: Vec<CardWait>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub on_board: Option<CardBoard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub shared_to: Option<SharedCopy>,
}

/// One wait shown on a card page (card #87): `who` waits; `on_this` when
/// it waits on this card, else it is one of this card's holders waiting on
/// something else (`text`: "#12 until done", "branch main moves").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CardWait {
    pub id: String,
    pub who: AgentRef,
    pub text: String,
    pub on_this: bool,
}

/// Home's idle line (card #341): idle agents and the open cards nobody
/// has been handed yet, so a work shortage reads differently from a
/// dispatch gap.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IdleReport {
    /// Agents idle on an online device with no working card in hand.
    pub idle: u32,
    /// Unheld cards nobody has been handed yet, the oldest first.
    pub unoffered: Vec<UnofferedCard>,
}

/// An unheld card nobody has been handed, and why.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UnofferedCard {
    pub id: String,
    pub short: String,
    pub title: String,
    pub why: String,
}

/// One page of the Done tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
pub struct DonePage {
    pub cards: Vec<CardSummary>,
    /// The cursor for the next page ("" on the last).
    pub next: String,
}

// ---- people --------------------------------------------------------------------

type OwnerFn = Box<dyn Fn(&str) -> bool>;

/// Resolves keys against the roster once per call (Go `people`).
#[derive(Default)]
pub(crate) struct People {
    by_key: HashMap<String, coord::roster::AgentCard>,
    user: String,
    owner: Option<OwnerFn>,
    self_id: String,
    names: HashMap<String, String>,
    /// Every agent key → the last name it went by, gone ones too (#98).
    known: std::collections::BTreeMap<String, String>,
    paired: bool,
    colors: BTreeMap<String, String>,
    conflicts: HashMap<String, String>,
    unsent: std::collections::HashSet<String>,
    /// A comment's notices, each with whether its recipient read it.
    heard: HashMap<String, Vec<(Delivery, bool)>>,
    /// Blocker answers' notices by card id (card #476): the person's answer
    /// on a blocker (a Not done) is heard when its agent reads this.
    answers_heard: HashMap<String, Vec<(Delivery, bool)>>,
    /// The shared boards' agent cards by board key (agent:<cardId>), so
    /// another member's agents are named and live on a card they hold
    /// rather than "gone" (card #146).
    board: HashMap<String, irori_core::BoardAgent>,
    /// The other people on the shared boards by board key
    /// (person:<personId>), so a mention of one is named (card #177).
    persons: HashMap<String, irori_core::BoardMember>,
    /// Each other person's colour on this screen by person id (card #385):
    /// their pick, nudged off this person's device colours and off each
    /// other.
    others: BTreeMap<String, String>,
    /// Agents' own faces by name key (card #407).
    faces: HashMap<String, String>,
    /// Live agents Irori started for a card (card #448).
    spawned: std::collections::HashSet<String>,
    /// Where the spawn queue stands here (card #450): each waiting card's
    /// place, a usage-limit hold.
    queue: irori_core::spawn_limits::SpawnQueue,
    /// Cards that wait for an agent here (queued or held).
    waiting: std::collections::HashSet<String>,
}

/// The other people's colours on this screen (card #385): every person
/// seen on a shared board, as a member or by their agents, with their own
/// pick, placed off the viewer's device colours and off each other.
fn place_others(
    persons: &HashMap<String, irori_core::BoardMember>,
    board: &HashMap<String, irori_core::BoardAgent>,
    devices: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut picks: BTreeMap<String, String> = BTreeMap::new();
    for a in board.values().filter(|a| !a.person_id.is_empty()) {
        picks
            .entry(a.person_id.clone())
            .or_default()
            .clone_from(&a.person_color);
    }
    for m in persons.values() {
        if let Some(pid) = m.key.strip_prefix(store::kinds::PERSON_ACTOR_PREFIX) {
            picks.insert(pid.to_string(), m.color.clone());
        }
    }
    let people: Vec<(String, String)> = picks.into_iter().collect();
    let taken: Vec<String> = devices.values().cloned().collect();
    coord::devcolor::place_people(&people, &taken)
}

fn unsent_key(agent: &str, r: &str) -> String {
    format!("{agent}\x00{r}")
}

impl Window {
    /// Go `BoardService.people`.
    /// Each other person's colour on this screen by person id (card
    /// #385), without the rest of [`Self::people`]: for the boards list
    /// (card #393), which runs on every boards change.
    pub(crate) fn person_colors(&self) -> BTreeMap<String, String> {
        let c = &self.core;
        let board: HashMap<String, irori_core::BoardAgent> = c
            .board_agents()
            .into_iter()
            .map(|a| (a.key.clone(), a))
            .collect();
        let persons: HashMap<String, irori_core::BoardMember> = c
            .board_people()
            .into_iter()
            .map(|m| (m.key.clone(), m))
            .collect();
        place_others(&persons, &board, &c.device_colors())
    }

    pub(crate) fn people(&self) -> People {
        let c = &self.core;
        let mut p = People {
            user: c.user_sender().key,
            self_id: c.device_id().to_string(),
            ..People::default()
        };
        for d in c.inbox().open() {
            let m = &d.message;
            if m.notice
                && m.reason == coord::NOTICE_ASSIGNED
                && (d.state == st::QUEUED || d.state == st::HELD)
                && m.ref_.starts_with("task:")
            {
                p.unsent.insert(unsent_key(&d.to, &m.ref_));
            }
        }
        // A mention is how the person usually reaches an agent on a card
        // (card #159): its notice counts, read ones too, so the thread says
        // when it was read.
        for (d, read) in c.inbox().comment_notices(
            &[coord::NOTICE_COMMENT, coord::NOTICE_MENTIONED],
            HEARD_NOTICES,
        ) {
            if let Some(cid) = d.message.ref_.strip_prefix("comment:") {
                p.heard.entry(cid.to_string()).or_default().push((d, read));
            }
        }
        for (d, read) in c.inbox().ref_notices(
            "task:",
            &[coord::NOTICE_ANSWERED, coord::NOTICE_MENTIONED],
            HEARD_NOTICES,
        ) {
            if let Some(id) = d.message.ref_.strip_prefix("task:") {
                p.answers_heard
                    .entry(id.to_string())
                    .or_default()
                    .push((d, read));
            }
        }
        // The Chain's rule, never coord's lock: `around` in person_comment runs
        // under it (coord.is_person would lock again).
        let core = self.core.clone();
        p.owner = Some(Box::new(move |k: &str| core.is_owner(k)));
        for l in c.coord().leases() {
            if !l.conflict.is_empty() {
                p.conflicts.insert(l.holder.clone(), l.resource.clone());
            }
        }
        for a in c.mesh_agents() {
            p.by_key.insert(a.key.clone(), a);
        }
        for a in c.board_agents() {
            p.board.insert(a.key.clone(), a);
        }
        for m in c.board_people() {
            p.persons.insert(m.key.clone(), m);
        }
        p.known = c.known_agent_names();
        p.colors = c.device_colors();
        p.others = place_others(&p.persons, &p.board, &p.colors);
        p.faces = c.agent_avatars();
        {
            let co = c.coord();
            p.spawned = p
                .by_key
                .keys()
                .filter(|k| co.is_spawned(k))
                .cloned()
                .collect();
            // Shared boards' cards wait on a usage hold whatever the
            // private board's switch says.
            p.queue = c.spawn_queue();
            p.waiting.extend(p.queue.on_boards.iter().cloned());
            if c.spawn_setting("").on {
                if p.queue.held_until.is_some() {
                    let private: Vec<String> = co
                        .cards_to_spawn()
                        .into_iter()
                        .filter(|t| {
                            t.spawn_request().is_none_or(|s| {
                                s.op != coord::SPAWN_FAILED || coord::usage_limited(s)
                            })
                        })
                        .map(|t| t.id)
                        .collect();
                    p.waiting.extend(private);
                }
                p.waiting.extend(p.queue.places.keys().cloned());
            }
        }
        let mut live = 0;
        p.names = c.device_labels().into_iter().collect();
        for d in c.certified_devices() {
            if !d.revoked {
                live += 1;
            }
        }
        p.paired = live > 1;
        p
    }
}

impl People {
    /// Whether `dev` is this device: empty, unknown here, or this one.
    fn is_here(&self, dev: &str) -> bool {
        dev.is_empty() || self.self_id.is_empty() || dev == self.self_id
    }

    /// This device as the window names it: its own name with " (this one)",
    /// e.g. "Macbook Pro (this one)", without the hostname's ".local"; "This
    /// device" with no name (card #489).
    pub(crate) fn this_device(&self) -> String {
        let n = self
            .names
            .get(&self.self_id)
            .map(|n| n.trim())
            .unwrap_or("");
        let n = n.strip_suffix(".local").unwrap_or(n).trim();
        if n.is_empty() {
            return "This device".into();
        }
        format!("{n} (this one)")
    }

    /// The device a key lives on: "<its own name> (this one)" for this one
    /// (card #489), else its label.
    pub(crate) fn where_(&self, dev: &str) -> String {
        if self.is_here(dev) {
            return self.this_device();
        }
        match self.names.get(dev) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => "Another device".into(),
        }
    }

    /// The device chip for something `dev` wrote.
    fn chip(&self, dev: &str) -> String {
        if !self.paired || dev.is_empty() || dev == self.self_id {
            return String::new();
        }
        self.where_(dev)
    }

    fn is_owner(&self, key: &str) -> bool {
        self.owner.as_ref().is_some_and(|f| f(key))
    }

    /// An Irori-written line (never a comment's words) with each agent key
    /// it names replaced by that agent's name (card #98).
    pub(crate) fn named(&self, line: &str) -> String {
        let mut out = line.to_string();
        let mut keys: Vec<&String> = self.known.keys().chain(self.by_key.keys()).collect();
        // Longer keys first, so no key is cut inside another.
        keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
        keys.dedup();
        for k in keys {
            if !k.contains('/') || !out.contains(k.as_str()) {
                continue;
            }
            let name = self.ref_(k).name;
            if !name.is_empty() && name != *k {
                out = out.replace(k.as_str(), &name);
            }
        }
        out
    }

    /// One key as the window shows it, with its face (card #407).
    pub(crate) fn ref_(&self, key: &str) -> AgentRef {
        let mut r = self.ref_plain(key);
        r.spawned = self.spawned.contains(key);
        // Another person's agent wears the face its board card carries;
        // never one of this person's faces that shares its name.
        let theirs = !self.by_key.contains_key(key) && self.board.contains_key(key);
        if r.status != "person" && !theirs {
            r.avatar = self
                .faces
                .get(&coord::avatar::name_key(&r.name))
                .cloned()
                .unwrap_or_default();
        }
        r
    }

    fn ref_plain(&self, key: &str) -> AgentRef {
        // A message bridged from a shared board comes from
        // board:<boardId>/<cardId> or board:<boardId>/person:<personId>
        // (v0.5 §7.1): the board's own agent:<cardId> / person:<personId>
        // name it (card #478), and the key it came as stays the key.
        if let Some((_, who)) = key.strip_prefix("board:").and_then(|r| r.split_once('/')) {
            let person = who.starts_with(store::kinds::PERSON_ACTOR_PREFIX);
            let as_board = if person {
                who.to_string()
            } else {
                format!("{}{who}", store::kinds::AGENT_ACTOR_PREFIX)
            };
            if !self.board.contains_key(&as_board)
                && !self.persons.contains_key(&as_board)
                && !self.known.contains_key(&as_board)
            {
                // A board's agent or person this device no longer knows:
                // not one of this person's agents, and not gone either.
                return AgentRef {
                    key: key.into(),
                    name: if person {
                        "Another person".into()
                    } else {
                        "unknown agent".into()
                    },
                    status: if person { "person" } else { "unknown" }.into(),
                    live: person,
                    where_: if person {
                        "On the board".into()
                    } else {
                        "Another person's agent".into()
                    },
                    ..AgentRef::default()
                };
            }
            return AgentRef {
                key: key.into(),
                ..self.ref_plain(&as_board)
            };
        }
        if key == "irori" {
            return AgentRef {
                key: key.into(),
                name: "Irori".into(),
                status: "irori".into(),
                live: true,
                where_: self.this_device(),
                here: true,
                ..AgentRef::default()
            };
        }
        if key == self.user || self.is_owner(key) {
            // The person carries the device they wrote from, so their face
            // is a user icon in that device's colour (card #108).
            let dev = if key == self.user {
                self.self_id.as_str()
            } else {
                key.strip_prefix(irori_identity::USER_KEY_PREFIX)
                    .unwrap_or(key)
            };
            return AgentRef {
                key: key.into(),
                name: "You".into(),
                status: "person".into(),
                live: true,
                where_: self.where_(dev),
                here: self.is_here(dev),
                device: self.chip(dev),
                color: if self.paired {
                    self.colors.get(dev).cloned().unwrap_or_default()
                } else {
                    String::new()
                },
                device_id: dev.to_string(),
                ..AgentRef::default()
            };
        }
        if let Some(m) = self.persons.get(key) {
            // Another person on a shared board (card #177).
            return AgentRef {
                key: key.into(),
                name: if m.name.is_empty() {
                    "Another person".into()
                } else {
                    m.name.clone()
                },
                status: "person".into(),
                live: true,
                where_: "On the board".into(),
                color: key
                    .strip_prefix(store::kinds::PERSON_ACTOR_PREFIX)
                    .and_then(|pid| self.others.get(pid))
                    .cloned()
                    .unwrap_or_default(),
                ..AgentRef::default()
            };
        }
        let mut dev = key
            .split_once('/')
            .map(|(d, _)| d)
            .unwrap_or("")
            .to_string();
        // A key is never a name (card #478): one nothing names is an
        // unknown agent.
        let mut r = AgentRef {
            key: key.into(),
            name: "unknown agent".into(),
            status: "gone".into(),
            conflict: self.conflicts.get(key).cloned().unwrap_or_default(),
            ..AgentRef::default()
        };
        // A gone agent keeps the last name it went by (card #98).
        if let Some(n) = self.known.get(key).filter(|n| !n.is_empty()) {
            r.name = n.clone();
        }
        if let Some(a) = self.by_key.get(key) {
            if !a.name.is_empty() {
                r.name = a.name.clone();
            }
            r.status = a.status.clone();
            r.live = a.status != "gone";
            r.offline = a.device_state == "offline";
            r.device_via = a.device_via.clone();
            if a.status == "idle" {
                r.idle_since = a.last_activity;
            }
            if !a.device_id.is_empty() {
                dev = a.device_id.clone();
            }
        }
        if !self.by_key.contains_key(key)
            && let Some(a) = self.board.get(key)
        {
            // Another member's agent on a shared board (card #146): the
            // board's card names it and says whether it is busy, idle or
            // gone; where is whose agent it is.
            if !a.name.is_empty() {
                r.name = a.name.clone();
            } else if !a.person.is_empty() {
                r.name = format!("an agent of {}'s", a.person);
            }
            r.status = if a.status.is_empty() {
                "idle".into()
            } else {
                a.status.clone()
            };
            r.live = r.status != "gone";
            r.where_ = if a.person.is_empty() {
                "Another person's agent".into()
            } else {
                format!("{}'s agent", a.person)
            };
            r.color = self.others.get(&a.person_id).cloned().unwrap_or_default();
            r.avatar = a.avatar.clone();
            return r;
        }
        r.where_ = self.where_(&dev);
        r.here = self.is_here(&dev);
        r.device = self.chip(&dev);
        if self.paired {
            r.color = self.colors.get(&dev).cloned().unwrap_or_default();
        }
        r.device_id = dev;
        r
    }

    pub(crate) fn refs(&self, keys: &[String]) -> Vec<AgentRef> {
        keys.iter().map(|k| self.ref_(k)).collect()
    }
}

fn ms(t: &GoTime) -> i64 {
    t.unix_milli_value()
}

// ---- summaries -----------------------------------------------------------------

/// A blocker on one of these waits on something that ends by itself, not on
/// the person (card #158): it never counts as a question for them.
fn waits_by_itself(kind: &str) -> bool {
    matches!(
        kind,
        coord::BLOCKED_ON_TASK
            | coord::BLOCKED_ON_LEASE
            | coord::BLOCKED_ON_AGENT
            | coord::BLOCKED_ON_EXTERNAL
            | coord::BLOCKED_ON_VERSION
    )
}

/// What a wait item waits on, named as the summary names its first wait
/// (card #481): "#392" and its title for a card, the agent's name, else
/// the resource or outside thing as raised.
fn wait_on_of(co: &coord::Coord, p: &People, on: &coord::BlockedBy) -> CardWaitOn {
    let mut w = CardWaitOn {
        kind: on.kind.clone(),
        label: on.reference.clone(),
        ..CardWaitOn::default()
    };
    match on.kind.as_str() {
        coord::BLOCKED_ON_TASK => {
            w.task_id = on.reference.clone();
            match co.get_task(&on.reference) {
                Some(k) => {
                    w.label = k.short_ref();
                    w.title = k.title.clone();
                }
                None => w.label = "a card".into(),
            }
        }
        coord::BLOCKED_ON_AGENT => {
            let r = p.ref_(&on.reference);
            w.gone = r.status == "gone";
            w.label = r.name;
        }
        coord::BLOCKED_ON_VERSION => w.label = version_wait_names(p, &on.reference).1,
        _ => {}
    }
    w
}

/// A version wait in words: "Irori 0.11.0" for the pill, and with its
/// devices for the wait row and the tooltip, "Irori 0.11.0 on
/// Stephens-MacBook-Pro-2.local and DESKTOP-STEPHEN" (card #485). A device
/// this one has no name for is "another device", never its id (card #553).
fn version_wait_names(p: &People, reference: &str) -> (String, String) {
    let (min, devs) = coord::parse_version_wait(reference);
    let mut names: Vec<String> = devs
        .iter()
        .filter_map(|d| p.names.get(d).filter(|n| !n.trim().is_empty()).cloned())
        .collect();
    match devs.len() - names.len() {
        0 => {}
        1 => names.push("another device".into()),
        n => names.push(format!("{n} other devices")),
    }
    let short = format!("Irori {min}");
    let full = match names.as_slice() {
        [] => short.clone(),
        [one] => format!("{short} on {one}"),
        [rest @ .., last] => format!("{short} on {} and {last}", rest.join(", ")),
    };
    (short, full)
}

/// A person blocker for another member of a shared board (card #264). The
/// board's keys come here as this device knows them, so this person's own
/// questions are "" or a user: key and only someone else's stay person:.
fn for_another(kind: &str, reference: &str) -> bool {
    kind == coord::BLOCKED_ON_PERSON
        && reference.starts_with(irori_identity::kinds::PERSON_ACTOR_PREFIX)
}

/// Names the card a summary's first wait is on ("#12"); a card this
/// device can't see stays "a card".
pub(crate) fn name_wait(s: &mut CardSummary, refr: &dyn Fn(&str) -> String) {
    if s.wait_task.is_empty() || !s.wait_on.is_empty() {
        return;
    }
    let r = refr(&s.wait_task);
    s.wait_on = if r.is_empty() { "a card".into() } else { r };
}

/// A failed spawn's reason for the card line (card #480). A missing
/// checkout is said in today's words from the card and the device's name,
/// whatever the stored row says: rows from before #480 read "no checkout of
/// remote:github.com/StephenSHorton/irori on 05f1844a…".
fn cant_start_text(t: &Task, sp: &coord::Spawn, p: &People) -> String {
    if !irori_core::spawn_stop::no_folder_reason(&sp.reason) {
        return sp.reason.clone();
    }
    // Mid-sentence, so where_'s "This PC" and "Another device" go lowercase.
    let dev = match p.names.get(&sp.device) {
        Some(n) if !n.trim().is_empty() => n.clone(),
        _ => match p.where_(&sp.device).as_str() {
            "This PC" => "this PC".into(),
            "Another device" => "another device".into(),
            w => w.to_string(),
        },
    };
    if t.repo_key.is_empty() {
        format!("this card names no repo, and {dev} has no folder for it")
    } else {
        format!("no copy of {} on {dev}", coord::repo_name(&t.repo_key))
    }
}

/// The spawn queue's words for card t (card #450): a usage-limit hold
/// first, then its place.
fn queue_line(s: &mut CardSummary, t: &Task, p: &People) {
    let w = queue_words(&t.id, p);
    s.at_cap = w.at_cap;
    s.queue_place = w.queue_place;
    s.usage_limit = w.usage_limit;
}

/// What a card or thread waiting in the spawn queue says (card #450).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct QueueWords {
    pub at_cap: String,
    pub queue_place: i64,
    pub usage_limit: Option<GoTime>,
}

/// Card or thread `id`'s place in the spawn queue: a usage-limit hold
/// first, then its place for this device.
pub(crate) fn queue_words(id: &str, p: &People) -> QueueWords {
    let mut w = QueueWords::default();
    if !p.waiting.contains(id) {
        return w;
    }
    let q = &p.queue;
    if let Some(h) = q.held_until {
        w.usage_limit = Some(h);
    } else if let Some(&n) = q.places.get(id) {
        w.at_cap = p.where_(&p.self_id);
        w.queue_place = n as i64;
    }
    w
}

pub(crate) fn card_summary(t: &Task, p: &People) -> CardSummary {
    let (d, n) = t.progress();
    let mut s = CardSummary {
        id: t.id.clone(),
        short: t.short.clone(),
        short_was: t.short_was.clone(),
        title: t.title.clone(),
        state: coord::effective_state(t).to_string(),
        repo_key: t.repo_key.clone(),
        side: t.side.clone(),
        side_wait: t.side_wait.clone(),
        scope: SCOPE_MINE.into(),
        scope_label: SCOPE_MINE_LABEL.into(),
        progress: CardProgress { done: d, total: n },
        updated_at: t.updated_at,
        subtasks: t.subtasks.len() as i64,
        pinned: t.pinned,
        pin_rank: t.pin_rank,
        paused: t.paused,
        main_red: coord::main_red_test(t).unwrap_or_default().to_string(),
        cant_start: t
            .spawn_request()
            .filter(|sp| sp.op == coord::SPAWN_FAILED && !coord::usage_limited(sp))
            .map(|sp| cant_start_text(t, sp, p))
            .unwrap_or_default(),
        starting: t
            .spawn_request()
            .filter(|sp| sp.pending())
            .map(|sp| p.where_(&sp.device))
            .unwrap_or_default(),
        ..CardSummary::default()
    };
    queue_line(&mut s, t, p);
    if let Some(r) = &t.rollup {
        s.progress = CardProgress {
            done: r.done,
            total: r.total,
        };
    }
    if !coord::is_terminal(&t.state) {
        s.where_ = t
            .where_
            .as_ref()
            .filter(|w| !w.worktree.is_empty())
            .map(|w| CardWhere {
                who: p.ref_(&w.who),
                worktree: w.worktree.clone(),
                branch: w.branch.clone(),
                head: w.head.clone(),
                pushed: w.pushed,
                at: w.at,
            });
        s.left_off = t.left_off.as_ref().map(|l| CardLeftOff {
            text: l.text.clone(),
            by: p.ref_(&l.by),
            at: l.at,
        });
    }
    let open = t.open_blockers();
    if !open.is_empty() {
        for bl in &open {
            if bl.status == coord::BLOCKER_ANSWERED {
                s.waiting += 1;
                continue;
            }
            // A question with options is the person's to decide, whatever
            // it is filed on (card #347), unless it's another member's.
            let theirs = bl
                .on
                .as_ref()
                .is_some_and(|on| for_another(&on.kind, &on.reference));
            if let Some(on) = bl
                .on
                .as_ref()
                .filter(|on| waits_by_itself(&on.kind) && bl.options.is_empty())
            {
                s.waiting_on += 1;
                if s.waiting_on == 1 {
                    s.wait_question = bl.question.clone();
                    s.wait_kind = on.kind.clone();
                    match on.kind.as_str() {
                        coord::BLOCKED_ON_TASK => s.wait_task = on.reference.clone(),
                        coord::BLOCKED_ON_AGENT => {
                            let r = p.ref_(&on.reference);
                            s.wait_gone = r.status == "gone";
                            s.wait_on = r.name;
                        }
                        coord::BLOCKED_ON_VERSION => {
                            (s.wait_on, s.wait_full) = version_wait_names(p, &on.reference);
                        }
                        _ => s.wait_on = on.reference.clone(),
                    }
                }
                continue;
            }
            if let Some(on) = bl.on.as_ref().filter(|_| theirs) {
                // Another member's question on a shared board (card #264):
                // it waits on them, not on this person.
                s.waiting_on += 1;
                if s.waiting_on == 1 {
                    s.wait_question = bl.question.clone();
                    s.wait_on = p.ref_(&on.reference).name;
                }
                continue;
            }
            s.open_blockers += 1;
            if bl.done_check {
                s.ready = true;
            }
            if s.open_blockers == 1 {
                s.blocker_question = bl.question.clone();
            }
        }
    } else if t.state == coord::BLOCKED {
        s.open_blockers = 1;
        s.blocker_question = t.reason.clone();
    } else if t.state == coord::INPUT_REQUIRED {
        s.open_blockers = 1;
        let note = crate::trim(&t.note);
        s.blocker_question = if note.is_empty() {
            NOTE_QUESTION.into()
        } else {
            note.to_string()
        };
    }
    for h in &t.holders {
        let mut r = p.ref_(h);
        r.assigned = t.assigned.contains(h);
        if r.assigned {
            r.handoff = t.handoffs.get(h).cloned().unwrap_or_default();
        }
        r.waiting = r.assigned && p.unsent.contains(&unsent_key(h, &format!("task:{}", t.id)));
        s.working.push(r);
    }
    for k in &t.co_holders {
        let mut r = p.ref_(k);
        r.steps = t
            .steps
            .iter()
            .filter(|st| !st.done && st.owner == *k)
            .map(|st| st.id.clone())
            .collect();
        s.working.push(r);
    }
    // A watcher that is gone is left out (card #485): #446 listed six,
    // every one a raw key.
    for w in &t.watchers {
        if !t.holders.contains(w) && !t.co_holders.contains(w) {
            let r = p.ref_(w);
            if r.status != "gone" {
                s.watching.push(r);
            }
        }
    }
    s.holder = holder_line(&s.working);
    for r in &s.working {
        if !r.device_id.is_empty() && !s.devices.contains(&r.device_id) {
            s.devices.push(r.device_id.clone());
        }
    }
    if s.devices.len() == 1 && p.paired {
        s.accent = p.colors.get(&s.devices[0]).cloned().unwrap_or_default();
    }
    s.ask = card_ask(t, p, None);
    s.search_text = search_text(t, p);
    if let Some(ret) = &t.returned
        && t.holders.is_empty()
    {
        s.returned = Some(p.ref_(&ret.was));
    }
    s
}

/// search_text is `t`'s words for the search box (card #171): objective,
/// done-when, each step's text and its creator's name, one per line.
fn search_text(t: &Task, p: &People) -> String {
    let mut lines: Vec<String> = Vec::new();
    for w in [&t.objective, &t.done_criteria] {
        let w = w.trim();
        if !w.is_empty() {
            lines.push(w.to_string());
        }
    }
    for st in &t.steps {
        let w = st.text.trim();
        if !w.is_empty() {
            lines.push(w.to_string());
        }
    }
    if !t.created_by.is_empty() {
        let who = p.ref_(&t.created_by);
        if !who.name.is_empty() {
            lines.push(who.name);
        }
    }
    lines.join("\n")
}

/// `t`'s CardAsk, or None when `t` isn't an ask. `refr` names another
/// card by id.
pub(crate) fn card_ask(
    t: &Task,
    p: &People,
    refr: Option<&dyn Fn(&str) -> String>,
) -> Option<CardAsk> {
    let a = t.ask.as_ref()?;
    let mut out = CardAsk {
        told: a.told,
        owner: p.where_(&a.owner),
        wake: a.wake,
        mentions: card_mentions(&a.mentions, p),
        ..CardAsk::default()
    };
    if let Some(o) = &a.outcome {
        out.state = ASK_CLOSED.into();
        out.outcome = Some(o.clone());
        if !o.task.is_empty()
            && let Some(f) = refr
        {
            out.outcome_ref = f(&o.task);
        }
    } else if !a.taker.is_empty() {
        out.state = if a.handed { ASK_HANDED } else { ASK_TAKEN }.into();
        out.taker = Some(p.ref_(&a.taker));
    } else if !a.waiting.is_empty() {
        out.state = ASK_WAITING.into();
    } else if a.unclaimed {
        out.state = ASK_UNCLAIMED.into();
    } else {
        out.state = ASK_OPEN.into();
    }
    Some(out)
}

/// The agents that wrote to each other on a card within the talk window,
/// newest first, when there are at least two.
fn talking(cs: &[Comment], p: &People, now: i64) -> Vec<AgentRef> {
    let mut keys: Vec<String> = Vec::new();
    let add = |k: &str, keys: &mut Vec<String>| {
        if k.is_empty()
            || k == "irori"
            || k == p.user
            || p.is_owner(k)
            || keys.iter().any(|x| x == k)
        {
            return;
        }
        keys.push(k.to_string());
    };
    for c in cs.iter().rev() {
        if now - ms(&c.at) > TALK_WINDOW_MS {
            break;
        }
        add(&c.by, &mut keys);
        add(&c.to, &mut keys);
    }
    if keys.len() < 2 {
        return Vec::new();
    }
    keys.truncate(MAX_TALKING);
    p.refs(&keys)
}

fn stale(s: &CardSummary, last: &GoTime, now: i64) -> bool {
    if s.state != coord::WORKING
        || s.holder.state == Holding::Active
        || s.holder.state == Holding::Person
    {
        return false;
    }
    !last.is_zero() && now - ms(last) >= STALE_AFTER_MS
}

/// The holder that took the card up (the first), else the first assigned.
/// A step owner (card #86) is never the holder; it counts among the others.
pub(crate) fn holder_line(working: &[AgentRef]) -> CardHolder {
    let holds = |r: &AgentRef| r.steps.is_empty();
    let Some(first) = working.iter().position(holds) else {
        return CardHolder {
            state: Holding::Nobody,
            ..CardHolder::default()
        };
    };
    let i = working
        .iter()
        .position(|r| holds(r) && !r.assigned)
        .unwrap_or(first);
    let w = working[i].clone();
    let state = if w.status == "person" {
        Holding::Person
    } else if w.waiting {
        Holding::Waiting
    } else if w.assigned {
        Holding::Assigned
    } else if w.status == "gone" {
        Holding::Gone
    } else if w.status == coord::roster::NOT_ANSWERING {
        Holding::Silent
    } else if w.offline {
        Holding::Offline
    } else if w.status == "idle" {
        Holding::Idle
    } else {
        Holding::Active
    };
    CardHolder {
        who: w,
        state,
        others: working.len() as i64 - 1,
    }
}

/// [`Window::cards_in`]'s scopes besides a board id: every card, or the
/// private ones.
pub const SCOPE_ALL: &str = "";
pub const SCOPE_OWN: &str = "own";

fn open_ask_summary(s: &CardSummary) -> bool {
    s.ask.as_ref().is_some_and(|a| a.state != ASK_CLOSED)
}

/// Whether board card `t` is the person's (cards #199, #237): the person or
/// one of their own agents created it, holds or co-holds it, was assigned
/// it, watches it, last changed it, raised one of its blockers, or
/// `commenters` (the card's comment authors) include them. Keys are board
/// keys: an `agent:<card>` or `person:<card>` key that `private` maps to a
/// local key is this person's (§2.4); another person's stays as it was.
/// Home lists every such card beside the private ones.
pub(crate) fn ours<'c>(
    t: &Task,
    commenters: impl IntoIterator<Item = &'c str>,
    private: impl Fn(&str) -> String,
) -> bool {
    let mine = |k: &str| {
        (k.starts_with(store::kinds::AGENT_ACTOR_PREFIX)
            || k.starts_with(store::kinds::PERSON_ACTOR_PREFIX))
            && private(k) != k
    };
    t.holders
        .iter()
        .chain(&t.co_holders)
        .chain(&t.assigned)
        .chain(&t.watchers)
        .map(String::as_str)
        .chain([t.created_by.as_str(), t.updated_by.as_str()])
        .chain(t.blockers.iter().map(|b| b.raised_by.as_str()))
        .any(&mine)
        || commenters.into_iter().any(&mine)
}

/// Go `time.Time.After` on the cards' order: newest first, then id.
fn newer(a: &GoTime, a_id: &str, b: &GoTime, b_id: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match b.unix_nano().cmp(&a.unix_nano()) {
        Ordering::Equal => b_id.cmp(a_id),
        o => o,
    }
}

impl Window {
    fn now_ms(&self) -> i64 {
        ms(&GoTime::from_system(std::time::SystemTime::now()))
    }

    /// Home's idle line (card #341).
    pub fn idle_report(&self) -> IdleReport {
        let r = self.core.idle_report();
        IdleReport {
            idle: u32::try_from(r.idle).unwrap_or(u32::MAX),
            unoffered: r
                .unoffered
                .into_iter()
                .map(|u| UnofferedCard {
                    id: u.task_id,
                    short: u.short,
                    title: u.title,
                    why: u.why,
                })
                .collect(),
        }
    }

    /// BoardService.Cards: every live card for the grid.
    pub fn cards(&self) -> Vec<CardSummary> {
        self.cards_in(SCOPE_ALL)
    }

    /// The live cards of one scope (card #161): [`SCOPE_ALL`] for every
    /// card, [`SCOPE_OWN`] for the private ones, or a board id for that
    /// board's. Pinned first, then families together, as the grid shows
    /// them.
    pub fn cards_in(&self, scope: &str) -> Vec<CardSummary> {
        let co = self.core.coord();
        let p = self.people();
        let now = self.now_ms();
        let own = scope == SCOPE_ALL || scope == SCOPE_OWN;
        let mut tasks = if own {
            co.list_tasks(&coord::TaskFilter {
                open: true,
                ..Default::default()
            })
        } else {
            Vec::new()
        };
        let mut out = Vec::with_capacity(tasks.len());
        let placed = co.positions();
        // Threads (card #116) live in their own panel, not the grid.
        for t in tasks.iter().filter(|t| !crate::threads::is_thread(t)) {
            let mut s = card_summary(t, &p);
            s.pos = pos_of(&placed, &t.id);
            let mut last = t.updated_at;
            if let Some(acts) = co.activity(&t.id)
                && let Some(a) = acts.first()
                && a.at.unix_nano() > last.unix_nano()
            {
                last = a.at;
            }
            s.stale = s.subtasks == 0 && stale(&s, &last, now);
            s.talking = talking(&co.comments(&t.id).unwrap_or_default(), &p, now);
            family_of(co, &p, t, &mut s);
            out.push(s);
        }
        // Every board's: "All" includes them (v0.5 §8).
        if scope != SCOPE_OWN {
            let only = (scope != SCOPE_ALL).then_some(scope);
            let (bs, bt) = self.board_cards(&p, now, only);
            out.extend(bs);
            tasks.extend(bt);
        }
        out.sort_by(|a, b| {
            let (aa, ab) = (open_ask_summary(a), open_ask_summary(b));
            if aa != ab {
                return ab.cmp(&aa);
            }
            let (ba, bb) = (a.open_blockers > 0, b.open_blockers > 0);
            if ba != bb {
                return bb.cmp(&ba);
            }
            newer(&a.updated_at, &a.id, &b.updated_at, &b.id)
        });
        let mut pins: Vec<CardSummary> = out.iter().filter(|s| s.pinned).cloned().collect();
        pins.sort_by(|a, b| {
            a.pin_rank
                .partial_cmp(&b.pin_rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let by_id: HashMap<String, CardSummary> =
            out.iter().map(|s| (s.id.clone(), s.clone())).collect();
        let kids: HashMap<String, Vec<String>> = tasks
            .iter()
            .map(|t| (t.id.clone(), t.subtasks.clone()))
            .collect();
        let mut pinned = Vec::new();
        let mut used = std::collections::HashSet::new();
        for s in pins {
            if used.contains(&s.id) {
                continue;
            }
            used.insert(s.id.clone());
            let id = s.id.clone();
            pinned.push(s);
            for k in kids.get(&id).into_iter().flatten() {
                if let Some(ks) = by_id.get(k)
                    && !ks.pinned
                {
                    pinned.push(ks.clone());
                    used.insert(k.clone());
                }
            }
        }
        let rest: Vec<CardSummary> = out.into_iter().filter(|s| !used.contains(&s.id)).collect();
        pinned.extend(families(rest, &tasks));
        pinned
    }

    /// Every closed card the window lists: private ones, then every ready
    /// board's (#115), the way the grid's All holds every live card. A
    /// board card's `from` is its index in the boards returned. Cards the
    /// person cleared (#138) are left out.
    #[allow(clippy::type_complexity)]
    fn closed(
        &self,
        scope: &str,
    ) -> (
        Vec<(Task, Option<usize>)>,
        Vec<(irori_core::BoardSummary, std::sync::Arc<coord::Coord>)>,
    ) {
        let co = self.core.coord();
        let cleared = co.archived_ids();
        let terminal = |t: &Task| coord::is_terminal(coord::effective_state(t));
        let own = scope == SCOPE_ALL || scope == SCOPE_OWN;
        let mut done: Vec<(Task, Option<usize>)> = if own {
            co.list_tasks(&coord::TaskFilter::default())
                .into_iter()
                .filter(|t| {
                    terminal(t) && !crate::threads::is_thread(t) && !cleared.contains(&t.id)
                })
                .map(|t| (t, None))
                .collect()
        } else {
            Vec::new()
        };
        let boards: Vec<_> = self
            .core
            .board_list()
            .into_iter()
            .filter(|s| s.ready)
            .filter(|s| scope == SCOPE_ALL || s.board_id == scope)
            .filter_map(|s| self.core.board_coord(&s.board_id).ok().map(|c| (s, c)))
            .collect();
        for (i, (_, bc)) in boards.iter().enumerate() {
            for t in bc.list_tasks(&coord::TaskFilter::default()) {
                if !t.withdrawn
                    && terminal(&t)
                    && !crate::threads::is_thread(&t)
                    && !cleared.contains(&t.id)
                {
                    done.push((t, Some(i)));
                }
            }
        }
        (done, boards)
    }

    /// The closed cards a clear would take (card #138): the ones given, or
    /// with none given, every one closed (last changed) more than
    /// `older_than_secs` ago (0: all of them), within `scope` (see
    /// [`Self::cards_in`]): a board's Closed clears that board's alone.
    fn to_clear(&self, older_than_secs: i64, ids: &[String], scope: &str) -> Vec<String> {
        let (done, _) = self.closed(scope);
        if !ids.is_empty() {
            let want: std::collections::HashSet<&str> =
                ids.iter().map(|s| crate::trim(s)).collect();
            return done
                .into_iter()
                .filter(|(t, _)| want.contains(t.id.as_str()))
                .map(|(t, _)| t.id)
                .collect();
        }
        let cut = self.now_ms() - older_than_secs.max(0) * 1000;
        done.into_iter()
            .filter(|(t, _)| older_than_secs <= 0 || t.updated_at.unix_milli_value() < cut)
            .map(|(t, _)| t.id)
            .collect()
    }

    /// BoardService.ClearDoneCount: how many closed cards a clear by age
    /// would take (card #138), for its confirm.
    pub fn clear_done_count(&self, older_than_secs: i64, scope: &str) -> i64 {
        self.to_clear(older_than_secs, &[], scope).len() as i64
    }

    /// BoardService.ClearDone (card #138): takes closed cards out of the
    /// window's lists (the ids given, else by age as clear_done_count).
    /// They stay in the log, so agents still find them with search and
    /// show. Returns how many it cleared.
    pub fn clear_done(
        &self,
        older_than_secs: i64,
        ids: &[String],
        scope: &str,
    ) -> Result<i64, String> {
        let ids = self.to_clear(older_than_secs, ids, scope);
        if ids.is_empty() {
            return Ok(0);
        }
        self.core
            .coord()
            .archive(&self.user_key(), &ids, true)
            .map(|n| n as i64)
            .map_err(|e| e.to_string())
    }

    /// BoardService.DoneCards: one page of the closed cards.
    pub fn done_cards(&self, limit: i64, before: &str) -> Result<DonePage, String> {
        self.done_cards_in(limit, before, SCOPE_ALL)
    }

    /// One page of one scope's closed cards (card #161; see
    /// [`Self::cards_in`]): paged on its own, so a board's Closed isn't
    /// crowded out by newer cards elsewhere.
    pub fn done_cards_in(&self, limit: i64, before: &str, scope: &str) -> Result<DonePage, String> {
        let co = self.core.coord();
        let limit = if limit <= 0 {
            coord::DEFAULT_DONE_CARD
        } else {
            limit
        }
        .min(coord::MAX_DONE_CARD) as usize;
        let before = crate::trim(before);
        let mut cursor: Option<(GoTime, String)> = None;
        if !before.is_empty() {
            let bad = || {
                format!(
                    "{}: cursor {}",
                    coord::ErrorKind::Invalid.text(),
                    gocompat::quote::quote(before)
                )
            };
            let (ts, id) = before.split_once(' ').ok_or_else(bad)?;
            let at = GoTime::parse_rfc3339(ts).ok_or_else(bad)?;
            if id.is_empty() {
                return Err(bad());
            }
            cursor = Some((at, id.to_string()));
        }
        let (mut done, boards) = self.closed(scope);
        done.sort_by(|(a, _), (b, _)| newer(&a.updated_at, &a.id, &b.updated_at, &b.id));
        if let Some((at, id)) = &cursor {
            let i = done.partition_point(|(t, _)| {
                newer(&t.updated_at, &t.id, at, id) == std::cmp::Ordering::Less
            });
            let i = if i < done.len() && done[i].0.id == *id && done[i].0.updated_at.equal(at) {
                i + 1
            } else {
                i
            };
            done.drain(..i);
        }
        let mut out = DonePage::default();
        if done.len() > limit {
            let last = &done[limit - 1].0;
            out.next = format!(
                "{} {}",
                last.updated_at.utc().format_rfc3339_nano(),
                last.id
            );
            done.truncate(limit);
        }
        let p = self.people();
        for (t, from) in done {
            let s = match from {
                None => {
                    let refr =
                        |id: &str| co.get_task(id).map(|x| x.short_ref()).unwrap_or_default();
                    let mut s = card_summary(&t, &p);
                    s.ask = card_ask(&t, &p, Some(&refr));
                    name_wait(&mut s, &refr);
                    s
                }
                Some(i) => {
                    let (b, bc) = &boards[i];
                    let refr =
                        |id: &str| bc.get_task(id).map(|x| x.short_ref()).unwrap_or_default();
                    let pt = self.priv_task(t.clone());
                    let mut s = card_summary(&pt, &p);
                    s.ask = card_ask(&pt, &p, Some(&refr));
                    name_wait(&mut s, &refr);
                    s.scope = crate::boards::SCOPE_BOARD.into();
                    s.scope_label = b.name.clone();
                    s.board = b.board_id.clone();
                    s.tag = b.tag.clone();
                    self.board_owner(&b.board_id, &t, &mut s);
                    s
                }
            };
            out.cards.push(s);
        }
        Ok(out)
    }

    /// The cards waiting on a decision (the tray's number).
    pub fn decide_count(&self) -> usize {
        let p = People::default();
        let co = self.core.coord();
        // Card #509: a wait on a PR's merge is the person's too, and card
        // #520: a wait on an agent that is gone.
        let private = co
            .list_tasks(&coord::TaskFilter {
                open: true,
                ..Default::default()
            })
            .iter()
            .filter(|t| {
                card_summary(t, &p).open_blockers > 0
                    || merge_pr(co, t).is_some()
                    || !co.gone_waits(&t.id).is_empty()
            })
            .count();
        private
            + self
                .board_cards(&p, self.now_ms(), None)
                .0
                .iter()
                .filter(|s| s.open_blockers > 0 || s.merge_pr.is_some() || s.gone_wait.is_some())
                .count()
    }

    /// The live cards of every board this device holds (or only board
    /// `only`), for the grid, their keys as this device knows them, with
    /// their tasks for grouping families. A withdrawn card or a thread is
    /// left out (§5.3; Go `boardCards`). The person's pins on board cards
    /// live in their own log (card #161).
    fn board_cards(
        &self,
        p: &People,
        now: i64,
        only: Option<&str>,
    ) -> (Vec<CardSummary>, Vec<Task>) {
        let (mut out, mut tasks) = (Vec::new(), Vec::new());
        let pins = self.core.coord().pinned_ranks();
        for s in self.core.board_list() {
            if only.is_some_and(|b| b != s.board_id) {
                continue;
            }
            let Ok(co) = self.core.board_coord(&s.board_id) else {
                continue;
            };
            if !s.ready {
                continue;
            }
            let placed = co.positions();
            for t in co.list_tasks(&coord::TaskFilter {
                open: true,
                ..Default::default()
            }) {
                // A board's threads show in its Threads panel (card #134).
                if t.withdrawn || crate::threads::is_thread(&t) {
                    continue;
                }
                let pt = self.priv_task(t.clone());
                let mut cs = card_summary(&pt, p);
                let private = |k: &str| self.core.private_key(k);
                // Comments are read only when nothing cheaper already said so.
                cs.ours = ours(&t, [], private)
                    || (t.comments > 0
                        && ours(
                            &t,
                            co.comments(&t.id)
                                .unwrap_or_default()
                                .iter()
                                .map(|c| c.by.as_str())
                                .collect::<Vec<_>>(),
                            private,
                        ));
                cs.pos = pos_of(&placed, &pt.id);
                cs.scope = crate::boards::SCOPE_BOARD.into();
                cs.scope_label = s.name.clone();
                cs.board = s.board_id.clone();
                cs.tag = s.tag.clone();
                self.board_owner(&s.board_id, &t, &mut cs);
                let mut last = t.updated_at;
                if let Some(acts) = co.activity(&t.id)
                    && let Some(a) = acts.first()
                    && a.at.unix_nano() > last.unix_nano()
                {
                    last = a.at;
                }
                cs.stale = cs.subtasks == 0 && stale(&cs, &last, now);
                cs.talking = talking(
                    &self.priv_comments(co.comments(&pt.id).unwrap_or_default()),
                    p,
                    now,
                );
                if let Some(r) = pins.get(&pt.id) {
                    cs.pinned = true;
                    cs.pin_rank = *r;
                }
                family_of(&co, p, &t, &mut cs);
                out.push(cs);
                tasks.push(t);
            }
        }
        (out, tasks)
    }

    /// Several cards at once, in the order asked (card #468): the window's
    /// card hooks (a Notifications row's, a Needs you row's, a thread's,
    /// the open card's) share one read per change instead of one each.
    pub fn cards_by_id(&self, ids: &[String]) -> Vec<CardRead> {
        ids.iter()
            .map(|id| match self.card(id) {
                Ok(c) => CardRead {
                    id: id.clone(),
                    card: Some(c),
                    error: String::new(),
                },
                Err(e) => CardRead {
                    id: id.clone(),
                    card: None,
                    error: e,
                },
            })
            .collect()
    }

    /// BoardService.Card: one card opened.
    pub fn card(&self, id: &str) -> Result<CardDetail, String> {
        let id = crate::trim(id);
        let co = self.read(id);
        let Some(mut t) = co.get_task(id) else {
            return Err(not_found_task(id));
        };
        let mut comments = co.comments(&t.id).unwrap_or_default();
        let mut acts = co.activity(&t.id).unwrap_or_default();
        let raw = co.is_board().then(|| t.clone());
        if co.is_board() {
            // A shared card's keys are board keys; the page names them as
            // this device knows them (v0.5 §2.4).
            t = self.priv_task(t);
            comments = self.priv_comments(comments);
            acts = self.priv_activity(acts);
        }
        let p = self.people();
        let mut d = CardDetail {
            summary: card_summary(&t, &p),
            goal: t.objective.clone(),
            done_when: t.done_criteria.clone(),
            note: t.note.clone(),
            steps: t.steps.clone(),
            links: t.links.clone(),
            comments,
            ..CardDetail::default()
        };
        self.board_detail(&t, &mut d);
        if let Some(raw) = &raw
            && !d.summary.board.is_empty()
        {
            let board = d.summary.board.clone();
            self.board_owner(&board, raw, &mut d.summary);
        }
        family_of(&co, &p, &t, &mut d.summary);
        for kid in &t.subtasks {
            let Some(mut k) = co.get_task(kid) else {
                continue;
            };
            if co.is_board() {
                k = self.priv_task(k); // whose questions are whose (#264)
            }
            let ks = card_summary(&k, &p);
            d.subtask_list.push(CardSubtask {
                id: k.id.clone(),
                short: k.short.clone(),
                title: k.title.clone(),
                state: ks.state.clone(),
                holder: ks.holder.clone(),
                progress: ks.progress.clone(),
                blocked: ks.open_blockers > 0,
            });
        }
        let refr = |id: &str| co.get_task(id).map(|x| x.short_ref()).unwrap_or_default();
        if !co.is_board() {
            for w in co.waits("") {
                let on_this = w.task_id == t.id;
                if !on_this && (!t.holders.contains(&w.by) || coord::is_terminal(&t.state)) {
                    continue;
                }
                let text = if on_this {
                    format!("until {}", coord::until_text(&w.until))
                } else {
                    coord::wait_line(&w, &refr(&w.task_id))
                };
                d.waits.push(CardWait {
                    id: w.id.clone(),
                    who: p.ref_(&w.by),
                    text,
                    on_this,
                });
            }
        }
        d.summary.ask = card_ask(&t, &p, Some(&refr));
        let (thread, mut by_blocker, earlier) = build_thread(&d.comments, &acts, &t, &p);
        d.thread = thread;
        d.thread_earlier = earlier as i64;
        self.held_on_thread(&mut d.thread);
        for items in by_blocker.values_mut() {
            self.held_on_thread(items);
        }
        if !co.is_board() {
            let edits = self.core.inbox().comment_edit_states(&p.user);
            edit_on_thread(&mut d.thread, &d.comments, &edits);
            for items in by_blocker.values_mut() {
                edit_on_thread(items, &d.comments, &edits);
            }
        }
        d.history_trimmed = acts.len() == coord::MAX_ACTIVITY;
        let mut resolved = Vec::new();
        if t.state == coord::INPUT_REQUIRED {
            d.blockers.push(CardBlocker {
                id: NOTE_ITEM.into(),
                question: d.summary.blocker_question.clone(),
                raised_by: p.ref_(&t.updated_by),
                at: t.updated_at,
                state: BLOCKER_OPEN.into(),
                ..CardBlocker::default()
            });
        }
        let stranded = co.stranded(&t.id);
        for bl in &t.blockers {
            let mut cb = CardBlocker {
                id: bl.id.clone(),
                question: bl.question.clone(),
                context: bl.context.clone(),
                raised_by: p.ref_(&bl.raised_by),
                at: bl.at,
                state: if bl.status.is_empty() {
                    BLOCKER_OPEN.into()
                } else {
                    bl.status.clone()
                },
                missing: bl.missing.clone(),
                options_aside: bl.options_aside && bl.open(),
                done_check: bl.done_check,
                transfer: bl.transfer,
                decider: bl
                    .on
                    .as_ref()
                    .filter(|on| for_another(&on.kind, &on.reference))
                    .map(|on| p.ref_(&on.reference)),
                waits_on: bl
                    .on
                    .as_ref()
                    // Once someone replied on it, it keeps its thread and
                    // reply box (the review of #481).
                    .filter(|on| {
                        waits_by_itself(&on.kind)
                            && bl.options.is_empty()
                            && (bl.status.is_empty() || bl.status == coord::BLOCKER_OPEN)
                    })
                    .map(|on| wait_on_of(&co, &p, on)),
                gone_agent: stranded
                    .iter()
                    .find(|s| s.blocker_id == bl.id)
                    .map(|s| p.ref_(&s.agent)),
                thread: by_blocker.remove(&bl.id).unwrap_or_default(),
                links: bl.links.clone(),
                attachments: bl.attachments.clone(),
                ..CardBlocker::default()
            };
            for o in &bl.options {
                cb.options.push(CardOption {
                    answer: o.answer.clone(),
                    why: o.why.clone(),
                    by: p.ref_(&o.by),
                    at: o.at,
                    recommended: o.recommended,
                    link: o.link.clone(),
                });
            }
            if let Some(r) = &bl.resolved {
                cb.resolved = Some(CardResolution {
                    answer: r.answer.clone(),
                    option: r.option,
                    by: p.ref_(&r.by),
                    at: r.at,
                    closed: r.closed,
                    note: r.note.clone(),
                });
                cb.state = if r.closed {
                    BLOCKER_CLOSED
                } else {
                    BLOCKER_RESOLVED
                }
                .into();
                cb.missing = Vec::new();
                resolved.push(cb);
                continue;
            }
            if cb.state == BLOCKER_ANSWERED && !bl.sent_to.is_empty() {
                cb.sent_to = Some(p.ref_(&bl.sent_to));
            }
            d.blockers.push(cb);
        }
        resolved.reverse();
        d.blockers.extend(resolved);
        previous_rounds(&mut d.blockers, &t);
        if !co.is_board() {
            for r in co.reviews_of(&t.id) {
                d.reviews.push(CardReview {
                    reviewer: p.ref_(&r.reviewer),
                    asked_by: p.ref_(&r.from),
                    state: r.state,
                    at: r.at,
                    pr: r.pr,
                    text: r.text,
                });
            }
        }
        let heard: HashMap<String, coord::PrState> = co
            .pr_states(&t.id)
            .into_iter()
            .map(|s| (s.pr.clone(), s))
            .collect();
        for l in &t.links {
            if let Some(mut pr) = self.card_pr(l, &p) {
                if let Some(s) = heard.get(&l.reference) {
                    pr.state = s.state.clone();
                    pr.review = s.review.clone();
                    pr.error = s.error.clone();
                    pr.checked_at = s.at;
                }
                d.prs.push(pr);
            }
        }
        for a in acts.iter().take(RECENT_LINES) {
            let who = p.ref_(&a.by);
            d.recent.push(CardActivity {
                at: a.at,
                text: format!("{} {}", who.name, p.named(&a.what)),
                who,
            });
        }
        Ok(d)
    }

    /// BoardService.Message: a message on a card's thread in full.
    pub fn message(&self, task_id: &str, msg_id: &str) -> Result<CardMessage, String> {
        let (task_id, msg_id) = (crate::trim(task_id), crate::trim(msg_id));
        let nf = coord::ErrorKind::NotFound.text();
        let cm = match self.core.coord().get_comment(msg_id) {
            Some(cm) if !cm.msg_id.is_empty() && cm.task_id == task_id => cm,
            _ => {
                return Err(format!(
                    "{nf}: no message {} on task {}",
                    gocompat::quote::quote(msg_id),
                    gocompat::quote::quote(task_id)
                ));
            }
        };
        let Some(m) = self.core.inbox().message(&cm.msg_id) else {
            return Err(format!(
                "{nf}: message {} isn't on this device yet",
                gocompat::quote::quote(msg_id)
            ));
        };
        Ok(CardMessage {
            text: m.text.clone(),
            files: m
                .attachments
                .iter()
                .map(|f| crate::inbox_blob_ref(f).describe())
                .collect(),
        })
    }

    /// The delivery behind each held message on a thread.
    fn held_on_thread(&self, items: &mut [CardThreadItem]) {
        for it in items.iter_mut() {
            if !it.held || it.kind != THREAD_MESSAGE {
                continue;
            }
            for d in self.core.inbox().of_message(&it.id) {
                if d.state == st::HELD {
                    it.delivery_id = d.id.clone();
                    it.dismissed = false;
                    break;
                }
                if d.state == st::DISMISSED && it.delivery_id.is_empty() {
                    it.delivery_id = d.id.clone();
                    it.dismissed = true;
                }
            }
        }
    }

    /// A GitHub PR link and what capture's ledger knows of it.
    fn card_pr(&self, l: &Link, p: &People) -> Option<CardPR> {
        let (repo, n) = coord::pr_link(l)?;
        let mut pr = CardPR {
            repo: repo.clone(),
            number: n,
            url: l.reference.clone(),
            label: l.label.clone(),
            ..CardPR::default()
        };
        let key = ledger::key(&repo, n);
        let mut seen = std::collections::HashSet::new();
        for e in self.core.capture().prs(&repo) {
            let k = format!("{}\x00{}", e.role.as_str(), e.session);
            if e.key != key || !seen.insert(k) {
                continue;
            }
            let r = p.ref_(&e.session);
            if e.role == ledger::Role::AUTHOR {
                pr.opened_by.push(r);
            } else if e.role == ledger::Role::REVIEWER {
                pr.reviewed_by.push(r);
            } else {
                pr.touched_by.push(r);
            }
        }
        Some(pr)
    }
}

pub(crate) fn not_found_task(id: &str) -> String {
    format!(
        "{}: task {}",
        coord::ErrorKind::NotFound.text(),
        gocompat::quote::quote(id)
    )
}

/// Keeps a parent's subtasks right after it and moves the family to where
/// its most pressing card sorted.
fn families(sorted: Vec<CardSummary>, tasks: &[Task]) -> Vec<CardSummary> {
    let kids: HashMap<&str, &Vec<String>> = tasks
        .iter()
        .filter(|t| !t.subtasks.is_empty())
        .map(|t| (t.id.as_str(), &t.subtasks))
        .collect();
    let at: HashMap<String, usize> = sorted
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), i))
        .collect();
    let head = |s: &CardSummary| -> String {
        if let Some(p) = &s.parent
            && at.contains_key(&p.id)
        {
            return p.id.clone();
        }
        s.id.clone()
    };
    let mut out = Vec::with_capacity(sorted.len());
    let mut placed = std::collections::HashSet::new();
    for s in &sorted {
        let h = head(s);
        if !placed.insert(h.clone()) {
            continue;
        }
        out.push(sorted[at[&h]].clone());
        for k in kids.get(h.as_str()).into_iter().flat_map(|v| v.iter()) {
            if let Some(&i) = at.get(k) {
                out.push(sorted[i].clone());
            }
        }
    }
    out
}

/// A row's place in the log: (HLC, device, seq).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ThreadPos {
    hlc: u64,
    dev: String,
    seq: i64,
}

/// Merges a card's comments and its activity (newest first) into one
/// thread in log order, keeping the newest MAX_THREAD_VIEW.
fn build_thread(
    comments: &[Comment],
    acts: &[Activity],
    t: &Task,
    p: &People,
) -> (
    Vec<CardThreadItem>,
    HashMap<String, Vec<CardThreadItem>>,
    usize,
) {
    let kept: std::collections::HashSet<&str> = t.blockers.iter().map(|b| b.id.as_str()).collect();
    let mut all: Vec<(ThreadPos, CardThreadItem)> = Vec::with_capacity(comments.len() + acts.len());
    let mut mine: Vec<(ThreadPos, CardThreadItem)> = Vec::new();
    for cm in comments {
        let mut it = CardThreadItem {
            id: cm.id.clone(),
            kind: THREAD_COMMENT.into(),
            at: cm.at,
            who: p.ref_(&cm.by),
            device: p.chip(&cm.device),
            text: cm.text.clone(),
            gone: cm.gone,
            held: cm.held,
            files: cm.files,
            attachments: cm.attachments.clone(),
            blocker_id: cm.blocker_id.clone(),
            blocker_gist: cm.blocker_gist.clone(),
            missing: cm.missing.clone(),
            unheard: cm.unheard,
            mentions: card_mentions(&cm.mentions, p),
            ..CardThreadItem::default()
        };
        if !cm.msg_id.is_empty() {
            it.kind = THREAD_MESSAGE.into();
        } else if it.who.status == "person" {
            // An agent that wrote on the card after it has answered: "read
            // it" says nothing more.
            let wrote_after = |k: &str| comments.iter().any(|c| c.by == k && c.hlc > cm.hlc);
            let answer;
            let heard = if cm.blocker_id.is_empty() {
                p.heard.get(&cm.id).map(Vec::as_slice).unwrap_or(&[])
            } else {
                answer = answer_notices(p, t, cm);
                &answer[..]
            };
            it.pending = pending_lines(heard, t, p, wrote_after);
            it.heard_by = heard_by(heard);
        }
        if !cm.to.is_empty() {
            it.to = Some(p.ref_(&cm.to));
        }
        let pos = ThreadPos {
            hlc: cm.hlc,
            dev: cm.device.clone(),
            seq: cm.seq,
        };
        if !cm.blocker_id.is_empty() && kept.contains(cm.blocker_id.as_str()) {
            mine.push((pos, it));
            continue;
        }
        all.push((pos, it));
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for a in acts.iter().rev() {
        if a.kind == store::kinds::TASK_COMMENT && a.blocker_id.is_empty() {
            continue;
        }
        let mut id = format!("{}:{}", a.device, a.seq);
        let n = seen.get(&id).copied().unwrap_or(0);
        if n > 0 {
            seen.insert(id.clone(), n + 1);
            id = format!("{id}.{n}");
        } else {
            seen.insert(id.clone(), 1);
        }
        let who = p.ref_(&a.by);
        let under = a.revised && kept.contains(a.blocker_id.as_str());
        let what = person_what(a, under);
        let ev = (
            ThreadPos {
                hlc: a.hlc,
                dev: a.device.clone(),
                seq: a.seq,
            },
            CardThreadItem {
                id,
                kind: THREAD_EVENT.into(),
                at: a.at,
                device: p.chip(&a.device),
                text: format!("{} {}", who.name, p.named(&what)),
                blocker_id: a.blocker_id.clone(),
                blocker_gist: a.blocker_gist.clone(),
                revised: a.revised,
                made: a.kind == coord::MADE_KIND,
                who,
                ..CardThreadItem::default()
            },
        );
        // A revision goes under its blocker, after the reply it answers.
        if under {
            mine.push(ev);
            continue;
        }
        all.push(ev);
    }
    mine.sort_by(|a, b| a.0.cmp(&b.0));
    let mut by_blocker: HashMap<String, Vec<CardThreadItem>> = HashMap::new();
    for (_, it) in mine {
        by_blocker
            .entry(it.blocker_id.clone())
            .or_default()
            .push(it);
    }
    all.sort_by(|a, b| a.0.cmp(&b.0));
    let mut agent_later = false;
    for (_, it) in all.iter_mut().rev() {
        if it.kind == THREAD_EVENT {
            continue;
        }
        it.unheard = it.unheard && !agent_later;
        agent_later = agent_later || (it.who.status != "person" && it.who.status != "irori");
    }
    let mut earlier = 0;
    if all.len() > MAX_THREAD_VIEW {
        earlier = all.len() - MAX_THREAD_VIEW;
        all.drain(..earlier);
    }
    (
        all.into_iter().map(|(_, it)| it).collect(),
        by_blocker,
        earlier,
    )
}

/// Marks the person's comments no agent has read yet (card #105). Only a
/// private card's: a shared board's members already have theirs.
fn edit_on_thread(
    items: &mut [CardThreadItem],
    comments: &[Comment],
    edits: &HashMap<String, inbox::EditState>,
) {
    for it in items.iter_mut() {
        if it.kind != THREAD_COMMENT || it.who.status != "person" {
            continue;
        }
        let Some(cm) = comments.iter().find(|c| c.id == it.id) else {
            continue;
        };
        it.edited = !cm.edited_at.is_zero();
        // A comment nobody was told of (or whose notices are long gone)
        // has no agent to wait for: it stays as it is.
        it.edit = match edits.get(&cm.id) {
            Some(inbox::EditState::Unread) => EDIT_CAN.into(),
            Some(inbox::EditState::Editing) => EDIT_EDITING.into(),
            _ => String::new(),
        };
    }
}

/// Whether an agent's status says it is in a turn: busy, running a shell
/// command, waiting on a prompt (Claude Code's own words). It hears a
/// comment only when that turn ends.
fn in_turn(status: &str) -> bool {
    !matches!(
        status,
        "" | "idle" | "gone" | "person" | "irori" | coord::roster::NOT_ANSWERING
    )
}

/// How many of the newest comment notices the thread looks at.
const HEARD_NOTICES: usize = 2000;

/// A person's comment's Pending lines from its notices: who still has to
/// hear it, and why it waits (a busy agent hears it when its turn ends,
/// card #159), then who read it. A watcher's line shows only when the
/// comment mentions it. `wrote_after` drops "read it" for an agent that
/// has written on the card since.
fn pending_lines(
    ds: &[(Delivery, bool)],
    t: &Task,
    p: &People,
    wrote_after: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (d, read) in ds {
        let mentioned = d.message.reason == coord::NOTICE_MENTIONED;
        if d.to.is_empty()
            || (!mentioned && t.watchers.contains(&d.to) && !t.holders.contains(&d.to))
            || !seen.insert(d.to.clone())
        {
            continue;
        }
        let who = p.ref_(&d.to);
        let waiting = d.state == st::QUEUED || d.state == st::DELIVERED;
        let line = if *read {
            if wrote_after(&d.to) {
                continue;
            }
            format!("{} read it", who.name)
        } else if waiting && who.status == coord::roster::NOT_ANSWERING {
            format!("{} isn't answering: it hasn't heard this yet", who.name)
        } else if waiting && in_turn(&who.status) {
            format!(
                "{} is busy: it hears this when its current turn ends",
                who.name
            )
        } else {
            match d.state.as_str() {
                st::QUEUED => format!("queued for {}", who.name),
                st::HELD if inbox::editing(d) => format!("held for {} while you edit", who.name),
                st::HELD => format!("held for {}", who.name),
                st::DELIVERED => format!("sent to {}, not read yet", who.name),
                _ => continue,
            }
        };
        out.push(line);
    }
    out
}

/// The agents that read a person's comment, from its notices (newest
/// first; an agent's newest notice decides, as in pending_lines).
/// Gives each done check the round before it (card #476): the done check
/// raised just before it, when the person sent that one back (their own
/// words on its thread, or as its answer).
fn previous_rounds(blockers: &mut [CardBlocker], t: &Task) {
    let mut checks: Vec<&coord::Blocker> = t.blockers.iter().filter(|b| b.done_check).collect();
    checks.sort_by_key(|b| b.at.unix_nano());
    let mut rounds = 1;
    for pair in checks.windows(2) {
        let (was, now) = (pair[0], pair[1]);
        // The person said Done on it (a reopened card): no round.
        if was.resolved.as_ref().is_some_and(|r| r.option >= 0) {
            continue;
        }
        let Some(old) = blockers.iter().find(|b| b.id == was.id) else {
            continue;
        };
        let said = old
            .thread
            .iter()
            .any(|it| it.kind == THREAD_COMMENT && it.who.status == "person");
        let answer = old
            .resolved
            .as_ref()
            .filter(|r| {
                !said
                    && !r.closed
                    && r.option < 0
                    && r.by.status == "person"
                    && !r.answer.is_empty()
            })
            .map(|r| r.answer.clone())
            .unwrap_or_default();
        if !said && answer.is_empty() {
            continue;
        }
        rounds += 1;
        let round = CardRound {
            id: was.id.clone(),
            round: rounds,
            proof: was.context.clone(),
            answer,
            thread: old.thread.clone(),
            same_proof: !was.context.is_empty() && was.context == now.context,
        };
        if let Some(b) = blockers.iter_mut().find(|b| b.id == now.id) {
            b.previous = Some(round);
        }
    }
}

/// The notices the person's answer `cm` on a blocker sent (card #476):
/// they carry the card, not the comment, so they are the card's blocker
/// answers that name its blocker, sent as it was written.
fn answer_notices(p: &People, t: &Task, cm: &Comment) -> Vec<(Delivery, bool)> {
    let named = format!(", blocker {}).", cm.blocker_id);
    p.answers_heard
        .get(&t.id)
        .into_iter()
        .flatten()
        .filter(|(d, _)| {
            let sent = d.message.sent_at.unix_nano();
            let at = cm.at.unix_nano();
            sent >= at && sent - at <= ANSWER_NOTICE_WITHIN && d.message.text.contains(&named)
        })
        .cloned()
        .collect()
}

/// How soon after the person's answer its notice is sent, in nanoseconds:
/// the same call writes both.
const ANSWER_NOTICE_WITHIN: i64 = 10_000_000_000;

fn heard_by(ds: &[(Delivery, bool)]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (d, read) in ds {
        if d.to.is_empty() || !seen.insert(d.to.as_str()) {
            continue;
        }
        if *read {
            out.push(d.to.clone());
        }
    }
    out
}

/// The PR an in-review card waits on (card #471): a linked PR GitHub
/// last called open, or that this device hasn't heard (the poller runs on
/// the holder's device).
fn wait_pr(co: &coord::Coord, t: &Task) -> Option<WaitPr> {
    if t.state != coord::IN_REVIEW {
        return None;
    }
    let heard = co.pr_states(&t.id);
    t.links.iter().find_map(|l| {
        let (repo, number) = coord::pr_link(l)?;
        let h = heard.iter().find(|h| h.pr == l.reference);
        let st = h.map(|h| h.state.as_str()).unwrap_or_default();
        let approval = h.is_some_and(|h| h.review == coord::PR_REVIEW_REQUIRED);
        (st.is_empty() || st == coord::PR_OPEN).then(|| WaitPr {
            repo,
            number,
            url: l.reference.clone(),
            approval,
        })
    })
}

/// The PR a blocked card's open external wait is on (card #509): the
/// person's merge, as coord's Needs you counts it, until GitHub is heard
/// to call it merged or closed.
/// On a shared board, only this person's own agents' waits.
fn merge_pr(co: &coord::Coord, t: &Task) -> Option<WaitPr> {
    co.merge_waits(&t.id).into_iter().find_map(|url| {
        let (repo, number) = coord::pr_link(&coord::parse_link(&url).ok()?)?;
        Some(WaitPr {
            repo,
            number,
            url,
            approval: false,
        })
    })
}

/// A card's first wait no agent will end (card #520), as coord's Needs
/// you counts it: on a gone agent, or an outside wait a gone agent raised,
/// while nobody holds the card. Waits on a PR's merge are #509's row.
fn gone_wait(co: &coord::Coord, p: &People, t: &Task) -> Option<GoneWait> {
    let gw = co.gone_waits(&t.id);
    let first = gw.first()?;
    let b = t.blockers.iter().find(|b| b.id == first.blocker_id)?;
    Some(GoneWait {
        blocker_id: b.id.clone(),
        agent: p.ref_(&first.agent),
        waited_on: first.waited_on,
        on: if first.waited_on {
            String::new()
        } else {
            b.on.as_ref()
                .map(|on| on.reference.clone())
                .unwrap_or_default()
        },
        question: b.question.clone(),
        more: (gw.len() - 1) as i64,
        no_hand_on: co.hand_on_refusal(&t.id),
    })
}

/// How many cards a wait chain follows before it stops looking.
const CHAIN_DEPTH: usize = 8;

/// Follows a card's wait on another card to where it stops (card #347):
/// "your answer on #12" when it waits on their decision, else empty (it
/// waits on an agent, a lease, a PR or something outside). A PR is never
/// the person's to review (card #471).
fn wait_you(co: &coord::Coord, first: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut id = first.to_string();
    for _ in 0..CHAIN_DEPTH {
        if id.is_empty() || !seen.insert(id.clone()) {
            break;
        }
        let Some(k) = co.get_task(&id) else { break };
        if coord::is_terminal(&k.state) {
            break;
        }
        let open = k.open_blockers();
        let asks_person = open.iter().any(|b| {
            b.status != coord::BLOCKER_ANSWERED
                && match &b.on {
                    Some(on) if for_another(&on.kind, &on.reference) => false,
                    Some(on) if waits_by_itself(&on.kind) => !b.options.is_empty(),
                    _ => true,
                }
        });
        if asks_person || k.state == coord::INPUT_REQUIRED {
            return format!("your answer on {}", k.short_ref());
        }
        id = open
            .iter()
            .filter_map(|b| b.on.as_ref())
            .find(|on| on.kind == coord::BLOCKED_ON_TASK)
            .map(|on| on.reference.clone())
            .unwrap_or_default();
    }
    String::new()
}

/// A card summary's family fields (its parent, the subtask a decision
/// waits on), from the log the card lives in (card #161: a board's for a
/// board card).
fn family_of(co: &coord::Coord, p: &People, t: &Task, s: &mut CardSummary) {
    s.wait_pr = wait_pr(co, t);
    s.merge_pr = merge_pr(co, t);
    s.gone_wait = gone_wait(co, p, t);
    s.wait_you = wait_you(co, &s.wait_task);
    if !coord::is_terminal(&t.state) && !coord::task_waits(t).is_empty() {
        let card = |id: &str| {
            co.get_task(id).map(|k| CardRef {
                id: k.id.clone(),
                short: k.short.clone(),
                title: k.title.clone(),
            })
        };
        let ch = coord::wait_chain(&|id: &str| co.get_task(id), &t.id);
        s.wait_loop = ch.loop_ids.iter().filter_map(|id| card(id)).collect();
        // A chain that ends at a card on an open PR waits on the PR, not
        // on nobody (card #471): the sweep hands an unheld one out to be
        // merged, or another member's agents approve it.
        if !ch.stuck.is_empty()
            && !co
                .get_task(&ch.stuck)
                .is_some_and(|k| wait_pr(co, &k).is_some())
        {
            s.wait_stuck = card(&ch.stuck);
        }
    }
    name_wait(s, &|id: &str| {
        co.get_task(id).map(|x| x.short_ref()).unwrap_or_default()
    });
    if !t.parent.is_empty()
        && let Some(pt) = co.get_task(&t.parent)
    {
        s.parent = Some(CardRef {
            id: pt.id.clone(),
            short: pt.short.clone(),
            title: pt.title.clone(),
        });
    }
    if !t.from.is_empty()
        && let Some(th) = co.get_task(&t.from)
    {
        s.from_thread = Some(CardRef {
            id: th.id.clone(),
            short: th.short.clone(),
            title: th.title.clone(),
        });
    }
    if let Some(r) = &t.rollup
        && !r.blocked.is_empty()
        && let Some(k) = co.get_task(&r.blocked)
    {
        s.subtask_blocked = format!("{}: {}", k.short_ref(), r.blocked_question);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(status: &str, assigned: bool) -> AgentRef {
        AgentRef {
            status: status.into(),
            assigned,
            ..AgentRef::default()
        }
    }

    // Card #199: a board card is ours when one of this person's agents
    // holds or watches it; the map stands in for private_key (this
    // person's agent:<card> keys map to a session, anyone else's stay).
    #[test]
    fn ours_is_the_persons_or_their_agents() {
        let private = |k: &str| match k {
            "agent:mine1" => "dev/sess-1".to_string(),
            "agent:mine2" => "dev/sess-2".to_string(),
            "person:me" => "user:dev".to_string(),
            other => other.to_string(),
        };
        let t = |holders: &[&str], watchers: &[&str]| Task {
            holders: holders.iter().map(|s| s.to_string()).collect(),
            watchers: watchers.iter().map(|s| s.to_string()).collect(),
            ..Task::default()
        };
        let none: [&str; 0] = [];
        assert!(ours(&t(&["agent:mine1"], &[]), none, private), "own holder");
        assert!(
            ours(&t(&["agent:other"], &["agent:mine2"]), none, private),
            "own watcher"
        );
        assert!(
            !ours(&t(&["agent:other"], &["agent:other2"]), none, private),
            "another person's agents"
        );
        assert!(
            ours(&t(&["person:me"], &[]), none, private),
            "the person holds it"
        );
        assert!(!ours(&t(&[], &[]), none, private), "nobody on it");
        let created = Task {
            created_by: "agent:mine1".into(),
            holders: vec!["agent:other".into()],
            ..Task::default()
        };
        assert!(ours(&created, none, private), "own agent created it");
        let co = Task {
            co_holders: vec!["agent:mine2".into()],
            ..Task::default()
        };
        assert!(ours(&co, none, private), "own agent owns a step");
        let raised = Task {
            blockers: vec![{
                let mut b = coord::Blocker::default();
                b.raised_by = "agent:mine1".into();
                b
            }],
            ..Task::default()
        };
        assert!(ours(&raised, none, private), "own agent raised a blocker");
        assert!(
            ours(
                &t(&["agent:other"], &[]),
                ["agent:other2", "person:me"],
                private
            ),
            "the person commented"
        );
        assert!(
            !ours(&t(&["agent:other"], &[]), ["agent:other2"], private),
            "only others commented"
        );
    }

    #[test]
    fn holder_line_prefers_who_took_it() {
        assert_eq!(holder_line(&[]).state, Holding::Nobody);
        let h = holder_line(&[r("busy", true), r("idle", false)]);
        assert_eq!((h.state, h.others), (Holding::Idle, 1));
        assert_eq!(holder_line(&[r("busy", true)]).state, Holding::Assigned);
        assert_eq!(holder_line(&[r("gone", false)]).state, Holding::Gone);
        assert_eq!(holder_line(&[r("person", false)]).state, Holding::Person);
        assert_eq!(holder_line(&[r("busy", false)]).state, Holding::Active);
    }

    /// Card #480: a failed row from before #480 names the key and the
    /// device id; the card line says it with names.
    #[test]
    fn old_no_checkout_reasons_read_with_names() {
        let mut p = People {
            self_id: "3dc30c94".into(),
            ..People::default()
        };
        p.names.insert("05f1844a".into(), "Desktop Computer".into());
        let t = Task {
            repo_key: "remote:github.com/stephenshorton/irori".into(),
            ..Task::default()
        };
        let sp = coord::Spawn {
            op: coord::SPAWN_FAILED.into(),
            device: "05f1844a".into(),
            reason: "no checkout of remote:github.com/StephenSHorton/irori on 05f1844a".into(),
            ..coord::Spawn::default()
        };
        assert_eq!(
            cant_start_text(&t, &sp, &p),
            "no copy of irori on Desktop Computer"
        );
        let none = Task::default();
        let sp = coord::Spawn {
            reason: "no checkout of  on 05f1844a".into(),
            ..sp
        };
        assert_eq!(
            cant_start_text(&none, &sp, &p),
            "this card names no repo, and Desktop Computer has no folder for it"
        );
        // A device it can't name reads as a phrase, not a title.
        let unknown = coord::Spawn {
            device: "ffff".into(),
            ..sp.clone()
        };
        assert_eq!(
            cant_start_text(&none, &unknown, &p),
            "this card names no repo, and another device has no folder for it"
        );
        // Other reasons pass through (the trust row parses its own).
        let sp = coord::Spawn {
            reason: "not trusted: /r on 05f1844a".into(),
            ..sp
        };
        assert_eq!(cant_start_text(&t, &sp, &p), "not trusted: /r on 05f1844a");
    }

    #[test]
    fn where_names_devices() {
        let mut p = People {
            self_id: "me".into(),
            ..People::default()
        };
        p.names.insert("mac".into(), "Mac".into());
        // No name for this device yet (card #489).
        assert_eq!(p.where_("me"), "This device");
        assert_eq!(p.this_device(), "This device");
        // The hostname fallback loses its ".local".
        p.names
            .insert("me".into(), "Stephens-MacBook-Pro-2.local".into());
        assert_eq!(p.where_("me"), "Stephens-MacBook-Pro-2 (this one)");
        p.names.insert("me".into(), "Macbook Pro".into());
        assert_eq!(p.where_(""), "Macbook Pro (this one)");
        assert_eq!(p.where_("me"), "Macbook Pro (this one)");
        assert_eq!(p.where_("mac"), "Mac");
        assert_eq!(p.where_("x"), "Another device");
        assert_eq!(p.chip("mac"), "");
        // `here` follows the same test as `where` (card #489).
        let irori = p.ref_("irori");
        assert_eq!(irori.where_, "Macbook Pro (this one)");
        assert!(irori.here);
        assert!(p.ref_("me/a").here);
        assert_eq!(p.ref_("me/a").where_, "Macbook Pro (this one)");
        assert!(!p.ref_("mac/a").here);
        assert_eq!(p.ref_("mac/a").where_, "Mac");
        p.paired = true;
        assert_eq!(p.chip("mac"), "Mac");
    }

    fn notice(to: &str, reason: &str, state: &str) -> Delivery {
        let mut d = Delivery {
            to: to.into(),
            state: state.into(),
            ..Delivery::default()
        };
        d.message.reason = reason.into();
        d.message.notice = true;
        d
    }

    fn agent(p: &mut People, key: &str, name: &str, status: &str) {
        p.by_key.insert(
            key.into(),
            coord::roster::AgentCard {
                key: key.into(),
                name: name.into(),
                status: status.into(),
                ..Default::default()
            },
        );
    }

    /// Card #159: the person @mentions a busy agent that only watches the
    /// card; the thread says it hears it when its turn ends, then that it
    /// read it, until it writes on the card.
    #[test]
    fn pending_lines_for_a_mentioned_busy_watcher() {
        let mut p = People::default();
        agent(&mut p, "d/g", "garnet", "busy");
        agent(&mut p, "d/o", "otter", "unresponsive");
        agent(&mut p, "d/w", "wren", "idle");
        agent(&mut p, "d/s", "shale", "shell");
        let t = Task {
            watchers: vec!["d/g".into(), "d/w".into()],
            ..Task::default()
        };
        let never = |_: &str| false;
        let queued = [(notice("d/g", coord::NOTICE_MENTIONED, st::QUEUED), false)];
        assert_eq!(
            pending_lines(&queued, &t, &p, never),
            ["garnet is busy: it hears this when its current turn ends"]
        );
        let sent = [(notice("d/g", coord::NOTICE_MENTIONED, st::DELIVERED), false)];
        assert_eq!(
            pending_lines(&sent, &t, &p, never),
            ["garnet is busy: it hears this when its current turn ends"]
        );
        let read = [(notice("d/g", coord::NOTICE_MENTIONED, st::SKIPPED), true)];
        assert_eq!(pending_lines(&read, &t, &p, never), ["garnet read it"]);
        assert!(pending_lines(&read, &t, &p, |k| k == "d/g").is_empty());
        // A plain comment notice to a watcher stays off the thread, as before.
        let plain = [(notice("d/w", coord::NOTICE_COMMENT, st::QUEUED), false)];
        assert!(pending_lines(&plain, &t, &p, never).is_empty());
        // Closed by Irori unread (gone, stale): no line.
        let stale = [(notice("d/g", coord::NOTICE_MENTIONED, st::SKIPPED), false)];
        assert!(pending_lines(&stale, &t, &p, never).is_empty());
        let stuck = [(notice("d/o", coord::NOTICE_MENTIONED, st::DELIVERED), false)];
        assert_eq!(
            pending_lines(&stuck, &t, &p, never),
            ["otter isn't answering: it hasn't heard this yet"]
        );
        // Running a long shell command is still mid-turn.
        let shell = [(notice("d/s", coord::NOTICE_MENTIONED, st::QUEUED), false)];
        assert_eq!(
            pending_lines(&shell, &t, &p, never),
            ["shale is busy: it hears this when its current turn ends"]
        );
        // Idle: plain queued line.
        let idle = [(notice("d/w", coord::NOTICE_MENTIONED, st::QUEUED), false)];
        assert_eq!(pending_lines(&idle, &t, &p, never), ["queued for wren"]);
        // The newest notice per agent decides.
        let both = [
            (notice("d/g", coord::NOTICE_MENTIONED, st::SKIPPED), true),
            (notice("d/g", coord::NOTICE_COMMENT, st::QUEUED), false),
        ];
        assert_eq!(pending_lines(&both, &t, &p, never), ["garnet read it"]);
        // card #299: who read it, by the same newest-notice rule.
        assert_eq!(heard_by(&both), ["d/g"]);
        assert!(heard_by(&sent).is_empty());
        let newer_unread = [
            (notice("d/g", coord::NOTICE_COMMENT, st::QUEUED), false),
            (notice("d/g", coord::NOTICE_MENTIONED, st::SKIPPED), true),
        ];
        assert!(heard_by(&newer_unread).is_empty());
    }

    // Card #385: another person and their agents wear one colour, their
    // pick unless one of this person's devices wears it.
    #[test]
    fn another_persons_agents_wear_their_colour_off_my_devices() {
        let mut p = People::default();
        p.colors.insert("pc".into(), "pink".into());
        p.persons.insert(
            "person:blake".into(),
            irori_core::BoardMember {
                key: "person:blake".into(),
                name: "Blake".into(),
                board_id: "b".into(),
                color: "pink".into(),
                ..Default::default()
            },
        );
        p.board.insert(
            "agent:c1".into(),
            irori_core::BoardAgent {
                key: "agent:c1".into(),
                name: "wren".into(),
                status: "busy".into(),
                board_id: "b".into(),
                person_id: "blake".into(),
                person: "Blake".into(),
                person_color: "pink".into(),
                avatar: String::new(),
                ..Default::default()
            },
        );
        p.others = place_others(&p.persons, &p.board, &p.colors);
        let (face, agent) = (p.ref_("person:blake"), p.ref_("agent:c1"));
        assert_eq!(face.color, "lime", "nudged off the PC's pink");
        assert_eq!(agent.color, face.color);
    }

    // Card #487: a wait carries its kind, so Home can name a lease plainly,
    // and a wait on an agent that is gone says so (nothing ends it).
    #[test]
    fn a_wait_carries_its_kind_and_a_gone_agent() {
        let p = People::default();
        let on = |kind: &str, reference: &str| Task {
            state: coord::BLOCKED.into(),
            blockers: vec![{
                let mut b = coord::Blocker::default();
                b.id = "b1".into();
                b.question = "Waiting".into();
                b.on = Some(coord::BlockedBy {
                    kind: kind.into(),
                    reference: reference.into(),
                });
                b
            }],
            ..Task::default()
        };
        let lease = card_summary(
            &on(
                coord::BLOCKED_ON_LEASE,
                "ww/dolphin-slot-1 ww/dolphin-slot-2",
            ),
            &p,
        );
        assert_eq!(lease.wait_kind, "lease");
        assert_eq!(lease.wait_on, "ww/dolphin-slot-1 ww/dolphin-slot-2");
        assert!(!lease.wait_gone);
        // An agent nobody on this device knows is gone.
        let agent = card_summary(&on(coord::BLOCKED_ON_AGENT, "dev/kestrel"), &p);
        assert_eq!(agent.wait_kind, "agent");
        assert!(agent.wait_gone, "{agent:?}");
    }

    // Card #553: a version wait's pill names the version, its tooltip and
    // wait row the devices, and no device id shows, named or not.
    #[test]
    fn a_version_wait_names_the_version_and_never_a_device_id() {
        let mut p = People::default();
        p.names.insert("05f1844a00".into(), "Desktop".into());
        p.names.insert("3dc30c9400".into(), "Macbook".into());
        p.names.insert("blank00000".into(), " ".into());
        let card = |reference: &str| Task {
            state: coord::BLOCKED.into(),
            blockers: vec![{
                let mut b = coord::Blocker::default();
                b.id = "b1".into();
                b.question = "After the update".into();
                b.on = Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_VERSION.into(),
                    reference: reference.into(),
                });
                b
            }],
            ..Task::default()
        };

        let both = card_summary(&card("0.12.4 05f1844a00 3dc30c9400"), &p);
        assert_eq!(both.wait_kind, "version");
        assert_eq!(both.wait_on, "Irori 0.12.4");
        assert_eq!(both.wait_full, "Irori 0.12.4 on Desktop and Macbook");
        assert_eq!(both.waiting_on, 1);

        for (reference, full) in [
            ("0.12.4", "Irori 0.12.4"),
            ("0.12.4 3dc30c9400", "Irori 0.12.4 on Macbook"),
            ("0.12.4 feedface00", "Irori 0.12.4 on another device"),
            ("0.12.4 blank00000", "Irori 0.12.4 on another device"),
            (
                "0.12.4 05f1844a00 feedface00",
                "Irori 0.12.4 on Desktop and another device",
            ),
            (
                "0.12.4 feedface00 deadbeef00",
                "Irori 0.12.4 on 2 other devices",
            ),
            (
                "0.12.4 05f1844a00 3dc30c9400 feedface00 deadbeef00",
                "Irori 0.12.4 on Desktop, Macbook and 2 other devices",
            ),
        ] {
            let s = card_summary(&card(reference), &p);
            assert_eq!(s.wait_on, "Irori 0.12.4", "{reference}");
            assert_eq!(s.wait_full, full, "{reference}");
        }
    }

    // Card #264: on a shared board a question counts under Needs a decision
    // only for the person it's for (keys as private_task leaves them: this
    // person's "" or user:, another member's person:<id>); everyone else's
    // tile says who it waits on.
    #[test]
    fn a_question_needs_only_the_person_it_is_for() {
        let mut p = People::default();
        p.persons.insert(
            "person:blake".into(),
            irori_core::BoardMember {
                key: "person:blake".into(),
                name: "Blake".into(),
                board_id: "b".into(),
                ..Default::default()
            },
        );
        let q = |id: &str, question: &str, reference: &str| {
            let mut b = coord::Blocker::default();
            b.id = id.into();
            b.question = question.into();
            b.on = Some(coord::BlockedBy {
                kind: coord::BLOCKED_ON_PERSON.into(),
                reference: reference.into(),
            });
            b
        };
        let t = |bs: Vec<coord::Blocker>| Task {
            state: coord::BLOCKED.into(),
            blockers: bs,
            ..Task::default()
        };

        let blakes = card_summary(&t(vec![q("b1", "Rename it?", "person:blake")]), &p);
        assert_eq!(blakes.open_blockers, 0, "not in my Needs a decision");
        assert_eq!(blakes.waiting_on, 1);
        assert_eq!(blakes.wait_on, "Blake", "the tile says Waiting on Blake");
        assert_eq!(blakes.wait_question, "Rename it?");

        for mine in ["", "user:dev"] {
            let s = card_summary(&t(vec![q("b1", "Ship it?", mine)]), &p);
            assert_eq!(s.open_blockers, 1, "{mine:?} is mine, as before");
            assert_eq!(s.waiting_on, 0);
            assert_eq!(s.blocker_question, "Ship it?");
        }

        let both = card_summary(
            &t(vec![
                q("b1", "Rename it?", "person:blake"),
                q("b2", "Ship it?", ""),
            ]),
            &p,
        );
        assert_eq!((both.open_blockers, both.waiting_on), (1, 1));
        assert_eq!(both.blocker_question, "Ship it?");
    }
}
