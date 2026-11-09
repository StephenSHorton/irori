//! Card #782 through the Core, with a fake namer in place of the desktop
//! app's `claude -p --model haiku`: the person's new thread gets a title
//! by itself, written as Irori; a name the person gave is never touched;
//! the person's replies make Irori look at it once more; Undo makes the
//! old name theirs; the switch only turns it off; and the assistant's own
//! folder never makes it an agent that is handed anything.

use crate::common;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use irori_core::thread_names::{NameAsk, Namer};

#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<NameAsk>>,
}

impl Fake {
    fn names(&self) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.prompt.starts_with("Name this thread"))
            .count()
    }
    fn relooks(&self) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.prompt.starts_with("Title now:"))
            .count()
    }
}

/// A namer that answers `name` for a new thread and `retitle` when asked
/// again.
fn namer(f: &Arc<Fake>, name: &'static str, retitle: &'static str) -> Namer {
    let f = f.clone();
    Arc::new(move |a: &NameAsk| {
        f.asked.lock().unwrap().push(a.clone());
        Ok(if a.prompt.starts_with("Title now:") {
            retitle.to_string()
        } else {
            format!("\"{name}\"\n")
        })
    })
}

fn person(r: &Rig) -> String {
    format!("user:{}", r.core.device_id())
}

fn thread(r: &Rig, text: &str) -> coord::Task {
    r.core
        .coord()
        .post_ask(
            &person(r),
            coord::NewAsk {
                text: text.into(),
                thread: true,
                ..coord::NewAsk::default()
            },
            &r.core.ask_agents(),
        )
        .unwrap()
        .0
}

/// The person's reply, as the window writes it.
fn reply(r: &Rig, id: &str, text: &str) {
    r.core
        .coord()
        .person_comment_files(&person(r), id, text, false, &[], Vec::new(), None)
        .unwrap();
}

fn title(r: &Rig, id: &str) -> coord::Task {
    r.core.coord().get_task(id).unwrap()
}

/// Gives the loops time to do what they shouldn't.
fn settle() {
    std::thread::sleep(Duration::from_millis(600));
}

#[test]
fn a_new_thread_gets_a_title_by_itself() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core
        .set_namer(Some(namer(&fake, "Login fails on Safari", "SAME")));
    assert!(r.core.names_on(), "on unless the person turns it off");
    let th = thread(
        &r,
        "hey so the login page is broken again on safari, can someone look",
    );
    wait_for(10, "the thread named", || {
        title(&r, &th.id).title == "Login fails on Safari"
    });
    let got = title(&r, &th.id);
    assert!(!got.title_by_person, "Irori's name, not the person's");
    assert_eq!(got.irori_titled, 1);
    assert_eq!(got.title_was, th.title, "Undo puts this back");
    let line = r
        .core
        .coord()
        .activity(&th.id)
        .unwrap()
        .into_iter()
        .find(|a| a.what.starts_with("renamed it from"))
        .expect("the thread says Irori renamed it");
    assert_eq!(line.by, "irori");
    let asked = fake.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "asked once: {asked:?}");
    assert!(asked[0].prompt.contains("the login page is broken again"));
}

#[test]
fn a_name_the_person_gave_meanwhile_stays() {
    let r = Rig::new();
    let (go, wait) = mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    let (asked, heard) = mpsc::channel::<()>();
    let asked = Mutex::new(asked);
    r.core.set_namer(Some(Arc::new(move |_: &NameAsk| {
        let _ = asked.lock().unwrap().send(());
        let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(30));
        Ok("Lobby join flow".to_string())
    })));
    let th = thread(&r, "the thing with the lobby");
    heard.recv_timeout(Duration::from_secs(60)).unwrap();
    // The person renames it while the turn runs.
    r.core
        .coord()
        .rename_task(&person(&r), &th.id, "Lobby")
        .unwrap();
    go.send(()).unwrap();
    settle();
    let got = title(&r, &th.id);
    assert_eq!(got.title, "Lobby");
    assert!(got.title_by_person);
    assert_eq!(got.irori_titled, 0);
}

