//! The huddle's pure parts (main:frontend/src/lib/call.ts and share.ts):
//! the words the panel, the bar, the notices and the share stage say, the
//! link health, the timers and the transcript search. Tested without a
//! window, as the TS was.

use irori_window::call::{CallMineView, CallPeerView, CallShareView, CallView, ShareSourceView};
use irori_window::talk::{TalkTranscript, TalkTranscriptLine};

// ---- names (lib/names.ts) --------------------------------------------------

pub const UNKNOWN_PERSON: &str = "another person";

/// plainLine: control and invisible characters become spaces, runs of
/// whitespace one space, trimmed.
pub fn plain_line(s: &str) -> String {
    let mapped: String = s.chars().map(|c| if hidden(c) { ' ' } else { c }).collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xFEFF | 0x00AD)
}

/// isKeyLike: `s` is (or holds) an actor key rather than a name
/// (`board:<b>/<card>`, `agent:<id>`, `person:<id>`, `<hex>/<session>`).
pub fn is_key_like(s: &str) -> bool {
    let b = s.as_bytes();
    let alnum = |c: u8| c.is_ascii_alphanumeric();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    for i in 0..b.len() {
        // \b: a word starts here.
        if i > 0 && word(b[i - 1]) {
            continue;
        }
        let rest = &s[i..];
        let run = |from: usize, ok: &dyn Fn(u8) -> bool| -> usize {
            b[from..].iter().take_while(|c| ok(**c)).count()
        };
        if rest.starts_with("board:") {
            let n = run(i + 6, &alnum);
            let at = i + 6 + n;
            if n > 0
                && b.get(at) == Some(&b'/')
                && b.get(at + 1).is_some_and(|c| !c.is_ascii_whitespace())
            {
                return true;
            }
        }
        for p in ["agent:", "person:"] {
            if rest.starts_with(p) && run(i + p.len(), &alnum) >= 6 {
                return true;
            }
        }
        let hex = run(i, &|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
        if hex >= 16
            && b.get(i + hex) == Some(&b'/')
            && run(i + hex + 1, &|c| {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || c == b'-'
            }) >= 8
        {
            return true;
        }
    }
    false
}

/// nameOr: `name` for a person to read, or `fallback` when it is empty or
/// a key.
pub fn name_or(name: &str, fallback: &str) -> String {
    let n = plain_line(name);
    if n.is_empty() || is_key_like(&n) {
        fallback.to_string()
    } else {
        n
    }
}

// ---- huddle words (lib/call.ts) ---------------------------------------------

/// personWord: the Core's "you" is "You", a key or nothing "another
/// person" (card #478).
pub fn person_word(name: &str) -> String {
    if name == "you" {
        "You".into()
    } else {
        name_or(name, UNKNOWN_PERSON)
    }
}

/// namesText: "You, Spencer and Aiko".
pub fn names_text(names: &[String]) -> String {
    let n: Vec<String> = names.iter().map(|x| person_word(x)).collect();
    match n.len() {
        0 => String::new(),
        1 => n[0].clone(),
        k => format!("{} and {}", n[..k - 1].join(", "), n[k - 1]),
    }
}

/// convLabel: "#suzuri", or the DM's person.
pub fn conv_label(kind: &str, name: &str) -> String {
    if kind == "channel" {
        format!("#{}", name_or(name, "channel"))
    } else {
        person_word(name)
    }
}

/// transcriptNotice: what the window says before the mic opens.
pub fn transcript_notice(
    kind: &str,
    name: &str,
    transcribe_on: bool,
    can_transcribe: bool,
) -> String {
    if kind == "personal" {
        if !can_transcribe {
            return "Your voice isn't transcribed on this device.".into();
        }
        if !transcribe_on {
            return "Your voice isn't transcribed (Settings > App).".into();
        }
        return format!(
            "This huddle is transcribed. What you say is written down in {} for you and your agents.",
            conv_label("channel", name)
        );
    }
    let others = if kind == "channel" {
        format!(
            "Others in the huddle may still be written down in {}.",
            conv_label(kind, name)
        )
    } else {
        "The other person may still be written down.".to_string()
    };
    if !can_transcribe {
        return format!("Your voice isn't transcribed on this device. {others}");
    }
    if !transcribe_on {
        return format!("Your voice isn't transcribed (Settings > App). {others}");
    }
    if kind == "channel" {
        format!(
            "This huddle is transcribed. What you say is written down in {} for its members and their agents.",
            conv_label(kind, name)
        )
    } else {
        "This huddle is transcribed. What you say is written down for the two of you.".into()
    }
}

/// transcriptOn: anyone in the huddle is written down (the badge).
pub fn transcript_on(m: &CallMineView) -> bool {
    m.transcribing
        || m.peers
            .iter()
            .any(|p| p.phase != "gone" && p.transcribing == Some(true))
}

/// whoCanRead: who reads a huddle's transcript.
pub fn who_can_read(kind: &str, name: &str) -> String {
    if kind == "channel" {
        format!(
            "Who can read it: the people and agents of {}.",
            conv_label(kind, name)
        )
    } else {
        "Who can read it: the two of you.".into()
    }
}

/// findLines: the lines with every word of `q` in what was said or who
/// said it, any case.
pub fn find_lines<'a>(lines: &'a [TalkTranscriptLine], q: &str) -> Vec<&'a TalkTranscriptLine> {
    let words: Vec<String> = q
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    lines
        .iter()
        .filter(|l| {
            if words.is_empty() {
                return true;
            }
            let hay = format!("{} {}", person_word(&l.name), l.text).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Good,
    Fair,
    Poor,
    Off,
}

fn finite(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

/// healthOf: off until it's live, poor past 5% lost or 300 ms, fair past
/// 1% or 150 ms.
pub fn health_of(p: &CallPeerView) -> Health {
    if p.phase != "live" {
        return Health::Off;
    }
    let (rtt, loss) = (finite(p.rtt_ms), finite(p.loss_pct));
    if loss >= 5.0 || rtt >= 300.0 {
        Health::Poor
    } else if loss >= 1.0 || rtt >= 150.0 {
        Health::Fair
    } else {
        Health::Good
    }
}

/// healthWord: "direct · 26 ms", "relay · 120 ms · 3% lost", "connecting…".
pub fn health_word(p: &CallPeerView) -> String {
    match p.phase.as_str() {
        "connecting" => return "connecting…".into(),
        "gone" => return "left".into(),
        _ => {}
    }
    let (rtt, loss) = (finite(p.rtt_ms), finite(p.loss_pct));
    let mut parts = vec![if p.path.is_empty() {
        "connected".to_string()
    } else {
        p.path.clone()
    }];
    if rtt > 0.0 {
        parts.push(format!("{} ms", rtt.round() as i64));
    }
    if loss >= 1.0 {
        parts.push(format!("{}% lost", loss.round() as i64));
    }
    parts.join(" · ")
}

/// durationText: a call timer, "4:12", "1:02:09".
pub fn duration_text(since_ms: i64, now_ms: i64) -> String {
    let s = ((now_ms - since_ms) / 1000).max(0);
    let (h, m, ss) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{ss:02}")
    } else {
        format!("{m}:{ss:02}")
    }
}

/// lengthText: "under a minute", "18 min", "1 h 5 min".
pub fn length_text(ms: i64) -> String {
    let min = ms / 60_000;
    if min < 1 {
        return "under a minute".into();
    }
    if min < 60 {
        return format!("{min} min");
    }
    let h = min / 60;
    if min % 60 != 0 {
        format!("{h} h {} min", min % 60)
    } else {
        format!("{h} h")
    }
}

/// inCallHere: this device is in `conv`'s huddle.
pub fn in_call_here(s: &CallView, conv: &str) -> bool {
    s.mine.as_ref().is_some_and(|m| m.conv_id == conv)
}

/// huddleNames: who is in `conv`'s huddle, each once, "You" first.
pub fn huddle_names(s: &CallView, conv: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut add = |n: String| {
        if !names.contains(&n) {
            names.push(n);
        }
    };
    if in_call_here(s, conv) {
        add("You".into());
    }
    if let Some(l) = s.live.iter().find(|l| l.conv_id == conv) {
        for p in &l.people {
            add(person_word(p));
        }
    }
    if let Some(i) = names.iter().position(|n| n == "You") {
        let you = names.remove(i);
        names.insert(0, you);
    }
    names
}

/// clockText: a time of day in this machine's zone, as React's
/// toLocaleTimeString with 2-digit hour and minute said it ("03:02 PM"),
/// the same as a post's time in Talk.
pub fn clock_text(ms: i64) -> String {
    crate::talk::logic::local(ms).1
}

/// The local day number of `ms` (days since the epoch, in this zone).
fn local_day(ms: i64) -> i64 {
    let secs = ms.div_euclid(1000);
    (secs + sys::tz::local_offset(secs) as i64).div_euclid(86_400)
}

/// dayLabel: "Today", "Yesterday", else "Monday, Nov 3".
pub fn day_label(at_ms: i64, now_ms: i64) -> String {
    let (a, n) = (local_day(at_ms), local_day(now_ms));
    match n - a {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            // 1970-01-01 was a Thursday.
            const DAYS: [&str; 7] = [
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
            ];
            const MONTHS: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let (_, m, d) = civil(a);
            format!(
                "{}, {} {d}",
                DAYS[a.rem_euclid(7) as usize],
                MONTHS[(m - 1) as usize]
            )
        }
    }
}

