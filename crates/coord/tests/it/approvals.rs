//! Port of I:internal/ask/ask_test.go (approvals).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use coord::approvals::{
    self, AnsweredRow, Ask, Book, CHOICE_REVIVE, Error, KIND_ANSWERED, KIND_POSTED, KIND_RESOLVED,
    KIND_REVIVE_REQUESTED, Options, ResolvedRow, ReviveRow,
};
use gocompat::time::GoTime;
use store::{Event, Filter, Store};

const AGENT1: &str = "dev1/aaaaaaaa-1111-4111-8111-111111111111";
const AGENT2: &str = "dev2/bbbbbbbb-2222-4222-8222-222222222222";

fn owners(k: &str) -> bool {
    k == "user:dev1" || k == "user:dev2"
}

fn wall() -> GoTime {
    GoTime::from_system(SystemTime::now())
}

fn hour() -> GoTime {
    coord_add(wall(), 3600)
}

fn coord_add(t: GoTime, secs: i64) -> GoTime {
    GoTime::unix(t.secs + secs, i64::from(t.nanos))
}

struct Rig {
    _dir: tempfile::TempDir,
    st: Arc<Store>,
    book: Arc<Book>,
    seq: HashMap<String, i64>,
    resolved: Arc<Mutex<Vec<Ask>>>,
    revived: Arc<Mutex<Vec<String>>>,
}

fn rig(now: Option<Arc<dyn Fn() -> GoTime + Send + Sync>>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let st = Arc::new(
        Store::open(
            dir.path().join("irori.db"),
            "dev1",
            Arc::new(hlc::Clock::new()),
        )
        .unwrap(),
    );
    let revived = Arc::new(Mutex::new(Vec::new()));
    let mut o = Options::new(st.clone(), "dev1");
    o.is_owner = Some(Arc::new(owners));
    o.now = now;
    let rv = revived.clone();
    o.revive = Some(Arc::new(move |t: &str| {
        rv.lock().unwrap().push(t.to_string());
        Ok(())
    }));
    let book = Book::start(o).unwrap();
    let resolved = Arc::new(Mutex::new(Vec::new()));
    let r = resolved.clone();
    book.on_resolved(move |a| r.lock().unwrap().push(a.clone()));
    Rig {
        _dir: dir,
        st,
        book,
        seq: HashMap::new(),
        resolved,
        revived,
    }
}

impl Rig {
    fn row<P: serde::Serialize>(&mut self, dev: &str, actor: &str, kind: &str, payload: &P) {
        let seq = self.seq.entry(dev.to_string()).or_default();
        *seq += 1;
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.st
            .insert(&Event {
                device_id: dev.into(),
                seq: *seq,
                hlc: hlc::Timestamp((ms << 16) + *seq as u64),
                actor: actor.into(),
                kind: kind.into(),
                payload: gocompat::json::marshal(payload).unwrap(),
                sig: Vec::new(),
            })
            .unwrap();
    }

    fn post(&self, by: &str) -> Ask {
        self.book
            .post(Ask {
                by: by.into(),
                name: "alpha".into(),
                text: "Ship it?".into(),
                actions: vec!["Yes".into(), "No".into()],
                until: hour(),
                ..Ask::default()
            })
            .unwrap()
    }

    fn resolved(&self) -> Vec<Ask> {
        self.resolved.lock().unwrap().clone()
    }
}

fn eventually(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn answered(ask: &str, choice: &str, by: &str) -> AnsweredRow {
    AnsweredRow {
        ask_id: ask.into(),
        choice: choice.into(),
        by: by.into(),
        ..AnsweredRow::default()
    }
}

#[test]
fn test_post_needs_the_fields() {
    let g = rig(None);
    let a = |by: &str, text: &str, actions: &[&str], until: GoTime| Ask {
        by: by.into(),
        text: text.into(),
        actions: actions.iter().map(|s| s.to_string()).collect(),
        until,
        ..Ask::default()
    };
    for ask in [
        a("", "x", &["Ok"], hour()),
        a(AGENT1, "", &["Ok"], hour()),
        a(AGENT1, "x", &[], hour()),
        a(AGENT1, "x", &["Ok"], GoTime::ZERO),
    ] {
        assert!(matches!(g.book.post(ask), Err(Error::Incomplete)));
    }
    assert!(g.book.open().is_empty());
}

#[test]
fn test_answer_here_resolves() {
    let g = rig(None);
    let sub = g.book.subscribe();
    let a = g.post(AGENT1);
    assert_eq!(a.device, "dev1");
    assert!(!a.resolved);
    let open = g.book.open();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, a.id);
    assert_eq!(
        sub.recv_timeout(Duration::from_secs(5)),
        Ok(Some(a.id.clone()))
    );
    g.book.answer(&a.id, "Yes", "user:dev1").unwrap();
    let got = g.book.get(&a.id).unwrap();
    assert!(
        got.resolved && got.answered && got.choice == "Yes" && !got.answer_ref.is_empty(),
        "{got:?}"
    );
    let r = g.resolved();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].choice, "Yes");
    g.book.answer(&a.id, "No", "user:dev1").unwrap();
    assert_eq!(g.book.get(&a.id).unwrap().choice, "Yes");
    assert!(g.book.open().is_empty());
    assert_eq!(g.resolved().len(), 1);
}

