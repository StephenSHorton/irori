//! The chair, routed questions and the end of a huddle (card #1053,
//! docs/design/call-agents.md §6 "Ambient fill", §3 "Over", L3, L3b, L4).
//!
//! Every assistant digests only its own person's lines (L3). A `question`
//! item that lands goes to the assistants whose people hold a matching card
//! (L3b), which write answers only. One assistant chairs: it writes the
//! title, and at the end the summary with its action items and `refs` (L4),
//! and, after a crash, the end mark. The chair comes from the huddle's
//! `call.assist` rows ([`huddle_rules::chair`]); each host evaluates the same
//! rows, so nothing needs coordinating.
//!
//! The decisions are pure and live in `huddle_rules` (card 6); this module
//! gathers their facts from the Core, acts on them from the host's sweep, and
//! words the looks.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coord::talk::{CALL_END, CallInfo, CallMark, is_person_key};

use super::Services;
use super::huddle_host::lock;
use super::huddle_packet::{self, DATA_TAG};
use super::huddle_rules::{
    self, Card, Chairing, Item, Kind, Layer, Look, Seat, WRITE_WAIT, Writer, due_writer,
};
use super::huddle_session::fence_lines;

/// `logf` with format arguments.
macro_rules! logf {
    ($($t:tt)*) => { crate::logf(format_args!($($t)*)) };
}

/// The chair's session stays open this long after the huddle is over, at
/// most, when no summary appears (§7).
pub(super) const CHAIR_STAYS: Duration = Duration::from_secs(5 * 60);
/// A crashed huddle gets its end mark only while it is this recent.
const CRASH_FOR: Duration = Duration::from_secs(6 * 60 * 60);
/// A title is proposed once the huddle has run this long (§3 "Titles").
pub(super) const TITLE_AFTER: Duration = Duration::from_secs(2 * 60);

impl Services {
    // ------------------------------------------------------------- the chair

    /// A person key as this device writes it: its own person's keys are
    /// `user:<device>`, everyone else's `person:<id>`.
    fn huddle_norm(&self, key: &str) -> String {
        if is_person_key(key) {
            self.user_sender().key
        } else {
            key.to_string()
        }
    }

    /// The huddle's start mark, if this device has it.
    fn huddle_info(&self, call: &str) -> Option<CallInfo> {
        self.talk.calls().into_iter().find(|c| c.call_id == call)
    }

    /// Every assistant with a `call.assist` row, as a [`Seat`]; and who
    /// started the huddle. None for a huddle this device has no rows of.
    pub(super) fn huddle_seats(&self, call: &str) -> Option<(String, Vec<Seat>)> {
        let info = self.huddle_info(call)?;
        let rows = self.talk.call_assists(call);
        let me_id = self.person_id();
        let (live, space) = self.call_people_now(call);
        let mut away = self.own_away_for(
            &self
                .spawn
                .agents
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        );
        self.others_away_for(&space, &rows, &mut away);
        let present: Vec<String> = live
            .iter()
            .map(|p| {
                if *p == me_id {
                    self.user_sender().key
                } else {
                    format!("{}{p}", store::kinds::PERSON_ACTOR_PREFIX)
                }
            })
            .collect();
        let starter = self.huddle_norm(&info.starter);
        let mut people: Vec<String> = Vec::new();
        for r in &rows {
            let k = self.huddle_norm(&r.by);
            if !people.contains(&k) {
                people.push(k);
            }
        }
        let seats = people
            .into_iter()
            .map(|p| {
                let mine: Vec<_> = rows
                    .iter()
                    .filter(|r| self.huddle_norm(&r.by) == p)
                    .collect();
                let on: Vec<_> = mine.iter().filter(|r| r.on).collect();
                let first = mine
                    .iter()
                    .map(|r| r.first_on)
                    .filter(|t| *t > 0)
                    .min()
                    .unwrap_or(0);
                // A person who started it and turned the switch off is
                // passed over.
                let declines = mine.iter().any(|r| r.declines_chair);
                Seat {
                    away: on
                        .iter()
                        .map(|r| away.get(&r.device).copied().unwrap_or(Duration::ZERO))
                        .min()
                        .unwrap_or(Duration::ZERO),
                    first_on: UNIX_EPOCH + Duration::from_millis(first.max(0) as u64),
                    on: !on.is_empty(),
                    chair: on.iter().any(|r| r.chair),
                    present: present.contains(&p),
                    will_chair: !(declines && p == starter),
                    person: p,
                }
            })
            .collect();
        Some((starter, seats))
    }

