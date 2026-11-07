//! The Spaces pages' pure logic (the React app's lib/boards.ts and
//! lib/spaces.ts, non-sample, plus boards-page.tsx's `boardSegs` and
//! space-boards.tsx's `cardsOnBoard`), with their `*.test.ts` cases.
//! Also the small text helpers lib/irori.ts had (`plainLine`,
//! `relativeTime`, `shortRepo`), which notes/ uses too.

use irori_window::boards::{BoardSummary, SpaceBoard, SummaryMember};
use irori_window::cards::CardSummary;
use irori_window::devices::HomeServerView;
use irori_window::freshness::LogFreshness;

// ---- text -------------------------------------------------------------------

/// lib/irori.ts `hidden`: C0/C1 controls, zero-width marks, bidi
/// embeddings, overrides and isolates, and the BOM.
fn hidden(c: char) -> bool {
    let c = c as u32;
    c < 0x20
        || (0x7f..=0x9f).contains(&c)
        || (0x200b..=0x200f).contains(&c)
        || (0x202a..=0x202e).contains(&c)
        || (0x2066..=0x2069).contains(&c)
        || c == 0xfeff
}

/// Untrusted text made safe to show on one line: hidden characters become
/// spaces and whitespace runs collapse.
pub fn plain_line(s: &str) -> String {
    let out: String = s.chars().map(|c| if hidden(c) { ' ' } else { c }).collect();
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// plain_line for multi-line text: newlines and tabs stay.
pub fn plain_block(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let out: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\t' || !hidden(c) { c } else { ' ' })
        .collect();
    let mut t = out.trim().to_string();
    while t.contains("\n\n\n\n") {
        t = t.replace("\n\n\n\n", "\n\n\n");
    }
    t
}

/// Milliseconds since the epoch, now.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// "just now", "42s ago", "5m ago", "3h ago", "2d ago" (then and now in
/// ms since the epoch).
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    let s = ((now_ms - then_ms) as f64 / 1000.0).round() as i64;
    if s < 5 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", h / 24)
}

/// A repo key in a row: "owner/name" for a hosted remote, "local repo
/// 1a2b3c4" for a root-commit key.
pub fn short_repo(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if let Some(rest) = key.strip_prefix("root:") {
        return format!("local repo {}", rest.chars().take(7).collect::<String>());
    }
    let parts: Vec<&str> = key.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() > 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        key.to_string()
    }
}

/// Seconds since the epoch from an RFC 3339 stamp ("2026-11-07T15:42:00Z",
/// fractions and offsets allowed).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| -> Option<i64> { s.get(a..b)?.parse().ok() };
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = s.get(19..)?;
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let off = match rest {
        "Z" | "z" => 0,
        r if r.len() == 6 => {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            let oh: i64 = r.get(1..3)?.parse().ok()?;
            let om: i64 = r.get(4..6)?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's days_from_civil).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - off)
}

/// A local clock time, "3:42 PM", from an RFC 3339 stamp ("" when it
/// doesn't parse).
pub fn clock_time(rfc3339: &str) -> String {
    let Some(secs) = parse_rfc3339(rfc3339) else { return String::new() };
    let t = secs as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which we own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return String::new();
    }
    let (h, m) = (tm.tm_hour, tm.tm_min);
    let (h12, ap) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {ap}")
}

// ---- boards (lib/boards.ts) ------------------------------------------------

/// MaxBoardName mirrors identity.MaxBoardName.
pub const MAX_BOARD_NAME: usize = 60;
/// MaxBoardTag mirrors identity.MAX_BOARD_TAG (card #590).
pub const MAX_BOARD_TAG: usize = 8;
/// How often a board page reads presence again while it is open.
pub const PRESENCE_RELOAD_SECS: u64 = 20;

/// A preset choice: its value, label and what it means.
pub struct Preset {
    pub value: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
}