/// The calendar date of a day number (Howard Hinnant's civil_from_days).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// whenText: the transcript's start and length, "Today 10:02 · 18 min".
pub fn when_text(t: &TalkTranscript, now_ms: i64) -> String {
    let mut parts = Vec::new();
    if let Some(at) = t.started_at_ms {
        parts.push(format!("{} {}", day_label(at, now_ms), clock_text(at)));
    }
    parts.push(if t.ended {
        length_text(t.duration_ms)
    } else {
        "still on".into()
    });
    parts.join(" · ")
}

// ---- the start sheet (huddle.tsx) -------------------------------------------

/// Where a huddle would start, as the start sheet tells it.
#[derive(Clone, Debug, PartialEq)]
pub struct StartWhere {
    /// "#suzuri", or the DM's other person.
    pub label: String,
    /// `channel` (in a space), `personal` (a Personal channel) or `dm`.
    pub kind: String,
}

/// toldText: who a huddle's start reaches.
pub fn told_text(w: &StartWhere) -> String {
    match w.kind.as_str() {
        "personal" => "Your other devices get a notice and can join. It's just you here.".into(),
        "dm" => format!(
            "{} gets a notice on each of their devices, and so do your other devices. Either of you can join from any of them.",
            w.label
        ),
        _ => format!(
            "Everyone in {}'s space gets a notice on each of their devices, and so do your other devices. They can join from any of them; agents aren't told.",
            w.label
        ),
    }
}