    pub(super) fn huddle_chairing(&self, call: &str) -> Chairing {
        match self.huddle_seats(call) {
            Some((starter, seats)) => huddle_rules::chair(&starter, &seats),
            None => Chairing::default(),
        }
    }

    /// Whether this device's person's assistant is the huddle's chair now.
    pub(super) fn huddle_is_chair(&self, call: &str) -> bool {
        self.huddle_chairing(call).chair.as_deref() == Some(self.user_sender().key.as_str())
    }

    /// This host's `call.assist` row says what the rows say it should: the
    /// chair flag when it chairs (or stands down), and the chair switch. A
    /// row is written only when one of them differs.
    pub(super) fn huddle_chair_row(&self, call: &str, conv: &str) {
        let me = self.user_sender().key;
        let want = self.huddle_chairing(call).chair.as_deref() == Some(me.as_str());
        let declines = !self.huddle_chair_on();
        let mine = self
            .talk
            .call_assists(call)
            .into_iter()
            .find(|a| a.on && a.device == self.device.id && is_person_key(&a.by));
        let Some(mine) = mine else { return };
        if mine.chair == want && mine.declines_chair == declines {
            return;
        }
        if let Err(e) = self.write_assist_as(conv, call, true, want) {
            logf!("huddle {call}: chair row: {e}");
        } else if want != mine.chair {
            logf!(
                "huddle {call}: this assistant {} the chair",
                if want { "takes" } else { "hands on" }
            );
        }
    }

    // ----------------------------------------------------------------- over

    /// How long this device has seen `call` over, None while it is live. The
    /// clock starts when the sweep first sees it over.
    pub(super) fn huddle_over_for(&self, call: &str, now: SystemTime) -> Option<Duration> {
        let over = self.call_over(call);
        let mut g = lock(&self.hosting.over);
        if !over {
            g.remove(call);
            return None;
        }
        let at = *g.entry(call.to_string()).or_insert(now);
        Some(now.duration_since(at).unwrap_or_default())
    }

    /// Whether the canvas has a `summary` item.
    pub(super) fn huddle_has_summary(&self, call: &str) -> bool {
        self.talk
            .call_items(call)
            .iter()
            .any(|i| i.kind == "summary")
    }

    /// Whether this host keeps its session after its person left or the
    /// huddle ended (§7): the chair, until it has written the summary or
    /// [`CHAIR_STAYS`] after the end; any other host until its turn to
    /// write has passed (the fallback of §3: the next assistant writes after
    /// [`WRITE_WAIT`]), so a chair that vanished at the end still gets
    /// covered.
    pub(super) fn huddle_lingers(&self, call: &str, now: SystemTime) -> bool {
        if self.huddle_has_summary(call) {
            return false;
        }
        // This host wrote the end (or tried to) and has nothing more to do.
        if lock(&self.feed.calls)
            .get(call)
            .is_some_and(|h| h.brain.end_finished())
        {
            return false;
        }
        match self.huddle_over_for(call, now) {
            // Not over: only the chair stays, and only a chair that is still
            // the chair (a person who leaves with others in the call hands
            // it on).
            None => self.huddle_is_chair(call),
            Some(over_for) => {
                if self.huddle_is_chair(call) {
                    over_for < CHAIR_STAYS
                } else {
                    over_for < WRITE_WAIT * 2
                }
            }
        }
    }