/// Receive presets (design §7.2): what other people on a board may do to
/// your agents. Open is the default.
pub const RECEIVE_PRESETS: [Preset; 4] = [
    Preset {
        value: "open",
        label: "Open",
        hint: "They can wake your agents, and their rows reach your bypass sessions.",
    },
    Preset {
        value: "members",
        label: "Members wake",
        hint: "Only the people you pick wake your agents; the rest arrive at the next turn.",
    },
    Preset {
        value: "ask",
        label: "Ask me",
        hint: "Wake requests wait for you to release them.",
    },
    Preset {
        value: "closed",
        label: "Closed",
        hint: "Nothing wakes your agents; everything arrives at the next turn.",
    },
];

/// Autonomy presets (design §7.3): what your agents may do on a board
/// without asking. Trusted is the default.
pub const AUTONOMY_PRESETS: [Preset; 3] = [
    Preset {
        value: "ask",
        label: "Ask me",
        hint: "New cards in the space ask you first; your private cards aren't shared.",
    },
    Preset {
        value: "trusted",
        label: "Trusted",
        hint: "They create cards in the space; sharing a private card asks you.",
    },
    Preset {
        value: "autonomous",
        label: "Autonomous",
        hint: "They also share private cards (after scrub) and mirror all their leases.",
    },
];

/// Where your threads on a shared board live (card #185).
pub const THREADS_MODES: [Preset; 2] = [
    Preset {
        value: "shared",
        label: "Shared",
        hint: "Every member and their agents see your threads in this space.",
    },
    Preset {
        value: "private",
        label: "Private",
        hint: "Only you and your own agents see them; they never reach other members' devices.",
    },
];

pub fn hint(presets: &[Preset], value: &str) -> &'static str {
    presets.iter().find(|p| p.value == value).map_or("", |p| p.hint)
}

/// identity.valid_board_tag: a lowercase letter, then lowercase letters,
/// digits or '-', MAX_BOARD_TAG at most.
pub fn valid_tag(tag: &str) -> bool {
    let mut cs = tag.chars();
    match cs.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    tag.len() <= MAX_BOARD_TAG
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A tag from a board's name: its first word, lowercased, with only what a
/// tag may hold.
pub fn suggest_tag(name: &str) -> String {
    let lower = name.to_lowercase();
    let kept: String = lower
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || c.is_whitespace())
        .collect();
    let word = kept.split_whitespace().next().unwrap_or("");
    let t: String = word
        .trim_start_matches(|c: char| !c.is_ascii_lowercase())
        .chars()
        .take(MAX_BOARD_TAG)
        .collect();
    if valid_tag(&t) { t } else { String::new() }
}

/// (The board package's member chips read it: card-grid.tsx.)
#[allow(dead_code)]
/// How a member is on a board (card #371): "online", "seen 5m ago" or
/// "offline · seen 2h ago", then how many of their agents are working.
pub fn presence_line(m: &SummaryMember, now_ms: i64) -> String {
    let at = (!m.last_seen.is_zero()).then(|| m.last_seen.unix_milli_value());
    if m.presence != "online" && m.presence != "recent" {
        return match at {
            Some(t) => format!("offline · seen {}", relative_time(t, now_ms)),
            None => "offline".into(),
        };
    }
    let head = if m.presence == "online" {
        "online".to_string()
    } else {
        match at {
            Some(t) => format!("seen {}", relative_time(t, now_ms)),
            None => "seen recently".into(),
        }
    };
    if m.working > 0 {
        return format!("{head} · {} working", m.working);
    }
    if m.free > 0 { head } else { format!("{head} · no agents") }
}

/// Why an invite link from this device may not reach someone away from
/// your home, or "" when it should (card #107).
pub fn invite_reach_line(h: &HomeServerView) -> String {
    if h.public_relays || !h.can || h.elsewhere {
        return String::new();
    }
    if !h.on {
        return "Your home server is off, so the link only works for someone who can reach this device directly, like on your network. Turn it on in Settings, Devices to invite people from anywhere.".into();
    }
    if !h.running || h.reach == "outside" {
        return String::new();
    }
    let fix = if !h.outside {
        "Turn on Reachable from outside home in Settings, Devices.".to_string()
    } else {
        let l = plain_line(&h.line);
        if l.is_empty() {
            "Irori is still asking your router to open a port; check Settings, Devices.".into()
        } else {
            l
        }
    };
    format!(
        "Your home server can't be reached from outside your home yet, so the link only works for someone on your network. {fix}"
    )
}