/// startNotice: the notice for the start sheet, "Checking…" until this
/// device says whether it can write down.
pub fn start_notice(w: &StartWhere, transcribe_on: bool, can: Option<bool>) -> String {
    let Some(can) = can else {
        return "Checking…".into();
    };
    let name = if w.kind == "dm" {
        w.label.clone()
    } else {
        w.label.strip_prefix('#').unwrap_or(&w.label).to_string()
    };
    transcript_notice(&w.kind, &name, transcribe_on, can)
}

/// hueOf's index: a name's colour, the same one every time (six hues).
pub fn hue_index(name: &str) -> usize {
    let mut h: i32 = 0;
    for c in name.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    (h.unsigned_abs() % 6) as usize
}

/// initials: a name's first letter, or its first two words' ("AP").
pub fn initials(name: &str) -> String {
    let w: Vec<&str> = name.split_whitespace().collect();
    let s: String = if w.len() > 1 {
        w[0].chars().take(1).chain(w[1].chars().take(1)).collect()
    } else {
        name.trim().chars().take(1).collect()
    };
    s.to_uppercase()
}

// ---- screen share words (lib/share.ts) --------------------------------------

/// qualityText: "1080p · 10 fps · direct".
pub fn quality_text(s: &CallShareView) -> String {
    let mut parts = vec![format!("{}p", s.height), format!("{} fps", s.fps)];
    if !s.path.is_empty() {
        parts.push(s.path.clone());
    }
    parts.join(" · ")
}

