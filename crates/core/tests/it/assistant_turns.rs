//! One assistant, card 3: the assistant's computer answers questions from
//! rows (one-assistant.md §5). A fake chat turn stands in for the window's
//! run: it records each job and answers with fixed text.

use crate::common;

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::chat::{apart, holds_for, link, meet, skewed, spawner};
use common::mesh::{person, person_with};
use common::{unix_ms, wait_for};
use irori_core::assistant_chat::{
    Asked, ChatDone, ChatJob, ChatRow, ChatTurnFn, Cleared, Posted, STOPPED, Said,
};
use irori_core::{Core, Options};
use store::{Filter, kinds};

const WAIT: u64 = 90;

/// A fake chat turn: records each job and answers "re: <question>". While
/// `hold` is set a turn waits, ending early when its cancel trips.
#[derive(Default)]
struct Fake {
    jobs: Mutex<Vec<ChatJob>>,
    hold: AtomicBool,
    /// Turns that ended on their cancel, and when.
    cancelled: Mutex<Vec<(String, Instant)>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake::default())
    }

    fn held() -> Arc<Fake> {
        let f = Fake::new();
        f.hold.store(true, Ordering::SeqCst);
        f
    }

    fn turn(self: &Arc<Self>) -> ChatTurnFn {
        let f = self.clone();
        Arc::new(move |job: &ChatJob, _emit| {
            f.jobs.lock().unwrap().push(job.clone());
            while f.hold.load(Ordering::SeqCst) {
                if job.cancel.load(Ordering::SeqCst) {
                    f.cancelled
                        .lock()
                        .unwrap()
                        .push((job.turn.clone(), Instant::now()));
                    return ChatDone {
                        text: "half".into(),
                        error: "Stopped.".into(),
                        ..ChatDone::default()
                    };
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let q = question(&job.prompt);
            ChatDone {
                text: format!("re: {q}"),
                // A question asking to post gets a thread, as the window's
                // run posts a draft by itself.
                posted: q.contains("post").then(|| Posted {
                    id: "task:01POSTED".into(),
                    short: "77".into(),
                    tag: String::new(),
                }),
                ..ChatDone::default()
            }
        })
    }

    fn release(&self) {
        self.hold.store(false, Ordering::SeqCst);
    }

    fn ran(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }

    /// The questions it ran, in order.
    fn asked(&self) -> Vec<String> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .map(|j| question(&j.prompt))
            .collect()
    }

    fn job(&self, q: &str) -> Option<ChatJob> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| question(&j.prompt) == q)
            .cloned()
    }
}

/// The question a prompt ends with.
fn question(prompt: &str) -> String {
    let at = prompt.rfind("The person: ").expect("a question") + "The person: ".len();
    prompt[at..].lines().next().unwrap_or_default().to_string()
}

/// One Core with no mesh.
struct Solo {
    core: Core,
    _dirs: Vec<tempfile::TempDir>,
}

fn solo() -> Solo {
    solo_with(|_| {})
}

fn solo_with(edit: impl FnOnce(&mut Options)) -> Solo {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut o = common::options(d.path(), c.path());
    edit(&mut o);
    let core = Core::start(o).unwrap();
    Solo {
        core,
        _dirs: vec![d, c],
    }
}

/// `c` is the assistant's computer: the person picked it.
fn home_here(c: &Core) {
    c.set_assistant_home(c.device_id()).unwrap();
    assert!(c.is_assistant_home());
}

fn ask(c: &Core, text: &str) -> String {
    c.assistant_chat_ask(text, "", "").unwrap()
}

/// Writes a question row as the window would, with `at` given.
fn write_asked(c: &Core, turn: &str, text: &str, at: i64, imported: bool) {
    c.assistant_chat_write(ChatRow::Asked(Asked {
        turn: turn.into(),
        text: text.into(),
        device: c.device_id().into(),
        at,
        imported,
        ..Asked::default()
    }))
    .unwrap();
}

fn said(c: &Core, turn: &str) -> Option<Said> {
    c.assistant_chat_turn(turn).and_then(|t| t.said)
}

