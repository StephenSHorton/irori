//! The chat turn the desktop gives Core (one-assistant card 3, design §5):
//! Core picks each question this computer answers, builds its prompt and
//! grants from rows, and hands it here; this runs it on the chat's held
//! session (card 8, `assistant_held`) and blocks until it ends.
//!
//! - It sets the asking device's screen for the `screen` tool, begins the
//!   question's grants, runs the question (its message on the held
//!   `claude`, or a new one seeded with its prompt), and ends the grants,
//!   whose changed lines go in the reply.
//! - A Stop or New chat trips the job's cancel: the run is stopped at once,
//!   even while it waits on a tool call.
//! - Each event goes to Core and to any `assistant_chat_watch` opened for
//!   the turn in this window.
//! - A reply whose thread posts by itself (the ports of the chat's
//!   `splitDraft` and `postsByItself`) posts it here, once, after checking
//!   again that this is still the assistant's computer and the turn has no
//!   reply or posted thread yet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use irori_core::assistant_chat::{ChatDone, ChatEvent, ChatJob, Posted};

use crate::assistant::{
    AssistantEvent, DRAFT_CLOSE, DRAFT_OPEN, DRAFT_OPEN_CHECK, Job, NO_CLAUDE, SCREEN_STATUS,
    SIGNED_OUT, STOPPED, Sink, TOO_LONG, chat_content_blocks, screen_label, stream_json_line,
};

/// How often a run's wait looks at its cancel.
const POLL: Duration = Duration::from_millis(100);

/// `failure`'s words for a failure with no reason.
const COULD_NOT: &str = "The assistant couldn't answer. Try again.";
/// `failure`'s words before a reason.
const COULD_NOT_HEAD: &str = "The assistant couldn't answer: ";

/// The open watches of this window's chat turns.
#[derive(Default)]
pub struct ChatWatch {
    m: Mutex<WatchState>,
}

#[derive(Default)]
struct WatchState {
    /// The turn running now and its text so far.
    running: Option<(String, String)>,
    subs: HashMap<String, Vec<Sink>>,
}

/// A reply split around its drafted thread (the chat's `Split`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    /// The reply without the draft.
    pub text: String,
    /// The drafted thread, "" when there is none.
    pub draft: String,
    /// The draft is still being written.
    pub partial: bool,
    /// The person asked to check it first: only a tap posts it.
    pub check: bool,
}

/// `s` without a start of `marker` at its end ("…<<thr").
fn cut_marker_tail<'a>(s: &'a str, marker: &str) -> &'a str {
    let most = (marker.len() - 1).min(s.len());
    for n in (1..=most).rev() {
        if s.ends_with(&marker[..n]) {
            return &s[..s.len() - n];
        }
    }
    s
}

/// A reply's drafted thread taken out of its text (the chat's
/// `splitDraft`).
pub fn split_draft(raw: &str) -> Split {
    let open = [(DRAFT_OPEN, false), (DRAFT_OPEN_CHECK, true)]
        .into_iter()
        .filter_map(|(m, check)| raw.find(m).map(|i| (i, m, check)))
        .min_by_key(|(i, ..)| *i);
    let Some((at, marker, check)) = open else {
        return Split {
            text: cut_marker_tail(cut_marker_tail(raw, DRAFT_OPEN), DRAFT_OPEN_CHECK)
                .trim()
                .to_string(),
            ..Split::default()
        };
    };
    let before = raw[..at].trim();
    let rest = &raw[at + marker.len()..];
    let Some(end) = rest.find(DRAFT_CLOSE) else {
        return Split {
            text: before.to_string(),
            draft: cut_marker_tail(rest, DRAFT_CLOSE).trim().to_string(),
            partial: true,
            check,
        };
    };
    let after = rest[end + DRAFT_CLOSE.len()..].trim();
    Split {
        text: if after.is_empty() {
            before.to_string()
        } else {
            format!("{before}\n\n{after}").trim().to_string()
        },
        draft: rest[..end].trim().to_string(),
        partial: false,
        check,
    }
}

/// Whether `s` ends where the chat's `postableBlock` may: nothing but
/// white space, or one go line and white space
/// (`\s*(?:<<go [^\n]*>>\s*)?$`).
fn tail_ok(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    // One go line: `<<go `, no line break, then `>>`.
    t.starts_with("<<go ") && t.ends_with(">>") && t.len() >= "<<go >>".len() && !t.contains('\n')
}