#[test]
fn test_answer_refuses_non_owners() {
    let g = rig(None);
    let a = g.post(AGENT1);
    for by in ["user:anything", "user:dev3", AGENT1, ""] {
        assert!(
            matches!(g.book.answer(&a.id, "Yes", by), Err(Error::NotPerson)),
            "{by}"
        );
    }
    assert!(g.book.answer("no-such-ask", "Yes", "user:dev1").is_err());
    assert!(!g.book.get(&a.id).unwrap().resolved);
}

#[test]
fn test_remote_answers() {
    let mut g = rig(None);
    let a = g.post(AGENT1);
    g.row(
        "dev3",
        "user",
        KIND_ANSWERED,
        &answered(&a.id, "No", "user:dev3"),
    );
    g.row(
        "dev2",
        "user",
        KIND_ANSWERED,
        &answered(&a.id, "No", "user:anything"),
    );
    g.row(
        "dev2",
        "irori",
        KIND_ANSWERED,
        &answered(&a.id, "No", "user:dev2"),
    );
    eventually("three bad answers ignored", || g.book.ignored() == 3);
    let by = g.book.ignored_by();
    assert_eq!((by["dev2"], by["dev3"]), (2, 1));
    assert!(!g.book.get(&a.id).unwrap().resolved);

    let mut r = answered(&a.id, "Yes", "user:dev2");
    r.at_device = "dev2".into();
    g.row("dev2", "user", KIND_ANSWERED, &r);
    eventually("the person's answer from dev2 resolves it", || {
        g.book.get(&a.id).unwrap().resolved
    });
    let got = g.book.get(&a.id).unwrap();
    assert_eq!(
        (got.choice.as_str(), got.answer_ref.as_str()),
        ("Yes", "dev2/3")
    );
}

#[test]
fn test_writer_rules() {
    let mut g = rig(None);
    let a = g.post(AGENT1);
    g.row(
        "dev2",
        "irori",
        KIND_RESOLVED,
        &ResolvedRow {
            ask_id: a.id.clone(),
            choice: "No".into(),
            answered: true,
            ..ResolvedRow::default()
        },
    );
    let ask = |id: &str, by: &str| Ask {
        id: id.into(),
        by: by.into(),
        text: "x".into(),
        actions: vec!["Ok".into()],
        until: hour(),
        ..Ask::default()
    };
    g.row("dev2", "irori", KIND_POSTED, &ask("forged", AGENT1));
    eventually("both rows ignored", || g.book.ignored() == 2);
    assert!(!g.book.get(&a.id).unwrap().resolved);
    assert!(g.book.get("forged").is_none());

    g.row("dev2", "irori", KIND_POSTED, &ask("theirs", AGENT2));
    eventually("dev2's ask shows", || g.book.get("theirs").is_some());
    g.book.answer("theirs", "Ok", "user:dev1").unwrap();
    assert!(!g.book.get("theirs").unwrap().resolved);
    g.row(
        "dev2",
        "irori",
        KIND_RESOLVED,
        &ResolvedRow {
            ask_id: "theirs".into(),
            choice: "Ok".into(),
            answered: true,
            ..ResolvedRow::default()
        },
    );
    eventually("dev2's resolve clears it", || {
        g.book.get("theirs").unwrap().resolved
    });
    assert!(g.resolved().is_empty());
}

#[test]
fn test_unanswered_resolves_at_until() {
    let now = Arc::new(Mutex::new(wall()));
    let n = now.clone();
    let g = rig(Some(Arc::new(move || *n.lock().unwrap())));
    let start = *now.lock().unwrap();
    let a = g
        .book
        .post(Ask {
            by: AGENT1.into(),
            text: "x".into(),
            actions: vec!["Ok".into()],
            until: coord_add(start, 60),
            ..Ask::default()
        })
        .unwrap();
    *now.lock().unwrap() = coord_add(start, 120);
    eventually("the ask runs out", || g.book.get(&a.id).unwrap().resolved);
    let got = g.book.get(&a.id).unwrap();
    assert!(!got.answered && got.choice.is_empty());
    let r = g.resolved();
    assert_eq!(r.len(), 1);
    assert!(!r[0].answered);
}

fn revive(target: &str, delivery: &str, by: &str) -> ReviveRow {
    ReviveRow {
        target: target.into(),
        delivery_id: delivery.into(),
        by: by.into(),
        ..ReviveRow::default()
    }
}