/// How many `said` rows `c` holds for `turn`.
fn said_rows(c: &Core, turn: &str) -> usize {
    c.store()
        .list(&Filter {
            kinds: vec![kinds::ASSISTANT_CHAT_SAID.into()],
            limit: store::MAX_LIST_LIMIT,
            ..Filter::default()
        })
        .unwrap()
        .iter()
        .filter(|e| serde_json::from_slice::<Said>(&e.payload).is_ok_and(|s| s.turn == turn))
        .count()
}

fn wait_said(c: &Core, turn: &str, what: &str) -> Said {
    wait_for(WAIT, what, || said(c, turn).is_some());
    said(c, turn).unwrap()
}

/// What `grants_for` gives `text` with nothing typed before it.
fn grants(text: &str, earlier: &[&str]) -> Vec<tools::Grant> {
    tools::grants_for(text, earlier)
}

// ---- answered once, by the recorded computer ----

/// A question asked on one Core is answered once, by the recorded computer
/// only, and its reply (with the thread it posted) shows on both.
#[test]
fn a_question_is_answered_once_by_the_recorded_computer_on_both() {
    let mut devs = person("stephen", 2);
    let (b, a) = (devs.pop().unwrap(), devs.pop().unwrap());
    let (fa, fb) = (Fake::new(), Fake::new());
    for (d, f) in [(&a, &fa), (&b, &fb)] {
        d.core.set_chat_turn(Some(f.turn()));
        d.core
            .set_spawner(Some(spawner(&Arc::new(AtomicBool::new(true)))));
    }
    wait_for(WAIT, "both agree on the assistant's computer", || {
        let h = a.core.assistant_home();
        h.is_some() && h == b.core.assistant_home()
    });
    let (home, other, fh, fo) = if a.core.is_assistant_home() {
        (&a, &b, &fa, &fb)
    } else {
        (&b, &a, &fb, &fa)
    };
    let turn = ask(&other.core, "please post the plan");
    let s = wait_said(&home.core, &turn, "the recorded computer answers");
    assert_eq!(s.text, "re: please post the plan");
    assert_eq!(s.device, home.core.device_id());
    wait_for(WAIT, "the reply shows on the asking Core", || {
        said(&other.core, &turn).is_some()
    });
    let t = other.core.assistant_chat_turn(&turn).unwrap();
    assert_eq!(t.posted.map(|p| p.short).as_deref(), Some("77"));
    holds_for(8, "it was answered once, by one computer", || {
        said_rows(&home.core, &turn) == 1 && said_rows(&other.core, &turn) == 1
    });
    assert_eq!(fh.ran(), 1);
    assert_eq!(fo.ran(), 0, "the other computer ran nothing");
    a.core.close().unwrap();
    b.core.close().unwrap();
}

// ---- one at a time, oldest first ----