/// Whether the reply's thread posts with no tap (the chat's
/// `postsByItself`): it holds a whole thread nobody asked to check first,
/// laid out as the prompt lays it out, its lines alone and ending the
/// reply (a go line may follow), and no marker comes before it.
pub fn posts_by_itself(text: &str) -> bool {
    let s = split_draft(text);
    if s.draft.is_empty() || s.partial || s.check {
        return false;
    }
    // The first `<<thread` anywhere must open the block.
    let Some(at) = text.find("<<thread") else {
        return false;
    };
    if !text[at..].starts_with(DRAFT_OPEN) {
        return false;
    }
    // Its line holds nothing else: spaces or tabs before it ...
    let line = &text[..at];
    let line = &line[line.rfind('\n').map_or(0, |i| i + 1)..];
    if !line.chars().all(|c| c == ' ' || c == '\t') {
        return false;
    }
    // ... and after it, then the line ends.
    let after = text[at + DRAFT_OPEN.len()..].trim_start_matches([' ', '\t']);
    let Some(body) = after.strip_prefix('\n') else {
        return false;
    };
    // A closing line (`\n[ \t]*<</thread>>`) that only white space or a go
    // line follows.
    let mut from = 0;
    while let Some(i) = body[from..].find(DRAFT_CLOSE) {
        let j = from + i;
        if body[..j].trim_end_matches([' ', '\t']).ends_with('\n')
            && tail_ok(&body[j + DRAFT_CLOSE.len()..])
        {
            return true;
        }
        from = j + DRAFT_CLOSE.len();
    }
    false
}

/// A failure in the words the chat shows on every device, naming the
/// computer (§5).
pub fn chat_error(name: &str, raw: &str) -> String {
    let raw = raw.trim();
    let head = format!("Claude Code on {name} couldn't answer");
    if raw == SIGNED_OUT {
        return format!(
            "{head}: it isn't signed in. Run `claude` in a terminal on {name} to sign in."
        );
    }
    if raw == NO_CLAUDE {
        return format!("{head}: Claude Code isn't installed there.");
    }
    if raw == TOO_LONG {
        return format!("{head}: it took too long. Try again.");
    }
    if raw.is_empty() || raw == COULD_NOT {
        return format!("{head}. Try again.");
    }
    match raw.strip_prefix(COULD_NOT_HEAD) {
        Some(why) => format!("{head}: {why}"),
        None => format!("{head}: {raw}"),
    }
}

impl crate::Window {
    /// Runs one question for Core's chat worker and blocks until it ends
    /// (installed with `Core::set_chat_turn` on desktops).
    pub fn chat_turn(&self, job: &ChatJob, emit: &dyn Fn(ChatEvent) -> bool) -> ChatDone {
        self.chat_turn_with(crate::roster::find_claude(), job, emit)
    }

    /// [`Self::chat_turn`] with the `claude` to run given (tests).
    #[doc(hidden)]
    pub fn chat_turn_with(
        &self,
        bin: Result<PathBuf, String>,
        job: &ChatJob,
        emit: &dyn Fn(ChatEvent) -> bool,
    ) -> ChatDone {
        let core = &self.core;
        core.set_assistant_screen(&job.screen);
        let run = core.assistant_grants().begin(job.grants.clone());
        let files = self.chat_file_bytes(&job.files);
        // The seed for a new held session, and the message for the held
        // one (card 8); a job with no message always seeds.
        let line = stream_json_line(&chat_content_blocks(&job.prompt, &files));
        let message = (!job.message.trim().is_empty())
            .then(|| stream_json_line(&chat_content_blocks(&job.message, &files)));
        let mut j = Job::assistant(core, line);
        if let Ok(j) = j.as_mut() {
            j.look = screen_label(&job.screen);
        }
        self.chat_watch_start(&job.turn);
        let (tx, rx) = mpsc::channel::<AssistantEvent>();
        let sink: Sink = Box::new(move |e| tx.send(e).is_ok());
        // Every chat question goes through the held session (#1064's
        // owner's-OK turn, when it comes, picks its own path by `kind`).
        // It may end the turn before it returns (no `claude`).
        self.held_chat
            .start_turn(bin, j, job.session, message, sink);
        let mut text = String::new();
        let mut looked = false;
        let mut stopping = false;
        let mut end: Option<AssistantEvent> = None;
        let mut changed: Vec<String> = Vec::new();
        while end.is_none() {
            if !stopping && job.cancel.load(Ordering::SeqCst) {
                stopping = true;
                self.held_chat.stop();
            }
            let ev = match rx.recv_timeout(POLL) {
                Ok(ev) => ev,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => AssistantEvent::error(STOPPED),
            };
            match ev.kind.as_str() {
                "delta" => text.push_str(&ev.text),
                "tool" if ev.text.starts_with(SCREEN_STATUS) => looked = true,
                _ => {}
            }
            if ev.ends() {
                // What it changed, just before its end, as today.
                for l in core.assistant_grants().end(run) {
                    let l = l.trim().to_string();
                    if l.is_empty() {
                        continue;
                    }
                    let c = AssistantEvent::changed(&l);
                    emit(ChatEvent {
                        kind: c.kind.clone(),
                        text: c.text.clone(),
                    });
                    self.chat_watch_send(&job.turn, &c);
                    changed.push(l);
                }
            }
            let go_on = emit(ChatEvent {
                kind: ev.kind.clone(),
                text: ev.text.clone(),
            });
            self.chat_watch_send(&job.turn, &ev);
            if !go_on && !stopping {
                stopping = true;
                self.held_chat.stop();
            }
            if ev.ends() {
                end = Some(ev);
            }
        }
        self.chat_watch_end(&job.turn);
        let end = end.unwrap_or_else(|| AssistantEvent::error(STOPPED));
        let cancelled = job.cancel.load(Ordering::SeqCst);
        let mut done = ChatDone {
            text: text.trim().to_string(),
            changed,
            looked: (looked && !job.screen.trim().is_empty()).then(|| screen_label(&job.screen)),
            ..ChatDone::default()
        };
        if end.kind == "error" {
            if end.text.trim() == STOPPED && !cancelled {
                // Stopped by Irori closing, not by the person: no reply,
                // so the next start answers it.
                done.later = true;
            } else if !cancelled {
                let name = core.device_label(core.device_id());
                done.error = chat_error(&name, &end.text);
            }
            return done;
        }
        if !cancelled && posts_by_itself(&text) {
            done.posted = self.chat_post(&job.turn, &text);
        }
        done
    }

