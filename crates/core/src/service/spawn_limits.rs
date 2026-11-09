//! The spawn queue's limits (docs/design/spawned-agents.md §6, card #450):
//! where each waiting card stands in the queue, and Claude plan usage
//! limits.
//!
//! A usage limit is not a crash. A spawn that fails on one, or a spawned
//! agent that stops on one, puts its card back in the queue as "queued:
//! usage limit", and this device starts nothing until the limit's reset
//! time (or [`USAGE_WAIT`] when it gives none). Claude Code writes the
//! limit into the session's transcript as an assistant row with
//! `"apiError":"usage_limit_reached"` and the reset as
//! `apiErrorParams.rate_limit_info.resetsAt` (Unix seconds); its text is
//! "You've hit your 5-hour limit · resets 4:30pm (America/Denver)". The
//! row is read first, the text only when there is no row (a failed
//! `claude --bg` prints text).
//!
//! The hold is this device's, in memory: after a restart the first spawn
//! hits the limit again and holds again. Its `spawn.host` row carries the
//! hold, so the person's other devices don't ask this one meanwhile. The
//! person can end it early (card #576): Start an agent, or Settings ›
//! Agents' Try now. A limit still in force holds again at the first spawn.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

use coord::{GONE_USAGE_LIMIT, SPAWN_FAILED, Task, USAGE_LIMIT};
use gocompat::time::GoTime;
use serde::Serialize;
use store::kinds;

use super::Services;
use crate::logf;

/// How long Irori holds spawns after a usage limit that names no reset
/// time ("a few minutes", §6).
pub const USAGE_WAIT: Duration = Duration::from_secs(5 * 60);

/// The longest hold a reset time may ask for (a weekly limit is the
/// longest Claude has): anything past it is read as a bad time.
pub const USAGE_MAX: Duration = Duration::from_secs(8 * 24 * 3600);

/// How much of a transcript's end is read for the limit row.
const TAIL: u64 = 64 << 10;

/// The line on a card whose agent stopped on a usage limit; the reset
/// follows.
pub const USAGE_STOPPED: &str = "The agent Irori started for this card stopped on a Claude usage limit, which isn't a crash. The card is queued: Irori starts a fresh agent from Where and Left off once the limit resets";

/// What a usage limit said: when it resets, if it said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageLimit {
    pub resets: Option<GoTime>,
}

impl UsageLimit {
    /// Until when spawns are held: the reset, else [`USAGE_WAIT`] from now;
    /// never in the past, never past [`USAGE_MAX`].
    pub fn hold_until(&self, now: &GoTime) -> GoTime {
        let wait = now.secs + USAGE_WAIT.as_secs() as i64;
        let secs = match self.resets {
            Some(r) if r.secs > now.secs && r.secs - now.secs <= USAGE_MAX.as_secs() as i64 => {
                r.secs
            }
            _ => wait,
        };
        GoTime::unix(secs, 0).with_offset(now.offset)
    }
}

/// Whether `text` is a Claude usage-limit message, and its reset when it
/// gives one. Knows "You've hit your 5-hour limit · resets 4:30pm
/// (America/Denver)", "Usage limit reached · continuing automatically at
/// 4:30pm", "Claude usage limit reached. Your limit will reset at 3pm" and
/// "Claude AI usage limit reached|1791700800". A clock time is read in
/// `now`'s offset (Claude shows the person's own zone), the next one to
/// come.
pub fn limit_in_text(text: &str, now: &GoTime) -> Option<UsageLimit> {
    let low = text.to_lowercase();
    let hit = low.contains("usage limit reached")
        || (low.contains("hit your") && low.contains("limit"))
        || low.contains("limit reached ∙ resets")
        || low.contains("limit reached · resets");
    if !hit {
        return None;
    }
    // The raw form: "...limit reached|<unix seconds>".
    if let Some((_, tail)) = low.split_once("limit reached|") {
        let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(s) = digits.parse::<i64>() {
            return Some(UsageLimit {
                resets: Some(GoTime::unix(s, 0).with_offset(now.offset)),
            });
        }
    }
    let after = ["resets at ", "resets ", "reset at ", "automatically at "]
        .iter()
        .find_map(|k| low.find(k).map(|i| &low[i + k.len()..]));
    Some(UsageLimit {
        resets: after.and_then(|a| clock_time(a, now)),
    })
}

