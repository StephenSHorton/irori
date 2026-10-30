//! I:internal/notify/notify_test.go.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use common::*;
use notify::{
    Action, CORNER_BOTTOM_LEFT, CORNER_TOP_RIGHT, Config, Level, Note, Notifier, Options,
    SETTINGS_FILE, TEST_ID, ask_id, held_id, message_id, needs_id, parse_level, returned_id,
    unclaimed_id,
};

fn note(id: &str, title: &str) -> Note {
    Note {
        id: id.into(),
        title: title.into(),
        ..Note::default()
    }
}

#[test]
fn configure_and_headless() {
    let f = Fake::new();
    let n = notifier(&f);
    {
        let st = f.lock();
        let cfg = st.cfg.as_ref().expect("the cards were never configured");
        assert_eq!(cfg.app_name, "Irori");
        assert_eq!(cfg.corner_radius, 0.0);
        assert!(cfg.capture_exclude);
        assert_eq!(cfg.palette, toast::DARK);
    }
    assert!(n.enabled() && !n.is_headless());

    // Go reads IRORI_TOAST_CAPTURE here too; setting the env in a parallel
    // test binary races, so the config field stands in for it.
    let f2 = Fake::new();
    Notifier::new(Config {
        toaster: Some(f2.clone()),
        capture_visible: true,
        ..Config::default()
    });
    assert!(!f2.lock().cfg.as_ref().unwrap().capture_exclude);

    // Headless never touches the cards and never shows one.
    let f3 = Fake::new();
    let h = Notifier::new(Config {
        toaster: Some(f3.clone()),
        headless: true,
        ..Config::default()
    });
    assert!(f3.lock().cfg.is_none() && f3.lock().opts.is_empty());
    assert!(!h.post(note("x", "hi")) && !h.enabled() && h.is_headless());
    h.clear("x");
    assert!(f3.post_count() == 0 && f3.lock().closes.is_empty());
    assert!(Notifier::headless().is_headless());
    assert_eq!(
        h.post_test(),
        Err("this Irori shows no notifications".into())
    );
}

#[test]
fn ids_never_stack() {
    assert_eq!(held_id("dlv_1"), "held:dlv_1");
    assert_eq!(needs_id("t1"), "needs:t1");
    assert_eq!(message_id("dlv_2"), "msg:dlv_2");
    assert_eq!(ask_id("a"), "ask:a");
    assert_eq!(unclaimed_id("t"), "unclaimed:t");
    assert_eq!(returned_id("t"), "returned:t");
    let f = Fake::new();
    let n = notifier(&f);
    assert!(!n.post(Note {
        title: "no id".into(),
        ..Note::default()
    }));
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let (a, b) = (first.clone(), second.clone());
    n.post(Note {
        actions: vec![Action::new("A", move || {
            a.fetch_add(1, Ordering::SeqCst);
        })],
        ..note(&held_id("d"), "one")
    });
    n.post(Note {
        actions: vec![Action::new("A", move || {
            b.fetch_add(1, Ordering::SeqCst);
        })],
        ..note(&held_id("d"), "two")
    });
    assert!(
        f.post_count() == 2 && f.lock().live.len() == 1,
        "same id replaces"
    );
    // The replaced card's callback is dead; only the live one's runs, once.
    let posts = f.lock().posts.clone();
    (posts[0].actions[0].run.as_ref().unwrap())();
    f.press(&held_id("d"), "A");
    (posts[1].actions[0].run.as_ref().unwrap())();
    assert_eq!(first.load(Ordering::SeqCst), 0);
    assert_eq!(second.load(Ordering::SeqCst), 1);
    assert!(!n.live(&held_id("d")), "an answered card is still live");
}

#[test]
fn clear_runs_no_callbacks() {
    let f = Fake::new();
    let n = notifier(&f);
    let ran = Arc::new(AtomicUsize::new(0));
    let (r1, r2, r3) = (ran.clone(), ran.clone(), ran.clone());
    n.post(Note {
        on_dismiss: Some(Arc::new(move || {
            r1.fetch_add(1, Ordering::SeqCst);
        })),
        on_expire: Some(Arc::new(move || {
            r2.fetch_add(1, Ordering::SeqCst);
        })),
        expire: Duration::from_millis(20),
        actions: vec![Action::new("Go", move || {
            r3.fetch_add(1, Ordering::SeqCst);
        })],
        ..note("c", "x")
    });
    let c = f.card("c");
    assert!(
        c.sticky && c.expire.is_zero(),
        "the Notifier owns the clock of a card with on_expire"
    );
    n.clear("c");
    assert!(!f.is_live("c") && f.lock().closes.len() == 1);
    std::thread::sleep(Duration::from_millis(60));
    let p = f.lock().posts[0].clone();
    (p.actions[0].run.as_ref().unwrap())();
    (p.on_dismiss.as_ref().unwrap())();
    assert_eq!(ran.load(Ordering::SeqCst), 0, "callbacks ran after clear");
    n.clear("nothing");
    assert_eq!(f.lock().closes.len(), 1, "an unknown id reached the cards");
}