/// A failed Invite in plain words: the mesh's own errors mean Irori isn't
/// online to make a link.
pub fn invite_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.starts_with("mesh:")
        || l.contains("transport closed")
        || l.contains("not connected")
        || l.contains("no relay")
    {
        return "Irori couldn't make an invite link: this device isn't connected right now. Check your network (or that Irori is running), then try Invite again.".into();
    }
    e.to_string()
}

/// The tail of an error the backend wrapped ("coord: invalid input: …" reads
/// as it is in React; this keeps it).
pub fn error_text(e: &str) -> String {
    plain_line(e)
}

// ---- spaces (lib/spaces.ts) ------------------------------------------------

/// The two picks that aren't a shared space.
pub const ALL_SPACES: &str = "";
pub const PERSONAL: &str = "personal";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Space {
    /// ALL_SPACES, PERSONAL, or a shared space's id.
    pub id: String,
    pub name: String,
    /// The other people in it, for "with Blake".
    pub others: Vec<String>,
}

/// The switcher's list: All spaces, Personal, then each shared space by
/// name.
pub fn spaces_of(boards: &[BoardSummary]) -> Vec<Space> {
    let mut shared: Vec<&BoardSummary> = boards.iter().filter(|b| !b.leaving).collect();
    shared.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let mut out = vec![
        Space { id: ALL_SPACES.into(), name: "All spaces".into(), others: vec![] },
        Space { id: PERSONAL.into(), name: "Personal".into(), others: vec![] },
    ];
    out.extend(shared.into_iter().map(|b| Space {
        id: b.board_id.clone(),
        name: [&b.name, &b.tag]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "(unnamed)".into()),
        others: b.members.iter().filter(|m| !m.me).map(|m| m.name.clone()).collect(),
    }));
    out
}

/// A space's boards, its default board first (card #558); the first
/// board's empty fields fall back to the space's.
pub fn boards_of(s: &BoardSummary) -> Vec<SpaceBoard> {
    if s.boards.is_empty() {
        return vec![SpaceBoard {
            board_id: s.board_id.clone(),
            name: s.name.clone(),
            tag: s.tag.clone(),
        }];
    }
    s.boards
        .iter()
        .enumerate()
        .map(|(i, x)| {
            if i > 0 {
                return x.clone();
            }
            SpaceBoard {
                board_id: x.board_id.clone(),
                name: if x.name.is_empty() { s.name.clone() } else { x.name.clone() },
                tag: if x.tag.is_empty() { s.tag.clone() } else { x.tag.clone() },
            }
        })
        .collect()
}

/// "with Blake", "with Blake and Casey", "with Blake +2"; "" alone.
pub fn with_line(others: &[String]) -> String {
    match others.len() {
        0 => String::new(),
        1 => format!("with {}", others[0]),
        2 => format!("with {} and {}", others[0], others[1]),
        n => format!("with {} +{}", others[0], n - 1),
    }
}

/// Keeps a pick that still names a space; one the person left (or a
/// stale id) falls back to All spaces.
pub fn known(id: &str, boards: &[BoardSummary]) -> String {
    if id == ALL_SPACES || id == PERSONAL {
        return id.into();
    }
    if boards.iter().any(|b| b.board_id == id && !b.leaving) {
        id.into()
    } else {
        ALL_SPACES.into()
    }
}

/// Personal's cards on one of its boards (card #558): a card with no inner
/// board is on the default board.
pub fn cards_on_personal<'a>(cards: &'a [CardSummary], board_id: &str) -> Vec<&'a CardSummary> {
    cards
        .iter()
        .filter(|c| {
            c.board.is_empty()
                && (if c.space_board.is_empty() { PERSONAL } else { c.space_board.as_str() }) == board_id
        })
        .collect()
}

/// Whether a card shows in the picked space.
pub fn card_in_space(c: &CardSummary, space: &str) -> bool {
    match space {
        ALL_SPACES => true,
        PERSONAL => c.board.is_empty(),
        _ => c.board == space,
    }
}