/// The next `h[:mm](am|pm)` at the start of `s`, in `now`'s offset.
fn clock_time(s: &str, now: &GoTime) -> Option<GoTime> {
    let s = s.trim_start();
    let num: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect();
    let rest = s[num.len()..].trim_start();
    let (h, m) = match num.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None => (num.parse::<i64>().ok()?, 0),
    };
    let h = if rest.starts_with("pm") {
        if h == 12 { 12 } else { h + 12 }
    } else if rest.starts_with("am") {
        if h == 12 { 0 } else { h }
    } else {
        h
    };
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    let midnight = local_midnight(now);
    let mut at = midnight.secs + h * 3600 + m * 60;
    // Claude rounds the reset to the minute: one just past (or a clock a
    // little off) is today's, which hold_until reads as USAGE_WAIT. Only
    // a time well past is tomorrow's.
    if at <= now.secs - 3600 {
        at += 86_400;
    }
    Some(GoTime::unix(at, 0).with_offset(now.offset))
}

/// The start of `now`'s day in its own offset.
pub fn local_midnight(now: &GoTime) -> GoTime {
    let (_, _, _, h, mi, s) = now.civil();
    GoTime::unix(
        now.secs - i64::from(h) * 3600 - i64::from(mi) * 60 - i64::from(s),
        0,
    )
    .with_offset(now.offset)
}

/// Whether the newest assistant row in the last part of transcript `path`
/// is a usage-limit error, and its reset. `None` when the session said
/// anything after it, or the file can't be read.
pub fn limit_in_transcript(path: &str, now: &GoTime) -> Option<UsageLimit> {
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let off = size.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = text.lines().collect();
    // The first line of a window that starts mid-file is cut.
    let first = usize::from(off > 0);
    for line in lines.iter().skip(first).rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant")
            || v.get("isSidechain").and_then(|s| s.as_bool()) == Some(true)
        {
            continue;
        }
        if v.get("apiError").and_then(|e| e.as_str()) != Some("usage_limit_reached") {
            return None;
        }
        let resets = [
            "/apiErrorParams/rate_limit_info/resetsAt",
            "/quotaLimits/resetsAt",
        ]
        .iter()
        .find_map(|p| v.pointer(p).and_then(|r| r.as_i64()))
        .map(|s| GoTime::unix(s, 0).with_offset(now.offset));
        return Some(match resets {
            Some(r) => UsageLimit { resets: Some(r) },
            None => {
                let said = v
                    .pointer("/message/content/0/text")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default();
                limit_in_text(said, now).unwrap_or(UsageLimit { resets: None })
            }
        });
    }
    None
}

/// The reason a request that hit a usage limit ends with.
pub(crate) fn usage_reason(until: &GoTime) -> String {
    format!("{USAGE_LIMIT}: held until {}", until.format_rfc3339())
}

/// Where the spawn queue stands on this device (card #450), as the card
/// lines read it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpawnQueue {
    /// The cards waiting for a slot, by id: their place, from 1.
    pub places: HashMap<String, usize>,
    /// Spawns are held on a usage limit until then.
    pub held_until: Option<GoTime>,
    /// Today's starts, as Settings › Agents shows them.
    pub today: i64,
    /// The spawned agents live here and starting, and the cap.
    pub running: i64,
    pub cap: i64,
    /// This person's cards on shared boards that this device would start
    /// but for the hold (card #576), by id.
    pub on_boards: HashSet<String>,
}

impl Services {
    /// Until when spawns are held on a usage limit, if they are now.
    pub(crate) fn usage_held(&self) -> Option<GoTime> {
        let now = (self.now)();
        let h = *self
            .spawn
            .usage_hold
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        h.filter(|h| h.secs > now.secs)
    }

