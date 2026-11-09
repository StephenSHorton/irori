//! The action set for cards given to a person (decision 69, plan
//! docs/design/task-proposals.md §3.1): `say_given`, `give_back` and
//! `give_batch`, the buttons' and the assistant's one way in.

use std::cmp::Ordering;

use store::kinds;

use super::MAX_HANDOFF_NOTE;
use super::error::{Result, bad_state, invalid, not_allowed};
use super::fold::Fold;
use super::rows::*;
use super::spawn::{GIVEN_NOT_NOW, GIVEN_QUESTION, GIVEN_START};
use super::tasks::clean_new;
use super::types::*;
use super::util::{caller, cut, new_id};
use crate::names::trim_space;

/// The question a give-back puts to the card's owner.
pub const GIVEN_BACK_QUESTION: &str = "Given back to you";
/// Keep the card on the board.
pub const GIVEN_BACK_KEEP: &str = "Keep it";
/// Keep it, when the owner may not cancel: the only option offered.
pub const GIVEN_BACK_KEEP_ONLY: &str = "Keep it on the board";
/// Cancel the card.
pub const GIVEN_BACK_CANCEL: &str = "Cancel it";

/// What comes right before the receiver's reason in a give-back item's
/// context. [`given_back_reason`] reads it back.
const REASON_MARK: &str = " Their reason: ";

/// The receiver's reason, as a give-back item's context carries it, or "".
/// The giver's Core shows it fenced as the other person's words.
pub fn given_back_reason(context: &str) -> &str {
    context
        .split_once(REASON_MARK)
        .map_or("", |(_, why)| why.trim())
}

/// A batch holds at most this many cards.
pub const MAX_BATCH: usize = 20;
/// How much of the receiver's reason a give-back's context carries.
const MAX_REASON: usize = 1000;

/// The person's answer to a card's given question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GivenSay {
    /// Start an agent.
    Start,
    /// Not now.
    NotNow,
}

/// What a batch gives: new cards made on the board, or cards already on it.
#[derive(Debug, Clone)]
pub enum GiveItems {
    Tasks(Vec<NewTask>),
    Cards(Vec<String>),
}

/// One card given to the viewer, in [`super::Coord::given_to`]'s order.
#[derive(Debug, Clone)]
pub struct GivenItem {
    pub task: Task,
    /// The giving person's id (`Given.by_person`).
    pub giver: String,
    pub batch: String,
}

impl Fold {
    /// Whether `person` may cancel card t: its creator, or a board owner
    /// (the person form of [`Fold::may_cancel`]).
    fn person_may_cancel(&self, t: &Task, person: &str) -> bool {
        let (true, Some(kp)) = (self.on_board, &self.key_person) else {
            return true;
        };
        !person.is_empty()
            && (person == kp(&t.created_by)
                || self.board_owner.as_ref().is_some_and(|f| f(person, None)))
    }

    /// The card, given to the person `by` is, on this person's device.
    fn given_to_caller(&self, id: &str, by: &str) -> Result<(Task, Given)> {
        let t = self.open_task(id)?.clone();
        let Some(g) = t.given.clone() else {
            return Err(bad_state(format!(
                "{} isn't given to anyone",
                t.short_ref()
            )));
        };
        if !self.is_owner(by) || self.person_of(by) != g.person || !self.mine_to_serve(&t) {
            return Err(not_allowed(format!(
                "{} was given to someone else: only the person it was given to answers for it",
                t.short_ref()
            )));
        }
        Ok((t, g))
    }

    /// Answers card `id`'s open given question Start or Not now, with one
    /// `blocker.resolved`, the way the button does. Nothing else.
    pub(crate) fn say_given(&mut self, by: &str, id: &str, say: GivenSay) -> Result<Task> {
        let by = caller(by)?;
        let (t, g) = self.given_to_caller(id, &by)?;
        let open = t
            .blockers
            .iter()
            .find(|b| b.open() && b.question == GIVEN_QUESTION && b.id == g.question);
        let Some(q) = open else {
            let r = t.short_ref();
            if self.agent_on_or_starting(&t) || t.given_said.as_ref().is_some_and(|s| s.yes) {
                return Err(bad_state(format!("{r} already has an agent")));
            }
            if let Some(why) = Self::given_failed(&t) {
                return Err(bad_state(format!("{r} couldn't start: {why}")));
            }
            if t.spawn_crashes >= self.crash_limit() {
                return Err(bad_state(format!(
                    "{r} crashed its agent too many times: start it from its page"
                )));
            }
            if g.asker.is_empty() {
                return Err(bad_state(format!(
                    "{r} hasn't been asked yet: it asks on the computer with its repo once that's on"
                )));
            }
            return Err(bad_state(format!("{r} is already in Not now")));
        };
        let want = match say {
            GivenSay::Start => GIVEN_START,
            GivenSay::NotNow => GIVEN_NOT_NOW,
        };
        let Some(pick) = q.options.iter().position(|o| o.answer == want) else {
            return Err(invalid("the question has no such answer"));
        };
        let qid = q.id.clone();
        let (t, _) = self.answer(
            &by,
            id,
            &qid,
            BlockerAnswer {
                pick: pick as i64 + 1,
                resolve: true,
                ..BlockerAnswer::default()
            },
        )?;
        Ok(t)
    }