    /// The end of the huddle, from the host's sweep: L4 for the assistant due
    /// to write the summary, and the end mark of a crashed huddle for
    /// whoever is due to write it. Nothing while the huddle is live.
    pub(super) fn huddle_end_tick(&self, call: &str, now: SystemTime) {
        let Some(over_for) = self.huddle_over_for(call, now) else {
            return;
        };
        let Some((starter, seats)) = self.huddle_seats(call) else {
            return;
        };
        let me = self.user_sender().key;
        let has_summary = self.huddle_has_summary(call);
        // A person who turned the chair off for what they started doesn't
        // write the end of it either.
        let willing: Vec<Seat> = seats.iter().filter(|s| s.will_chair).cloned().collect();
        let due = due_writer(&starter, &willing, over_for, false) == Writer::Person(me.clone());
        if due && !has_summary {
            self.huddle_end_look(call, now);
        }
        self.huddle_end_mark_if_due(call, &starter, &seats, over_for, &me);
    }

    fn huddle_end_look(&self, call: &str, now: SystemTime) {
        let Some(conv) = self.talk.call_conv(call) else {
            return;
        };
        let step = {
            let me = self.user_sender().key;
            let mut g = lock(&self.feed.calls);
            let hd = g.entry(call.to_string()).or_insert_with(|| {
                super::huddle_feed::Huddle::new(&conv, &me, self.huddle_notes_on())
            });
            hd.brain.end(now, true, false, true)
        };
        self.huddle_step(call, step);
    }

    /// A huddle that is over with no end mark (its last device crashed) gets
    /// one from the chair's host; if none appears within [`WRITE_WAIT`], from
    /// the next connected assistant's; with no assistant connected at all, from
    /// any member device that judges it over (§3). The fold keeps the first.
    fn huddle_end_mark_if_due(
        &self,
        call: &str,
        starter: &str,
        seats: &[Seat],
        over_for: Duration,
        me: &str,
    ) {
        let Some(info) = self.huddle_info(call) else {
            return;
        };
        if info.end.is_some() {
            return;
        }
        match due_writer(starter, seats, over_for, true) {
            Writer::Person(p) if p == me => {}
            Writer::AnyDevice => {}
            _ => return,
        }
        self.huddle_write_end_mark(&info);
    }

    fn huddle_write_end_mark(&self, info: &CallInfo) {
        let ms = |t: gocompat::time::GoTime| t.unix_milli_value();
        let from = info.start_at.map(ms).unwrap_or(0);
        let to = info.last_line.map(ms).unwrap_or_else(|| ms(info.last));
        let mark = CallMark {
            call_id: info.call_id.clone(),
            op: CALL_END.into(),
            duration_ms: if from > 0 { (to - from).max(0) } else { 0 },
            people: info.people.clone(),
            transcript: info.lines > 0,
        };
        match self.post_call_mark(&info.conv_id, "ended the huddle", mark) {
            Ok(_) => logf!(
                "huddle {}: wrote the end mark of a crashed huddle",
                info.call_id
            ),
            Err(e) => logf!("huddle {}: end mark: {e}", info.call_id),
        }
    }

    /// For devices that are not hosting anything: a huddle this device heard,
    /// judges over and sees no end mark, when no assistant is connected at
    /// all, gets its end mark from here (§3 "any member device"). Called
    /// from the calls ticker.
    pub(crate) fn huddle_crash_check(&self) {
        self.huddle_crash_check_at(SystemTime::now());
    }

    /// [`Services::huddle_crash_check`] at `now` (tests move the clock).
    pub(crate) fn huddle_crash_check_at(&self, now: SystemTime) {
        for info in self.talk.calls() {
            // Only a recent huddle: an old one that crashed long ago isn't
            // this device's to close now.
            let recent = info
                .last
                .unix_milli_value()
                .saturating_add(CRASH_FOR.as_millis() as i64)
                > now
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
            if info.end.is_some() || !recent {
                continue;
            }
            let Some(over_for) = self.huddle_over_for(&info.call_id, now) else {
                continue;
            };
            let Some((starter, seats)) = self.huddle_seats(&info.call_id) else {
                // No assistant rows at all: no assistant, so any device.
                if over_for >= WRITE_WAIT {
                    self.huddle_write_end_mark(&info);
                }
                continue;
            };
            if due_writer(&starter, &seats, over_for, true) == Writer::AnyDevice {
                self.huddle_write_end_mark(&info);
            }
        }
    }

