//! Port of I:internal/core/names_test.go (the namer; the attach half of
//! TestUnnamedSessionGetsANameAndARenameWins waits for the API crate).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use coord::names::{fold, pick};
use coord::roster::AgentCard;
use gocompat::time::GoTime;
use irori_core::namer::{NAMES_BUSY, NAMES_KEEP, Namer};

type Cards = Arc<Mutex<Vec<AgentCard>>>;

fn test_namer(self_id: &str, cards: &Cards) -> (tempfile::TempDir, Namer) {
    let dir = tempfile::tempdir().unwrap();
    let n = Namer::new(dir.path(), self_id);
    let c = cards.clone();
    n.set_mesh(move || c.lock().unwrap().clone());
    (dir, n)
}

fn card(dev: &str, sid: &str, name: &str, source: &str, started: GoTime) -> AgentCard {
    AgentCard {
        key: format!("{dev}/{sid}"),
        device_id: dev.into(),
        session_id: sid.into(),
        agent: "claude".into(),
        name: name.into(),
        name_source: source.into(),
        status: "idle".into(),
        started_at: started,
        ..AgentCard::default()
    }
}

fn t0() -> GoTime {
    GoTime::parse_rfc3339("2026-10-28T12:00:00Z").unwrap()
}

fn plus(t: GoTime, d: Duration) -> GoTime {
    capture::gotime::add(&t, d)
}

fn minus(t: GoTime, d: Duration) -> GoTime {
    capture::gotime::sub(&t, d)
}

#[test]
fn test_namer_assigns_once_and_keeps_it_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let n = Namer::new(dir.path(), "pc");
    n.set_mesh(Vec::new);
    let got = n.assign("s1");
    assert_eq!(got, pick("s1", None), "first name: the seed's word");
    assert_eq!(n.assign("s1"), got, "assigned twice");
    assert_eq!(n.assigned("s2"), "", "a session never named has no name");
    let re = Namer::new(dir.path(), "pc");
    assert_eq!(re.assigned("s1"), got, "after a restart");
}

#[test]
fn test_namer_skips_names_live_agents_have() {
    let first = pick("s1", None);
    let cards: Cards = Arc::new(Mutex::new(vec![card(
        "mac",
        "x",
        &first,
        "user",
        GoTime::ZERO,
    )]));
    let (_d, n) = test_namer("pc", &cards);
    let got = n.assign("s1");
    assert_ne!(
        fold(&got),
        fold(&first),
        "picked {got:?}, which a live agent has"
    );
    // A gone agent's name is free.
    cards.lock().unwrap()[0].status = "gone".into();
    assert!(!n.assign("s9").is_empty());
}

#[test]
fn test_namer_skips_names_other_local_sessions_hold() {
    let cards: Cards = Arc::default();
    let (_d, n) = test_namer("pc", &cards);
    n.assign("s1");
    // Give s1 the word s2's id hashes to, so s2 must probe past it.
    n.set_entry("s1", &pick("s2", None), presence::now_local());
    assert_ne!(n.assign("s2"), pick("s2", None), "s2 got the name s1 holds");
}

#[test]
fn test_settle_moves_the_later_of_two_irori_names_on_both_devices() {
    // Two devices named their sessions while apart and both picked heron.
    let cards: Cards = Arc::new(Mutex::new(vec![
        card("pc", "s1", "heron", presence::SOURCE_IRORI, t0()),
        card(
            "mac",
            "s2",
            "heron",
            presence::SOURCE_IRORI,
            plus(t0(), Duration::from_secs(60)),
        ),
    ]));
    let (_a, pc) = test_namer("pc", &cards);
    pc.set_entry("s1", "heron", t0());
    let (_b, mac) = test_namer("mac", &cards);
    mac.set_entry("s2", "heron", t0());

    assert!(
        !pc.settle(),
        "the PC's session started first and keeps heron"
    );
    assert!(mac.settle(), "the Mac's later session moves");
    let got = mac.assigned("s2");
    assert!(
        got != "heron" && !got.is_empty(),
        "the Mac's session is now {got:?}"
    );
    assert_eq!(pc.assigned("s1"), "heron", "the PC's name moved");
    // Claude keeps the title the mod set; it still reads as Irori's.
    assert!(
        mac.gave("s2", "Heron")
            && mac.gave("s2", &mac.assigned("s2"))
            && !mac.gave("s2", "api work"),
        "the Mac's namer forgot which names it gave"
    );
}