/// Two questions from two Cores at once run one after the other, oldest
/// first, and the second turn's prompt holds the first's answer.
#[test]
fn two_questions_from_two_cores_run_in_order_and_the_second_sees_the_first() {
    let mut devs = person("stephen", 2);
    let (b, a) = (devs.pop().unwrap(), devs.pop().unwrap());
    let fa = Fake::held();
    a.core.set_chat_turn(Some(fa.turn()));
    home_here(&a.core);
    wait_for(WAIT, "B follows the pick", || {
        b.core.assistant_home().as_deref() == Some(a.core.device_id())
    });
    let t1 = ask(&a.core, "what's blocked?");
    wait_for(WAIT, "the first turn runs", || fa.ran() == 1);
    let t2 = ask(&b.core, "and who holds it?");
    wait_for(WAIT, "the second reaches A", || {
        a.core.assistant_chat_turn(&t2).is_some()
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fa.ran(), 1, "one turn at a time");
    fa.release();
    wait_said(&a.core, &t2, "the second is answered");
    assert_eq!(fa.asked(), vec!["what's blocked?", "and who holds it?"]);
    let second = fa.job("and who holds it?").unwrap();
    assert!(
        second.prompt.contains("You: re: what's blocked?"),
        "{}",
        second.prompt
    );
    wait_for(WAIT, "both replies show on B", || {
        said(&b.core, &t1).is_some() && said(&b.core, &t2).is_some()
    });
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// A question with an older store order that arrives while another is
/// queued runs first.
#[test]
fn an_older_question_that_arrives_late_runs_before_a_newer_queued_one() {
    let (devs, links, lans) = apart("stephen", 2);
    let (h, p) = (&devs[0], &devs[1]);
    let f = Fake::held();
    h.core.set_chat_turn(Some(f.turn()));
    home_here(&h.core);
    // Written on P while apart: older than what H asks next.
    write_asked(&p.core, "older", "the older question", unix_ms(), false);
    std::thread::sleep(Duration::from_millis(50));
    let first = ask(&h.core, "the running question");
    wait_for(WAIT, "the first turn runs", || f.ran() == 1);
    let newer = ask(&h.core, "the newer question");
    meet(&links, &lans);
    wait_for(WAIT, "the older question reaches H", || {
        h.core.assistant_chat_turn("older").is_some()
    });
    f.release();
    wait_said(&h.core, &newer, "the newer one is answered");
    wait_said(&h.core, "older", "the older one is answered");
    assert!(said(&h.core, &first).is_some());
    assert_eq!(
        f.asked(),
        vec![
            "the running question",
            "the older question",
            "the newer question"
        ]
    );
    for d in devs {
        d.core.close().unwrap();
    }
}

// ---- two ways in, one turn ----

/// A question delivered both live and by the catch-up scan runs once.
#[test]
fn a_question_both_live_and_scanned_runs_once() {
    let s = solo();
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    home_here(&s.core);
    let running = ask(&s.core, "first");
    wait_for(WAIT, "the first runs", || f.ran() == 1);
    let queued = ask(&s.core, "second");
    // Each install scans: neither the running nor the queued question is
    // queued again.
    for _ in 0..3 {
        s.core.set_chat_turn(Some(f.turn()));
    }
    f.release();
    wait_said(&s.core, &queued, "the second is answered");
    holds_for(7, "each ran once", || {
        said_rows(&s.core, &running) == 1 && said_rows(&s.core, &queued) == 1
    });
    assert_eq!(f.asked(), vec!["first", "second"]);
    s.core.close().unwrap();
}

// ---- the catch-up scan ----

/// A question that arrived before the hook was installed is answered by
/// the scan.
#[test]
fn a_question_from_before_the_hook_is_answered_by_the_scan() {
    let s = solo();
    home_here(&s.core);
    let turn = ask(&s.core, "anything new?");
    holds_for(6, "nothing answers without the hook", || {
        said(&s.core, &turn).is_none()
    });
    let f = Fake::new();
    s.core.set_chat_turn(Some(f.turn()));
    let r = wait_said(&s.core, &turn, "the scan answers it");
    assert_eq!(r.text, "re: anything new?");
    assert!(!r.skipped);
    s.core.close().unwrap();
}

/// A question inside a burst that overflows the kind loop's subscription
/// (a resync) is answered by the scan.
#[test]
fn a_question_inside_a_burst_that_overflows_is_answered() {
    let (devs, links, lans) = apart("stephen", 2);
    let (h, p) = (&devs[0], &devs[1]);
    let f = Fake::new();
    h.core.set_chat_turn(Some(f.turn()));
    home_here(&h.core);
    // Far past the subscription's 256 rows, of a kind the kind loop folds
    // again from the store each time, so it falls behind. They are P's
    // own claims, which the person's pick of H beats.
    let claim = irori_core::assistant_home::HomeRow {
        device: p.core.device_id().into(),
        how: "auto".into(),
        by: format!("user:{}", p.core.device_id()),
        at: gocompat::time::GoTime::unix(unix_ms() / 1000, 0),
    };
    for i in 0..1500 {
        if i == 700 {
            write_asked(&p.core, "buried", "the buried question", unix_ms(), false);
        }
        p.core
            .store()
            .append("user", kinds::ASSISTANT_HOME, &claim)
            .unwrap();
    }
    let before = h.core.chat_resyncs();
    meet(&links, &lans);
    let r = wait_said(&h.core, "buried", "the buried question is answered");
    assert_eq!(r.text, "re: the buried question");
    assert!(!r.skipped);
    assert!(
        h.core.chat_resyncs() > before,
        "the burst overflowed the kind loop"
    );
    for d in devs {
        d.core.close().unwrap();
    }
}

/// A question that waited while another computer was recorded is answered
/// once a person's row moves the assistant here.
#[test]
fn a_question_that_waited_for_another_computer_is_answered_after_a_move() {
    let mut devs = person("stephen", 2);
    let (b, a) = (devs.pop().unwrap(), devs.pop().unwrap());
    // A is recorded but has no chat turn: nothing answers there.
    home_here(&a.core);
    let f = Fake::new();
    b.core.set_chat_turn(Some(f.turn()));
    wait_for(WAIT, "B follows the pick", || {
        b.core.assistant_home().as_deref() == Some(a.core.device_id())
    });
    let turn = ask(&b.core, "can you check #4?");
    holds_for(8, "B doesn't answer for A", || {
        said(&b.core, &turn).is_none()
    });
    assert_eq!(f.ran(), 0);
    b.core.set_assistant_home(b.core.device_id()).unwrap();
    let r = wait_said(&b.core, &turn, "B answers it after the move");
    assert_eq!(r.text, "re: can you check #4?");
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// Six questions sent while the computer is connected are all answered.
/// Twenty found by the scan give five answers, the newest, and fifteen
/// `skipped`.
#[test]
fn six_live_questions_all_answer_and_twenty_scanned_give_five() {
    let s = solo();
    home_here(&s.core);
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    let live: Vec<String> = (0..6).map(|i| ask(&s.core, &format!("live {i}"))).collect();
    f.release();
    for t in &live {
        let r = wait_said(&s.core, t, "every live question is answered");
        assert!(!r.skipped && r.error.is_empty(), "{r:?}");
    }
    assert_eq!(f.ran(), 6);
    s.core.close().unwrap();

    let s = solo();
    home_here(&s.core);
    let found: Vec<String> = (0..20)
        .map(|i| ask(&s.core, &format!("waiting {i}")))
        .collect();
    let f = Fake::new();
    s.core.set_chat_turn(Some(f.turn()));
    for t in &found {
        wait_said(&s.core, t, "every found question gets a reply");
    }
    let skipped: Vec<bool> = found
        .iter()
        .map(|t| said(&s.core, t).unwrap().skipped)
        .collect();
    assert_eq!(skipped, [vec![true; 15], vec![false; 5]].concat());
    assert_eq!(
        f.asked(),
        (15..20).map(|i| format!("waiting {i}")).collect::<Vec<_>>()
    );
    s.core.close().unwrap();
}

// ---- late questions ----

/// A question an hour old when it arrives gets the age note and no grants:
/// its "close #12" closes nothing.
#[test]
fn an_hour_old_question_gets_the_note_and_no_grants() {
    let s = solo();
    home_here(&s.core);
    let f = Fake::new();
    s.core.set_chat_turn(Some(f.turn()));
    write_asked(&s.core, "late", "close #12", unix_ms() - 3_600_000, false);
    wait_said(&s.core, "late", "the late question is answered");
    let job = f.job("close #12").unwrap();
    assert!(job.grants.is_empty(), "{:?}", job.grants);
    assert!(
        job.prompt
            .contains("Changes asked for this long ago aren't made"),
        "{}",
        job.prompt
    );
    s.core.close().unwrap();
}

/// A question the catch-up scan finds ten seconds after it was asked isn't
/// late and keeps its grants.
#[test]
fn a_question_scanned_ten_seconds_after_it_was_asked_keeps_its_grants() {
    let s = solo();
    home_here(&s.core);
    write_asked(&s.core, "fresh", "close #12", unix_ms() - 10_000, false);
    let f = Fake::new();
    s.core.set_chat_turn(Some(f.turn()));
    wait_said(&s.core, "fresh", "the scan answers it");
    let job = f.job("close #12").unwrap();
    assert_eq!(job.grants, grants("close #12", &[]));
    assert!(!job.grants.is_empty());
    assert!(!job.prompt.contains("aren't made"));
    s.core.close().unwrap();
}

/// A question from a device whose clock is four minutes behind, with that
/// skew known, isn't late: asked two minutes before it arrived, it reads
/// six by that device's clock, two by this one's.
#[test]
fn a_question_from_a_clock_four_minutes_behind_isnt_late() {
    let behind = Arc::new(AtomicI64::new(-240));
    let b2 = behind.clone();
    let mut devs = person_with("stephen", 2, move |i| (i == 0).then(|| skewed(b2.clone())));
    let (mac, phone) = (devs.pop().unwrap(), devs.pop().unwrap());
    let f = Fake::new();
    mac.core.set_chat_turn(Some(f.turn()));
    home_here(&mac.core);
    wait_for(WAIT, "the Mac knows the phone's skew", || {
        mac.core
            .mesh()
            .skew(phone.core.device_id())
            .is_some_and(|s| s < -200_000_000_000)
    });
    let phone_now = unix_ms() + behind.load(Ordering::SeqCst) * 1000;
    write_asked(
        &phone.core,
        "skewed",
        "close #12",
        phone_now - 120_000,
        false,
    );
    wait_said(&mac.core, "skewed", "the Mac answers the phone");
    let job = f.job("close #12").unwrap();
    assert_eq!(job.grants, grants("close #12", &[]));
    assert!(!job.prompt.contains("aren't made"), "{}", job.prompt);
    mac.core.close().unwrap();
    phone.core.close().unwrap();
}

/// A "close #12" queued live behind a long turn still closes #12: how long
/// it waits in the queue never makes it late.
#[test]
fn a_close_queued_live_behind_a_long_turn_keeps_its_grant() {
    let ahead = Arc::new(AtomicI64::new(0));
    let clock = skewed(ahead.clone());
    let s = solo_with(|o| clock(o));
    home_here(&s.core);
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    ask(&s.core, "a long question");
    wait_for(WAIT, "the long turn runs", || f.ran() == 1);
    let close = ask(&s.core, "close #12");
    // Its row reaches the kind loop now; then the long turn runs ten
    // minutes.
    std::thread::sleep(Duration::from_millis(500));
    ahead.store(600, Ordering::SeqCst);
    f.release();
    wait_said(&s.core, &close, "the close is answered");
    let job = f.job("close #12").unwrap();
    assert_eq!(job.grants, grants("close #12", &[]));
    assert!(!job.grants.is_empty());
    assert!(!job.prompt.contains("aren't made"));
    s.core.close().unwrap();
}

// ---- grants from rows ----

/// `grants_for`'s `earlier` holds only the questions before this one, and
/// never an imported one: "close it" after "what about #5?" closes #5,
/// whatever was imported or asked after it.
#[test]
fn earlier_holds_only_the_questions_before_and_none_imported() {
    let s = solo();
    home_here(&s.core);
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    ask(&s.core, "what about #5?");
    wait_for(WAIT, "the first runs", || f.ran() == 1);
    write_asked(&s.core, "imported", "#7 looks done", unix_ms(), true);
    let close = ask(&s.core, "close it");
    ask(&s.core, "is #9 done?");
    f.release();
    wait_said(&s.core, &close, "close it is answered");
    let job = f.job("close it").unwrap();
    assert_eq!(job.grants, grants("close it", &["what about #5?"]));
    assert_eq!(job.grants, grants("close #5", &[]));
    s.core.close().unwrap();
}

// ---- Stop and New chat ----

/// A Stop row before the turn starts gives "Stopped", and the turn never
/// runs.
#[test]
fn a_stop_before_the_turn_starts_gives_stopped() {
    let s = solo();
    home_here(&s.core);
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    ask(&s.core, "first");
    wait_for(WAIT, "the first runs", || f.ran() == 1);
    let second = ask(&s.core, "second");
    s.core.assistant_chat_stop(&second).unwrap();
    f.release();
    let r = wait_said(&s.core, &second, "the stopped question gets its reply");
    assert_eq!(r.error, STOPPED);
    assert!(r.text.is_empty());
    assert_eq!(f.asked(), vec!["first"]);
    s.core.close().unwrap();
}

/// A Stop row during a run ends it within a second, and its reply says
/// "Stopped".
#[test]
fn a_stop_during_a_run_ends_it_within_a_second() {
    let s = solo();
    home_here(&s.core);
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    let turn = ask(&s.core, "a long one");
    wait_for(WAIT, "it runs", || f.ran() == 1);
    let t0 = Instant::now();
    s.core.assistant_chat_stop(&turn).unwrap();
    wait_for(WAIT, "the run ends", || {
        !f.cancelled.lock().unwrap().is_empty()
    });
    let took = f.cancelled.lock().unwrap()[0].1 - t0;
    assert!(took < Duration::from_secs(1), "took {took:?}");
    let r = wait_said(&s.core, &turn, "its reply is written");
    assert_eq!(r.error, STOPPED);
    assert_eq!(r.text, "half");
    s.core.close().unwrap();
}

/// A New chat during a run stops it too.
#[test]
fn a_new_chat_during_a_run_stops_it() {
    let s = solo();
    home_here(&s.core);
    let f = Fake::held();
    s.core.set_chat_turn(Some(f.turn()));
    let turn = ask(&s.core, "a long one");
    wait_for(WAIT, "it runs", || f.ran() == 1);
    s.core
        .assistant_chat_write(ChatRow::Cleared(Cleared {
            by: format!("user:{}", s.core.device_id()),
            at: unix_ms(),
        }))
        .unwrap();
    let r = wait_said(&s.core, &turn, "its reply is written");
    assert_eq!(r.error, STOPPED);
    assert_eq!(f.cancelled.lock().unwrap().len(), 1);
    s.core.close().unwrap();
}

// ---- imported questions ----

/// An imported question with no reply is never run.
#[test]
fn an_imported_question_with_no_reply_never_runs() {
    let s = solo();
    home_here(&s.core);
    write_asked(&s.core, "old", "close #12", unix_ms(), true);
    let f = Fake::new();
    s.core.set_chat_turn(Some(f.turn()));
    holds_for(8, "the imported question never runs", || {
        f.ran() == 0 && said(&s.core, "old").is_none()
    });
    s.core.close().unwrap();
}

// ---- moves and fresh Cores ----

/// A Core that gets a question and a newer `assistant.home` row naming
/// another device in one sync runs no turn.
#[test]
fn a_question_with_a_newer_move_in_one_sync_runs_no_turn() {
    let (devs, links, lans) = apart("stephen", 2);
    let (a, b) = (&devs[0], &devs[1]);
    let f = Fake::new();
    a.core.set_chat_turn(Some(f.turn()));
    home_here(&a.core);
    std::thread::sleep(Duration::from_millis(50));
    write_asked(&b.core, "moved", "who answers this?", unix_ms(), false);
    b.core.set_assistant_home(b.core.device_id()).unwrap();
    meet(&links, &lans);
    wait_for(WAIT, "A has the question and the move", || {
        a.core.assistant_chat_turn("moved").is_some()
            && a.core.assistant_home().as_deref() == Some(b.core.device_id())
    });
    holds_for(8, "A runs no turn", || {
        f.ran() == 0 && said(&a.core, "moved").is_none()
    });
    for d in devs {
        d.core.close().unwrap();
    }
}

/// A fresh Core that gets the phone's questions before the old computer's
/// replies and `assistant.home` row neither claims nor runs them: it is
/// caught up with the device it syncs from before either.
#[test]
fn a_fresh_core_neither_claims_nor_runs_the_phones_questions() {
    let (devs, links, lans) = apart("stephen", 3);
    let (old, phone, fresh) = (&devs[0], &devs[1], &devs[2]);
    let f_old = Fake::new();
    old.core.set_chat_turn(Some(f_old.turn()));
    home_here(&old.core);
    link(&links, &lans, &[(0, 1), (1, 0)]);
    let turns: Vec<String> = (0..3)
        .map(|i| ask(&phone.core, &format!("phone question {i}")))
        .collect();
    wait_for(
        WAIT,
        "the old computer answers, and the phone has it",
        || turns.iter().all(|t| said(&phone.core, t).is_some()),
    );
    // The fresh desktop has its chat turn, and syncs from the phone only.
    let f_fresh = Fake::new();
    fresh.core.set_chat_turn(Some(f_fresh.turn()));
    link(&links, &lans, &[(2, 1), (1, 2)]);
    wait_for(WAIT, "the fresh desktop is connected to the phone", || {
        fresh
            .core
            .mesh()
            .devices(false)
            .iter()
            .any(|d| d.device_id == phone.core.device_id() && d.state == "connected")
    });
    // Now it could claim, were it not caught up first.
    fresh
        .core
        .set_spawner(Some(spawner(&Arc::new(AtomicBool::new(true)))));
    wait_for(
        WAIT,
        "the fresh desktop has the claim and the replies",
        || {
            fresh.core.assistant_home().as_deref() == Some(old.core.device_id())
                && turns.iter().all(|t| said(&fresh.core, t).is_some())
        },
    );
    let mine = |c: &Core| {
        c.store()
            .list(&Filter {
                kinds: vec![kinds::ASSISTANT_HOME.into()],
                device_id: fresh.core.device_id().to_string(),
                ..Filter::default()
            })
            .unwrap()
            .len()
    };
    holds_for(12, "the fresh desktop neither claims nor runs", || {
        f_fresh.ran() == 0 && mine(&fresh.core) == 0 && !fresh.core.is_assistant_home()
    });
    for d in devs {
        d.core.close().unwrap();
    }
}

// ---- assistantChat ----

/// A Core without the hook says no `assistantChat` and never claims; with
/// it, it says so and claims.
#[test]
fn a_core_without_the_hook_says_no_assistant_chat_and_never_claims() {
    let s = solo();
    s.core
        .set_spawner(Some(spawner(&Arc::new(AtomicBool::new(true)))));
    let me = s.core.device_id().to_string();
    wait_for(WAIT, "its spawn.host says assistantTurn", || {
        s.core
            .spawn_hosts()
            .get(&me)
            .is_some_and(|h| h.assistant_turn)
    });
    holds_for(12, "no assistantChat and no claim", || {
        s.core
            .spawn_hosts()
            .get(&me)
            .is_some_and(|h| !h.assistant_chat)
            && s.core.assistant_home().is_none()
    });
    s.core.set_chat_turn(Some(Fake::new().turn()));
    wait_for(WAIT, "it says assistantChat and claims", || {
        s.core
            .spawn_hosts()
            .get(&me)
            .is_some_and(|h| h.assistant_chat)
            && s.core.is_assistant_home()
    });
    s.core.close().unwrap();
}

/// A connected, higher-ranked Core whose `spawn.host` has `assistantTurn`
/// but no `assistantChat` doesn't stop an able Core from claiming.
#[test]
fn a_higher_ranked_core_without_assistant_chat_doesnt_block_the_claim() {
    let mut devs = person("stephen", 2);
    devs.sort_by(|x, y| x.core.device_id().cmp(y.core.device_id()));
    let (able, old) = (devs.pop().unwrap(), devs.pop().unwrap());
    // The lower id ranks first; it has Claude Code but an older Irori.
    old.core
        .set_spawner(Some(spawner(&Arc::new(AtomicBool::new(true)))));
    let oid = old.core.device_id().to_string();
    wait_for(
        WAIT,
        "the older one's spawn.host reaches the able one",
        || {
            able.core
                .spawn_hosts()
                .get(&oid)
                .is_some_and(|h| h.assistant_turn && !h.assistant_chat)
        },
    );
    able.core.set_chat_turn(Some(Fake::new().turn()));
    able.core
        .set_spawner(Some(spawner(&Arc::new(AtomicBool::new(true)))));
    wait_for(WAIT, "the able Core claims", || {
        able.core.is_assistant_home()
            && old.core.assistant_home().as_deref() == Some(able.core.device_id())
    });
    old.core.close().unwrap();
    able.core.close().unwrap();
}