    // ------------------------------------------------------------- the title

    /// The chair proposes a title once the huddle has run [`TITLE_AFTER`]
    /// with none (a person's, or an earlier one of its own): one look, once.
    /// The end updates it.
    pub(super) fn huddle_title_tick(&self, call: &str, now: SystemTime) {
        if !self.huddle_is_chair(call) || self.huddle_over_for(call, now).is_some() {
            return;
        }
        if !self.huddle_wants_title(call) {
            return;
        }
        let Some(conv) = self.talk.call_conv(call) else {
            return;
        };
        let me = self.user_sender().key;
        let step = {
            let mut g = lock(&self.feed.calls);
            let hd = g.entry(call.to_string()).or_insert_with(|| {
                super::huddle_feed::Huddle::new(&conv, &me, self.huddle_notes_on())
            });
            hd.title_look(now)
        };
        if let Some(step) = step {
            self.huddle_step(call, step);
        }
    }

    // ------------------------------------------------------ routed questions

    /// The open cards in the packet as the rules take them: holders are the
    /// people who hold them or whose agents do, by the key the feed uses.
    fn huddle_route_cards(&self, conv: &str) -> Vec<Card> {
        let me_id = self.person_id();
        let Ok(p) = self.huddle_packet(conv) else {
            return Vec::new();
        };
        p.cards
            .iter()
            .map(|c| Card {
                id: c.card_ref.clone(),
                title: c.title.clone(),
                holders: c
                    .people
                    .iter()
                    .map(|pid| {
                        if *pid == me_id {
                            self.user_sender().key
                        } else {
                            format!("{}{pid}", store::kinds::PERSON_ACTOR_PREFIX)
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// A `question` item landed (from a digest, L1, L2, Ask or a person): if
    /// this host's person, or one of their agents, holds a card that matches
    /// it, their assistant gets one look at it (L3b). Only a question
    /// triggers; the look writes answers only, which trigger nothing.
    pub(super) fn huddle_route_question(&self, call: &str, item: &coord::talk::CanvasItem) {
        let Some(kind) = Kind::parse(&item.kind) else {
            return;
        };
        if !huddle_rules::triggers_look(kind) || !self.huddle_in(call) {
            return;
        }
        // The person's own Ask already has its look (L1).
        if item.why.as_deref() == Some("ask") && is_person_key(&item.by) {
            return;
        }
        let Some(conv) = self.talk.call_conv(call) else {
            return;
        };
        let at = UNIX_EPOCH + Duration::from_millis(item.at.unix_milli_value().max(0) as u64);
        let cards = self.huddle_route_cards(&conv);
        let it = Item {
            id: item.item_id.clone(),
            kind,
            text: item.text.clone(),
            by: self.huddle_norm(&item.by),
            assistant: item.assistant,
            at,
        };
        let me = self.user_sender().key;
        let step = {
            let mut g = lock(&self.feed.calls);
            let hd = g.entry(call.to_string()).or_insert_with(|| {
                super::huddle_feed::Huddle::new(&conv, &me, self.huddle_notes_on())
            });
            hd.brain.question(SystemTime::now(), &it, &cards)
        };
        self.huddle_step(call, step);
    }

    // -------------------------------------------------------------- prompts

    /// What a look is asked to do, after the lines (the Digest, Routed and
    /// End layers; the others word theirs in `huddle_feed`).
    pub(super) fn huddle_layer_note(
        &self,
        call: &str,
        conv: &str,
        look: &Look,
        me_name: &str,
    ) -> Option<String> {
        let chair = look.layer == Layer::End || self.huddle_is_chair(call);
        match look.layer {
            Layer::Digest => {
                let mut s = format!(
                    "These are the lines since your last look. You are {me_name}'s assistant: write notes only for what {me_name} said. Each other person's assistant notes their own words, so don't repeat them. Their questions to the room become question items; their proposals and decisions become decision items (if a line agrees with or settles another item on the canvas, reply to that item's id); a commitment (\"I'll take signing\") becomes a task item. Don't answer questions: the people who know do. Write nothing when nothing belongs."
                );
                if chair && self.huddle_wants_title(call) {
                    s.push_str(
                        "\nYou chair this huddle and it has no title yet. Add a <<title>> of a few words that says what the call is about.",
                    );
                } else if chair {
                    s.push_str("\nYou chair this huddle. Keep to your notes; the title is set.");
                }
                Some(s)
            }
            Layer::Routed => {
                let q = self.huddle_look_text(call, look);
                let asker = look.caller.as_deref().map(|c| self.huddle_name_of(call, c));
                let cards = self.huddle_card_lines(conv, &look.cards);
                let id = look.question.clone().unwrap_or_default();
                Some(format!(
                    "{} put this question on the canvas (item {id}):\n{}\n\nThese open cards match it and are held by {me_name} or their agents:\n{}\n\nWrite only an answer, with reply=\"{id}\", from those cards and what was said. If they don't answer it, write nothing. Anything else you write is dropped.",
                    asker.unwrap_or_else(|| "Someone".into()),
                    fence_lines(&[q]),
                    huddle_packet::fence(DATA_TAG, "space-cards", &cards.join("\n")),
                ))
            }
            Layer::End => Some(
                "The huddle is over. You write its end, once. Put in your reply:\n\
- one <<item kind=\"summary\" refs=\"ID,ID\">> with a short summary of what was discussed and decided, and the action items (who does what). refs lists the ids of open canvas items that look answered or superseded; leave refs out if none do. You change no status: a person settles them.\n\
- up to four <<item kind=\"task\">> items for action items that are not already on the canvas as tasks.\n\
- a <<title>> that says what the call was about, if the title is missing or was only a first guess."
                    .to_string(),
            ),
            _ => None,
        }
    }

    /// How long a huddle runs before its chair proposes a title.
    fn huddle_title_after(&self) -> Duration {
        match self
            .hosting
            .title_after_ms
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            0 => TITLE_AFTER,
            ms => Duration::from_millis(ms),
        }
    }

    /// Sets that time (tests shorten it).
    pub(crate) fn set_huddle_title_after(&self, d: Duration) {
        self.hosting
            .title_after_ms
            .store(d.as_millis() as u64, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether the chair should propose a title in a digest: it has none,
    /// and the huddle has run [`TITLE_AFTER`].
    fn huddle_wants_title(&self, call: &str) -> bool {
        self.huddle_info(call).is_some_and(|i| {
            i.title.is_empty()
                && i.start_at.is_some_and(|t| {
                    let at = UNIX_EPOCH + Duration::from_millis(t.unix_milli_value().max(0) as u64);
                    SystemTime::now().duration_since(at).unwrap_or_default()
                        >= self.huddle_title_after()
                })
        })
    }

    fn huddle_name_of(&self, call: &str, key: &str) -> String {
        let me = self.user_sender().key;
        self.huddle_roomy(call, &me).name_of(key)
    }

    /// One line per matching card: `<ref> <title> · <state> · held by …`.
    fn huddle_card_lines(&self, conv: &str, refs: &[String]) -> Vec<String> {
        let Ok(p) = self.huddle_packet(conv) else {
            return refs.to_vec();
        };
        p.cards
            .iter()
            .filter(|c| refs.contains(&c.card_ref))
            .map(|c| {
                let mut l = format!(
                    "{} {} \u{b7} {}",
                    c.card_ref,
                    huddle_packet::clip(&c.title, huddle_packet::MAX_TITLE),
                    c.state
                );
                if !c.holders.is_empty() {
                    l.push_str(&format!(" \u{b7} held by {}", c.holders.join(", ")));
                }
                l
            })
            .collect()
    }
}
