//! The live line feed of a huddle's assistant (card #1050,
//! docs/design/call-agents.md §8 "The feed", §6 L1 and L2).
//!
//! One path: the `call.segment` rows. [`Services::huddle_line`] is called
//! after the fold on the three live paths (this device's own write, the
//! space follow, the private-log follow). A refold never calls it, so the
//! refold at start and a resync feed nothing.
//!
//! Per huddle this device serves, the feed keeps a buffer of the last
//! [`KEEP`] of lines (context for a look and the seed of a restart), the ids
//! it has seen (the follows also deliver this device's own writes), and the
//! person's [`Brain`] from `huddle_rules`, which decides L1 and L2. Digests,
//! routed questions and the end (L3, L3b, L4) are card 10's, so the brain's
//! digest is switched off here.
//!
//! A look runs on its own thread: it blocks on the held session. The reply
//! is not read here; card 8 turns blocks into canvas rows.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coord::talk::{SegmentRow, is_dm, is_person_key};

use super::Services;
use super::huddle_rules::{Assistant, Brain, Layer, Line, Look, Person, Room, Settings, Step};
use super::huddle_session::{HuddleSeed, fence_lines};

/// `logf` with format arguments.
macro_rules! logf {
    ($($t:tt)*) => { crate::logf(format_args!($($t)*)) };
}

/// How much talk the buffer keeps: a restart is seeded with the last
/// [`SEED_FOR`] of it.
const KEEP: Duration = Duration::from_secs(10 * 60);
/// A restart's seed carries this much of the last talk (§7).
const SEED_FOR: Duration = Duration::from_secs(5 * 60);
/// A look with no new lines still carries this many of the last ones.
const CONTEXT_LINES: usize = 3;
/// The longest one line is kept at.
const LINE_CHARS: usize = 600;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One finished phrase as the feed receives it.
#[derive(Debug, Clone)]
pub(crate) struct Heard {
    pub call_id: String,
    pub conv_id: String,
    /// The device that wrote the row.
    pub device: String,
    pub seg: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// The speaker's notice let assistants hear the line (§10).
    pub agents: bool,
    /// The speaker, as the log names them.
    pub by: String,
}

impl Heard {
    pub(crate) fn of_row(device: &str, r: &SegmentRow) -> Heard {
        Heard {
            call_id: r.call_id.clone(),
            conv_id: r.conv_id.clone(),
            device: device.to_string(),
            seg: r.seg,
            start_ms: r.start_ms,
            end_ms: r.end_ms,
            text: r.text.clone(),
            agents: r.agents,
            by: r.by.clone(),
        }
    }

    /// The line's id (`seg_<call>_<device>_<seg>`), the same on every device.
    fn id(&self) -> String {
        format!("seg_{}_{}_{}", self.call_id, self.device, self.seg)
    }
}

/// A buffered line: the rules' [`Line`], and a running number.
#[derive(Debug, Clone)]
struct Buffered {
    n: u64,
    line: Line,
}

/// One huddle this device serves.
struct Huddle {
    conv: String,
    seen: BTreeSet<String>,
    lines: VecDeque<Buffered>,
    next: u64,
    /// Lines numbered below this went to the session already.
    sent: u64,
    brain: Brain,
    /// A timer thread is waiting for the hold's end.
    timer: bool,
    /// The assistant's name in the session's system prompt.
    name: String,
}

impl Huddle {
    fn new(conv: &str, me: &str) -> Huddle {
        Huddle {
            conv: conv.to_string(),
            seen: BTreeSet::new(),
            lines: VecDeque::new(),
            next: 0,
            sent: 0,
            // L3 (the digest) is card 10's.
            brain: Brain::new(me, Settings { digest: false }),
            timer: false,
            name: String::new(),
        }
    }

    /// One line in. None when it doesn't count: a line seen already (the
    /// follows also get this device's own writes), or in a person DM one
    /// whose speaker's notice didn't let assistants hear it (§10). Otherwise
    /// it joins the buffer and the brain decides whether it calls a look; a
    /// line older than the freshness window only becomes context.
    fn take(&mut self, h: &Heard, me: &str, now: SystemTime, room: &Room<'_>) -> Option<Step> {
        if is_dm(&h.conv_id) && !h.agents {
            return None;
        }
        let line = Line {
            id: h.id(),
            speaker: if is_person_key(&h.by) {
                me.to_string()
            } else {
                h.by.clone()
            },
            text: clip(&h.text),
            began: at_ms(h.start_ms),
            ended: at_ms(h.end_ms.max(h.start_ms)),
        };
        if !self.seen.insert(line.id.clone()) {
            return None;
        }
        let n = self.next;
        self.next += 1;
        self.lines.push_back(Buffered {
            n,
            line: line.clone(),
        });
        while self.lines.len() > 1
            && self
                .lines
                .front()
                .is_some_and(|b| now.duration_since(b.line.ended).unwrap_or_default() > KEEP)
        {
            self.lines.pop_front();
        }
        Some(self.brain.line(now, &line, room))
    }
}