    /// Ends the chat's held session: this computer is no longer the
    /// assistant's (installed on the Core's home listener).
    pub fn chat_held_end(&self) {
        self.held_chat.end();
    }

    /// How many held chat processes have started (tests).
    #[doc(hidden)]
    pub fn chat_held_starts(&self) -> u64 {
        self.held_chat.starts()
    }

    /// Posts the reply's thread, after checking again that this is the
    /// assistant's computer and that no device has replied to the turn or
    /// posted its thread yet.
    fn chat_post(&self, turn: &str, text: &str) -> Option<Posted> {
        let fresh = self
            .core
            .assistant_chat_turn(turn)
            .is_some_and(|t| t.said.is_none() && t.posted.is_none());
        if !self.core.is_assistant_home() || !fresh {
            return None;
        }
        let draft = split_draft(text).draft;
        // A post that fails leaves the draft with Post thread, to tap.
        let r = self
            .start_thread_with(&draft, &[], &[], "", None, true)
            .ok()?;
        Some(Posted {
            id: r.task.id,
            short: r.task.short,
            tag: r.task.tag,
        })
    }

    /// Watches turn `turn` in this window: `sink` gets each of its events
    /// from now until it ends, and first the text it has said so far, if
    /// it is running. A turn already answered gets nothing.
    pub fn assistant_chat_watch(&self, turn: &str, sink: Sink) {
        let answered = |t: &str| {
            self.core
                .assistant_chat_turn(t)
                .is_some_and(|t| t.said.is_some())
        };
        if answered(turn) {
            return;
        }
        let mut w = self.chat_watch.m.lock().unwrap_or_else(|e| e.into_inner());
        // Turns answered without running here (stopped, skipped, another
        // computer) end their watches now.
        let running = w.running.as_ref().map(|(t, _)| t.clone());
        w.subs
            .retain(|t, _| running.as_deref() == Some(t.as_str()) || !answered(t));
        if let Some((t, so_far)) = &w.running
            && t == turn
            && !so_far.is_empty()
            && !sink(AssistantEvent::delta(so_far))
        {
            return;
        }
        w.subs.entry(turn.to_string()).or_default().push(sink);
    }

    fn chat_watch_start(&self, turn: &str) {
        let mut w = self.chat_watch.m.lock().unwrap_or_else(|e| e.into_inner());
        w.running = Some((turn.to_string(), String::new()));
    }

    fn chat_watch_send(&self, turn: &str, ev: &AssistantEvent) {
        let mut w = self.chat_watch.m.lock().unwrap_or_else(|e| e.into_inner());
        if ev.kind == "delta"
            && let Some((t, so_far)) = w.running.as_mut()
            && t == turn
        {
            so_far.push_str(&ev.text);
        }
        if let Some(subs) = w.subs.get_mut(turn) {
            subs.retain(|s| s(ev.clone()));
        }
    }