#[test]
fn replies_make_it_look_once_more_and_only_once() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core.set_namer(Some(namer(
        &fake,
        "Build check",
        "Build cache misses on Windows\nMOVED",
    )));
    let th = thread(&r, "can we look at the build");
    wait_for(10, "the thread named", || {
        title(&r, &th.id).title == "Build check"
    });
    // One reply isn't enough to say the talk moved on.
    reply(&r, &th.id, "it's slow every time on the PC");
    settle();
    assert_eq!(fake.relooks(), 0);
    reply(&r, &th.id, "I think the cache misses after each pull");
    wait_for(10, "the thread retitled", || {
        title(&r, &th.id).title == "Build cache misses on Windows"
    });
    let got = title(&r, &th.id);
    assert_eq!(got.irori_titled, 2);
    assert_eq!(got.title_was, "Build check");
    let asked = fake.asked.lock().unwrap().clone();
    let relook = asked
        .iter()
        .find(|a| a.prompt.starts_with("Title now:"))
        .unwrap();
    assert!(relook.prompt.contains("Title now: Build check"));
    assert!(relook.prompt.contains("Person: I think the cache misses"));
    // Never again.
    reply(&r, &th.id, "and on the Mac too");
    reply(&r, &th.id, "anyway");
    settle();
    assert_eq!(fake.relooks(), 1);
    assert_eq!(fake.names(), 1);
}

#[test]
fn a_thread_the_assistant_posted_keeps_its_title() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core.set_namer(Some(namer(
        &fake,
        "Build check",
        "Build cache misses on Windows\nMOVED",
    )));
    let th = r
        .core
        .coord()
        .post_ask(
            &person(&r),
            coord::NewAsk {
                text: "Speed up the Windows build cache".into(),
                thread: true,
                assistant: true,
                ..coord::NewAsk::default()
            },
            &r.core.ask_agents(),
        )
        .unwrap()
        .0;
    // A person's thread beside it is named, so the namer is really running.
    let own = thread(&r, "can we look at the build");
    wait_for(10, "the person's thread named", || {
        title(&r, &own.id).title == "Build check"
    });
    settle();
    assert_eq!(fake.names(), 1, "never asked to name the assistant's");
    // Two replies are what make Irori look at a thread again.
    reply(&r, &th.id, "it's slow every time on the PC");
    reply(&r, &th.id, "I think the cache misses after each pull");
    settle();
    let got = title(&r, &th.id);
    assert_eq!(got.title, th.title);
    assert_eq!(got.irori_titled, 0);
    assert!(got.title_was.is_empty(), "no Undo");
    assert!(!got.title_by_person);
    assert_eq!(fake.relooks(), 0);
    let lines = r.core.coord().activity(&th.id).unwrap();
    assert!(
        !lines.iter().any(|l| l.what.starts_with("renamed it from")),
        "no renamed line: {lines:?}"
    );
}

#[test]
fn a_title_that_still_fits_stays_and_is_asked_once() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core
        .set_namer(Some(namer(&fake, "Build check", "Build check speed\nSAME")));
    let th = thread(&r, "can we look at the build");
    wait_for(10, "the thread named", || {
        title(&r, &th.id).title == "Build check"
    });
    reply(&r, &th.id, "one");
    reply(&r, &th.id, "two");
    wait_for(10, "looked at again", || fake.relooks() == 1);
    reply(&r, &th.id, "three");
    settle();
    assert_eq!(fake.relooks(), 1);
    assert_eq!(title(&r, &th.id).title, "Build check");
    assert_eq!(title(&r, &th.id).irori_titled, 1);
}

#[test]
fn undo_gives_the_old_name_back_and_it_stays_the_persons() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core
        .set_namer(Some(namer(&fake, "Lobby join", "Lobby rework\nMOVED")));
    let th = thread(&r, "the thing with the lobby");
    wait_for(10, "the thread named", || {
        title(&r, &th.id).title == "Lobby join"
    });
    // Undo is the person's rename back to what it was.
    let was = title(&r, &th.id).title_was;
    r.core
        .coord()
        .rename_task(&person(&r), &th.id, &was)
        .unwrap();
    reply(&r, &th.id, "one");
    reply(&r, &th.id, "two");
    settle();
    let got = title(&r, &th.id);
    assert_eq!(got.title, th.title);
    assert!(got.title_by_person);
    assert_eq!(fake.relooks(), 0, "Irori leaves a name the person chose");
}