/// The feed's state on [`Services`].
#[derive(Default)]
pub(crate) struct Feed {
    calls: Mutex<HashMap<String, Huddle>>,
}

fn at_ms(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)
}

fn clip(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= LINE_CHARS {
        t.to_string()
    } else {
        t.chars().take(LINE_CHARS).collect()
    }
}

/// Who the room has, as the rules want them.
struct Roomy {
    people: Vec<Person>,
    assistants: Vec<Assistant>,
}

impl Roomy {
    fn room(&self) -> Room<'_> {
        Room {
            people: &self.people,
            assistants: &self.assistants,
        }
    }

    fn name_of(&self, key: &str) -> String {
        self.people
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Someone".into())
    }
}

impl Services {
    /// A finished line of a huddle, after the fold on a live path (§8). Drops
    /// what this host doesn't serve, what it has seen (the follows also get
    /// this device's own writes), and in a person DM a line whose speaker's
    /// notice didn't let assistants hear it. The rest joins the buffer; the
    /// brain then decides whether the line calls the assistant (L1) or is its
    /// own person asking (L2), and only a fresh line can.
    pub(crate) fn huddle_line(&self, h: &Heard) {
        if h.call_id.is_empty() || h.text.trim().is_empty() {
            return;
        }
        // The fold bound this huddle to a conversation: a row that names
        // another one is not this huddle's.
        if self.talk.call_conv(&h.call_id).as_deref() != Some(h.conv_id.as_str()) {
            return;
        }
        if !self.huddle_served(&h.call_id) {
            // Not a huddle this device's person is in: nothing to keep.
            lock(&self.feed.calls).remove(&h.call_id);
            return;
        }
        let me = self.user_sender().key;
        // The room is gathered outside the feed's lock.
        let roomy = self.huddle_roomy(&h.call_id, &me);
        let now = SystemTime::now();
        let step = {
            let mut g = lock(&self.feed.calls);
            let hd = g
                .entry(h.call_id.clone())
                .or_insert_with(|| Huddle::new(&h.conv_id, &me));
            let Some(step) = hd.take(h, &me, now, &roomy.room()) else {
                return;
            };
            hd.name = roomy
                .assistants
                .iter()
                .find(|a| a.person == me)
                .map(|_| self.assistant_label())
                .unwrap_or_default();
            step
        };
        self.huddle_step(&h.call_id, step);
    }

    /// Whether this device's person is in the huddle, which is whom this
    /// device may serve (the host check proper runs before each look).
    fn huddle_served(&self, call: &str) -> bool {
        self.huddle_in(call)
    }

    /// A row of a log, if it is a `call.segment`: feeds it.
    pub(crate) fn huddle_line_row(&self, ev: &store::Event) {
        if ev.kind != store::kinds::CALL_SEGMENT {
            return;
        }
        if let Ok(r) = serde_json::from_slice::<SegmentRow>(&ev.payload) {
            self.huddle_line(&Heard::of_row(&ev.device_id, &r));
        }
    }

    /// The assistant's label for its system prompt: "Juno · Addam's
    /// assistant".
    fn assistant_label(&self) -> String {
        coord::assistant_name::label(&self.ident.current().name, &self.assistant_name.chosen())
    }

