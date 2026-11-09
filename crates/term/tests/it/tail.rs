//! A tail terminal on a run's output file (live-tasks.md section 5.1):
//! output appears as it is appended, the trailer ends it, and the engine
//! still scrolls and redraws after.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use term::{Scope, TailSpec, TermId, TermMsg, Terms, Viewer};

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A test's terminals and folder. On drop, even when the test fails, it
/// closes every tail (joining its threads) before the folder goes, so
/// Windows can delete it.
struct Scratch {
    terms: Terms,
    dir: tempfile::TempDir,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch {
            terms: Terms::new(),
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn file(&self, name: &str, text: &[u8]) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn tail(&self, path: &Path) -> TermId {
        let info = self
            .terms
            .open_tail(TailSpec {
                scope: Scope::Card("t1".into()),
                path: path.to_path_buf(),
                label: "cargo test".into(),
                run: "b123".into(),
            })
            .unwrap();
        let t = info.tail.as_ref().expect("a tail says so");
        assert_eq!(t.label, "cargo test");
        assert_eq!(t.run, "b123");
        info.id
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.terms.close_all();
    }
}

fn append(p: &Path, text: &[u8]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
    f.write_all(text).unwrap();
}

/// A view that records frames and messages.
#[derive(Clone, Default)]
struct Seen {
    frames: Arc<AtomicUsize>,
    msgs: Arc<Mutex<Vec<TermMsg>>>,
}

impl Seen {
    fn attach(&self, terms: &Terms, id: TermId) {
        let (f, m) = (self.frames.clone(), self.msgs.clone());
        terms
            .attach(
                id,
                Viewer::new(
                    move |b| {
                        assert!(!b.is_empty());
                        f.fetch_add(1, Ordering::SeqCst);
                        true
                    },
                    move |msg| {
                        m.lock().push(msg);
                        true
                    },
                ),
            )
            .unwrap();
    }

    fn frames(&self) -> usize {
        self.frames.load(Ordering::SeqCst)
    }

