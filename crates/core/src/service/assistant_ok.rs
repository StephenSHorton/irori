//! The owner's OK (docs/design/assistants-talk.md §7, card 8; one-assistant.md
//! §9): **Answer this** on a `needs` line starts a turn of its own on the
//! assistant's computer, through the one chat worker as
//! [`ChatKind::OwnerOk`].
//!
//! - The turn has its own system prompt and renderer
//!   ([`assistant_packet::owner_system`], [`assistant_packet::owner_prompt`]):
//!   the asker's words in a real fence, no history, no markers taught or
//!   parsed, the owner's read tools only. It never calls `Grants::begin`,
//!   and it starts only after the previous chat run ended its grants, so
//!   its open set is empty.
//! - Its reply is never a chat turn. It is normalised (hidden characters
//!   and marker lines stripped, links shown in full, capped), scrubbed, and
//!   written as an `assistant.answer.draft` row: private text that every
//!   device of the owner shows in full.
//! - **Send** ([`Core::assistant_answer_send`]) posts exactly the draft's
//!   normalised text as the assistant's card, with the answer id, from any
//!   device. A post with that id there already refuses it, so there is one
//!   answer. **Don't send** ([`Core::assistant_answer_dismiss`]) writes a
//!   line that closes the draft.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use inbox::MAX_TEXT;

use super::Core;
use super::Services;
use super::assistant_answers::answer_id;
use super::assistant_chat::{AnswerDraft, ChatEvent, ChatJob, ChatKind, ChatRow, Update};
use super::assistant_packet;
use super::assistant_requests::{Checked, HOW_DRAFT};
use crate::logf;

/// What the line of a sent draft starts with; the chat shows it quietly.
pub const SENT_KEY: &str = "sent:";
/// What the line of a draft the owner chose not to send starts with.
pub const DISMISSED_KEY: &str = "dismissed:";

/// One tap of Answer this for the worker.
#[derive(Debug, Clone)]
pub(crate) struct OkAsk {
    pub(crate) request: String,
    pub(crate) post: String,
}

impl OkAsk {
    /// The worker's turn id: never a chat question's.
    pub(crate) fn turn(&self) -> String {
        format!("ok:{}", self.request)
    }
}

impl Services {
    /// Queues the draft for tap `req` (how = `draft`), checked against the
    /// post itself. Called by the sweep and the tap on the assistant's
    /// computer.
    pub(crate) fn serve_draft_request(&self, req: &super::assistant_chat::AnswerRequested) {
        if !self.has_chat_turn() {
            // Installing the chat turn serves it.
            return;
        }
        if let Some(why) = self.request_too_old(req) {
            self.request_record(&req.request, &why);
            return;
        }
        let checked = match self.checked_question(&req.post) {
            None => return,
            Some(Err(why)) => {
                self.request_record(&req.request, why);
                return;
            }
            Some(Ok(c)) => c,
        };
        if self
            .talk
            .has_post(&answer_id(&checked.card, &checked.q.post_id))
        {
            self.request_record(&req.request, "That question is answered already.");
            return;
        }
        let Some((at, _)) = self.assistant_chat.request_at(&req.request) else {
            return;
        };
        self.chat_queue_ok(
            at,
            OkAsk {
                request: req.request.clone(),
                post: req.post.clone(),
            },
        );
    }