    /// The people and connected assistants of `call` as the rules take them.
    fn huddle_roomy(&self, call: &str, me_key: &str) -> Roomy {
        let (_, space) = self.call_people_now(call);
        let board = (!space.is_empty())
            .then(|| self.board(&space).ok())
            .flatten();
        let me_name = self.ident.current().name.clone();
        let mut people: Vec<Person> = Vec::new();
        let mut assistants: Vec<Assistant> = Vec::new();
        let states = self.call_assistants(call);
        // Always the person themself, even before an assistant row shows.
        let mut keys: Vec<String> = vec![me_key.to_string()];
        for s in &states {
            let k = if is_person_key(&s.person) {
                me_key.to_string()
            } else {
                s.person.clone()
            };
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        for k in keys {
            let (name, chosen) = if k == me_key {
                (me_name.clone(), self.assistant_name.chosen())
            } else {
                let pid = k
                    .strip_prefix(store::kinds::PERSON_ACTOR_PREFIX)
                    .unwrap_or(&k)
                    .to_string();
                match board.as_ref() {
                    Some(b) => (
                        b.verifier
                            .profile(&pid)
                            .or_else(|| {
                                b.verifier.membership().person(&pid).map(|p| p.name.clone())
                            })
                            .unwrap_or_default(),
                        b.verifier.profile_assistant_name(&pid),
                    ),
                    None => (String::new(), String::new()),
                }
            };
            let connected = states.iter().any(|s| {
                s.state == "connected"
                    && (s.person == k || (k == me_key && is_person_key(&s.person)))
            });
            if connected {
                let chosen = chosen.trim();
                assistants.push(Assistant {
                    person: k.clone(),
                    chosen: (!chosen.is_empty()).then(|| chosen.to_string()),
                });
            }
            people.push(Person { key: k, name });
        }
        Roomy { people, assistants }
    }

    /// The seed of `call`'s session: how it is called, where it is, and the
    /// last minutes of talk for a restart.
    pub(crate) fn huddle_seed(&self, call: &str, conv: &str) -> HuddleSeed {
        let me = self.user_sender().key;
        let roomy = self.huddle_roomy(call, &me);
        let now = SystemTime::now();
        let recent = lock(&self.feed.calls)
            .get(call)
            .map(|hd| {
                hd.lines
                    .iter()
                    .filter(|b| now.duration_since(b.line.ended).unwrap_or_default() <= SEED_FOR)
                    .map(|b| format!("{}: {}", roomy.name_of(&b.line.speaker), b.line.text))
                    .collect()
            })
            .unwrap_or_default();
        HuddleSeed {
            call_id: call.to_string(),
            conv_id: conv.to_string(),
            name: self.assistant_label(),
            recent,
            ..HuddleSeed::default()
        }
    }

    /// Lines buffered for `call`.
    pub(crate) fn huddle_fed(&self, call: &str) -> usize {
        lock(&self.feed.calls)
            .get(call)
            .map_or(0, |h| h.lines.len())
    }

    /// Forgets `call`'s buffer (the huddle isn't served any more).
    pub(crate) fn huddle_feed_forget(&self, call: &str) {
        lock(&self.feed.calls).remove(call);
    }

    /// Acts on what the brain decided: starts a look on its own thread and
    /// arms the timer for a hold that is waiting.
    fn huddle_step(&self, call: &str, step: Step) {
        if step.cap_note {
            // The canvas note is card 8's to write.
            logf!("huddle {call}: the assistant has done its looks");
        }
        let Some(me) = self.this.get().and_then(std::sync::Weak::upgrade) else {
            return;
        };
        if let Some(look) = step.look {
            let (c, s) = (call.to_string(), me.clone());
            let spawned = std::thread::Builder::new()
                .name("irori-huddle-look".into())
                .spawn(move || s.huddle_run(&c, look));
            if let Err(e) = spawned {
                logf!("huddle {call}: look thread: {e}");
                let step = {
                    let mut g = lock(&self.feed.calls);
                    g.get_mut(call).map(|h| h.brain.done(SystemTime::now()))
                };
                drop(step);
            }
        }
        me.huddle_arm(call);
    }

    /// Wakes the brain when its hold ends (§6: after a match, wait for the
    /// speaker's next line or 1.5 s).
    fn huddle_arm(self: &Arc<Self>, call: &str) {
        let wake = {
            let mut g = lock(&self.feed.calls);
            let Some(h) = g.get_mut(call) else { return };
            if h.timer {
                return;
            }
            let Some(t) = h.brain.wake_at() else { return };
            h.timer = true;
            t
        };
        let (w, call) = (Arc::downgrade(self), call.to_string());
        let key = call.clone();
        let spawned = std::thread::Builder::new()
            .name("irori-huddle-hold".into())
            .spawn(move || {
                let mut wake = wake;
                loop {
                    let wait = wake
                        .duration_since(SystemTime::now())
                        .unwrap_or_default()
                        .max(Duration::from_millis(20));
                    std::thread::sleep(wait);
                    let Some(s) = w.upgrade() else { return };
                    let (step, again) = {
                        let mut g = lock(&s.feed.calls);
                        let Some(h) = g.get_mut(&call) else { return };
                        let step = h.brain.tick(SystemTime::now());
                        let again = h.brain.wake_at();
                        if again.is_none() {
                            h.timer = false;
                        }
                        (step, again)
                    };
                    if let Some(look) = step.look {
                        let (c, s2) = (call.clone(), s.clone());
                        let _ = std::thread::Builder::new()
                            .name("irori-huddle-look".into())
                            .spawn(move || s2.huddle_run(&c, look));
                    }
                    match again {
                        Some(t) => wake = t,
                        None => return,
                    }
                }
            });
        if spawned.is_err()
            && let Some(h) = lock(&self.feed.calls).get_mut(&key)
        {
            h.timer = false;
        }
    }

    /// Runs `look`, then whatever queued behind it, on this thread.
    fn huddle_run(&self, call: &str, first: Look) {
        let mut next = Some(first);
        while let Some(look) = next.take() {
            // The pick runs again before each look: a host back from sleep
            // may find another took over.
            if self.huddle_hosting(call) {
                match self.huddle_build(call, &look) {
                    Some((seed, message)) => match self.huddle_look(&seed, &message) {
                        Ok(r) => logf!(
                            "huddle {call}: {:?} look answered ({} ms)",
                            look.layer,
                            r.ms
                        ),
                        Err(e) => logf!("huddle {call}: {:?} look failed: {e}", look.layer),
                    },
                    None => logf!("huddle {call}: no longer fed: look dropped"),
                }
            }
            next = lock(&self.feed.calls)
                .get_mut(call)
                .and_then(|h| h.brain.done(SystemTime::now()).look);
        }
    }

    /// The seed and the message of `look`: the lines since the last look,
    /// each marked with its speaker, fenced as untrusted, and what woke it.
    fn huddle_build(&self, call: &str, look: &Look) -> Option<(HuddleSeed, String)> {
        let conv = lock(&self.feed.calls).get(call)?.conv.clone();
        let mut seed = self.huddle_seed(call, &conv);
        let me = self.user_sender().key;
        let roomy = self.huddle_roomy(call, &me);
        let mut g = lock(&self.feed.calls);
        let h = g.get_mut(call)?;
        let mut lines: Vec<&Buffered> = h.lines.iter().filter(|b| b.n >= h.sent).collect();
        if lines.is_empty() {
            let skip = h.lines.len().saturating_sub(CONTEXT_LINES);
            lines = h.lines.iter().skip(skip).collect();
        }
        let shown: Vec<String> = lines
            .iter()
            .map(|b| format!("{}: {}", roomy.name_of(&b.line.speaker), b.line.text))
            .collect();
        h.sent = h.next;
        if !h.name.is_empty() {
            seed.name = h.name.clone();
        }
        let mut message = format!("Lines since your last look:\n{}", fence_lines(&shown));
        let caller = look.caller.as_deref().map(|c| roomy.name_of(c));
        match (look.layer, caller) {
            (Layer::Named, Some(who)) => message.push_str(&format!(
                "\n\n{who} called you in these lines. Answer from the packet and the call if they support one; otherwise write nothing."
            )),
            (Layer::Own, Some(who)) => message.push_str(&format!(
                "\n\n{who}, whose assistant you are, asked a question in these lines. Answer from the packet only if it supports one; otherwise write nothing."
            )),
            _ => {}
        }
        Some((seed, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "user:dev1";
    const BOB: &str = "person:bob";

    fn people() -> Vec<Person> {
        vec![
            Person {
                key: ME.into(),
                name: "Alice".into(),
            },
            Person {
                key: BOB.into(),
                name: "Bob".into(),
            },
        ]
    }

    fn assistants() -> Vec<Assistant> {
        vec![Assistant {
            person: ME.into(),
            chosen: Some("Juno".into()),
        }]
    }

    fn ms(now: SystemTime) -> i64 {
        now.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
    }

    fn heard(conv: &str, seg: i64, by: &str, text: &str, ended: SystemTime, agents: bool) -> Heard {
        Heard {
            call_id: "c1".into(),
            conv_id: conv.into(),
            device: "dev1".into(),
            seg,
            start_ms: ms(ended) - 1500,
            end_ms: ms(ended),
            text: text.into(),
            agents,
            by: by.into(),
        }
    }

    /// Feeds `lines` and returns the looks, waiting out the hold after each.
    fn feed(hd: &mut Huddle, now: SystemTime, h: &Heard) -> Vec<Look> {
        let (p, a) = (people(), assistants());
        let room = Room {
            people: &p,
            assistants: &a,
        };
        let mut out = Vec::new();
        if let Some(step) = hd.take(h, ME, now, &room) {
            out.extend(step.look);
        }
        // The hold's end.
        out.extend(hd.brain.tick(now + Duration::from_secs(2)).look);
        out
    }

    #[test]
    fn a_line_calling_the_assistant_gets_a_look_and_a_mid_line_mention_does_not() {
        let now = SystemTime::now();
        let mut hd = Huddle::new("ch_general", ME);
        let calls = feed(
            &mut hd,
            now,
            &heard("ch_general", 0, ME, "Juno, who is on signing", now, true),
        );
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].layer, Layer::Named);

        let mut hd = Huddle::new("ch_general", ME);
        let none = feed(
            &mut hd,
            now,
            &heard(
                "ch_general",
                0,
                ME,
                "I told Juno about the signing",
                now,
                true,
            ),
        );
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn another_persons_question_wakes_nothing() {
        let now = SystemTime::now();
        let mut hd = Huddle::new("ch_general", ME);
        for (i, t) in [
            "do we have anyone working on signing?",
            "can someone take the release notes",
        ]
        .into_iter()
        .enumerate()
        {
            let l = feed(
                &mut hd,
                now,
                &heard("ch_general", i as i64, BOB, t, now, true),
            );
            assert!(l.is_empty(), "{t}: {l:?}");
        }
        // The person's own question wakes their own assistant (L2).
        let l = feed(
            &mut hd,
            now,
            &heard(
                "ch_general",
                9,
                ME,
                "do we have anyone working on signing",
                now,
                true,
            ),
        );
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].layer, Layer::Own);
    }

    #[test]
    fn old_rows_after_a_reconnect_run_no_look_but_are_context() {
        let now = SystemTime::now();
        let mut hd = Huddle::new("ch_general", ME);
        for i in 0..6 {
            let old = now - Duration::from_secs(300 + i);
            let l = feed(
                &mut hd,
                now,
                &heard(
                    "ch_general",
                    i as i64,
                    ME,
                    "Juno, who is on signing",
                    old,
                    true,
                ),
            );
            assert!(l.is_empty(), "{l:?}");
        }
        assert_eq!(hd.lines.len(), 6, "kept as context");
    }

    #[test]
    fn a_line_seen_again_feeds_once() {
        let now = SystemTime::now();
        let mut hd = Huddle::new("ch_general", ME);
        let h = heard("ch_general", 0, ME, "Juno, who is on signing", now, true);
        assert_eq!(feed(&mut hd, now, &h).len(), 1);
        // The follow delivers the same row: nothing more, and one line.
        assert!(feed(&mut hd, now, &h).is_empty());
        assert_eq!(hd.lines.len(), 1);
    }

    #[test]
    fn an_unmarked_dm_line_reaches_no_look_and_no_buffer() {
        let now = SystemTime::now();
        let mut hd = Huddle::new("dm:0123456789abcdef", ME);
        let l = feed(
            &mut hd,
            now,
            &heard(
                "dm:0123456789abcdef",
                0,
                ME,
                "Juno, who is on signing",
                now,
                false,
            ),
        );
        assert!(l.is_empty(), "{l:?}");
        assert!(hd.lines.is_empty() && hd.seen.is_empty());
        // The same words, marked, are heard.
        let l = feed(
            &mut hd,
            now,
            &heard(
                "dm:0123456789abcdef",
                1,
                ME,
                "Juno, who is on signing",
                now,
                true,
            ),
        );
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn the_buffer_drops_talk_past_ten_minutes() {
        let now = SystemTime::now();
        let mut hd = Huddle::new("ch_general", ME);
        feed(
            &mut hd,
            now,
            &heard(
                "ch_general",
                0,
                BOB,
                "morning",
                now - Duration::from_secs(700),
                true,
            ),
        );
        feed(
            &mut hd,
            now,
            &heard("ch_general", 1, BOB, "all set", now, true),
        );
        assert_eq!(hd.lines.len(), 1);
        assert_eq!(hd.lines[0].line.text, "all set");
    }
}
