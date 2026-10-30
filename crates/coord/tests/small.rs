//! Ports of the names, devcolor and roster tests
//! (I:internal/names/names_test.go, devcolor/devcolor_test.go,
//! roster/roster_test.go, roster/fold_test.go).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use coord::devcolor::{self, Device, PALETTE, Picks};
use coord::names::{self, WORDS};
use coord::roster::{self, AgentCard, Roster};
use gocompat::time::GoTime;
use store::{Event, Store, kinds};

#[test]
fn test_words_are_distinct_and_plain() {
    let mut seen = std::collections::HashSet::new();
    assert_eq!(WORDS.len(), 127);
    for w in WORDS {
        assert!(seen.insert(*w), "{w:?} twice");
        assert!(
            *w == w.to_lowercase() && !w.contains([' ', '-', '_']),
            "{w:?} isn't a plain lowercase word"
        );
    }
}

#[test]
fn test_pick_is_deterministic() {
    let a = names::pick("0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0", None);
    assert_eq!(a, names::pick("0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0", None));
    // coord.md §3.13 vectors.
    assert_eq!(a, "lemur");
    assert_eq!(names::pick("s1", None), "mesa");
}

#[test]
fn test_pick_probes_past_taken_names() {
    let first = names::pick("s1", None);
    let taken = |n: &str| n == first;
    let got = names::pick("s1", Some(&taken));
    let i = WORDS.iter().position(|w| *w == first).unwrap();
    assert_eq!(got, WORDS[(i + 1) % WORDS.len()]);
}

#[test]
fn test_pick_numbers_when_every_word_is_taken() {
    let taken = |n: &str| !n.contains(|c: char| c.is_ascii_digit());
    let got = names::pick("s1", Some(&taken));
    assert_eq!(got, format!("{}2", names::pick("s1", None)));
}

#[test]
fn test_fold() {
    assert_eq!(names::fold("  Heron "), "heron");
}

fn t0() -> GoTime {
    GoTime::parse_rfc3339("2026-10-28T12:00:00Z").unwrap()
}

fn devs(ids: &[&str]) -> Vec<Device> {
    ids.iter()
        .enumerate()
        .map(|(i, id)| Device {
            id: id.to_string(),
            at: GoTime::unix(t0().secs + 3600 * i as i64, 0),
        })
        .collect()
}

#[test]
fn test_defaults_are_unique_while_the_palette_lasts() {
    let ids: Vec<String> = (0..PALETTE.len())
        .map(|i| format!("{}-device", (b'a' + i as u8) as char))
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let got = devcolor::assign(&devs(&refs), &HashMap::new());
    let mut seen = std::collections::HashSet::new();
    for id in &ids {
        let c = &got[id];
        assert!(devcolor::valid(c) && seen.insert(c.clone()), "{id} got {c}");
    }
}

#[test]
fn test_adding_a_device_never_moves_an_older_one() {
    let before = devcolor::assign(&devs(&["pc", "mac"]), &HashMap::new());
    let after = devcolor::assign(&devs(&["pc", "mac", "aaa-new"]), &HashMap::new());
    assert_eq!(before["pc"], after["pc"]);
    assert_eq!(before["mac"], after["mac"]);
    // coord.md §3.13 vector: both hash to pink; pc joined first.
    assert_eq!(
        (before["pc"].as_str(), before["mac"].as_str()),
        ("pink", "lime")
    );
}

#[test]
fn test_a_pick_wins_and_defaults_avoid_it() {
    let base = devcolor::assign(&devs(&["pc", "mac"]), &HashMap::new());
    let chosen = HashMap::from([("mac".to_string(), base["pc"].clone())]);
    let got = devcolor::assign(&devs(&["pc", "mac"]), &chosen);
    assert_eq!(got["mac"], base["pc"]);
    assert_ne!(got["pc"], base["pc"]);
    let plaid = HashMap::from([("pc".to_string(), "plaid".to_string())]);
    assert!(devcolor::valid(
        &devcolor::assign(&devs(&["pc"]), &plaid)["pc"]
    ));
}