/// A conversation's space, for [`conv_in_space`].
pub struct ConvSpace<'a> {
    pub space_id: &'a str,
    pub kind: &'a str,
    /// (kind, key) of each member.
    pub members: Vec<(&'a str, &'a str)>,
}

/// Whether a channel, DM or thread shows in the picked space (card #461,
/// #558, #538).
pub fn conv_in_space(c: &ConvSpace, space: &str) -> bool {
    if space == ALL_SPACES {
        return true;
    }
    if c.kind == "dm"
        && !c.members.is_empty()
        && c.members.iter().all(|(k, _)| *k == "person")
        && c.members.iter().any(|(_, key)| key.starts_with("person:"))
    {
        return true;
    }
    if space == PERSONAL {
        return c.space_id.is_empty();
    }
    c.space_id == space
}

/// The Spaces page in the picked space.
pub fn boards_in_space<'a>(boards: &'a [BoardSummary], space: &str) -> Vec<&'a BoardSummary> {
    match space {
        ALL_SPACES => boards.iter().collect(),
        PERSONAL => vec![],
        _ => boards.iter().filter(|b| b.board_id == space).collect(),
    }
}

/// The board id a new thread or channel starts on.
pub fn threads_space(space: &str) -> &str {
    if space == ALL_SPACES || space == PERSONAL { "" } else { space }
}

/// The space's cards on one of its boards: a card with no inner board (or
/// the space's own id) is on the first board.
pub fn cards_on_board<'a>(
    b: &BoardSummary,
    cards: &'a [CardSummary],
    board_id: &str,
) -> Vec<&'a CardSummary> {
    let first = boards_of(b)[0].board_id.clone();
    cards
        .iter()
        .filter(|c| {
            let on = if c.space_board.is_empty() || c.space_board == b.board_id {
                first.as_str()
            } else {
                c.space_board.as_str()
            };
            c.board == b.board_id && on == board_id
        })
        .collect()
}

/// A board's newest few cards.
pub fn recent<'a>(cards: &[&'a CardSummary], n: usize) -> Vec<&'a CardSummary> {
    let mut v = cards.to_vec();
    v.sort_by_key(|c| std::cmp::Reverse(c.updated_at.unix_milli_value()));
    v.truncate(n);
    v
}

// ---- the state bar (boards-page.tsx) ----------------------------------------

/// A card chip's tone (cards/bits.tsx `cardTone`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Destructive,
    Info,
    Slate,
    Success,
}

pub fn card_tone(c: &CardSummary) -> Tone {
    if c.ready && c.open_blockers == 1 {
        return Tone::Success;
    }
    if c.state == "input_required" || c.open_blockers > 0 {
        return Tone::Destructive;
    }
    if c.state == "completed" {
        return Tone::Success;
    }
    if c.state == "canceled" || c.state == "failed" || c.working.is_empty() {
        return Tone::Slate;
    }
    Tone::Info
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    pub key: &'static str,
    pub label: &'static str,
    pub n: usize,
    pub tone: Tone,
}

/// A board's live cards by state, as the cards' own chips colour them,
/// plus the board's done this week.
pub fn board_segs(mine: &[&CardSummary], done_week: usize) -> Vec<Seg> {
    let (mut blocked, mut working, mut open, mut done) = (0, 0, 0, done_week);
    for c in mine {
        match card_tone(c) {
            Tone::Destructive => blocked += 1,
            Tone::Info => working += 1,
            Tone::Success => done += 1,
            Tone::Slate if c.state != "canceled" && c.state != "failed" => open += 1,
            Tone::Slate => {}
        }
    }
    vec![
        Seg { key: "working", label: "working", n: working, tone: Tone::Info },
        Seg { key: "open", label: "open", n: open, tone: Tone::Slate },
        Seg { key: "blocked", label: "blocked", n: blocked, tone: Tone::Destructive },
        Seg { key: "done", label: "done this week", n: done, tone: Tone::Success },
    ]
}

