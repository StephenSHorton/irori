//! Let it answer (docs/design/assistants-talk.md §8, card 6; one-assistant.md
//! §9): the person taps the button on any device, which appends an
//! `assistant.answer.requested` row. The assistant's computer serves each
//! request that has no record yet, and writes a record naming it (an
//! `assistant.chat.update` row carrying the request), so a request is never
//! served twice, after a restart or a move.
//!
//! A tap skips the waiting-line check, the rate limit, the hour and the
//! receive setting, since the owner chose it. It keeps the author and
//! channel check, the answer-id check (so a second tap elsewhere does
//! nothing), the hop limit, the scrub and the cap, and it is checked against
//! the question post itself, never the row's `space`.

use std::sync::Arc;

use coord::talk::PostRow;
use inbox::MAX_HOP;
use store::Event;
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};

use super::Core;
use super::Services;
use super::assistant_answers::{
    Done, Job, JobKind, MAX_AGE_SECS, Question, REQUEST_KEY, answer_id,
};
use super::assistant_chat::{AnswerRequested, ChatRow, Update};
use super::boards::BoardLog;

/// Let it answer: the tap that runs the tool-less turn.
pub const HOW_LET: &str = "let";
/// Answer this: the owner's-OK draft (card 8).
pub const HOW_DRAFT: &str = "draft";

/// A question that passed [`Services::checked_question`].
pub(crate) struct Checked {
    pub(crate) b: Arc<BoardLog>,
    /// The person's assistant card on the space.
    pub(crate) card: String,
    pub(crate) q: Question,
}

/// A space where the person has an assistant: what the chat needs to read
/// the channel for lines about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantTalkCard {
    pub space: String,
    /// The person's assistant card on the space (`agent:<card>` posts).
    pub card: String,
    /// The space's `#office-assistants` channel.
    pub conv: String,
    /// The person's name as the space knows it, for "Addam's assistant".
    pub owner: String,
}

impl Services {
    /// Serves each tap nothing has served yet, here, when this device is
    /// the assistant's computer: `let` runs the tool-less turn, `draft`
    /// (Answer this) queues the owner's-OK turn (card 8).
    pub(crate) fn serve_answer_requests(&self) {
        if !self.is_assistant_home() {
            return;
        }
        for req in self.assistant_chat.unserved_requests() {
            match req.how.as_str() {
                HOW_LET => self.serve_request(&req),
                HOW_DRAFT => self.serve_draft_request(&req),
                _ => {}
            }
        }
    }