#[test]
fn the_switch_only_turns_it_off() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core.set_namer(Some(namer(&fake, "Named", "SAME")));
    r.core.set_names_on(false).unwrap();
    assert!(!r.core.names_on());
    let th = thread(&r, "leave my words as the title");
    settle();
    assert_eq!(title(&r, &th.id).title, th.title);
    assert_eq!(fake.names(), 0);
    // Back on: the next new thread is named.
    r.core.set_names_on(true).unwrap();
    let th2 = thread(&r, "and this one gets a name");
    wait_for(10, "the next thread named", || {
        title(&r, &th2.id).title == "Named"
    });
}

#[test]
fn without_a_namer_nothing_is_named() {
    // A headless Core, or the phone: no `claude` to ask.
    let r = Rig::new();
    let th = thread(&r, "a thread on a Core with no namer");
    settle();
    assert_eq!(title(&r, &th.id).title, th.title);
    assert_eq!(title(&r, &th.id).irori_titled, 0);
}

#[test]
fn an_agents_own_thread_and_a_plain_ask_arent_named() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core.set_namer(Some(namer(&fake, "Named", "SAME")));
    let (mine, _) = r
        .core
        .coord()
        .post_agent_thread(
            &r.key_a,
            coord::NewAsk {
                text: "an agent's own thread".into(),
                thread: true,
                ..coord::NewAsk::default()
            },
        )
        .unwrap();
    let (ask, _) = r
        .core
        .coord()
        .post_ask(
            &person(&r),
            coord::NewAsk {
                text: "a plain ask, not a thread".into(),
                ..coord::NewAsk::default()
            },
            &[],
        )
        .unwrap();
    settle();
    assert_eq!(fake.names(), 0);
    assert_eq!(title(&r, &mine.id).title, mine.title);
    assert_eq!(title(&r, &ask.id).title, ask.title);
}

#[test]
fn the_assistant_is_never_an_agent_and_never_handed_a_thread() {
    let r = Rig::new();
    let fake = Arc::new(Fake::default());
    r.core.set_namer(Some(namer(&fake, "Named", "SAME")));
    let me = r.core.device_id().to_string();
    let sessions = r.claude.join("sessions");
    // A Claude Code turn in the assistant's folder, and a real agent.
    let dir = r.core.assistant_dir().join("names");
    std::fs::create_dir_all(&dir).unwrap();
    let assistant = "eeeeeeee-5555-4555-8555-000000000005";
    let gamma = "ffffffff-6666-4666-8666-000000000006";
    write_session(&sessions, "900005.json", assistant, "assistant", &dir);
    // Inside the rig's own folders, so nothing outlives its TempDirs.
    let elsewhere = r.claude.join("gamma-work");
    std::fs::create_dir_all(&elsewhere).unwrap();
    write_session(&sessions, "900006.json", gamma, "gamma", &elsewhere);
    let gamma_key = format!("{me}/{gamma}");
    wait_for(60, "presence lists gamma", || {
        r.core.local_agents().iter().any(|c| c.key == gamma_key)
    });
    let assistant_key = format!("{me}/{assistant}");
    assert!(
        !r.core.local_agents().iter().any(|c| c.key == assistant_key),
        "never on the roster"
    );
    assert!(
        !r.core.ask_agents().iter().any(|c| c.key == assistant_key),
        "never offered work"
    );
    let th = thread(&r, "who takes this?");
    wait_for(10, "the thread named", || {
        title(&r, &th.id).title == "Named"
    });
    let got = title(&r, &th.id);
    let taker = got
        .ask
        .as_ref()
        .map(|a| a.taker.clone())
        .unwrap_or_default();
    assert_ne!(taker, assistant_key, "never handed the thread");
    assert!(!got.holders.contains(&assistant_key));
    assert!(
        r.notices_to(&assistant_key).is_empty(),
        "nothing sent to it"
    );
}
