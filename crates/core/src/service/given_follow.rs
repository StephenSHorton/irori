//! What the sender hears about the cards they gave (decision 69, plan
//! docs/design/task-proposals.md §6.2).
//!
//! The sender's own Core writes the lines, on the assistant's computer, as
//! `assistant_answers.rs`'s `hold()` does. It reads the shared board's
//! cards directly, so the lines appear with no window open and the
//! two-Core tests can check them. Each card's outcome is one
//! `assistant.chat.update` row; the first row per key wins on every device,
//! so a sweep and a live row can both ask for the same line.
//!
//! | fact                      | line                          | dot |
//! |---------------------------|-------------------------------|-----|
//! | `start:<card>:<gen>`      | "Spencer started an agent on #12." | no  |
//! | `notnow:<card>:<question>`| "Spencer put #15 aside for now."   | no  |
//! | `back:<card>:<blocker>`   | "Spencer gave back #15."           | yes |
//!
//! A give-back's reason is the receiver's, so it goes in the row's `from`,
//! never in `text`. The thread is `given:<batch>`, or `given:<giver id>`
//! when the give had no batch. The chat folds the start and not-now lines
//! of one thread when it shows them (`frontend/src/lib/assistant.ts`); the
//! constants below are the strings both sides use.

use coord::{Task, TaskFilter};

use super::Services;
use super::assistant_chat::{Act, ChatRow, Update, UpdateFrom};
use super::boards::BoardLog;
use super::huddle_packet::{clip, escape};
use crate::logf;

/// What a given-follow thread starts with.
pub const THREAD_PREFIX: &str = "given:";
/// The fact of a card the person started an agent on.
pub const FACT_START: &str = "start";
/// The fact of a card the person put aside.
pub const FACT_NOT_NOW: &str = "notnow";
/// The fact of a card the person gave back.
pub const FACT_BACK: &str = "back";
/// The words a start line is made of: `<name>` + this + `<ref>` + `.`.
pub const START_VERB: &str = " started an agent on ";
/// The words a not-now line is made of: `<name> put <ref>` + this + `.`
pub const NOT_NOW_VERB: &str = " put ";
/// The tail of a not-now line.
pub const NOT_NOW_TAIL: &str = " aside for now";
/// The words a give-back line is made of: `<name>` + this + `<ref>` + `.`.
pub const BACK_VERB: &str = " gave back ";

/// The longest name a line carries.
const MAX_NAME: usize = 60;

/// The thread of the lines about a give: its batch, or the giver.
fn thread_of(batch: &str, giver: &str) -> String {
    if batch.is_empty() {
        format!("{THREAD_PREFIX}{giver}")
    } else {
        format!("{THREAD_PREFIX}{batch}")
    }
}

/// A line's row key: the chat's `updateKey(thread, fact)`.
fn key_of(thread: &str, fact: &str) -> String {
    format!("{thread}|{fact}")
}

impl Services {
    /// The host sweep's part: on the assistant's computer, every open card
    /// this person gave gets its outcome lines (the backstop for rows that
    /// came while this wasn't running).
    pub(crate) fn given_follow_sweep(&self) {
        if !self.is_assistant_home() {
            return;
        }
        let open = TaskFilter {
            open: true,
            ..TaskFilter::default()
        };
        for b in self.visible_boards() {
            if self.board_leaving(&b) || self.is_hidden_space(&b.id) {
                continue;
            }
            for t in b.coord().list_tasks(&open) {
                self.given_follow_task(&b, &t);
            }
        }
    }

    /// A row of another member just came in on `t`: say what it changed,
    /// when this is the assistant's computer. Its card may not have folded
    /// the row yet, so fold what the log holds first.
    pub(crate) fn given_follow_row(&self, b: &BoardLog, t: &Task) {
        if !self.is_assistant_home() {
            return;
        }
        let co = b.coord();
        let _ = co.catch_up();
        let t = co.get_task(&t.id).unwrap_or_else(|| t.clone());
        self.given_follow_task(b, &t);
    }

    /// The lines card `t` owes this person, each written once.
    pub(crate) fn given_follow_task(&self, b: &BoardLog, t: &Task) {
        let me = self.person_id();
        let name = |person: &str| escape(&clip(&self.board_person_name(b, person), MAX_NAME));
        let short = t.short_ref();
        // The receiver's start or Not now on a card this person gave.
        if let Some(g) = t
            .given
            .as_ref()
            .filter(|g| g.by_person == me && g.person != me)
            && let Some(said) = t.given_said.as_ref().filter(|s| s.person == g.person)
        {
            let who = name(&g.person);
            let thread = thread_of(&g.batch, &g.by_person);
            if said.yes {
                self.given_line(
                    key_of(
                        &thread,
                        &format!("{FACT_START}:{}:{}", t.id, said.generation),
                    ),
                    &thread,
                    format!("{who}{START_VERB}{short}."),
                    None,
                    None,
                );
            } else if !g.question.is_empty() {
                self.given_line(
                    key_of(&thread, &format!("{FACT_NOT_NOW}:{}:{}", t.id, g.question)),
                    &thread,
                    format!("{who}{NOT_NOW_VERB}{short}{NOT_NOW_TAIL}."),
                    None,
                    None,
                );
            }
        }
        // A card given back to this person: it waits on them.
        let me_key = format!("{}{me}", store::kinds::PERSON_ACTOR_PREFIX);
        for bl in &t.blockers {
            let to_me = bl.on.as_ref().is_some_and(|on| on.reference == me_key);
            let from = b.verifier.key_person(&bl.raised_by);
            if !bl.given_back || !bl.open() || !to_me || from.is_empty() || from == me {
                continue;
            }
            let who = name(&from);
            let thread = thread_of(&bl.batch, &me);
            let reason = coord::given_back_reason(&bl.context);
            self.given_line(
                key_of(&thread, &format!("{FACT_BACK}:{}:{}", t.id, bl.id)),
                &thread,
                format!("{who}{BACK_VERB}{short}."),
                Some(Act {
                    kind: "answer".into(),
                    card: t.id.clone(),
                    blocker: bl.id.clone(),
                }),
                (!reason.is_empty()).then(|| UpdateFrom {
                    name: self.board_person_name(b, &from),
                    words: reason.to_string(),
                }),
            );
        }
    }

    /// Writes one line unless the chat has it already.
    fn given_line(
        &self,
        key: String,
        thread: &str,
        text: String,
        act: Option<Act>,
        from: Option<UpdateFrom>,
    ) {
        if self.assistant_chat.update_for(&key).is_some() {
            return;
        }
        let row = ChatRow::Update(Update {
            key: key.clone(),
            thread: thread.to_string(),
            text,
            act,
            at: (self.now)().unix_milli_value(),
            from,
            ..Update::default()
        });
        if let Err(e) = self.write_chat(row) {
            logf(format_args!("assistant: given line {key}: {e}"));
        }
    }
}