    /// "Tap again" when the tap is too old to become a public post.
    pub(crate) fn request_too_old(&self, req: &AnswerRequested) -> Option<String> {
        let now_ms = (self.now)().unix_milli_value();
        if now_ms - req.at <= MAX_AGE_SECS * 1000 {
            return None;
        }
        let name = self
            .device_labels()
            .get(&self.device.id)
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "The assistant's computer".into());
        Some(format!(
            "{name} was away when you tapped this. Tap again if it still needs an answer."
        ))
    }

    /// The question post `post`, checked against the post itself (its own
    /// space and channel, its author, the hop; never a row's `space`).
    /// `None` when it isn't found; `Err` says why it isn't one this
    /// person's assistant can answer.
    pub(crate) fn checked_question(&self, post: &str) -> Option<Result<Checked, &'static str>> {
        let (b, ev, r, author) = self.find_question(post)?;
        let card = b.verifier.profile_assistant_card(&self.person_id());
        let asker = ev.actor.clone();
        let me = format!("{AGENT_ACTOR_PREFIX}{card}");
        let ok = !card.is_empty()
            && !author.is_empty()
            && author != self.person_id()
            && b.verifier.membership().active(&author)
            && (asker.starts_with(AGENT_ACTOR_PREFIX) || asker.starts_with(PERSON_ACTOR_PREFIX))
            && r.reply_to.as_deref().is_none_or(str::is_empty)
            && r.mentions.contains(&me)
            && self.talk.channel(&r.conv_id).is_some_and(|c| !c.archived)
            && coord::row_hop(&ev.payload) < MAX_HOP;
        if !ok {
            return Some(Err("That isn't a question your assistant can answer."));
        }
        let hop = coord::row_hop(&ev.payload);
        Some(Ok(Checked {
            b,
            card,
            q: Question {
                post_id: r.post_id.clone(),
                conv_id: r.conv_id.clone(),
                text: r.text.clone(),
                asker,
                hop,
            },
        }))
    }

    fn serve_request(&self, req: &AnswerRequested) {
        // A tap made long ago never becomes a public post: tap again.
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
        self.assistant_queue(Job {
            kind: JobKind::Answer,
            board_id: checked.b.id.clone(),
            q: checked.q,
            request: req.request.clone(),
        });
    }

    /// Writes the record of a served tap: the worker calls it with how the
    /// job ended.
    pub(crate) fn request_served(&self, job: &Job, done: Done) {
        let Ok(b) = self.board(&job.board_id) else {
            return;
        };
        let asker = self.asker_name(&b, &job.q.asker);
        let text = match done {
            // The held line itself carries the request.
            Done::Held(_) | Done::Later => return,
            Done::Answered => format!("Answered {asker}'s question."),
            Done::NeedsOwner => {
                format!("{asker}'s question needs you: tap Answer this.")
            }
            Done::Failed => format!(
                "Your assistant couldn't answer {asker} just now. Tap Let it answer to try again."
            ),
            Done::AlreadyAnswered => "That question is answered already.".to_string(),
        };
        self.request_record(&job.request, &text);
    }

    /// The `update` line that names a request: from then on it is served.
    pub(crate) fn request_record(&self, request: &str, text: &str) {
        let _ = self.write_chat(ChatRow::Update(Update {
            key: format!("{REQUEST_KEY}{request}"),
            text: text.to_string(),
            request: request.to_string(),
            at: (self.now)().unix_milli_value(),
            ..Update::default()
        }));
    }

    /// The question post `post_id`, found in the assistants channel of a
    /// space: its board, event, row and the person who wrote it.
    fn find_question(&self, post_id: &str) -> Option<(Arc<BoardLog>, Event, PostRow, String)> {
        for b in self.visible_boards() {
            if self.board_leaving(&b) || self.is_hidden_space(&b.id) {
                continue;
            }
            let conv = coord::talk::assistants_channel_id(&b.id);
            let f = store::Filter {
                kinds: vec![kinds::TALK_POST.to_string()],
                ..Default::default()
            };
            let mut found: Option<(Event, PostRow)> = None;
            let _ = super::boards::each_row(&b.store, f, |ev| {
                if found.is_some() {
                    return;
                }
                if let Ok(r) = serde_json::from_slice::<PostRow>(&ev.payload)
                    && r.post_id == post_id
                    && r.conv_id == conv
                {
                    found = Some((ev.clone(), r));
                }
            });
            if let Some((ev, r)) = found {
                let author = b.verifier.person_of(&ev.device_id)?;
                return Some((b, ev, r, author));
            }
        }
        None
    }

    /// The spaces where this person has an assistant card.
    pub(crate) fn assistant_talk_cards(&self) -> Vec<AssistantTalkCard> {
        let mut out = Vec::new();
        for b in self.visible_boards() {
            if self.board_leaving(&b) || self.is_hidden_space(&b.id) {
                continue;
            }
            let card = b.verifier.profile_assistant_card(&self.person_id());
            if card.is_empty() {
                continue;
            }
            out.push(AssistantTalkCard {
                space: b.id.clone(),
                card,
                conv: coord::talk::assistants_channel_id(&b.id),
                owner: self.owner_name(&b),
            });
        }
        out
    }

    /// One toast for a line that waits on the person: the same id per
    /// conversation, so a burst is one card. A click opens the Office
    /// Assistant chat (tab `assistant`); the id is a Talk one, so a phone
    /// shows it and opens the channel.
    pub(crate) fn assistant_talk_toast(&self, conv: &str, title: &str, body: &str) -> bool {
        let n = self.notifier.0.clone();
        self.notifier.0.post(notify::Note {
            id: notify::talk_id(conv, "assistant"),
            title: title.to_string(),
            body: notify::preview(body),
            on_click: Some(Arc::new(move || {
                n.open(notify::Target {
                    tab: TAB_ASSISTANT.into(),
                    ref_: String::new(),
                })
            })),
            ..notify::Note::default()
        })
    }
}

/// The desktop's target tab for the Office Assistant chat.
pub const TAB_ASSISTANT: &str = "assistant";

impl Core {
    /// The person taps Let it answer (`how` = `let`) or Answer this
    /// (`draft`) on question `post` of `space`: appends the request, which
    /// the assistant's computer serves. Returns the request's id.
    pub fn assistant_answer_request(
        &self,
        post: &str,
        space: &str,
        how: &str,
    ) -> Result<String, String> {
        let how = how.trim();
        if how != HOW_LET && how != HOW_DRAFT {
            return Err(format!("how is {HOW_LET} or {HOW_DRAFT}"));
        }
        let post = post.trim();
        if post.is_empty() {
            return Err("a request needs the question's post".into());
        }
        let now = (self.s.now)();
        let request = coord::new_id(&now);
        self.s
            .write_chat(ChatRow::AnswerRequested(AnswerRequested {
                request: request.clone(),
                post: post.to_string(),
                space: space.trim().to_string(),
                how: how.to_string(),
                by: format!("user:{}", self.s.device.id),
                at: now.unix_milli_value(),
            }))?;
        self.s.serve_answer_requests();
        Ok(request)
    }

    /// The spaces where this person has an assistant, with its card and
    /// channel (the chat's labelled lines are built from them).
    pub fn assistant_talk_cards(&self) -> Vec<AssistantTalkCard> {
        self.s.assistant_talk_cards()
    }

    /// Raises the toast for a line that waits on the person.
    pub fn assistant_talk_toast(&self, conv: &str, title: &str, body: &str) -> bool {
        self.s.assistant_talk_toast(conv, title, body)
    }
}