#[test]
fn test_revive_from_another_device() {
    let mut g = rig(None);
    g.row(
        "dev3",
        "user",
        KIND_REVIVE_REQUESTED,
        &revive(AGENT1, "d0", "user:dev3"),
    );
    g.row(
        "dev2",
        "irori",
        KIND_REVIVE_REQUESTED,
        &revive(AGENT1, "d0", "user:dev2"),
    );
    g.row(
        "dev2",
        "user",
        KIND_REVIVE_REQUESTED,
        &revive(AGENT2, "d0", "user:dev2"),
    );
    g.row(
        "dev2",
        "user",
        KIND_REVIVE_REQUESTED,
        &revive(AGENT1, "d1", "user:dev2"),
    );
    eventually("the revive ask is posted", || g.book.open().len() == 1);
    let ra = g.book.open().remove(0);
    assert_eq!(ra.id, "revive-dev2-3");
    assert_eq!(
        (
            ra.target.as_str(),
            ra.delivery_id.as_str(),
            ra.device.as_str()
        ),
        (AGENT1, "d1", "dev1")
    );
    assert_eq!(ra.title, format!("Revive {AGENT1}?"));

    let mut r = answered(&ra.id, CHOICE_REVIVE, "user:dev2");
    r.target = AGENT1.into();
    r.delivery_id = "other".into();
    g.row("dev2", "user", KIND_ANSWERED, &r);
    eventually("the mismatched answer resolves it", || {
        g.book.get(&ra.id).unwrap().resolved
    });
    assert!(g.revived.lock().unwrap().is_empty());

    g.row(
        "dev2",
        "user",
        KIND_REVIVE_REQUESTED,
        &revive(AGENT1, "d2", "user:dev2"),
    );
    eventually("the second revive ask", || g.book.open().len() == 1);
    let rb = g.book.open().remove(0);
    g.book.answer(&rb.id, CHOICE_REVIVE, "user:dev1").unwrap();
    assert_eq!(*g.revived.lock().unwrap(), [AGENT1]);
    assert!(g.resolved().is_empty());
}

#[test]
fn test_request_revive() {
    let g = rig(None);
    assert!(matches!(
        g.book.request_revive(AGENT2, "d1", "user:anything"),
        Err(Error::NotPerson)
    ));
    assert!(g.book.request_revive(AGENT1, "d1", "user:dev1").is_err());
    g.book.request_revive(AGENT2, "d1", "user:dev1").unwrap();
    let evs =
        g.st.list(&Filter {
            kinds: vec![KIND_REVIVE_REQUESTED.into()],
            ..Filter::default()
        })
        .unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].actor, "user");
    let r: ReviveRow = coord::gojson::decode(&evs[0].payload).unwrap();
    assert_eq!(
        (
            r.target.as_str(),
            r.delivery_id.as_str(),
            r.by.as_str(),
            r.at_device.as_str()
        ),
        (AGENT2, "d1", "user:dev1", "dev1")
    );
}

#[test]
fn test_default_owner_is_this_device() {
    let dir = tempfile::tempdir().unwrap();
    let st = Arc::new(
        Store::open(
            dir.path().join("irori.db"),
            "dev1",
            Arc::new(hlc::Clock::new()),
        )
        .unwrap(),
    );
    let b = Book::start(Options::new(st, "dev1")).unwrap();
    let a = b
        .post(Ask {
            by: AGENT1.into(),
            text: "x".into(),
            actions: vec!["Ok".into()],
            until: hour(),
            ..Ask::default()
        })
        .unwrap();
    assert!(matches!(
        b.answer(&a.id, "Ok", "user:dev2"),
        Err(Error::NotPerson)
    ));
    b.answer(&a.id, "Ok", "user:dev1").unwrap();
}

#[test]
fn test_ask_id_and_posted_payload() {
    // Go: strconv.FormatInt(UnixNano, 36) + "-" + counter; 2026-10-26T12:00Z
    // → dmer4j36rk00 (coord.md §2.2).
    let t = GoTime::parse_rfc3339("2026-10-26T12:00:00Z").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let st = Arc::new(
        Store::open(
            dir.path().join("irori.db"),
            "dev1",
            Arc::new(hlc::Clock::new()),
        )
        .unwrap(),
    );
    let mut o = Options::new(st.clone(), "dev1");
    o.now = Some(Arc::new(move || t));
    let b = Book::new(o).unwrap();
    let a = b
        .post(Ask {
            by: AGENT1.into(),
            text: "x<y".into(),
            actions: vec!["Ok".into()],
            until: t,
            ..Ask::default()
        })
        .unwrap();
    assert_eq!(a.id, "dmer4j36rk00-1");
    let evs = st.list(&Filter::default()).unwrap();
    assert_eq!(
        String::from_utf8(evs[0].payload.clone()).unwrap(),
        format!(
            r#"{{"askId":"dmer4j36rk00-1","by":"{AGENT1}","name":"","text":"x\u003cy","actions":["Ok"],"until":"2026-10-26T12:00:00Z","device":"","resolved":false}}"#
        )
    );
    let _ = approvals::SUB_BUFFER;
}