/// whatText: "is sharing a window: Windows Terminal".
pub fn what_text(s: &CallShareView) -> String {
    match (s.mine, s.kind == "display") {
        (true, true) => format!("sharing your screen: {}", s.title),
        (true, false) => format!("sharing a window: {}", s.title),
        (false, true) => format!("is sharing their screen: {}", s.title),
        (false, false) => format!("is sharing a window: {}", s.title),
    }
}

/// sourceLine: a picker row's second line, "Terminal · 1280 × 800".
pub fn source_line(s: &ShareSourceView) -> String {
    let size = format!("{} × {}", s.width, s.height);
    if s.kind == "window" && !s.app.is_empty() {
        format!("{} · {size}", s.app)
    } else {
        size
    }
}

/// sortSources: displays first, then windows, each in the order they came.
pub fn sort_sources(list: Vec<ShareSourceView>) -> Vec<ShareSourceView> {
    let (mut d, w): (Vec<_>, Vec<_>) = list.into_iter().partition(|s| s.kind == "display");
    d.extend(w);
    d
}

/// sharesFrom: the share comes from peer `p` (its person, and the device
/// when the share names one).
pub fn shares_from(s: &CallShareView, p: &CallPeerView) -> bool {
    if s.mine || s.name != p.name {
        return false;
    }
    s.device.is_empty() || s.device == p.device
}

/// One window message of a share (irori_net::share's view_config and
/// view_chunk).
#[derive(Clone, Debug, PartialEq)]
pub enum ShareFrame<'a> {
    Config {
        codec: &'a str,
        description: &'a [u8],
    },
    Chunk {
        key: bool,
        timestamp_us: u64,
        data: &'a [u8],
    },
}

/// parseShareFrame:
///   config: [1] [codecLen u16 BE] [codec UTF-8] [avcC description...]
///   chunk:  [2] [flags u8, bit0 key] [timestamp µs u64 BE] [AVCC access unit...]
/// None for anything else.
pub fn parse_share_frame(b: &[u8]) -> Option<ShareFrame<'_>> {
    match *b.first()? {
        1 => {
            if b.len() < 3 {
                return None;
            }
            let n = u16::from_be_bytes([b[1], b[2]]) as usize;
            if b.len() < 3 + n {
                return None;
            }
            let codec = std::str::from_utf8(&b[3..3 + n]).ok()?;
            if codec.is_empty() {
                return None;
            }
            Some(ShareFrame::Config {
                codec,
                description: &b[3 + n..],
            })
        }
        2 => {
            if b.len() < 10 {
                return None;
            }
            let mut ts = [0u8; 8];
            ts.copy_from_slice(&b[2..10]);
            Some(ShareFrame::Chunk {
                key: b[1] & 1 == 1,
                timestamp_us: u64::from_be_bytes(ts),
                data: &b[10..],
            })
        }
        _ => None,
    }
}