    /// The worker takes a draft: waits (the caller did) until caught up,
    /// checks it again, runs the turn and writes the draft row.
    pub(crate) fn owner_ok_take(&self, turn: &str, ok: &OkAsk, cancel: &Arc<AtomicBool>) {
        // Another computer, or a request that has a record by now.
        if !self.is_assistant_home()
            || !self
                .assistant_chat
                .unserved_requests()
                .iter()
                .any(|r| r.request == ok.request && r.how == HOW_DRAFT)
        {
            return;
        }
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        let checked = match self.checked_question(&ok.post) {
            None => return,
            Some(Err(why)) => {
                self.request_record(&ok.request, why);
                return;
            }
            Some(Ok(c)) => c,
        };
        let Checked { b, card, q, .. } = &checked;
        if self.talk.has_post(&answer_id(card, &q.post_id)) {
            self.request_record(&ok.request, "That question is answered already.");
            return;
        }
        let Some(hook) = self.chat_hook() else {
            return;
        };
        let asker = self.asker_name(b, &q.asker);
        let owner = self.owner_name(b);
        let assistant = {
            let chosen = b.verifier.profile_assistant_name(&self.person_id());
            if chosen.trim().is_empty() {
                coord::assistant_name::default_name(&owner)
            } else {
                chosen
            }
        };
        let job = ChatJob {
            turn: turn.to_string(),
            prompt: assistant_packet::owner_prompt(&asker, &q.text, MAX_TEXT),
            system: assistant_packet::owner_system(
                &owner,
                &assistant,
                &b.verifier.membership().name,
            ),
            // No grants: nothing here is the person's own message.
            grants: Vec::new(),
            cancel: cancel.clone(),
            kind: ChatKind::OwnerOk,
            ..ChatJob::default()
        };
        let c = cancel.clone();
        let t = turn.to_string();
        let done = hook(&job, &move |ev: ChatEvent| {
            self.chat_stream_send(&t, &ev);
            !c.load(Ordering::SeqCst)
        });
        if done.later || cancel.load(Ordering::SeqCst) {
            // Irori is closing: the next start serves it again.
            return;
        }
        if !done.error.is_empty() {
            self.request_record(
                &ok.request,
                &format!("{} Tap Answer this to try again.", done.error.trim()),
            );
            return;
        }
        let text = assistant_packet::normalise(&done.text);
        if text.is_empty() {
            self.request_record(
                &ok.request,
                &format!(
                    "Your assistant had nothing to draft for {asker}. Tap Answer this to try again."
                ),
            );
            return;
        }
        let mut why = scrub::board_reasons(&text);
        why.extend(assistant_packet::held_reasons(&text));
        if !why.is_empty() {
            logf(format_args!(
                "assistant: a draft for {} was held: {}",
                q.post_id,
                why.join(", ")
            ));
            self.request_record(
                &ok.request,
                &format!(
                    "The draft for {asker} was held back: it held {}. Tap Answer this to try again.",
                    why.join(", ")
                ),
            );
            return;
        }
        if let Err(e) = self.write_chat(ChatRow::AnswerDraft(AnswerDraft {
            request: ok.request.clone(),
            post: q.post_id.clone(),
            space: b.id.clone(),
            asker,
            text,
            device: self.device.id.clone(),
            at: (self.now)().unix_milli_value(),
        })) {
            logf(format_args!("assistant: draft for {}: {e}", q.post_id));
            // Without a record the sweep would run the turn again.
            self.request_record(
                &ok.request,
                "Your assistant couldn't save the draft. Tap Answer this to try again.",
            );
        }
    }

    /// Posts the draft of tap `request` as the assistant's card, exactly
    /// its (normalised) text. Refused when there is no draft, the owner
    /// chose not to send it, or an answer to the question exists.
    pub(crate) fn answer_send(&self, request: &str) -> Result<(), String> {
        let request = request.trim();
        let draft = self
            .assistant_chat
            .draft_for(request)
            .ok_or_else(|| "There is no draft to send.".to_string())?;
        // One Send at a time: the check and the post are one step.
        let _one = self.asst_send_lock();
        if self
            .assistant_chat
            .update_for(&format!("{DISMISSED_KEY}{request}"))
            .is_some()
        {
            return Err("You chose not to send this answer.".into());
        }
        let checked = match self.checked_question(&draft.post) {
            None => return Err("That question isn't here any more.".into()),
            Some(Err(why)) => return Err(why.to_string()),
            Some(Ok(c)) => c,
        };
        let Checked { b, card, q, .. } = &checked;
        let id = answer_id(card, &q.post_id);
        if self.talk.has_post(&id) {
            return Err("That question is answered already.".into());
        }
        // What was shown is what is posted: the draft is already
        // normalised, and normalising again changes nothing.
        let text = assistant_packet::normalise(&draft.text);
        if text.is_empty() {
            return Err("The draft is empty.".into());
        }
        let mut why = scrub::board_reasons(&text);
        why.extend(assistant_packet::held_reasons(&text));
        if !why.is_empty() {
            return Err(format!(
                "The draft can't be sent: it holds {}.",
                why.join(", ")
            ));
        }
        self.assistant_post(b, card, q, id, text, None)?;
        let asker = self.asker_name(b, &q.asker);
        let _ = self.write_chat(ChatRow::Update(Update {
            key: format!("{SENT_KEY}{request}"),
            text: format!("Sent your answer to {asker}."),
            request: request.to_string(),
            at: (self.now)().unix_milli_value(),
            ..Update::default()
        }));
        Ok(())
    }

    /// The owner chose not to send the draft of tap `request`.
    pub(crate) fn answer_dismiss(&self, request: &str) -> Result<(), String> {
        let request = request.trim();
        if self.assistant_chat.draft_for(request).is_none() {
            return Err("There is no draft.".into());
        }
        let _one = self.asst_send_lock();
        if self
            .assistant_chat
            .update_for(&format!("{SENT_KEY}{request}"))
            .is_some()
        {
            return Err("That answer was sent already.".into());
        }
        self.write_chat(ChatRow::Update(Update {
            key: format!("{DISMISSED_KEY}{request}"),
            text: "You chose not to send this answer.".into(),
            request: request.to_string(),
            at: (self.now)().unix_milli_value(),
            ..Update::default()
        }))
        .map(|_| ())
    }
}

impl Core {
    /// Sends the owner's-OK draft of tap `request`: posts exactly its
    /// normalised text as the assistant's card, once. Works from any
    /// device.
    pub fn assistant_answer_send(&self, request: &str) -> Result<(), String> {
        self.s.answer_send(request)
    }

    /// Don't send: closes the draft of tap `request` on every device.
    pub fn assistant_answer_dismiss(&self, request: &str) -> Result<(), String> {
        self.s.answer_dismiss(request)
    }
}