fn pick_row(dev: &str, seq: i64, at: u64, actor: &str, device: &str, color: &str) -> Event {
    Event {
        device_id: dev.into(),
        seq,
        hlc: hlc::Timestamp(at),
        actor: actor.into(),
        kind: devcolor::KIND.into(),
        payload: format!(r#"{{"color":"{color}","deviceId":"{device}"}}"#).into_bytes(),
        sig: Vec::new(),
    }
}

#[test]
fn test_picks_keep_the_newest_user_row() {
    let p = Picks::new();
    p.apply(&pick_row("pc", 1, 10, "user", "mac", "lime"));
    p.apply(&pick_row("mac", 1, 20, "user", "mac", "pink"));
    p.apply(&pick_row("pc", 2, 15, "user", "mac", "cyan"));
    assert_eq!(p.chosen()["mac"], "pink");
    assert!(!p.apply(&pick_row("pc", 3, 99, "agent:pc/x", "mac", "sand")));
    assert_eq!(p.chosen()["mac"], "pink");
    assert!(!p.apply(&pick_row("pc", 4, 98, "user", "mac", "plaid")));
    p.apply(&pick_row("pc", 5, 100, "user", "mac", ""));
    assert!(!p.chosen().contains_key("mac"));
}

// --- roster ---

struct Rig {
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    seq: HashMap<String, i64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let st = Store::open(
            dir.path().join("irori.db"),
            "dev1",
            Arc::new(hlc::Clock::new()),
        )
        .unwrap();
        Rig {
            _dir: dir,
            st: Arc::new(st),
            seq: HashMap::new(),
        }
    }

    fn row(&mut self, dev: &str, kind: &str, payload: Vec<u8>) {
        let seq = self.seq.entry(dev.to_string()).or_default();
        *seq += 1;
        let at = ((now_ms() as u64) << 16) + *seq as u64;
        self.st
            .insert(&Event {
                device_id: dev.into(),
                seq: *seq,
                hlc: hlc::Timestamp(at),
                actor: "irori".into(),
                kind: kind.into(),
                payload,
                sig: Vec::new(),
            })
            .unwrap();
    }

    fn card(&mut self, dev: &str, c: AgentCard) {
        self.row(dev, kinds::AGENT_SEEN, gocompat::json::marshal(&c).unwrap());
    }

    fn opts(&self) -> roster::Options {
        roster::Options {
            store: self.st.clone(),
            self_id: "dev1".into(),
            local: None,
            liveness: None,
            revoked: None,
        }
    }
}

fn card(key: &str, name: &str, status: &str) -> AgentCard {
    AgentCard {
        key: key.into(),
        agent: "claude".into(),
        name: name.into(),
        status: status.into(),
        ..AgentCard::default()
    }
}

fn keys(cs: &[AgentCard]) -> Vec<String> {
    cs.iter().map(|c| c.key.clone()).collect()
}