    /// The receiver returns card `id` to its owner: first the item for the
    /// owner (so the card stays BLOCKED the whole time another device folds
    /// the two rows one at a time), then their `task.left`.
    pub(crate) fn give_back(&mut self, by: &str, id: &str, reason: &str) -> Result<Task> {
        let by = caller(by)?;
        let (t, g) = self.given_to_caller(id, &by)?;
        if self.agent_on_or_starting(&t) {
            return Err(bad_state(format!(
                "{} has an agent on it: stop it before giving it back",
                t.short_ref()
            )));
        }
        let reason = trim_space(reason).to_string();
        let owner = self.owner_person(&t);
        if owner.is_empty() {
            return Err(invalid(
                "a card is given back to a person on a shared board",
            ));
        }
        let now = self.stamp();
        let opt = |answer: &str, why: &str| BlockerOption {
            answer: answer.into(),
            why: why.into(),
            by: by.clone(),
            at: now,
            recommended: false,
            link: None,
        };
        let opts = if self.person_may_cancel(&t, &owner) {
            vec![
                opt(GIVEN_BACK_KEEP, "It goes back on the board for you."),
                opt(GIVEN_BACK_CANCEL, "The card is canceled."),
            ]
        } else {
            vec![opt(
                GIVEN_BACK_KEEP_ONLY,
                "It goes back on the board for you.",
            )]
        };
        let mut cx = format!("{} gave this card back.", self.named(&g.key));
        if !reason.is_empty() {
            cx.push_str(&format!("{REASON_MARK}{}", cut(&reason, MAX_REASON)));
        }
        self.raise_with(
            &by,
            &t,
            GIVEN_BACK_QUESTION,
            GIVEN_BACK_QUESTION,
            &cx,
            opts,
            Some(BlockedBy {
                kind: BLOCKED_ON_PERSON.into(),
                reference: format!("{}{owner}", kinds::PERSON_ACTOR_PREFIX),
            }),
            false,
            now,
            Vec::new(),
            Vec::new(),
            false,
            false,
            None,
            Some(g.batch.clone()),
        )?;
        let at = self.stamp();
        let left = self.write(
            &by,
            kinds::TASK_LEFT,
            &TaskMemberRow {
                task_id: id.into(),
                who: g.key.clone(),
                by: g.key.clone(),
                at,
                ..TaskMemberRow::default()
            },
        );
        if let Err(e) = left {
            return Err(bad_state(format!(
                "{} is asked of its owner but still given to you (taking you off it failed: {e})",
                t.short_ref()
            )));
        }
        let t = &self.tasks[id];
        if t.holders.is_empty() && t.state == WORKING && t.subtasks.is_empty() && !t.shared {
            self.write(
                super::util::SWEEPER,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: SUBMITTED.into(),
                    note: format!("{}: it was given back", super::fold::NOBODY_NOTE),
                    by: super::util::SWEEPER.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        Ok(self.tasks[id].clone())
    }

    /// Gives a batch to one person: every check first, then every write,
    /// under the caller's one lock.
    pub(crate) fn give_batch(
        &mut self,
        by: &str,
        board: &str,
        to: &str,
        items: GiveItems,
        note: &str,
        assistant: bool,
    ) -> Result<Vec<Task>> {
        let by = caller(by)?;
        let to = trim_space(to).to_string();
        let board = trim_space(board).to_string();
        let note = trim_space(note).to_string();
        super::util::check_text("the batch note", &note, 0, MAX_HANDOFF_NOTE)?;
        if !self.on_board {
            return Err(invalid(
                "a batch goes to a person on a shared space's board",
            ));
        }
        super::util::check_key(&to)?;
        if !to.starts_with(kinds::PERSON_ACTOR_PREFIX) || self.person_of(&to).is_empty() {
            return Err(invalid("a batch goes to a member of the space, a person"));
        }
        if self.person_of(&to) == self.person_of(&by) {
            return Err(invalid("a batch goes to another person, not to yourself"));
        }
        if !board.is_empty() && self.board_tag(&board).is_none() {
            return Err(invalid(format!("no board {board:?} in this space")));
        }
        if self.board_closed(&board) {
            return Err(self.closed_refusal(&board));
        }
        // All of it before the first row.
        enum Plan {
            New(Box<super::tasks::CleanTask>),
            Card(String),
        }
        let mut plan = Vec::new();
        match &items {
            GiveItems::Tasks(ts) => {
                if ts.is_empty() || ts.len() > MAX_BATCH {
                    return Err(invalid(format!("a batch holds 1 to {MAX_BATCH} tasks")));
                }
                for nt in ts {
                    let mut ct = clean_new(&by, nt)?;
                    ct.board = board.clone();
                    self.check_new_weight(&by, &ct.weight, &ct.weight_reason)?;
                    plan.push(Plan::New(Box::new(ct)));
                }
            }
            GiveItems::Cards(ids) => {
                if ids.is_empty() || ids.len() > MAX_BATCH {
                    return Err(invalid(format!("a batch holds 1 to {MAX_BATCH} cards")));
                }
                let mut seen: Vec<String> = Vec::new();
                for r in ids {
                    let id = self.task_ref_locked(r)?;
                    if seen.contains(&id) {
                        return Err(invalid(format!("{r} is in the batch twice")));
                    }
                    let t = self.open_task(&id)?;
                    if t.board != board {
                        return Err(invalid(format!(
                            "{} isn't on the board the batch goes to",
                            t.short_ref()
                        )));
                    }
                    if let Some(t) = self.check_assign(&by, &id, &to)? {
                        return Err(bad_state(format!(
                            "{} is already given to them",
                            t.short_ref()
                        )));
                    }
                    seen.push(id.clone());
                    plan.push(Plan::Card(id));
                }
            }
        }
        let batch = new_id(&self.stamp());
        let agent = self.is_agent_key(&by);
        let mut out = Vec::new();
        for p in plan {
            let id = match p {
                Plan::New(ct) => self.create_locked(&by, *ct)?.id,
                Plan::Card(id) => id,
            };
            let t = self.write_assign(&by, &id, &to, &note, &batch, assistant)?;
            if agent {
                self.watch(&by, &id, true)?;
            }
            out.push(self.tasks.get(&id).cloned().unwrap_or(t));
        }
        Ok(out)
    }

    /// What is given to `viewer` and still open, in a fixed order: giver,
    /// then batch (oldest first), then card number. Never by `updatedAt`.
    pub(crate) fn given_to(&self, viewer: &str) -> Vec<GivenItem> {
        let me = self.person_of(viewer);
        if me.is_empty() {
            return Vec::new();
        }
        let mut v: Vec<GivenItem> = self
            .tasks
            .values()
            .filter(|t| !is_terminal(&t.state))
            .filter_map(|t| {
                let g = t.given.as_ref().filter(|g| g.person == me)?;
                Some(GivenItem {
                    task: t.clone(),
                    giver: g.by_person.clone(),
                    batch: g.batch.clone(),
                })
            })
            .collect();
        let num = |t: &Task| t.short.parse::<u64>().unwrap_or(u64::MAX);
        v.sort_by(|a, b| {
            a.giver
                .cmp(&b.giver)
                .then_with(|| a.batch.cmp(&b.batch))
                .then_with(|| num(&a.task).cmp(&num(&b.task)))
                .then_with(|| a.task.id.cmp(&b.task.id))
        });
        let _ = Ordering::Equal;
        v
    }

    /// The open cards this person gave that their receiver has answered,
    /// and the open cards given back to them: what the giver's assistant
    /// tells them of (decision 69). Filtered under the lock, so a sweep
    /// copies only these.
    pub(crate) fn given_news(&self, me: &str) -> Vec<Task> {
        let me_key = format!("{}{me}", kinds::PERSON_ACTOR_PREFIX);
        self.tasks
            .values()
            .filter(|t| !is_terminal(&t.state))
            .filter(|t| {
                t.given
                    .as_ref()
                    .is_some_and(|g| g.by_person == me && t.given_said.is_some())
                    || t.blockers.iter().any(|b| {
                        b.given_back
                            && b.open()
                            && b.on.as_ref().is_some_and(|o| o.reference == me_key)
                    })
            })
            .cloned()
            .collect()
    }

    /// The give-back item's pick, when `answer` is on one (decision 69).
    /// `Ok(Some(task))` when it settled the card (Cancel it); `Ok(None)`
    /// leaves the plain resolve to the caller.
    pub(crate) fn answer_given_back(
        &mut self,
        by: &str,
        t: &Task,
        b: &Blocker,
        pick: i64,
    ) -> Result<Option<Task>> {
        let owner =
            b.on.as_ref()
                .map(|o| o.reference.clone())
                .unwrap_or_default();
        let ok = match &self.key_person {
            Some(kp) if self.on_board => !owner.is_empty() && kp(&owner) == self.person_of(by),
            _ => true,
        };
        if !ok {
            return Err(not_allowed(
                "a card given back is for its owner to keep or cancel",
            ));
        }
        let picked = usize::try_from(pick - 1)
            .ok()
            .and_then(|i| b.options.get(i))
            .map(|o| o.answer.as_str());
        // Given to someone else since: a plain resolve.
        if picked == Some(GIVEN_BACK_CANCEL) && t.given.is_none() {
            return self
                .set_task_state(by, &t.id, CANCELED, "canceled after it was given back")
                .map(Some);
        }
        Ok(None)
    }
}