    /// Holds spawns until `l`'s reset (never shortening a longer hold).
    pub(crate) fn hold_usage(&self, l: &UsageLimit) -> GoTime {
        let now = (self.now)();
        let until = l.hold_until(&now);
        let mut h = self
            .spawn
            .usage_hold
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let until = match *h {
            Some(o) if o.secs > until.secs => o,
            _ => until,
        };
        let changed = h.is_none_or(|o| o.secs != until.secs);
        if changed {
            logf(format_args!(
                "spawn: Claude usage limit, holding new spawns until {}",
                until.format_rfc3339()
            ));
            // The person's other devices stop asking this one.
            self.republish_host();
        }
        *h = Some(until);
        drop(h);
        // Every card's queue words read the hold (list-signals.md §3.2).
        if changed {
            self.tell_agents_changed();
        }
        until
    }

    /// The stored hold has passed: clear it and tell the window once. The
    /// spawn sweep calls it; nothing ticks for a hold, and
    /// [`Self::usage_held`] already reads a passed one as none.
    pub(crate) fn expire_usage_hold(&self) {
        let now = (self.now)();
        let ended = {
            let mut h = self
                .spawn
                .usage_hold
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let ended = h.is_some_and(|h| h.secs <= now.secs);
            if ended {
                *h = None;
            }
            ended
        };
        if ended {
            logf(format_args!("spawn: the usage hold ended"));
            self.republish_host();
            self.tell_agents_changed();
        }
    }

    /// Ends the usage hold now, for the person (card #576): they asked for
    /// an agent, or they know the limit changed. If it still stands, the
    /// next spawn fails on it and holds again.
    pub(crate) fn release_usage_hold(&self) {
        let was = self
            .spawn
            .usage_hold
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if was.is_some() {
            logf(format_args!("spawn: the person ended the usage hold"));
            self.republish_host();
            self.asks_kick.kick();
            self.tell_agents_changed();
        }
    }

    /// The agents Irori started today for the private board, every
    /// device's (Settings › Agents counts them).
    pub(crate) fn spawned_today(&self) -> Vec<(String, coord::Spawned)> {
        let now = (self.now)();
        self.coord.spawned_since(&local_midnight(&now))
    }

    /// How many agents Irori started today, every device's: bound marks
    /// only, so a card still queued or starting isn't counted. Shared
    /// boards' agents are in it too, their marks are in the private log.
    pub(crate) fn started_today(&self) -> i64 {
        self.spawned_today().len() as i64
    }

    /// The queue as the sweep would walk it now (card #450): every card
    /// that waits for an agent and would take a slot, past the free slots,
    /// gets its place. A card whose last start failed here (no checkout,
    /// not trusted, the spawner said no) shows that instead.
    pub fn spawn_queue(&self) -> SpawnQueue {
        let me = self.device.id.clone();
        let cap = self.spawn_cap(&me);
        let running = self.spawned_here();
        let mut q = SpawnQueue {
            held_until: self.usage_held(),
            today: self.started_today(),
            running,
            cap,
            ..SpawnQueue::default()
        };
        let waiting: Vec<Task> = self
            .coord
            .cards_to_spawn()
            .into_iter()
            .filter(|t| {
                t.spawn_request().is_none_or(|s| {
                    s.op != SPAWN_FAILED
                        || coord::usage_limited(s)
                        || self.coord.spawn_asked_now(&t.id)
                })
            })
            .collect();
        let held = q.held_until.is_some();
        let mut free = if held {
            0
        } else {
            (cap - running).max(0) as usize
        };
        let mut place = 0;
        for t in waiting {
            // Bound for another device, or waiting for one: it takes no
            // slot here and isn't queued for this one (card #455's routing,
            // as the sweep does).
            if !held && self.spawn_device(&t).as_ref() != Some(&me) {
                continue;
            }
            if free > 0 {
                free -= 1;
                continue;
            }
            place += 1;
            q.places.insert(t.id.clone(), place);
        }
        if held {
            q.on_boards = self.board_cards_held();
        }
        q
    }

    /// The cards on shared boards that the hold stops here: what
    /// [`Self::sweep_board_spawns`] would start, less a card whose last
    /// start failed here (it says "can't start" instead).
    fn board_cards_held(&self) -> HashSet<String> {
        let person = self.person_id();
        let mut out = HashSet::new();
        for b in self.shown_boards() {
            let co = b.coord();
            if !co.spawning() {
                continue;
            }
            for t in co.cards_to_spawn() {
                if b.verifier.key_person(&t.created_by) != person
                    || t.open_ask().is_some_and(|a| !a.thread)
                    || t.spawn_request()
                        .is_some_and(|s| s.op == SPAWN_FAILED && !coord::usage_limited(s))
                {
                    continue;
                }
                out.insert(t.id);
            }
        }
        out
    }