#[test]
fn test_names_of_long_idle_sessions_are_free_for_new_ones() {
    let cards: Cards = Arc::default();
    let (_d, n) = test_namer("pc", &cards);
    let now = t0();
    n.set_now(move || now);
    let want = pick("new", None);
    n.set_entry(
        "idle",
        &want,
        minus(now, NAMES_BUSY + Duration::from_secs(60)),
    );
    assert_eq!(
        n.assign("new"),
        want,
        "a session idle that long holds nothing"
    );
    n.set_entry("busy", &pick("new2", None), now);
    assert_ne!(
        n.assign("new2"),
        pick("new2", None),
        "a session in use holds its name"
    );
}

#[test]
fn test_settle_tie_goes_to_the_lower_key() {
    let cards: Cards = Arc::new(Mutex::new(vec![
        card("aaa", "s1", "heron", presence::SOURCE_IRORI, t0()),
        card("zzz", "s2", "heron", presence::SOURCE_IRORI, t0()),
    ]));
    let (_a, low) = test_namer("aaa", &cards);
    low.set_entry("s1", "heron", t0());
    let (_b, high) = test_namer("zzz", &cards);
    high.set_entry("s2", "heron", t0());
    assert!(
        !low.settle() && high.settle(),
        "on equal starts the lower key keeps it"
    );
}

#[test]
fn test_settle_a_chosen_name_always_keeps_it() {
    let cards: Cards = Arc::new(Mutex::new(vec![
        card("pc", "s1", "heron", presence::SOURCE_IRORI, t0()),
        // Renamed later, by hand.
        card(
            "mac",
            "s2",
            "Heron",
            presence::SOURCE_USER,
            plus(t0(), Duration::from_secs(3600)),
        ),
    ]));
    let (_d, pc) = test_namer("pc", &cards);
    pc.set_entry("s1", "heron", t0());
    assert!(pc.settle(), "an Irori name gives way to a chosen one");
    assert_ne!(pc.assigned("s1"), "heron");
}

#[test]
fn test_settle_leaves_chosen_names_alone() {
    let cards: Cards = Arc::new(Mutex::new(vec![
        card(
            "pc",
            "s1",
            "api",
            presence::SOURCE_USER,
            plus(t0(), Duration::from_secs(3600)),
        ),
        card("mac", "s2", "api", presence::SOURCE_USER, t0()),
    ]));
    let (_d, pc) = test_namer("pc", &cards);
    pc.set_entry("s1", "heron", t0());
    assert!(
        !pc.settle(),
        "two chosen names are the person's to sort out"
    );
}

#[test]
fn test_settle_forgets_long_gone_sessions() {
    let cards: Cards = Arc::default();
    let (_d, n) = test_namer("pc", &cards);
    let now = t0();
    n.set_now(move || now);
    n.set_entry(
        "old",
        "heron",
        minus(now, NAMES_KEEP + Duration::from_secs(3600)),
    );
    n.set_entry("recent", "otter", minus(now, Duration::from_secs(3600)));
    n.settle();
    assert_eq!(n.assigned("old"), "");
    assert_eq!(n.assigned("recent"), "otter");
}

/// names.json is a Go `MarshalIndent` of the map, and an unreadable one
/// starts over.
#[test]
fn test_names_file_shape_and_bad_file() {
    let dir = tempfile::tempdir().unwrap();
    let n = Namer::new(dir.path(), "pc");
    n.set_now(t0);
    n.set_mesh(Vec::new);
    let name = n.assign("s1");
    let raw = std::fs::read_to_string(dir.path().join("names.json")).unwrap();
    assert_eq!(
        raw,
        format!(
            "{{\n  \"s1\": {{\n    \"name\": \"{name}\",\n    \"at\": \"2026-10-28T12:00:00Z\"\n  }}\n}}"
        )
    );
    std::fs::write(dir.path().join("names.json"), b"[not a map").unwrap();
    assert_eq!(Namer::new(dir.path(), "pc").assigned("s1"), "");
}