/// The counts under the bar: "2 working · 1 open".
pub fn segs_text(segs: &[Seg]) -> String {
    segs.iter()
        .filter(|s| s.n > 0)
        .map(|s| format!("{} {}", s.n, s.label))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "1 card needs a decision", "3 cards need a decision".
pub fn decide_line(n: usize) -> String {
    if n == 1 {
        "1 card needs a decision".into()
    } else {
        format!("{n} cards need a decision")
    }
}

// ---- freshness (lib/freshness.ts) ------------------------------------------

/// When a line turns amber: a day without catching up.
pub const STALE_AFTER_MS: i64 = 24 * 3600 * 1000;

fn fresh_at(f: &LogFreshness) -> Option<i64> {
    (!f.at.is_zero()).then(|| f.at.unix_milli_value())
}

pub fn stale(f: Option<&LogFreshness>, now_ms: i64) -> bool {
    let Some(f) = f.filter(|f| f.shared) else { return false };
    fresh_at(f).is_none_or(|at| now_ms - at > STALE_AFTER_MS)
}

/// "Last synced with your PC 3h ago", "Synced with your PC just now", or
/// "Not synced with another device since Irori started here".
pub fn fresh_line(f: Option<&LogFreshness>, now_ms: i64) -> String {
    let Some(f) = f.filter(|f| f.shared) else { return String::new() };
    let Some(at) = fresh_at(f) else {
        return "Not synced with another device since Irori started here".into();
    };
    let ago = relative_time(at, now_ms);
    let who = if f.with.is_empty() { String::new() } else { format!(" with {}", f.with) };
    if ago == "just now" {
        format!("Synced{who} just now")
    } else {
        format!("Last synced{who} {ago}")
    }
}

/// The same in a few words: "Synced 3h ago", "Not synced yet".
pub fn short_line(f: Option<&LogFreshness>, now_ms: i64) -> String {
    let Some(f) = f.filter(|f| f.shared) else { return String::new() };
    match fresh_at(f) {
        None => "Not synced yet".into(),
        Some(at) => format!("Synced {}", relative_time(at, now_ms)),
    }
}

/// What an amber line adds.
pub const STALE_TAIL: &str = ": newer changes from other devices may not be here yet";

#[cfg(test)]
mod tests {
    use super::*;
    use gocompat::time::GoTime;
    use irori_window::cards::AgentRef;

    fn member(name: &str, me: bool) -> SummaryMember {
        SummaryMember {
            person_id: name.to_lowercase(),
            name: name.into(),
            me,
            presence: "online".into(),
            ..Default::default()
        }
    }

    fn board(id: &str, name: &str, tag: &str) -> BoardSummary {
        BoardSummary {
            board_id: id.into(),
            name: name.into(),
            tag: tag.into(),
            ..Default::default()
        }
    }

    fn card(id: &str) -> CardSummary {
        CardSummary { id: id.into(), ..Default::default() }
    }

    #[test]
    fn tags_are_a_lowercase_letter_then_letters_digits_or_dash() {
        for t in ["mlg", "a", "site-2", "agility", "abcdefgh"] {
            assert!(valid_tag(t), "{t}");
        }
        for t in ["", "MLG", "1mlg", "-x", "has space", "abcdefghi", "abcdefghijklmnop", "m#1"] {
            assert!(!valid_tag(t), "{t}");
        }
    }

    #[test]
    fn new_board_suggests_a_tag_from_the_first_word() {
        assert_eq!(suggest_tag("MLG"), "mlg");
        assert_eq!(suggest_tag("Website refresh"), "website");
        assert_eq!(suggest_tag("Infrastructure plans"), "infrastr");
        assert_eq!(suggest_tag("2026 plans"), "");
        assert_eq!(suggest_tag(""), "");
    }

    fn ago(now: i64, ms: i64) -> GoTime {
        GoTime::unix_milli(now - ms)
    }

    #[test]
    fn presence_online_with_agents() {
        let now = 1_800_000_000_000;
        let m = |working, free| SummaryMember { working, free, ..member("Blake", false) };
        assert_eq!(presence_line(&m(2, 1), now), "online · 2 working");
        assert_eq!(presence_line(&m(0, 1), now), "online");
        assert_eq!(presence_line(&m(0, 0), now), "online · no agents");
    }

    #[test]
    fn presence_recent_says_when() {
        let now = 1_800_000_000_000;
        let m = SummaryMember {
            presence: "recent".into(),
            last_seen: ago(now, 5 * 60_000),
            working: 1,
            ..member("Blake", false)
        };
        assert_eq!(presence_line(&m, now), "seen 5m ago · 1 working");
    }

    #[test]
    fn presence_offline_leaves_out_stale_agent_counts() {
        let now = 1_800_000_000_000;
        let m = SummaryMember {
            presence: "offline".into(),
            last_seen: ago(now, 2 * 3600_000),
            working: 3,
            free: 1,
            ..member("Blake", false)
        };
        assert_eq!(presence_line(&m, now), "offline · seen 2h ago");
        let m = SummaryMember { presence: "offline".into(), ..member("Blake", false) };
        assert_eq!(presence_line(&m, now), "offline");
        let m = SummaryMember { presence: "".into(), ..member("Blake", false) };
        assert_eq!(presence_line(&m, now), "offline");
    }

    #[test]
    fn invite_open_only_while_in_progress() {
        use irori_window::invites::InviteStatus;
        let st = |s: &str| InviteStatus { state: s.into(), ..Default::default() };
        assert!(!st("").open());
        for s in ["waiting", "words", "joining"] {
            assert!(st(s).open());
        }
        for s in ["done", "failed"] {
            assert!(!st(s).open());
        }
    }

    fn mlg() -> BoardSummary {
        BoardSummary {
            members: vec![member("Stephen", true), member("Blake", false)],
            ..board("b-mlg", "MLG", "mlg")
        }
    }

    #[test]
    fn switcher_lists_all_personal_then_spaces_by_name() {
        let gone = BoardSummary { leaving: true, ..board("b-gone", "Gone", "gone") };
        let s = spaces_of(&[mlg(), board("b-iro", "Irori", "iro"), gone]);
        let ids: Vec<&str> = s.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, [ALL_SPACES, PERSONAL, "b-iro", "b-mlg"]);
        assert_eq!(s[3].others, ["Blake"]);
    }

    #[test]
    fn with_lines() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(with_line(&v(&[])), "");
        assert_eq!(with_line(&v(&["Blake"])), "with Blake");
        assert_eq!(with_line(&v(&["Blake", "Addam"])), "with Blake and Addam");
        assert_eq!(with_line(&v(&["Blake", "Addam", "Casey"])), "with Blake +2");
    }

    #[test]
    fn a_pick_of_a_space_you_left_goes_back_to_all() {
        let gone = BoardSummary { leaving: true, ..board("b-gone", "Gone", "gone") };
        assert_eq!(known("b-mlg", &[mlg()]), "b-mlg");
        assert_eq!(known("b-gone", &[mlg(), gone]), ALL_SPACES);
        assert_eq!(known("nope", &[mlg()]), ALL_SPACES);
        assert_eq!(known(PERSONAL, &[]), PERSONAL);
    }

    #[test]
    fn cards_personal_is_the_private_ones_a_space_its_own() {
        let private = card("a");
        let on = CardSummary { board: "b-mlg".into(), ..card("b") };
        assert!(card_in_space(&private, ALL_SPACES) && card_in_space(&on, ALL_SPACES));
        assert!(card_in_space(&private, PERSONAL));
        assert!(!card_in_space(&on, PERSONAL));
        assert!(card_in_space(&on, "b-mlg"));
        assert!(!card_in_space(&private, "b-mlg"));
    }

    #[test]
    fn boards_and_threads_follow_the_pick() {
        let all = [mlg(), board("b-iro", "Irori", "iro")];
        assert_eq!(boards_in_space(&all, ALL_SPACES).len(), 2);
        assert!(boards_in_space(&all, PERSONAL).is_empty());
        assert_eq!(boards_in_space(&all, "b-iro")[0].board_id, "b-iro");
        assert_eq!(threads_space(ALL_SPACES), "");
        assert_eq!(threads_space(PERSONAL), "");
        assert_eq!(threads_space("b-mlg"), "b-mlg");
    }

    #[test]
    fn channels_and_dms_follow_the_pick() {
        let c = |space_id, kind| ConvSpace { space_id, kind, members: vec![] };
        let all = [("c1", c("", "channel")), ("c2", c("b-mlg", "channel")), ("d1", c("", "dm"))];
        let ids = |space: &str| {
            all.iter().filter(|(_, x)| conv_in_space(x, space)).map(|(id, _)| *id).collect::<Vec<_>>()
        };
        assert_eq!(ids(ALL_SPACES), ["c1", "c2", "d1"]);
        assert_eq!(ids(PERSONAL), ["c1", "d1"]);
        assert_eq!(ids("b-mlg"), ["c2"]);
        assert!(ids("b-iro").is_empty());
    }

    #[test]
    fn a_dm_with_another_person_shows_under_every_pick() {
        let bob = ConvSpace {
            space_id: "",
            kind: "dm",
            members: vec![("person", "user"), ("person", "person:bob")],
        };
        for space in [ALL_SPACES, PERSONAL, "b-mlg", "b-iro"] {
            assert!(conv_in_space(&bob, space));
        }
    }

    #[test]
    fn cards_on_personal_boards() {
        let def = CardSummary { space_board: PERSONAL.into(), ..card("d") };
        let old = card("o");
        let home = CardSummary { tag: "home".into(), space_board: "b-home".into(), ..card("h") };
        let shared = CardSummary {
            board: "b-mlg".into(),
            tag: "mlg".into(),
            space_board: "b-mlg".into(),
            ..card("s")
        };
        let cards = [def, old, home, shared];
        let ids = |b: &str| cards_on_personal(&cards, b).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(PERSONAL), ["d", "o"]);
        assert_eq!(ids("b-home"), ["h"]);
    }

    fn wind() -> BoardSummary {
        BoardSummary {
            boards: vec![
                SpaceBoard { board_id: "s".into(), name: "Wind Waker Multiplayer".into(), tag: "wind".into() },
                SpaceBoard { board_id: "n".into(), name: "Netcode".into(), tag: "net".into() },
            ],
            ..board("s", "Wind Waker Multiplayer", "wind")
        }
    }

    fn on(id: &str, space_board: &str, tag: &str) -> CardSummary {
        CardSummary {
            board: "s".into(),
            space_board: space_board.into(),
            tag: tag.into(),
            ..card(id)
        }
    }

    #[test]
    fn cards_on_one_board() {
        let b = wind();
        let cards = [on("a", "s", "wind"), on("b", "n", "net"), on("c", "", "")];
        let ids = |x: &str| cards_on_board(&b, &cards, x).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("n"), ["b"]);
        assert_eq!(ids("s"), ["a", "c"]);
    }

    #[test]
    fn another_spaces_cards_arent_on_it() {
        let x = CardSummary { board: "t".into(), space_board: "n".into(), ..card("x") };
        assert!(cards_on_board(&wind(), &[x], "n").is_empty());
    }

    #[test]
    fn two_boards_with_the_same_tag_keep_their_own_cards() {
        let mut twin = wind();
        twin.boards.push(SpaceBoard { board_id: "m".into(), name: "Netcode (old)".into(), tag: "net".into() });
        let both = [on("b", "n", "net"), on("d", "m", "net")];
        let ids = |x: &str| cards_on_board(&twin, &both, x).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids("n"), ["b"]);
        assert_eq!(ids("m"), ["d"]);
        assert!(ids("s").is_empty());
    }

    #[test]
    fn boards_of_falls_back_to_the_space() {
        let b = BoardSummary {
            boards: vec![SpaceBoard { board_id: "s".into(), name: "".into(), tag: "".into() }],
            ..board("s", "MLG", "mlg")
        };
        assert_eq!(boards_of(&b)[0].name, "MLG");
        assert_eq!(boards_of(&b)[0].tag, "mlg");
        assert_eq!(boards_of(&board("x", "X", "x")).len(), 1);
    }

    fn who() -> AgentRef {
        AgentRef { key: "a".into(), name: "a".into(), live: true, ..Default::default() }
    }

    fn counts(segs: &[Seg]) -> Vec<(&'static str, usize)> {
        segs.iter().map(|s| (s.key, s.n)).collect()
    }

    #[test]
    fn board_segs_count_as_the_chips_colour_them() {
        let cs = [
            CardSummary { state: "working".into(), working: vec![who()], ..card("1") },
            CardSummary { state: "working".into(), working: vec![who()], open_blockers: 1, ..card("2") },
            CardSummary { state: "submitted".into(), ..card("3") },
            CardSummary { state: "submitted".into(), ..card("4") },
        ];
        let refs: Vec<&CardSummary> = cs.iter().collect();
        assert_eq!(
            counts(&board_segs(&refs, 2)),
            [("working", 1), ("open", 2), ("blocked", 1), ("done", 2)]
        );
    }

    #[test]
    fn a_card_ready_for_your_done_counts_as_done() {
        let c = CardSummary {
            state: "working".into(),
            working: vec![who()],
            open_blockers: 1,
            ready: true,
            ..card("1")
        };
        assert_eq!(
            counts(&board_segs(&[&c], 0)),
            [("working", 0), ("open", 0), ("blocked", 0), ("done", 1)]
        );
    }

    #[test]
    fn relative_times() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(now - 2_000, now), "just now");
        assert_eq!(relative_time(now - 42_000, now), "42s ago");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5m ago");
        assert_eq!(relative_time(now - 3 * 3600_000, now), "3h ago");
        assert_eq!(relative_time(now - 50 * 3600_000, now), "2d ago");
    }

    #[test]
    fn rfc3339_parses() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-03T20:00:00.5Z"), Some(1_793_736_000));
        assert_eq!(parse_rfc3339("2026-11-03T21:00:00+01:00"), Some(1_793_736_000));
        assert_eq!(parse_rfc3339("nope"), None);
    }

    #[test]
    fn plain_line_neutralises_hidden_characters() {
        assert_eq!(plain_line(" a\u{202e}b \n c "), "a b c");
        assert_eq!(short_repo("remote:github.com/o/irori"), "o/irori");
        assert_eq!(short_repo("root:1a2b3c4d5e"), "local repo 1a2b3c4");
    }

    #[test]
    fn freshness_lines() {
        let now = 10 * STALE_AFTER_MS;
        let f = LogFreshness {
            log: "personal".into(),
            at: GoTime::unix_milli(now - 3 * 3600_000),
            with: "your PC".into(),
            shared: true,
        };
        assert_eq!(fresh_line(Some(&f), now), "Last synced with your PC 3h ago");
        assert_eq!(short_line(Some(&f), now), "Synced 3h ago");
        assert!(!stale(Some(&f), now));
        let old = LogFreshness { at: GoTime::unix_milli(now - 2 * STALE_AFTER_MS), ..f.clone() };
        assert!(stale(Some(&old), now));
        let never = LogFreshness { at: GoTime::default(), ..f.clone() };
        assert_eq!(fresh_line(Some(&never), now), "Not synced with another device since Irori started here");
        assert_eq!(short_line(Some(&never), now), "Not synced yet");
        let alone = LogFreshness { shared: false, ..f };
        assert_eq!(fresh_line(Some(&alone), now), "");
    }

    #[test]
    fn invite_reach() {
        let h = HomeServerView { public_relays: true, ..Default::default() };
        assert_eq!(invite_reach_line(&h), "");
        let h = HomeServerView { can: true, on: false, ..Default::default() };
        assert!(invite_reach_line(&h).starts_with("Your home server is off"));
        let h = HomeServerView { can: true, on: true, running: true, outside: false, ..Default::default() };
        assert!(invite_reach_line(&h).ends_with("Reachable from outside home in Settings, Devices."));
        assert!(invite_error("mesh: transport closed").starts_with("Irori couldn't make"));
        assert_eq!(invite_error("nope"), "nope");
    }
}