    fn exit(&self) -> Option<Option<i32>> {
        self.msgs.lock().iter().find_map(|m| match m {
            TermMsg::Exit { code } => Some(*code),
            _ => None,
        })
    }
}

fn exit_of(terms: &Terms, id: TermId) -> Option<Option<i32>> {
    terms
        .in_scope(&Scope::Card("t1".into()))
        .into_iter()
        .find(|t| t.id == id)
        .and_then(|t| t.tail)
        .and_then(|t| t.exit)
}

#[test]
fn a_tail_shows_appended_lines_and_ends_with_the_trailer() {
    let s = Scratch::new();
    let p = s.file("b123.output", b"first\n");
    let id = s.tail(&p);
    let seen = Seen::default();
    seen.attach(&s.terms, id);

    wait_for("the first line", || {
        s.terms.screen_text(id).unwrap().contains("first")
    });
    append(&p, b"second\nthird\r\n");
    // Bare LFs became CRLF: each line starts at column 0, not under the
    // end of the line above it.
    wait_for("appended lines", || {
        let t = s.terms.screen_text(id).unwrap();
        t.lines().any(|l| l == "second") && t.lines().any(|l| l == "third")
    });
    let t = s.terms.screen_text(id).unwrap();
    assert!(t.lines().any(|l| l == "first"), "{t:?}");
    assert!(seen.frames() > 0, "frames reach the view");
    assert!(
        s.terms
            .list()
            .iter()
            .any(|(i, _, running)| *i == id && *running)
    );
    assert_eq!(exit_of(&s.terms, id), None);

    // Typing into a tail is refused.
    assert_eq!(s.terms.write(id, b"x".to_vec()).unwrap_err(), "read only");
    assert_eq!(s.terms.paste(id, "x").unwrap_err(), "read only");

    append(&p, b"\n[exited with code 3]\n");
    wait_for("the exit", || seen.exit().is_some());
    assert_eq!(seen.exit(), Some(Some(3)));
    assert_eq!(exit_of(&s.terms, id), Some(Some(3)));
    assert!(
        s.terms
            .list()
            .iter()
            .any(|(i, _, running)| *i == id && !running)
    );

    // After the end the engine is still there: scroll, resize, redraw and
    // screen text all work, and frames still come.
    let before = seen.frames();
    s.terms.resize(id, 100, 30).unwrap();
    s.terms.scroll(id, 3).unwrap();
    s.terms.scroll(id, -3).unwrap();
    s.terms.redraw(id).unwrap();
    s.terms.ack(id, 1_000_000).unwrap();
    s.terms.focus(id, true).unwrap();
    s.terms.cell_px(id, 8, 16).unwrap();
    wait_for("a frame after exit", || seen.frames() > before);
    assert!(s.terms.screen_text(id).unwrap().contains("third"));
    assert!(s.terms.write(id, b"x".to_vec()).is_err());

    // A view attached later gets the end at once, then a frame.
    let late = Seen::default();
    late.attach(&s.terms, id);
    assert_eq!(late.exit(), Some(Some(3)));
    wait_for("a frame for the late view", || late.frames() > 0);

    // Stop is remembered for the tab.
    s.terms.mark_stopped(id).unwrap();
    let info = s.terms.in_scope(&Scope::Card("t1".into()));
    assert!(info[0].tail.as_ref().unwrap().stopped);
}

#[test]
fn a_killed_run_and_a_deleted_file_end_with_no_code() {
    let s = Scratch::new();
    let killed = s.file("bk.output", b"work\n\n[killed]\n");
    let k = s.tail(&killed);
    wait_for("the killed end", || exit_of(&s.terms, k).is_some());
    assert_eq!(exit_of(&s.terms, k), Some(None));

    let gone = s.file("bg.output", b"some output\n");
    let g = s.tail(&gone);
    wait_for("the output", || {
        s.terms.screen_text(g).unwrap().contains("some output")
    });
    std::fs::remove_file(&gone).unwrap();
    wait_for("the end of a deleted file", || {
        exit_of(&s.terms, g).is_some()
    });
    assert_eq!(exit_of(&s.terms, g), Some(None));
    assert!(s.terms.screen_text(g).unwrap().contains("some output"));

    // A file that was never there ends at once.
    let m = s.tail(&s.dir.path().join("missing.output"));
    wait_for("the end of a missing file", || {
        exit_of(&s.terms, m).is_some()
    });
    assert_eq!(exit_of(&s.terms, m), Some(None));
}

#[test]
fn a_trailer_mid_output_is_not_the_end() {
    let s = Scratch::new();
    let p = s.file("b1.output", b"a\n\n[exited with code 1]\nmore\n");
    let id = s.tail(&p);
    wait_for("the output", || {
        s.terms.screen_text(id).unwrap().contains("more")
    });
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(exit_of(&s.terms, id), None);
}

#[test]
fn a_truncated_file_starts_over() {
    let s = Scratch::new();
    let p = s.file("bt.output", b"old old old old\n");
    let id = s.tail(&p);
    wait_for("the old text", || {
        s.terms.screen_text(id).unwrap().contains("old")
    });
    std::fs::write(&p, b"new\n").unwrap();
    wait_for("the new text", || {
        s.terms.screen_text(id).unwrap().contains("new")
    });
}

#[test]
fn a_big_file_shows_its_end() {
    let s = Scratch::new();
    let line = b"0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrs\n";
    let mut big = Vec::new();
    while big.len() < 3 * 1024 * 1024 {
        big.extend_from_slice(line);
    }
    big.extend_from_slice(b"LAST LINE\n\n[exited with code 0]\n");
    let p = s.file("bbig.output", &big);
    let id = s.tail(&p);
    wait_for("the end", || exit_of(&s.terms, id).is_some());
    assert_eq!(exit_of(&s.terms, id), Some(Some(0)));
    let t = s.terms.screen_text(id).unwrap();
    assert!(t.contains("LAST LINE"), "{t:?}");
}

#[test]
fn closing_stops_tail_threads() {
    let s = Scratch::new();
    let a = s.file("ba.output", b"a\n");
    let b = s.file("bb.output", b"b\n");
    let ia = s.tail(&a);
    s.tail(&b);
    assert_eq!(s.terms.tail_threads(), 4);

    // close() ends one tail's threads and leaves the run's file alone.
    s.terms.close(ia).unwrap();
    assert_eq!(s.terms.tail_threads(), 2);
    assert!(a.exists());

    s.terms.close_all();
    assert_eq!(s.terms.tail_threads(), 0, "close_all joins tail threads");
    assert!(s.terms.list().is_empty());
}
