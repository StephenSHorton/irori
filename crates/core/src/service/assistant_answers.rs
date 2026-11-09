//! The answering path of Office Assistants talking across people
//! (docs/design/assistants-talk.md §5, card 4).
//!
//! A post in a space's #office-assistants channel that mentions this
//! person's assistant card is a question to it. On the assistant's computer
//! (the person's first desktop that can run the turn) the question goes
//! through the checks of §5's table, in order, and the first that fails
//! stops it. What passes is queued for one worker, which runs the turn
//! through an [`Answerer`] the desktop app installs (a fake in tests) and
//! posts the text as the assistant's card, with a deterministic answer id so
//! two desktops that both answer show one post. Ask me posts the fixed
//! waiting line instead, under a waiting id.
//!
//! The turn itself has no tools: its packet, prompt and parse are in
//! [`assistant_packet`] (card 5). A reply that doesn't parse, or that only
//! the owner can answer, posts the waiting line with `waiting: "owner"`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, mpsc};

use coord::talk::PostRow;
use inbox::{BoardFrom, MAX_HOP, board_sender_key, mode, receive_decision, row_kind};
use sha2::{Digest, Sha256};
use store::Event;
use store::kinds::{self, AGENT_ACTOR_PREFIX, PERSON_ACTOR_PREFIX};

use super::Services;
use super::assistant_packet::{self, Verdict};
use super::boards::BoardLog;
use super::thread_names::{NameAsk, Namer};
use crate::logf;

/// The turn: the question in, the text to post out.
pub type Answerer = Namer;

/// What [`answer_id`] hashes before the card and the question.
pub const ANSWER_DOMAIN: &str = "irori.assistants.answer.1";
/// What [`waiting_id`] hashes before the card and the question.
pub const WAITING_DOMAIN: &str = "irori.assistants.waiting.1";

/// A question older than this (by the asker's clock) isn't answered.
pub const MAX_AGE_SECS: i64 = 60 * 60;
/// Questions answered per asking person per window.
pub const RATE_MAX: usize = 8;
/// The rate limit's window.
pub const RATE_WINDOW_SECS: i64 = 10 * 60;
/// The longest reply Irori posts, in characters (counted after
/// normalising).
pub const MAX_REPLY: usize = assistant_packet::MAX_REPLY;
/// How many held lines are kept in memory.
const HELD_KEEP: usize = 50;

/// The id of the answer to `question` by assistant card `card`: `post_` and
/// a hash of a fixed domain, the card and the question's post id. Both of a
/// person's desktops compute the same, so Talk's fold keeps one post.
pub fn answer_id(card: &str, question: &str) -> String {
    hashed_id(ANSWER_DOMAIN, card, question)
}

/// The id of the waiting line for `question`: the answer id's hash under a
/// different domain, so a later answer is never blocked by it.
pub fn waiting_id(card: &str, question: &str) -> String {
    hashed_id(WAITING_DOMAIN, card, question)
}

fn hashed_id(domain: &str, card: &str, question: &str) -> String {
    let sum = Sha256::digest(format!("{domain}|{card}|{question}").as_bytes());
    format!("post_{}", &hex::encode(sum)[..24])
}

/// A line the person's assistant didn't post because a check held it back
/// (§8's `held`): kept in memory on the assistant's computer only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub board_id: String,
    /// The question's post id; empty for a rate-limited batch.
    pub post_id: String,
    pub text: String,
}

/// What a queued job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    /// Run the turn and post its text.
    Answer,
    /// Post the waiting line, `waiting: "ask"`.
    Wait,
}

/// One question for the worker.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    kind: JobKind,
    board_id: String,
    q: Question,
}

/// What the worker needs of a question post.
#[derive(Debug, Clone)]
pub(crate) struct Question {
    post_id: String,
    conv_id: String,
    text: String,
    /// The asker's board key (`person:<id>` or `agent:<cardId>`).
    asker: String,
    hop: i64,
}