fn eventually(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

const A2: &str = "dev2/aaaaaaaa-1111-4111-8111-111111111111";
const B2: &str = "dev2/bbbbbbbb-2222-4222-8222-222222222222";
const A3: &str = "dev3/cccccccc-3333-4333-8333-333333333333";

#[test]
fn test_card_from_another_device_is_ignored() {
    let mut g = Rig::new();
    g.card("dev2", card(A2, "pc", "busy"));
    g.card("dev2", card(A3, "forged", ""));
    g.card("dev3", card(A3, "mac", ""));
    g.row(
        "dev2",
        kinds::AGENT_STATUS,
        format!(r#"{{"key":"{A3}","name":"renamed","status":"busy"}}"#).into_bytes(),
    );
    g.row(
        "dev2",
        kinds::AGENT_GONE,
        format!(r#"{{"key":"{A3}"}}"#).into_bytes(),
    );
    let r = Roster::new(g.opts()).unwrap();
    let got = r.remote();
    assert_eq!(keys(&got), [A2, A3]);
    assert_eq!(
        (
            got[1].name.as_str(),
            got[1].status.as_str(),
            got[1].device_id.as_str()
        ),
        ("mac", "idle", "dev3")
    );
    assert_eq!(r.ignored(), 1);
    assert_eq!(r.ignored_by()["dev2"], 1);
}

#[test]
fn test_card_row_scrubs_local_fields() {
    let mut g = Rig::new();
    let mut c = card(A2, "pc", "gone");
    c.last_text = "secret prose".into();
    c.transcript = r"C:\Users\x\t.jsonl".into();
    c.pid = 42;
    g.card("dev2", c);
    let r = Roster::new(g.opts()).unwrap();
    let c = &r.remote()[0];
    assert!(
        c.last_text.is_empty() && c.transcript.is_empty() && c.pid == 0 && c.status == "idle",
        "{c:?}"
    );
}

#[test]
fn test_cards_retire() {
    let mut g = Rig::new();
    g.card("dev2", card(A2, "", ""));
    g.card("dev2", card(B2, "", ""));
    g.card("dev3", card(A3, "", ""));
    let r = Roster::start(g.opts()).unwrap();
    let ch = r.subscribe();
    assert_eq!(r.remote().len(), 3);

    g.row(
        "dev2",
        kinds::AGENT_STATUS,
        format!(r#"{{"key":"{A2}","status":"gone"}}"#).into_bytes(),
    );
    eventually("status gone retires a2", || r.remote().len() == 2);
    assert!(
        matches!(ch.recv_timeout(Duration::from_secs(5)), Ok(Some(()))),
        "Subscribe did not signal"
    );

    g.row("dev2", kinds::DEVICE_STARTED, b"{}".to_vec());
    eventually("dev2's restart retires b2", || r.remote().len() == 1);
    assert_eq!(keys(&r.remote()), [A3]);

    g.row(
        "dev3",
        kinds::AGENT_GONE,
        format!(r#"{{"key":"{A3}"}}"#).into_bytes(),
    );
    eventually("agent.gone retires a3", || r.remote().is_empty());
}

#[test]
fn test_agents_liveness_and_revoked() {
    let mut g = Rig::new();
    g.card("dev2", card(A2, "", ""));
    g.card("dev3", card(A3, "", ""));
    let local = card("dev1/dddddddd-4444-4444-8444-444444444444", "", "");
    let revoked = Arc::new(AtomicBool::new(false));
    let mut o = g.opts();
    let l = local.clone();
    o.local = Some(Arc::new(move || vec![l.clone()]));
    o.liveness = Some(Arc::new(|dev: &str| {
        match dev {
            "dev2" => roster::CONNECTED,
            "dev3" => roster::RECENT,
            _ => "",
        }
        .to_string()
    }));
    let rv = revoked.clone();
    o.revoked = Some(Arc::new(move |dev: &str| {
        rv.load(Ordering::SeqCst) && dev == "dev3"
    }));
    let r = Roster::new(o).unwrap();
    let all = r.agents();
    assert_eq!(keys(&all), [local.key.as_str(), A2, A3]);
    assert_eq!(
        (all[1].device_state.as_str(), all[2].device_state.as_str()),
        (roster::CONNECTED, roster::RECENT)
    );
    revoked.store(true, Ordering::SeqCst);
    assert_eq!(keys(&r.agents()), [local.key.as_str(), A2]);

    let r2 = Roster::new(g.opts()).unwrap();
    for c in r2.remote() {
        assert_eq!(c.device_state, roster::OFFLINE, "{}", c.key);
    }
}

#[test]
fn test_own_rows_are_not_folded() {
    let g = Rig::new();
    g.st.append(
        "irori",
        kinds::AGENT_SEEN,
        &card("dev1/eeeeeeee-5555-4555-8555-555555555555", "", ""),
    )
    .unwrap();
    let r = Roster::new(g.opts()).unwrap();
    assert!(r.remote().is_empty());
}

#[test]
fn test_agent_status_carries_a_later_permission_mode() {
    let mut g = Rig::new();
    const KEY: &str = "dev2/rrrrrrrr-9999-4999-8999-999999999999";
    g.card("dev2", card(KEY, "mac", "idle"));
    g.row(
        "dev2",
        kinds::AGENT_STATUS,
        format!(
            r#"{{"key":"{KEY}","name":"mac","permissionMode":"bypassPermissions","status":"busy"}}"#
        )
        .into_bytes(),
    );
    let r = Roster::start(g.opts()).unwrap();
    let mode = |r: &Roster| {
        r.remote()
            .into_iter()
            .find(|c| c.key == KEY)
            .unwrap()
            .permission_mode
    };
    assert_eq!(mode(&r), "bypassPermissions");
    g.row(
        "dev2",
        kinds::AGENT_STATUS,
        format!(r#"{{"key":"{KEY}","name":"mac","status":"idle"}}"#).into_bytes(),
    );
    eventually("the second status row folds", || {
        r.remote()
            .iter()
            .any(|c| c.key == KEY && c.status == "idle")
    });
    assert_eq!(mode(&r), "bypassPermissions");
}

#[test]
fn test_agent_status_carries_the_name_source() {
    let mut g = Rig::new();
    const KEY: &str = "dev2/nnnnnnnn-9999-4999-8999-999999999999";
    let mut c = card(KEY, "heron", "idle");
    c.name_source = "irori".into();
    g.card("dev2", c);
    g.row(
        "dev2",
        kinds::AGENT_STATUS,
        format!(r#"{{"key":"{KEY}","name":"Bob","nameSource":"user","status":"idle"}}"#)
            .into_bytes(),
    );
    let r = Roster::new(g.opts()).unwrap();
    let c = r.remote().into_iter().find(|c| c.key == KEY).unwrap();
    assert_eq!((c.name.as_str(), c.name_source.as_str()), ("Bob", "user"));
}

#[test]
fn test_agent_card_json_is_gos() {
    // presence.AgentCard field order and omit rules, as agent.seen rows hold them.
    let c = card(A2, "pc", "busy");
    let got = String::from_utf8(gocompat::json::marshal(&c).unwrap()).unwrap();
    assert_eq!(
        got,
        r#"{"key":"dev2/aaaaaaaa-1111-4111-8111-111111111111","deviceId":"","agent":"claude","sessionId":"","pid":0,"name":"pc","title":"","cwd":"","status":"busy","sessionKind":"","entrypoint":"","version":"","peerFeatures":null,"startedAt":"0001-01-01T00:00:00Z","updatedAt":"0001-01-01T00:00:00Z","lastActivity":"0001-01-01T00:00:00Z","lastText":"","transcript":"","repoKey":"","permissionMode":""}"#
    );
    // Go's json.Unmarshal matches keys case-insensitively.
    let back: AgentCard = coord::gojson::decode(br#"{"KEY":"x/y","Status":"idle"}"#).unwrap();
    assert_eq!((back.key.as_str(), back.status.as_str()), ("x/y", "idle"));
}