    /// Spawned agents here that stopped on a usage limit (card #450): their
    /// newest words are the limit and the transcript's last row says so.
    /// Each is ended as not a crash, its card queued, and spawns held until
    /// the reset. A session is read only when its last words change.
    pub(crate) fn sweep_usage_limits(&self) {
        let now = (self.now)();
        let me = self.device.id.clone();
        for a in self.presence.snapshot() {
            if a.device_id != me
                || a.status == "busy"
                || !self.coord.is_spawned(&a.key)
                || self.coord.ended_by_irori(&a.key)
            {
                continue;
            }
            {
                let mut seen = self
                    .spawn
                    .limit_seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let said = format!("{}\u{0}{}", a.status, a.last_text);
                if seen.get(&a.key) == Some(&said) {
                    continue;
                }
                seen.insert(a.key.clone(), said);
            }
            let Some(said) = limit_in_text(&a.last_text, &now) else {
                continue;
            };
            let found = if a.transcript.is_empty() {
                Some(said)
            } else {
                limit_in_transcript(&a.transcript, &now)
            };
            let Some(l) = found else { continue };
            let until = self.hold_usage(&l);
            // One waiting on a done check, a review or the person, or on a
            // closed card, ends the usual way; the hold still applies.
            if !self.works_its_card(&a.key) {
                continue;
            }
            if let Err(e) = self.end_usage_limited(&a.key, &until) {
                logf(format_args!("spawn usage limit {}: {e}", a.key));
                self.spawn
                    .limit_seen
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&a.key);
            }
        }
    }

    /// Whether spawned agent `key` is working its card or ask, on the
    /// private board or a shared one: it holds an open card that is working
    /// or waiting for an agent, or has taken an open ask.
    fn works_its_card(&self, key: &str) -> bool {
        let Some((_, t, k)) = self.spawn_card_of(key) else {
            return false;
        };
        if let Some(a) = t.open_ask() {
            return a.taker == k;
        }
        t.holders.contains(&k) && (t.state == coord::WORKING || t.state == coord::SUBMITTED)
    }

    /// Ends spawned agent `key`, stopped on a usage limit held until
    /// `until`: a line on its card, its `agent.gone` as not a crash (no
    /// hand-off is asked: it can't answer), then `claude stop` and `rm`.
    fn end_usage_limited(&self, key: &str, until: &GoTime) -> Result<(), String> {
        let card = self.spawn_card_of(key);
        if let Some((co, t, _)) = &card {
            let line = format!("{USAGE_STOPPED} ({}).", until.format_short());
            co.add_comment(super::spawn::SWEEPER, &t.id, &line, false)
                .map_err(|e| e.to_string())?;
        }
        // On a shared board its agent:<cardId> leaves the card the same
        // way, as end_now's does (card #576).
        if let Some((co, _, k)) = &card
            && k != key
        {
            co.mark_usage_limited(k, (self.now)());
        }
        #[derive(Serialize)]
        struct Gone<'a> {
            key: &'a str,
            reason: &'a str,
        }
        self.store
            .append(
                "irori",
                kinds::AGENT_GONE,
                &Gone {
                    key,
                    reason: GONE_USAGE_LIMIT,
                },
            )
            .map_err(|e| e.to_string())?;
        self.coord.mark_usage_limited(key, (self.now)());
        match self.remove_job(key) {
            Ok(()) => {
                self.spawn
                    .removed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.to_string());
            }
            // Its agent.gone is written: sweep_ending tries stop and rm
            // again, a few times (card #474).
            Err(e) => {
                logf(format_args!("spawn usage limit {key}: {e}"));
                self.spawn
                    .ending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.to_string(), GoTime::unix(0, 0));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc: &str) -> GoTime {
        GoTime::parse_rfc3339(rfc).unwrap()
    }

    #[test]
    fn the_messages_claude_writes_are_limits() {
        // 2026-11-01 14:00 in Denver (-07:00).
        let now = at("2026-11-01T14:00:00-07:00");
        let l = limit_in_text(
            "You've hit your 5-hour limit · resets 4:30pm (America/Denver)",
            &now,
        )
        .unwrap();
        assert_eq!(l.resets.unwrap().secs, at("2026-11-01T16:30:00-07:00").secs);
        let l = limit_in_text(
            "Usage limit reached · continuing automatically at 4:30pm · esc to cancel",
            &now,
        )
        .unwrap();
        assert_eq!(l.resets.unwrap().secs, at("2026-11-01T16:30:00-07:00").secs);
        let l = limit_in_text("Claude AI usage limit reached|1791700800", &now).unwrap();
        assert_eq!(l.resets.unwrap().secs, 1791700800);
        // A time already past today is tomorrow's.
        let l = limit_in_text(
            "Claude usage limit reached. Your limit will reset at 9am (America/Denver).",
            &now,
        )
        .unwrap();
        assert_eq!(l.resets.unwrap().secs, at("2026-11-02T09:00:00-07:00").secs);
        // Rounded to the minute and just past: today's, so a short wait,
        // never tomorrow's.
        let l = limit_in_text("You've hit your 5-hour limit · resets 1:59pm", &now).unwrap();
        assert_eq!(l.resets.unwrap().secs, at("2026-11-01T13:59:00-07:00").secs);
        assert_eq!(
            l.hold_until(&now).secs,
            now.secs + USAGE_WAIT.as_secs() as i64
        );
        let l = limit_in_text("You've hit your weekly limit", &now).unwrap();
        assert_eq!(l.resets, None);
        assert_eq!(
            l.hold_until(&now).secs,
            now.secs + USAGE_WAIT.as_secs() as i64
        );
    }

    #[test]
    fn other_words_are_not_limits() {
        let now = at("2026-11-01T14:00:00-07:00");
        for s in [
            "",
            "Done: the queue order is in, tests pass.",
            "I removed the 5h gauge from your status line",
            "claude: exit status 1",
        ] {
            assert_eq!(limit_in_text(s, &now), None, "{s}");
        }
    }

    #[test]
    fn a_hold_is_never_past_or_too_far() {
        let now = at("2026-11-01T14:00:00Z");
        let past = UsageLimit {
            resets: Some(at("2026-11-01T13:00:00Z")),
        };
        assert_eq!(
            past.hold_until(&now).secs,
            now.secs + USAGE_WAIT.as_secs() as i64
        );
        let far = UsageLimit {
            resets: Some(at("2027-01-01T00:00:00Z")),
        };
        assert_eq!(
            far.hold_until(&now).secs,
            now.secs + USAGE_WAIT.as_secs() as i64
        );
    }

    #[test]
    fn the_transcript_row_gives_the_reset() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        let now = at("2026-11-01T14:00:00-07:00");
        let limit = r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"You've hit your 5-hour limit · resets 4:30pm (America/Denver)"}]},"apiError":"usage_limit_reached","apiErrorParams":{"rate_limit_info":{"status":"rejected","resetsAt":1791700800,"rateLimitType":"five_hour"}},"isApiErrorMessage":true}"#;
        let system = r#"{"type":"system","subtype":"informational","content":"Usage limit reached · continuing automatically at 4:30pm · esc to cancel"}"#;
        let said = r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"Back at it."}]}}"#;
        std::fs::write(&p, format!("{said}\n{limit}\n{system}\n")).unwrap();
        let l = limit_in_transcript(&p.to_string_lossy(), &now).unwrap();
        assert_eq!(l.resets.unwrap().secs, 1791700800);
        // Words after the limit: it's over.
        std::fs::write(&p, format!("{limit}\n{system}\n{said}\n")).unwrap();
        assert_eq!(limit_in_transcript(&p.to_string_lossy(), &now), None);
    }

    #[test]
    fn midnight_is_in_the_times_own_offset() {
        let now = at("2026-11-04T19:09:00-07:00");
        assert_eq!(
            local_midnight(&now).secs,
            at("2026-11-04T00:00:00-07:00").secs
        );
    }
}