/// The answering path's state.
#[derive(Default)]
pub(crate) struct Answering {
    answerer: Mutex<Option<Answerer>>,
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    /// Questions queued or running, so a burst asks once.
    busy: Mutex<HashSet<String>>,
    /// Questions that reached the rate limit: counted once, never again.
    settled: Mutex<HashSet<String>>,
    /// Recent counted questions per asking person, in Unix ms.
    rate: Mutex<HashMap<String, VecDeque<i64>>>,
    held: Mutex<Vec<Held>>,
    was_home: std::sync::atomic::AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Services {
    /// Installs (or takes away) the turn that answers other people's
    /// questions (the desktop app's `claude -p`; a fake in tests). Installing
    /// it scans for questions that came before it.
    pub(crate) fn set_answerer(&self, a: Option<Answerer>) {
        let some = a.is_some();
        *lock(&self.asst.answerer) = a;
        if some {
            self.assistant_scan();
        }
    }

    /// Sets the queue the answering worker reads.
    pub(crate) fn answers_queue(&self, tx: Option<mpsc::Sender<Job>>) {
        *lock(&self.asst.jobs) = tx;
    }

    /// The lines the assistant held back, oldest first.
    pub(crate) fn assistant_held(&self) -> Vec<Held> {
        lock(&self.asst.held).clone()
    }

    fn hold(&self, board_id: &str, post_id: &str, text: String) {
        logf(format_args!("assistant: held on {board_id}: {text}"));
        let mut h = lock(&self.asst.held);
        h.push(Held {
            board_id: board_id.into(),
            post_id: post_id.into(),
            text,
        });
        let over = h.len().saturating_sub(HELD_KEEP);
        h.drain(..over);
    }

    /// This person's assistant card on `b`, or "" before it exists.
    fn my_assistant_card(&self, b: &BoardLog) -> String {
        b.verifier.profile_assistant_card(&self.person_id())
    }

    /// A question row met live by the bridge: `ev` is the post, `r` its
    /// row, `author` the person who wrote it. The channel and author checks
    /// of the bridge ran already.
    pub(crate) fn assistant_question(&self, b: &BoardLog, ev: &Event, r: &PostRow, author: &str) {
        let q = Question {
            post_id: r.post_id.clone(),
            conv_id: r.conv_id.clone(),
            text: r.text.clone(),
            asker: ev.actor.clone(),
            hop: coord::row_hop(&ev.payload),
        };
        self.assistant_check(b, q, &r.at, author);
    }

    /// §5's table, in order. The first check that fails stops the question.
    fn assistant_check(
        &self,
        b: &BoardLog,
        q: Question,
        at: &gocompat::time::GoTime,
        author: &str,
    ) {
        let card = self.my_assistant_card(b);
        // Another active member asked, in this space's channel.
        if card.is_empty()
            || author.is_empty()
            || author == self.person_id()
            || !b.verifier.membership().active(author)
            || !(q.asker.starts_with(AGENT_ACTOR_PREFIX)
                || q.asker.starts_with(PERSON_ACTOR_PREFIX))
        {
            return;
        }
        // This device is the assistant's computer; the other one handles it.
        if !self.is_assistant_home() {
            return;
        }
        // (The post has no `replyTo`: the bridge sends only those here.)
        // Not already answered, or waiting on.
        if self.talk.has_post(&answer_id(&card, &q.post_id))
            || self.talk.has_post(&waiting_id(&card, &q.post_id))
        {
            return;
        }
        if q.hop >= MAX_HOP {
            return;
        }
        if lock(&self.asst.settled).contains(&q.post_id) {
            return;
        }
        // The rate limit, per asking person in this space.
        let now_ms = (self.now)().unix_nano() / 1_000_000;
        let key = board_sender_key(&b.id, "", author);
        let name = self.board_person_name(b, author);
        {
            let mut rate = lock(&self.asst.rate);
            let seen = rate.entry(key).or_default();
            while seen
                .front()
                .is_some_and(|t| now_ms - t > RATE_WINDOW_SECS * 1000)
            {
                seen.pop_front();
            }
            if seen.len() >= RATE_MAX {
                drop(rate);
                lock(&self.asst.settled).insert(q.post_id.clone());
                self.hold(
                    &b.id,
                    "",
                    format!("Questions from {name} were not answered"),
                );
                return;
            }
            seen.push_back(now_ms);
        }
        lock(&self.asst.settled).insert(q.post_id.clone());
        // Younger than an hour by the asker's clock; the future is now.
        let asked_ms = (at.unix_nano() / 1_000_000).min(now_ms);
        if now_ms - asked_ms > MAX_AGE_SECS * 1000 {
            self.hold(
                &b.id,
                &q.post_id,
                format!(
                    "{}'s assistant didn't get to this in time",
                    self.ident.current().name
                ),
            );
            return;
        }
        let kind = match self.answer_mode(b, author) {
            Some(k) => k,
            None => return,
        };
        self.assistant_queue(Job {
            kind,
            board_id: b.id.clone(),
            q,
        });
    }

    /// What the pause and the receive setting make of a question: answer it,
    /// post the waiting line, or nothing.
    fn answer_mode(&self, b: &BoardLog, author: &str) -> Option<JobKind> {
        // Pause all is treated as Ask me.
        if self.inbox.policy().paused {
            return Some(JobKind::Wait);
        }
        let set = self.inbox.receive_setting(&b.id, author);
        let from = BoardFrom {
            board_id: b.id.clone(),
            person_id: author.to_string(),
            kind: row_kind::COMMENT.into(),
            wake: true,
            ..BoardFrom::default()
        };
        match receive_decision(&set, &from, false) {
            (mode::WAKE, "", _) => Some(JobKind::Answer),
            (_, inbox::hold::MEMBER_WAKE, _) => Some(JobKind::Wait),
            _ => None,
        }
    }

    fn assistant_queue(&self, job: Job) {
        let id = job.q.post_id.clone();
        if !lock(&self.asst.busy).insert(id.clone()) {
            return;
        }
        let queued = lock(&self.asst.jobs)
            .as_ref()
            .is_some_and(|q| q.send(job).is_ok());
        if !queued {
            lock(&self.asst.busy).remove(&id);
            lock(&self.asst.settled).remove(&id);
        }
    }

    /// The worker: one job at a time until the Core stops.
    pub(crate) fn answers_loop(&self, rx: mpsc::Receiver<Job>) {
        while let Ok(job) = rx.recv() {
            let id = job.q.post_id.clone();
            let run =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.answers_run(&job)));
            if run.is_err() {
                logf(format_args!("assistant: answering {id} failed"));
            }
            lock(&self.asst.busy).remove(&id);
        }
    }

    fn answers_run(&self, job: &Job) {
        let Ok(b) = self.board(&job.board_id) else {
            return;
        };
        let card = self.my_assistant_card(&b);
        if card.is_empty() {
            return;
        }
        let q = &job.q;
        if self.talk.has_post(&answer_id(&card, &q.post_id)) {
            return;
        }
        match job.kind {
            JobKind::Wait => self.post_waiting(&b, &card, q, "ask"),
            JobKind::Answer => {
                let Some(turn) = lock(&self.asst.answerer).clone() else {
                    // No turn here yet: set_answerer scans for it later.
                    lock(&self.asst.settled).remove(&q.post_id);
                    return;
                };
                let packet = self.answer_packet(&b, &q.post_id);
                let ask = NameAsk {
                    system: assistant_packet::system(&packet),
                    prompt: assistant_packet::prompt(
                        &packet,
                        &self.asker_name(&b, &q.asker),
                        &q.text,
                        // The inbox's limit on a message (§6).
                        inbox::MAX_TEXT,
                    ),
                };
                let reply = match turn(&ask) {
                    Ok(r) => r,
                    Err(e) => {
                        logf(format_args!("assistant: answering {}: {e}", q.post_id));
                        return;
                    }
                };
                let text = match assistant_packet::parse(&reply) {
                    Verdict::Answer(t) => assistant_packet::normalise(&t),
                    Verdict::NeedsOwner(why) => {
                        logf(format_args!(
                            "assistant: {} needs its owner: {}",
                            q.post_id,
                            assistant_packet::clip(&why, 200)
                        ));
                        return self.post_waiting(&b, &card, q, "owner");
                    }
                };
                // Nothing visible was left: only the owner can answer.
                if text.is_empty() {
                    return self.post_waiting(&b, &card, q, "owner");
                }
                let why = scrub::board_reasons(&text);
                if !why.is_empty() {
                    self.hold(
                        &b.id,
                        &q.post_id,
                        format!("An answer was held back: it held {}", why.join(", ")),
                    );
                    return;
                }
                let id = answer_id(&card, &q.post_id);
                if let Err(e) = self.assistant_post(&b, &card, q, id, text, None) {
                    logf(format_args!("assistant: posting {}: {e}", q.post_id));
                }
            }
        }
    }

    /// Posts the fixed waiting line for `q` (`waiting` is `ask` or
    /// `owner`) under its waiting id, unless one is there already.
    fn post_waiting(&self, b: &BoardLog, card: &str, q: &Question, waiting: &str) {
        let id = waiting_id(card, &q.post_id);
        if self.talk.has_post(&id) {
            return;
        }
        let text = format!("I've asked {}", self.owner_name(b));
        if let Err(e) = self.assistant_post(b, card, q, id, text, Some(waiting)) {
            logf(format_args!(
                "assistant: waiting line on {}: {e}",
                q.post_id
            ));
        }
    }

    /// The owner's name as the space knows it.
    fn owner_name(&self, b: &BoardLog) -> String {
        let me = self.person_id();
        b.verifier
            .profile(&me)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| self.ident.current().name)
    }

    /// The asker's name for the turn, from the space log only: a person's
    /// profile, or their agent's card name.
    fn asker_name(&self, b: &BoardLog, key: &str) -> String {
        let person = b.verifier.key_person(key);
        let name = self.board_person_name(b, &person);
        match key.strip_prefix(AGENT_ACTOR_PREFIX) {
            Some(id) => {
                let agent = lock(&b.maps)
                    .cards
                    .get(id)
                    .map(|c| c.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| "an agent".into());
                format!("{agent} ({name}'s agent)")
            }
            None => name,
        }
    }

    /// Posts `text` in the question's channel as the assistant's card.
    fn assistant_post(
        &self,
        b: &BoardLog,
        card: &str,
        q: &Question,
        post_id: String,
        text: String,
        waiting: Option<&str>,
    ) -> Result<(), String> {
        let by = format!("{AGENT_ACTOR_PREFIX}{card}");
        let row = PostRow {
            post_id,
            conv_id: q.conv_id.clone(),
            text,
            mentions: vec![q.asker.clone()],
            agents_now: true,
            reply_to: Some(q.post_id.clone()),
            waiting: waiting.map(str::to_string),
            by: by.clone(),
            at: (self.now)(),
            ..PostRow::default()
        };
        let mut v = serde_json::to_value(&row).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            // The assistant has no session key for board_hop to find.
            o.insert("hop".into(), (q.hop + 1).into());
        }
        self.space_talk_post(&b.id, &by, v)
    }

    /// Writes a raw talk.post row to space `space` as `by`, folds it and
    /// tells the window. `PostRow` has no hop, so the assistant's posts go
    /// in this way rather than through `Talk::post`.
    pub(crate) fn space_talk_post(
        &self,
        space: &str,
        by: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        let ev = self.append_space_talk(space, by, kinds::TALK_POST, payload)?;
        if self.talk.apply(&ev) {
            self.hub.publish(crate::events::CoreEvent::Talk);
        }
        Ok(())
    }

    /// Looks through every space's assistants channel for questions to this
    /// person's assistant that came before the turn was here (or while this
    /// device wasn't the assistant's computer), and queues them as live ones
    /// are. Only questions under an hour old.
    pub(crate) fn assistant_scan(&self) {
        if !self.is_assistant_home() {
            return;
        }
        for b in self.visible_boards() {
            if self.board_leaving(&b) || self.is_hidden_space(&b.id) {
                continue;
            }
            let card = self.my_assistant_card(&b);
            if card.is_empty() {
                continue;
            }
            let conv = coord::talk::assistants_channel_id(&b.id);
            let me = format!("{AGENT_ACTOR_PREFIX}{card}");
            let now_ms = (self.now)().unix_nano() / 1_000_000;
            let f = store::Filter {
                kinds: vec![kinds::TALK_POST.to_string()],
                ..Default::default()
            };
            let mut found: Vec<(Event, PostRow)> = Vec::new();
            let _ = super::boards::each_row(&b.store, f, |ev| {
                let Ok(r) = serde_json::from_slice::<PostRow>(&ev.payload) else {
                    return;
                };
                let young = now_ms - r.at.unix_nano() / 1_000_000 <= MAX_AGE_SECS * 1000;
                if r.conv_id == conv
                    && r.reply_to.as_deref().is_none_or(str::is_empty)
                    && r.mentions.contains(&me)
                    && young
                {
                    found.push((ev.clone(), r));
                }
            });
            for (ev, r) in found {
                let Some(author) = b.verifier.person_of(&ev.device_id) else {
                    continue;
                };
                if self.talk.channel(&conv).is_none_or(|c| c.archived) {
                    continue;
                }
                self.assistant_question(&b, &ev, &r, &author);
            }
        }
    }

    /// The sweep's part: scan when this device has just become the
    /// assistant's computer.
    pub(crate) fn assistant_home_moved(&self) {
        let home = self.is_assistant_home();
        let was = self
            .asst
            .was_home
            .swap(home, std::sync::atomic::Ordering::SeqCst);
        if home && !was {
            self.assistant_scan();
        }
    }
}