#[test]
fn expire_owned_by_notifier() {
    let f = Fake::new();
    let n = notifier(&f);
    let (tx, rx) = mpsc::channel::<&str>();
    let (t1, t2) = (tx.clone(), tx.clone());
    n.post(Note {
        expire: Duration::from_millis(20),
        on_expire: Some(Arc::new(move || t1.send("expired").unwrap())),
        on_dismiss: Some(Arc::new(move || t2.send("dismissed").unwrap())),
        ..note("e", "x")
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok("expired"));
    assert!(!f.is_live("e"), "the expired card was not closed");
    let p = f.lock().posts[0].clone();
    (p.on_dismiss.as_ref().unwrap())(); // too late
    assert!(
        rx.recv_timeout(Duration::from_millis(30)).is_err(),
        "a second outcome ran"
    );
}

#[test]
fn a_replace_stops_the_old_expiry() {
    let f = Fake::new();
    let n = notifier(&f);
    let (tx, rx) = mpsc::channel::<&str>();
    let t1 = tx.clone();
    n.post(Note {
        expire: Duration::from_millis(30),
        on_expire: Some(Arc::new(move || t1.send("old").unwrap())),
        ..note("e", "x")
    });
    n.post(note("e", "y"));
    assert!(rx.recv_timeout(Duration::from_millis(120)).is_err());
    assert!(n.live("e"));
}

#[test]
fn options_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_FILE);
    // Another setting in the file survives.
    std::fs::write(&path, br#"{"other":{"x":1}}"#).unwrap();
    let f = Fake::new();
    let n = Notifier::new(Config {
        dir: Some(dir.path().into()),
        toaster: Some(f.clone()),
        ..Config::default()
    });
    assert_eq!(n.options(), Options::default());
    let heard = Arc::new(std::sync::Mutex::new(None));
    let h = heard.clone();
    n.on_options(move |o| *h.lock().unwrap() = Some(o.clone()));
    let (got, err) = n.set_options(Options {
        enabled: false,
        sound: false,
        volume: 3.0,
        corner: "middle".into(),
    });
    err.unwrap();
    let want = Options {
        enabled: false,
        sound: false,
        volume: 1.0,
        corner: CORNER_TOP_RIGHT.into(),
    };
    assert_eq!(got, want);
    assert_eq!(heard.lock().unwrap().clone(), Some(want.clone()));
    let last = f.lock().opts.last().cloned().unwrap();
    assert!(!last.enabled && last.volume == 1.0);
    assert!(!n.post(note("x", "off")), "posted while off");
    assert_eq!(
        n.post_test(),
        Err("notifications are off; turn them on first".into())
    );

    let n2 = Notifier::new(Config {
        dir: Some(dir.path().into()),
        toaster: Some(Fake::new()),
        ..Config::default()
    });
    assert_eq!(n2.options(), want, "reloaded");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        raw,
        "{\n  \"notifications\": {\n    \"enabled\": false,\n    \"sound\": false,\n    \"volume\": 1,\n    \"corner\": \"top-right\"\n  },\n  \"other\": {\n    \"x\": 1\n  }\n}\n"
    );

    // A partial or broken file falls back to the defaults field by field.
    std::fs::write(&path, br#"{"notifications":{"corner":"bottom-left"}}"#).unwrap();
    let o = notify::load(dir.path());
    assert!(o.corner == CORNER_BOTTOM_LEFT && o.enabled && o.volume == Options::default().volume);
    std::fs::write(&path, b"{not json").unwrap();
    assert_eq!(notify::load(dir.path()), Options::default());
    // No temp files left behind.
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![SETTINGS_FILE.to_string()]);
}

#[test]
fn levels() {
    for (inp, want) in [
        ("", Level::Info),
        ("INFO", Level::Info),
        ("success", Level::Success),
        (" warn ", Level::Warn),
        ("error", Level::Error),
    ] {
        assert_eq!(parse_level(inp), Some(want), "{inp:?}");
    }
    assert_eq!(parse_level("loud"), None);
    let f = Fake::new();
    let n = notifier(&f);
    n.post(Note {
        level: Level::Warn,
        ..note("w", "x")
    });
    assert_eq!(f.card("w").kind, toast::Kind::Warn);
}

#[test]
fn test_card_opens_the_board() {
    let f = Fake::new();
    let n = notifier(&f);
    let (tx, rx) = mpsc::channel();
    n.set_opener(move |t| tx.send(t).unwrap());
    n.post_test().unwrap();
    let c = f.card(TEST_ID);
    assert_eq!(c.title, "Irori notifications are on");
    assert_eq!(
        c.actions
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Open Board", "Close"]
    );
    f.press(TEST_ID, "Open Board");
    let t = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(t.tab == "board" && t.ref_.is_empty());
}
