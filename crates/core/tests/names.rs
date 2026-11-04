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

/// Card #472: another person's live agent on a shared board holds its word
/// against a new terminal session's name and a spawn's pick, but never
/// moves a name this person's agents already have.
#[test]
fn test_namer_skips_names_of_other_peoples_agents() {
    let first = pick("s1", None);
    let mine: Cards = Arc::default();
    let (_d, n) = test_namer("pc", &mine);
    let theirs: Cards = Arc::new(Mutex::new(vec![AgentCard {
        key: "agent:friend-card".into(),
        name: first.clone(),
        status: "idle".into(),
        ..AgentCard::default()
    }]));
    let th = theirs.clone();
    n.set_others(move || th.lock().unwrap().clone());
    assert_ne!(
        fold(&n.assign("s1")),
        fold(&first),
        "a friend's agent has it"
    );
    assert_ne!(fold(&n.pick_new("s1", &|_| false)), fold(&first));
    // The spawn's own refusals count too.
    let second = n.pick_new("s1", &|_| false);
    assert_ne!(n.pick_new("s1", &|w| w == second), second);
    // Gone, it is no longer taken, only used: it comes out once every
    // unused word is refused.
    theirs.lock().unwrap()[0].status = "gone".into();
    assert_eq!(n.pick_new("s1", &|w| w != first), first);
}

/// Card #472: a word picked for an agent Irori is starting is held, so a
/// terminal session named before the agent shows never gets it.
#[test]
fn test_a_spawns_word_is_held_while_it_starts() {
    let cards: Cards = Arc::default();
    let (_d, n) = test_namer("pc", &cards);
    let w = n.pick_new("s1", &|_| false);
    assert_eq!(w, pick("s1", None));
    assert_ne!(n.assign("s1"), w, "the starting agent holds it");
}

/// Card #472: a friend's agent whose device has been offline for weeks no
/// longer holds its word against a new pick.
#[test]
fn test_a_long_offline_friends_agent_frees_its_word() {
    let first = pick("s1", None);
    let cards: Cards = Arc::default();
    let (_d, n) = test_namer("pc", &cards);
    n.set_now(t0);
    let theirs: Cards = Arc::new(Mutex::new(vec![AgentCard {
        key: "agent:friend-card".into(),
        name: first.clone(),
        status: "idle".into(),
        device_state: coord::roster::OFFLINE.into(),
        device_seen: minus(t0(), Duration::from_secs(3600)),
        ..AgentCard::default()
    }]));
    let th = theirs.clone();
    n.set_others(move || th.lock().unwrap().clone());
    let seed = (0..100_000)
        .map(|i| format!("x{i}"))
        .find(|s| pick(s, None) == first)
        .unwrap();
    // Offline an hour: still held.
    assert_ne!(n.pick_new(&seed, &|_| false), first, "offline an hour");
    // Offline past OTHERS_STALE: no longer taken; only "used" (it can
    // still come out once every unused word is gone).
    theirs.lock().unwrap()[0].device_seen = minus(
        t0(),
        irori_core::namer::OTHERS_STALE + Duration::from_secs(60),
    );
    let only = |w: &str| w != first;
    assert_eq!(n.pick_new(&seed, &only), first, "free again");
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
fn test_names_of_idle_sessions_wait_while_other_words_are_free() {
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
    // Card #224: no longer held, but used a few hours ago, so a new
    // session gets another word while the list has one.
    assert_ne!(
        n.assign("new"),
        want,
        "a name from a few hours ago came straight back"
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

/// Card #224: days of new sessions on one device, each live a few hours
/// and then gone (its name still remembered in agent-names.json); returns
/// every (time, name) given, in order.
fn simulate(days: u64, per_day: u64) -> Vec<(i64, String)> {
    use std::collections::BTreeMap;
    let now = Arc::new(Mutex::new(t0()));
    let cards: Cards = Arc::default();
    let seen: Arc<Mutex<BTreeMap<String, (String, GoTime)>>> = Arc::default();
    let (_d, n) = test_namer("pc", &cards);
    {
        let now = now.clone();
        n.set_now(move || *now.lock().unwrap());
        let seen = seen.clone();
        n.set_seen(move || {
            seen.lock()
                .unwrap()
                .iter()
                .map(|(k, (name, at))| (k.clone(), name.clone(), *at))
                .collect()
        });
    }
    let live = Duration::from_secs(3 * 3600);
    let step = Duration::from_secs(24 * 3600 / per_day);
    let mut given = Vec::new();
    for d in 0..days {
        for i in 0..per_day {
            let t = plus(t0(), Duration::from_secs(d * 24 * 3600) + step * i as u32);
            *now.lock().unwrap() = t;
            // Sessions past their few hours are gone (still on the roster
            // for a while, and remembered).
            for c in cards.lock().unwrap().iter_mut() {
                if plus(c.started_at, live).unix_nano() <= t.unix_nano() {
                    c.status = "gone".into();
                }
            }
            cards.lock().unwrap().retain(|c| {
                plus(c.started_at, Duration::from_secs(24 * 3600)).unix_nano() > t.unix_nano()
            });
            let sid = format!("sess-{d}-{i}");
            let name = n.assign(&sid);
            cards
                .lock()
                .unwrap()
                .push(card("pc", &sid, &name, presence::SOURCE_IRORI, t));
            seen.lock()
                .unwrap()
                .insert(format!("pc/{sid}"), (name.clone(), plus(t, live)));
            given.push((t.secs, name));
        }
    }
    given
}

fn shortest_reuse(given: &[(i64, String)]) -> Option<(String, i64)> {
    use std::collections::HashMap;
    let mut last: HashMap<&str, i64> = HashMap::new();
    let mut worst: Option<(String, i64)> = None;
    for (t, name) in given {
        if let Some(prev) = last.insert(name, *t) {
            let gap = t - prev;
            if worst.as_ref().is_none_or(|w| gap < w.1) {
                worst = Some((name.clone(), gap));
            }
        }
    }
    worst
}

#[test]
fn test_no_name_comes_back_within_a_week() {
    // 30 days of 20 sessions a day: 600 names, all different.
    let given = simulate(30, 20);
    assert_eq!(given.len(), 600);
    assert_eq!(shortest_reuse(&given), None, "a name came back");
    // A busier device runs past the list; still nothing within 7 days.
    let given = simulate(30, 60);
    let (name, gap) = shortest_reuse(&given).expect("1800 names from about 1000 words");
    assert!(
        gap >= 7 * 24 * 3600,
        "{name} came back after {:.1} days",
        gap as f64 / 86400.0
    );
}

#[test]
fn test_a_gone_agents_name_is_not_given_again() {
    let want = pick("new", None);
    let cards: Cards = Arc::new(Mutex::new(vec![{
        let mut c = card("mac", "old", &want, presence::SOURCE_IRORI, t0());
        c.status = "gone".into();
        c
    }]));
    let (_d, n) = test_namer("pc", &cards);
    assert_ne!(n.assign("new"), want, "a gone agent still on its cards");
}

#[test]
fn test_a_running_session_keeps_a_name_the_list_no_longer_gives() {
    // names.json from before the list grew: a running session keeps its
    // name on restart, even one the new list would never land on.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("names.json"),
        br#"{"s1": {"name": "oldword", "at": "2026-10-28T12:00:00Z"}}"#,
    )
    .unwrap();
    let n = Namer::new(dir.path(), "pc");
    n.set_mesh(Vec::new);
    assert_eq!(n.assigned("s1"), "oldword");
    assert_eq!(n.assign("s1"), "oldword", "renamed on restart");
}