// ---- tests (lib/call.test.ts, lib/share.test.ts) ------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::call::CallLiveView;

    fn link() -> CallPeerView {
        CallPeerView {
            phase: "live".into(),
            path: "direct".into(),
            rtt_ms: 26.0,
            loss_pct: 0.0,
            ..Default::default()
        }
    }

    fn mine_in(conv: &str) -> CallMineView {
        CallMineView {
            call_id: "c1".into(),
            conv_id: conv.into(),
            conv_name: "a".into(),
            conv_kind: "channel".into(),
            can_transcribe: true,
            transcribe_on: true,
            ..Default::default()
        }
    }

    #[test]
    fn health_of_grades_the_link_off_until_live() {
        assert_eq!(health_of(&link()), Health::Good);
        assert_eq!(
            health_of(&CallPeerView {
                rtt_ms: 180.0,
                ..link()
            }),
            Health::Fair
        );
        assert_eq!(
            health_of(&CallPeerView {
                loss_pct: 2.0,
                ..link()
            }),
            Health::Fair
        );
        assert_eq!(
            health_of(&CallPeerView {
                rtt_ms: 320.0,
                ..link()
            }),
            Health::Poor
        );
        assert_eq!(
            health_of(&CallPeerView {
                loss_pct: 6.0,
                ..link()
            }),
            Health::Poor
        );
        assert_eq!(
            health_of(&CallPeerView {
                phase: "connecting".into(),
                ..link()
            }),
            Health::Off
        );
        assert_eq!(
            health_of(&CallPeerView {
                phase: "gone".into(),
                ..link()
            }),
            Health::Off
        );
    }

    #[test]
    fn health_word_says_path_time_and_loss() {
        assert_eq!(health_word(&link()), "direct · 26 ms");
        let p = CallPeerView {
            path: "relay".into(),
            rtt_ms: 119.6,
            loss_pct: 3.2,
            ..link()
        };
        assert_eq!(health_word(&p), "relay · 120 ms · 3% lost");
        assert_eq!(
            health_word(&CallPeerView {
                loss_pct: 0.4,
                ..link()
            }),
            "direct · 26 ms"
        );
        assert_eq!(
            health_word(&CallPeerView {
                path: "".into(),
                rtt_ms: 0.0,
                ..link()
            }),
            "connected"
        );
        assert_eq!(
            health_word(&CallPeerView {
                phase: "connecting".into(),
                ..link()
            }),
            "connecting…"
        );
        assert_eq!(
            health_word(&CallPeerView {
                phase: "gone".into(),
                ..link()
            }),
            "left"
        );
    }

    #[test]
    fn nan_health_numbers_read_as_zero() {
        let p = CallPeerView {
            rtt_ms: f64::NAN,
            loss_pct: f64::NAN,
            ..link()
        };
        assert_eq!(health_of(&p), Health::Good);
        assert_eq!(health_word(&p), "direct");
    }

    #[test]
    fn duration_text_is_a_call_timer() {
        assert_eq!(duration_text(0, 0), "0:00");
        assert_eq!(duration_text(0, 252_000), "4:12");
        assert_eq!(duration_text(0, 3_729_000), "1:02:09");
        assert_eq!(duration_text(5_000, 0), "0:00");
    }

    #[test]
    fn length_text_says_how_long_it_ran() {
        assert_eq!(length_text(30_000), "under a minute");
        assert_eq!(length_text(18 * 60_000 + 5_000), "18 min");
        assert_eq!(length_text(65 * 60_000), "1 h 5 min");
        assert_eq!(length_text(120 * 60_000), "2 h");
    }

    #[test]
    fn names_text_joins_as_a_sentence() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(names_text(&v(&[])), "");
        assert_eq!(names_text(&v(&["you"])), "You");
        assert_eq!(names_text(&v(&["you", "Spencer"])), "You and Spencer");
        assert_eq!(
            names_text(&v(&["Aiko", "you", "Spencer"])),
            "Aiko, You and Spencer"
        );
    }

    #[test]
    fn huddle_names_each_once_you_first() {
        let s = CallView {
            live: vec![
                CallLiveView {
                    call_id: "c1".into(),
                    conv_id: "ch-a".into(),
                    people: vec!["Spencer".into(), "you".into(), "Aiko".into()],
                },
                CallLiveView {
                    call_id: "c2".into(),
                    conv_id: "ch-b".into(),
                    people: vec!["Spencer".into()],
                },
            ],
            transcribe_on: true,
            ..Default::default()
        };
        assert_eq!(huddle_names(&s, "ch-a"), vec!["You", "Spencer", "Aiko"]);
        assert_eq!(huddle_names(&s, "ch-b"), vec!["Spencer"]);
        assert!(huddle_names(&s, "ch-c").is_empty());
        assert!(huddle_names(&CallView::default(), "ch-a").is_empty());
    }

    #[test]
    fn in_call_here_is_this_devices_conversation() {
        let s = CallView {
            mine: Some(mine_in("ch-a")),
            ..Default::default()
        };
        assert!(in_call_here(&s, "ch-a"));
        assert!(!in_call_here(&s, "ch-b"));
        assert!(!in_call_here(&CallView::default(), "ch-a"));
        assert_eq!(huddle_names(&s, "ch-a"), vec!["You"]);
    }

    #[test]
    fn conv_label_puts_hash_before_a_channel_only() {
        assert_eq!(conv_label("channel", "suzuri"), "#suzuri");
        assert_eq!(conv_label("dm", "Spencer"), "Spencer");
        assert_eq!(conv_label("dm", "person:0123abcdef"), "another person");
        assert_eq!(conv_label("channel", ""), "#channel");
    }

    #[test]
    fn person_word_you_and_keys() {
        assert_eq!(person_word("you"), "You");
        assert_eq!(person_word("Aiko"), "Aiko");
        assert_eq!(person_word(""), "another person");
        assert_eq!(person_word("person:0123abcdef"), "another person");
    }

    #[test]
    fn transcript_notice_says_who_reads_or_why_not() {
        assert_eq!(
            transcript_notice("channel", "suzuri", true, true),
            "This huddle is transcribed. What you say is written down in #suzuri for its members and their agents."
        );
        assert_eq!(
            transcript_notice("dm", "Spencer", true, true),
            "This huddle is transcribed. What you say is written down for the two of you."
        );
        assert_eq!(
            transcript_notice("personal", "us", true, true),
            "This huddle is transcribed. What you say is written down in #us for you and your agents."
        );
        assert_eq!(
            transcript_notice("personal", "us", false, true),
            "Your voice isn't transcribed (Settings > App)."
        );
        assert_eq!(
            transcript_notice("channel", "suzuri", false, true),
            "Your voice isn't transcribed (Settings > App). Others in the huddle may still be written down in #suzuri."
        );
        assert_eq!(
            transcript_notice("dm", "Spencer", false, true),
            "Your voice isn't transcribed (Settings > App). The other person may still be written down."
        );
        for on in [true, false] {
            assert_eq!(
                transcript_notice("channel", "suzuri", on, false),
                "Your voice isn't transcribed on this device. Others in the huddle may still be written down in #suzuri."
            );
            assert_eq!(
                transcript_notice("dm", "Spencer", on, false),
                "Your voice isn't transcribed on this device. The other person may still be written down."
            );
        }
        assert!(
            transcript_notice("channel", "person:0123abcdef", true, true)
                .contains("in #channel for")
        );
        assert!(
            transcript_notice("channel", "person:0123abcdef", false, true)
                .contains("written down in #channel.")
        );
    }

    #[test]
    fn transcript_on_is_anyone_written_down() {
        let peer = CallPeerView {
            name: "Aiko".into(),
            transcribing: None,
            ..link()
        };
        let m = mine_in("ch-a");
        assert!(!transcript_on(&m));
        assert!(transcript_on(&CallMineView {
            transcribing: true,
            ..m.clone()
        }));
        assert!(!transcript_on(&CallMineView {
            peers: vec![peer.clone()],
            ..m.clone()
        }));
        let off = CallPeerView {
            transcribing: Some(false),
            ..peer.clone()
        };
        assert!(!transcript_on(&CallMineView {
            peers: vec![off],
            ..m.clone()
        }));
        let on = CallPeerView {
            transcribing: Some(true),
            ..peer.clone()
        };
        assert!(transcript_on(&CallMineView {
            peers: vec![on.clone()],
            ..m.clone()
        }));
        let gone = CallPeerView {
            phase: "gone".into(),
            ..on
        };
        assert!(!transcript_on(&CallMineView {
            peers: vec![gone],
            ..m
        }));
    }

    #[test]
    fn who_can_read_names_the_channel() {
        assert_eq!(
            who_can_read("channel", "suzuri"),
            "Who can read it: the people and agents of #suzuri."
        );
        assert_eq!(
            who_can_read("dm", "Spencer"),
            "Who can read it: the two of you."
        );
    }

    #[test]
    fn find_lines_keeps_lines_with_every_word() {
        let l = |name: &str, at: i64, text: &str| TalkTranscriptLine {
            name: name.into(),
            mine: name == "you",
            at_ms: at,
            text: text.into(),
        };
        let lines = vec![
            l("you", 1, "Tag at two"),
            l("Spencer", 2, "Rerun the latency TEST"),
            l("Aiko", 3, "I can do the click"),
        ];
        let at = |q: &str| {
            find_lines(&lines, q)
                .iter()
                .map(|x| x.at_ms)
                .collect::<Vec<_>>()
        };
        assert_eq!(at(""), vec![1, 2, 3]);
        assert_eq!(at("  "), vec![1, 2, 3]);
        assert_eq!(at("test"), vec![2]);
        assert_eq!(at("spencer latency"), vec![2]);
        assert_eq!(at("you tag"), vec![1]);
        assert!(at("click two").is_empty());
    }

    #[test]
    fn start_sheet_words() {
        let ch = StartWhere {
            label: "#suzuri".into(),
            kind: "channel".into(),
        };
        assert!(told_text(&ch).starts_with("Everyone in #suzuri's space"));
        assert_eq!(start_notice(&ch, true, None), "Checking…");
        assert!(start_notice(&ch, true, Some(true)).contains("in #suzuri for its members"));
        let dm = StartWhere {
            label: "Spencer".into(),
            kind: "dm".into(),
        };
        assert!(told_text(&dm).starts_with("Spencer gets a notice"));
    }

    #[test]
    fn initials_and_hues() {
        assert_eq!(initials("another person"), "AP");
        assert_eq!(initials("spencer"), "S");
        assert_eq!(initials("You"), "Y");
        assert_eq!(hue_index("Aiko"), hue_index("Aiko"));
        assert!(hue_index("Spencer") < 6);
    }

    #[test]
    fn key_like_names_are_never_shown() {
        assert!(is_key_like("person:0123abcdef"));
        assert!(is_key_like("agent:abcdef12"));
        assert!(is_key_like("board:ab12/card-1"));
        assert!(is_key_like("0123456789abcdef0/0123abcd-ef"));
        assert!(!is_key_like("Spencer"));
        assert!(!is_key_like("person:abc"));
        assert_eq!(plain_line("  a\u{200b}b \n c "), "a b c");
    }

    #[test]
    fn day_and_clock() {
        assert_eq!(day_label(1_000_000_000_000, 1_000_000_000_000), "Today");
        assert_eq!(
            day_label(1_000_000_000_000 - 86_400_000, 1_000_000_000_000),
            "Yesterday"
        );
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        let c = clock_text(0);
        assert_eq!(c.len(), 8, "{c}");
        assert!(c.ends_with(" AM") || c.ends_with(" PM"), "{c}");
    }

    // ---- share.test.ts ----

    fn config(codec: &str, desc: &[u8]) -> Vec<u8> {
        let mut b = vec![1];
        b.extend_from_slice(&(codec.len() as u16).to_be_bytes());
        b.extend_from_slice(codec.as_bytes());
        b.extend_from_slice(desc);
        b
    }

    fn chunk(key: bool, ts: u64, data: &[u8]) -> Vec<u8> {
        let mut b = vec![2, u8::from(key)];
        b.extend_from_slice(&ts.to_be_bytes());
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn parse_share_frame_reads_a_config() {
        let b = config("avc1.640028", &[1, 0x64, 0, 0x28, 0xff]);
        assert_eq!(
            parse_share_frame(&b),
            Some(ShareFrame::Config {
                codec: "avc1.640028",
                description: &[1, 0x64, 0, 0x28, 0xff]
            })
        );
    }

    #[test]
    fn parse_share_frame_reads_a_chunk() {
        let b = chunk(true, 5_000_000_123, &[0, 0, 0, 2, 9, 0x10]);
        assert_eq!(
            parse_share_frame(&b),
            Some(ShareFrame::Chunk {
                key: true,
                timestamp_us: 5_000_000_123,
                data: &[0, 0, 0, 2, 9, 0x10]
            })
        );
        let d = chunk(false, 33_333, &[7]);
        assert!(matches!(
            parse_share_frame(&d),
            Some(ShareFrame::Chunk {
                key: false,
                timestamp_us: 33_333,
                ..
            })
        ));
        let mut b = chunk(false, 1, &[7]);
        b[1] = 0b10;
        assert!(matches!(
            parse_share_frame(&b),
            Some(ShareFrame::Chunk { key: false, .. })
        ));
    }

    #[test]
    fn parse_share_frame_refuses_short_or_unknown() {
        assert_eq!(parse_share_frame(&[]), None);
        assert_eq!(parse_share_frame(&[3, 0, 0]), None);
        assert_eq!(parse_share_frame(&[2, 1, 0, 0]), None);
        assert_eq!(parse_share_frame(&[1, 0, 9, 97]), None);
        assert_eq!(parse_share_frame(&[1, 0, 0]), None);
    }

    fn theirs() -> CallShareView {
        CallShareView {
            share_id: "s1".into(),
            name: "Spencer".into(),
            kind: "window".into(),
            title: "Windows Terminal".into(),
            app: "Windows Terminal".into(),
            width: 1920,
            height: 1080,
            fps: 10,
            kbps: 1800,
            path: "direct".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_stages_words() {
        assert_eq!(quality_text(&theirs()), "1080p · 10 fps · direct");
        assert_eq!(
            quality_text(&CallShareView {
                path: "".into(),
                ..theirs()
            }),
            "1080p · 10 fps"
        );
        assert_eq!(
            what_text(&theirs()),
            "is sharing a window: Windows Terminal"
        );
        let d = CallShareView {
            kind: "display".into(),
            title: "Display 1".into(),
            ..theirs()
        };
        assert_eq!(what_text(&d), "is sharing their screen: Display 1");
        assert_eq!(
            what_text(&CallShareView {
                mine: true,
                ..theirs()
            }),
            "sharing a window: Windows Terminal"
        );
    }

    #[test]
    fn the_picker_displays_first() {
        let w = |id: &str, kind: &str| ShareSourceView {
            id: id.into(),
            kind: kind.into(),
            title: id.into(),
            app: if kind == "window" {
                "Terminal".into()
            } else {
                String::new()
            },
            width: 1280,
            height: 800,
        };
        let list = vec![w("a", "window"), w("b", "display"), w("c", "window")];
        assert_eq!(source_line(&list[0]), "Terminal · 1280 × 800");
        assert_eq!(source_line(&list[1]), "1280 × 800");
        let ids: Vec<String> = sort_sources(list).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["b", "a", "c"]);
    }
}