    fn chat_watch_end(&self, turn: &str) {
        let mut w = self.chat_watch.m.lock().unwrap_or_else(|e| e.into_inner());
        w.subs.remove(turn);
        if w.running.as_ref().is_some_and(|(t, _)| t == turn) {
            w.running = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str, draft: &str, partial: bool, check: bool) -> Split {
        Split {
            text: text.into(),
            draft: draft.into(),
            partial,
            check,
        }
    }

    fn draft(body: &str) -> String {
        format!("{DRAFT_OPEN}\n{body}\n{DRAFT_CLOSE}")
    }

    // The chat's own cases (assistant.test.ts, cards #783 and #988).
    #[test]
    fn split_draft_as_the_chat_splits_it() {
        assert_eq!(
            split_draft("Two cards wait on you."),
            split("Two cards wait on you.", "", false, false)
        );
        assert_eq!(
            split_draft(&format!(
                "Drafted it, naming #8.\n\n{}",
                draft("Add a changelog page. #8 has the notes.")
            )),
            split(
                "Drafted it, naming #8.",
                "Add a changelog page. #8 has the notes.",
                false,
                false
            )
        );
        let s = split_draft(&format!(
            "Here it is.\n{}\nTap Post thread.",
            draft("Fix the lobby.")
        ));
        assert_eq!(s.text, "Here it is.\n\nTap Post thread.");
        assert_eq!(s.draft, "Fix the lobby.");
        assert_eq!(
            split_draft("Drafting.\n<<thr"),
            split("Drafting.", "", false, false)
        );
        assert_eq!(
            split_draft(&format!("Drafting.\n{DRAFT_OPEN}\nFix the lob")),
            split("Drafting.", "Fix the lob", true, false)
        );
        assert_eq!(
            split_draft(&format!("{DRAFT_OPEN}\nFix the lobby.\n<</thr")),
            split("", "Fix the lobby.", true, false)
        );
        let held = format!("Waiting for you.\n{DRAFT_OPEN_CHECK}\nFix the lobby.\n{DRAFT_CLOSE}");
        assert_eq!(
            split_draft(&held),
            split("Waiting for you.", "Fix the lobby.", false, true)
        );
        let s = split_draft(&format!("{DRAFT_OPEN_CHECK}\nFix the"));
        assert!(s.check && s.partial);
        for tail in ["<<thread>", "<<thread ", "<<thread chec"] {
            assert_eq!(split_draft(&format!("Writing.\n{tail}")).text, "Writing.");
        }
        assert_eq!(
            split_draft(&format!(
                "A thread goes between {DRAFT_OPEN} and {DRAFT_CLOSE}."
            ))
            .draft,
            "and"
        );
    }

    #[test]
    fn only_a_whole_thread_nobody_asked_to_check_posts_itself() {
        let whole = format!("Posting it.\n{DRAFT_OPEN}\nFix the lobby.\n{DRAFT_CLOSE}");
        let held = format!("Waiting for you.\n{DRAFT_OPEN_CHECK}\nFix the lobby.\n{DRAFT_CLOSE}");
        assert!(posts_by_itself(&whole));
        assert!(!posts_by_itself(&held));
        assert!(!posts_by_itself("Two cards wait on you."));
        assert!(!posts_by_itself(&format!("{DRAFT_OPEN}\nFix the")));
        assert!(!posts_by_itself(&format!("{DRAFT_OPEN}\n{DRAFT_CLOSE}")));
        let quoted = format!("A thread goes between {DRAFT_OPEN} and {DRAFT_CLOSE}.");
        assert!(!posts_by_itself(&quoted));
        assert!(!posts_by_itself(&format!("{quoted}\n\n{whole}")));
        assert!(!posts_by_itself(&format!("{DRAFT_OPEN}Fix.{DRAFT_CLOSE}")));
        assert!(!posts_by_itself(&format!("{whole}\nAnd more words.")));
        assert!(posts_by_itself(&format!("{whole}\n\n<<go card #8>>")));
        // Indented marker lines are still the block's own lines.
        assert!(posts_by_itself(&format!(
            "Posting it.\n  {DRAFT_OPEN}  \nFix the lobby.\n\t{DRAFT_CLOSE}\n"
        )));
        // A go line with a break inside isn't one go line.
        assert!(!posts_by_itself(&format!("{whole}\n<<go card\n#8>>")));
    }

    #[test]
    fn failures_name_the_computer() {
        assert_eq!(
            chat_error("Mac", SIGNED_OUT),
            "Claude Code on Mac couldn't answer: it isn't signed in. Run `claude` in a terminal on Mac to sign in."
        );
        assert_eq!(
            chat_error("Mac", "The assistant couldn't answer: usage limit reached."),
            "Claude Code on Mac couldn't answer: usage limit reached."
        );
        assert_eq!(
            chat_error("Mac", COULD_NOT),
            "Claude Code on Mac couldn't answer. Try again."
        );
        assert_eq!(
            chat_error("Mac", TOO_LONG),
            "Claude Code on Mac couldn't answer: it took too long. Try again."
        );
    }
}
