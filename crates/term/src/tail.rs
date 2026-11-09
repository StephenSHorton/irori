//! A read-only terminal over a run's output file (live-tasks.md section
//! 5.1, route B): no PTY. A reader thread feeds the file into the same
//! rsz-term engine a shell pane uses, and a frame thread sends the same
//! frames, so the card's view can't tell a tail from a shell except that
//! typing does nothing.
//!
//! The engine outlives the run: after the run ends the tab still scrolls,
//! redraws and resizes until it is closed.

use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use rsz_term::{Alacritty, Engine, EngineEvent};

use crate::{TailInfo, TermMsg};

/// How much of an existing file a new tail shows: its last 2 MB, from the
/// first whole line.
const START_BACK: u64 = 2 * 1024 * 1024;
/// The most read in one go, so frames keep flowing through a big file.
const CHUNK: u64 = 1024 * 1024;
/// How often the file is checked for more.
const POLL: Duration = Duration::from_millis(100);
/// Scrollback lines, as a shell pane keeps (rsz-term's default).
const SCROLLBACK: usize = 10_000;
/// The size a tail starts at; the view resizes it on attach.
const COLS: u16 = 80;
const ROWS: u16 = 24;
/// The raw bytes kept from the file's end, to find claude's trailer.
const KEEP: usize = 128;

/// As suzuri's pane (`suzuri:crates/term/src/pane.rs`): a frame may stay
/// unacknowledged this long before the next one goes anyway, and frames
/// are paced at an unfocused pane's interval (a tail takes no input).
const ACK_TIMEOUT: Duration = Duration::from_millis(250);
const INTERVAL: Duration = Duration::from_millis(33);

/// What a tail sends its terminal. Answers false once the terminals are
/// gone, and the tail's reader then stops.
pub(crate) enum TailOut {
    Frame(Vec<u8>),
    Msg(TermMsg),
}
pub(crate) type Emit = Arc<dyn Fn(TailOut) -> bool + Send + Sync>;

pub(crate) struct Tail {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    label: String,
    run: String,
    stopped: AtomicBool,
}

struct Shared {
    engine: Mutex<Alacritty>,
    wake: Mutex<Wake>,
    cv: Condvar,
    /// The reader's stop flag, on its own condvar so frames never wake it.
    halt: Mutex<bool>,
    halt_cv: Condvar,
}

struct Wake {
    dirty: bool,
    force_full: bool,
    stop: bool,
    sent: u32,
    sent_at: Instant,
    acked: u32,
}

impl Shared {
    fn kick(&self, f: impl FnOnce(&mut Wake)) {
        let mut w = self.wake.lock();
        f(&mut w);
        self.cv.notify_one();
    }
}

/// Counts a tail's live threads, so tests can see `close_all` end them.
struct Live(Arc<AtomicUsize>);
impl Live {
    fn new(n: &Arc<AtomicUsize>) -> Live {
        n.fetch_add(1, Ordering::SeqCst);
        Live(n.clone())
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Tail {
    /// Starts tailing `path`. The file need not be readable yet: a missing
    /// file ends the tail with no code.
    pub(crate) fn start(
        path: PathBuf,
        label: String,
        run: String,
        emit: Emit,
        live: &Arc<AtomicUsize>,
    ) -> Result<Tail, String> {
        let shared = Arc::new(Shared {
            engine: Mutex::new(Alacritty::new(COLS, ROWS, SCROLLBACK)),
            wake: Mutex::new(Wake {
                dirty: false,
                force_full: false,
                stop: false,
                sent: 0,
                sent_at: Instant::now() - Duration::from_secs(1),
                acked: 0,
            }),
            cv: Condvar::new(),
            halt: Mutex::new(false),
            halt_cv: Condvar::new(),
        });
        let tail = Tail {
            shared: shared.clone(),
            threads: Mutex::new(Vec::new()),
            label,
            run,
            stopped: AtomicBool::new(false),
        };
        let (s, e, l) = (shared.clone(), emit.clone(), Live::new(live));
        let frames = std::thread::Builder::new()
            .name("irori-tail-frames".into())
            .spawn(move || {
                let _l = l;
                frame_loop(&s, &e)
            })
            .map_err(|e| format!("could not start a tail: {e}"))?;
        tail.threads.lock().push(frames);
        let (s, l) = (shared, Live::new(live));
        let reader = std::thread::Builder::new()
            .name("irori-tail-read".into())
            .spawn(move || {
                let _l = l;
                read_loop(&path, &s, &emit)
            });
        match reader {
            Ok(h) => tail.threads.lock().push(h),
            Err(e) => {
                tail.stop();
                return Err(format!("could not start a tail: {e}"));
            }
        }
        Ok(tail)
    }

    pub(crate) fn info(&self, exit: Option<Option<i32>>) -> TailInfo {
        TailInfo {
            label: self.label.clone(),
            run: self.run.clone(),
            exit,
            stopped: self.stopped.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) {
        self.shared.engine.lock().resize(cols.max(2), rows.max(1));
        self.redraw();
    }

    pub(crate) fn scroll(&self, delta: i32) {
        self.shared.engine.lock().scroll(delta);
        self.shared.kick(|w| w.dirty = true);
    }

    pub(crate) fn redraw(&self) {
        self.shared.kick(|w| {
            w.dirty = true;
            w.force_full = true;
        });
    }

    pub(crate) fn ack(&self, seq: u32) {
        self.shared.kick(|w| w.acked = w.acked.max(seq));
    }

    pub(crate) fn screen_text(&self) -> String {
        self.shared.engine.lock().screen_text()
    }

    /// Tells both threads to end; doesn't wait.
    pub(crate) fn stop(&self) {
        *self.shared.halt.lock() = true;
        self.shared.halt_cv.notify_all();
        self.shared.kick(|w| w.stop = true);
    }

    /// Stops both threads and waits for them. Bounded: the reader wakes
    /// within one poll and the frame thread at once. Never call it while
    /// holding the terminals' map lock (the threads take it to send).
    pub(crate) fn join(&self) {
        self.stop();
        let me = std::thread::current().id();
        let threads = std::mem::take(&mut *self.threads.lock());
        for h in threads {
            if h.thread().id() != me {
                let _ = h.join();
            }
        }
    }
}

impl Drop for Tail {
    fn drop(&mut self) {
        // A tail forgotten without a close must not poll forever.
        self.stop();
    }
}

/// Where the reader is in the file, and what it has fed so far.
struct Reader {
    /// The next byte to read; `None` before the file was first opened.
    pos: Option<u64>,
    /// Drop bytes up to the first newline (a tail starting mid-file).
    skip_line: bool,
    /// The last byte fed was a CR, so a LF that starts the next chunk is
    /// already a CRLF.
    cr: bool,
    /// The last raw bytes read, and how many were read in all.
    last: Vec<u8>,
    total: u64,
}

impl Reader {
    fn new() -> Reader {
        Reader {
            pos: None,
            skip_line: false,
            cr: false,
            last: Vec::new(),
            total: 0,
        }
    }

    /// Reads what the file gained since the last poll. The file is opened
    /// and closed every poll: a handle held across polls would keep a
    /// deleted file's name in delete-pending on Windows, where it reads as
    /// "access denied", not "not found".
    fn poll(&mut self, path: &std::path::Path) -> std::io::Result<Vec<u8>> {
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        let pos = match self.pos {
            None => {
                let back = len.saturating_sub(START_BACK);
                self.skip_line = back > 0;
                back
            }
            // Shorter than what was read: it was truncated, so start over.
            Some(p) if len < p => {
                self.cr = false;
                self.last.clear();
                self.total = 0;
                self.skip_line = false;
                0
            }
            Some(p) => p,
        };
        let want = (len - pos).min(CHUNK);
        let mut buf = Vec::with_capacity(want as usize);
        if want > 0 {
            f.seek(SeekFrom::Start(pos))?;
            f.take(want).read_to_end(&mut buf)?;
        }
        self.pos = Some(pos + buf.len() as u64);
        Ok(buf)
    }

    /// The bytes to feed for `raw`: bare LFs become CRLF (no PTY, so no
    /// ONLCR), and a tail that started mid-file drops its partial line.
    fn convert(&mut self, mut raw: &[u8]) -> Vec<u8> {
        if self.skip_line {
            match raw.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    raw = &raw[i + 1..];
                    self.skip_line = false;
                }
                None => return Vec::new(),
            }
        }
        self.total += raw.len() as u64;
        self.last.extend_from_slice(raw);
        if self.last.len() > KEEP {
            self.last.drain(..self.last.len() - KEEP);
        }
        let mut out = Vec::with_capacity(raw.len() + raw.len() / 16);
        for &b in raw {
            if b == b'\n' && !self.cr {
                out.push(b'\r');
            }
            out.push(b);
            self.cr = b == b'\r';
        }
        out
    }

    /// claude's end-of-run trailer, when the bytes read end with one.
    fn trailer(&self) -> Option<Option<i32>> {
        let start_of_file = self.total == self.last.len() as u64;
        trailer(&self.last, start_of_file)
    }
}

/// The run's end as claude writes it at the file's end: a blank line,
/// then `[exited with code N]` or `[killed]`, then a newline. A run with
/// no output leaves only `\n[exited with code N]\n`. `Some(None)` is
/// killed. `start_of_file`: `tail` holds the whole file.
pub(crate) fn trailer(tail: &[u8], start_of_file: bool) -> Option<Option<i32>> {
    let body = tail.strip_suffix(b"\n")?;
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    let nl = body.iter().rposition(|&b| b == b'\n')?;
    let (before, line) = (&body[..=nl], &body[nl + 1..]);
    // The line before the trailer is blank, or the file is just "\n".
    let blank = before.ends_with(b"\n\n")
        || before.ends_with(b"\n\r\n")
        || (start_of_file && before == b"\n");
    if !blank {
        return None;
    }
    if line == b"[killed]" {
        return Some(None);
    }
    let n = line
        .strip_prefix(b"[exited with code ")?
        .strip_suffix(b"]")?;
    let n = std::str::from_utf8(n).ok()?;
    // Windows codes like 0xC0000005 are printed unsigned.
    let code = n
        .parse::<i32>()
        .ok()
        .or_else(|| n.parse::<u32>().ok().map(|c| c as i32))?;
    Some(Some(code))
}

fn read_loop(path: &std::path::Path, s: &Shared, emit: &Emit) {
    let mut r = Reader::new();
    let mut events = Vec::new();
    // Polls in a row at EOF with nothing new.
    let mut quiet = 0u32;
    loop {
        let mut more = false;
        match r.poll(path) {
            Ok(raw) if !raw.is_empty() => {
                more = raw.len() as u64 == CHUNK;
                quiet = 0;
                let bytes = r.convert(&raw);
                if !bytes.is_empty() {
                    s.engine.lock().feed(&bytes, &mut events);
                    s.kick(|w| w.dirty = true);
                    if !dispatch(&mut events, emit) {
                        return;
                    }
                }
            }
            Ok(_) => quiet += 1,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                emit(TailOut::Msg(TermMsg::Exit { code: None }));
                return;
            }
            // Anything else (a sharing hiccup) is tried again next poll.
            Err(_) => {}
        }
        // A trailer counts once the file has stayed at it for a poll.
        if quiet >= 1
            && let Some(code) = r.trailer()
        {
            emit(TailOut::Msg(TermMsg::Exit { code }));
            return;
        }
        let mut halt = s.halt.lock();
        if *halt {
            return;
        }
        if !more {
            s.halt_cv.wait_for(&mut halt, POLL);
            if *halt {
                return;
            }
        }
    }
}

/// The engine's events, as a shell pane maps them (`suzuri:pane.rs`
/// `dispatch`). Replies to queries have no PTY to go to and are dropped.
fn dispatch(events: &mut Vec<EngineEvent>, emit: &Emit) -> bool {
    let mut alive = true;
    for ev in events.drain(..) {
        let msg = match ev {
            EngineEvent::PtyWrite(_) => continue,
            EngineEvent::Title(title) => TermMsg::Title { title },
            EngineEvent::Bell => TermMsg::Bell,
            EngineEvent::Clipboard(text) => TermMsg::Clipboard { text },
        };
        alive &= emit(TailOut::Msg(msg));
    }
    alive
}

/// A copy of suzuri's `frame_loop` (its `Shared` is private) without the
/// input window: a frame goes when the screen changed and the last one was
/// acked or 250 ms passed, at most one per 33 ms.
fn frame_loop(s: &Shared, emit: &Emit) {
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut seq = 0u32;
    let mut events = Vec::new();
    loop {
        let force_full = {
            let mut w = s.wake.lock();
            loop {
                if w.stop {
                    return;
                }
                let ack_deadline = w.sent_at + ACK_TIMEOUT;
                let waiting_ack = w.acked < w.sent && Instant::now() < ack_deadline;
                if w.dirty && !waiting_ack {
                    break;
                }
                if waiting_ack {
                    s.cv.wait_until(&mut w, ack_deadline);
                } else {
                    s.cv.wait(&mut w);
                }
            }
            w.force_full
        };
        let since = last.elapsed();
        if since < INTERVAL {
            std::thread::sleep(INTERVAL - since);
        }
        let mut out = Vec::with_capacity(16 * 1024);
        let sent = {
            let mut eng = s.engine.lock();
            // DEC 2026: hold the frame while a synchronized update is
            // open, up to its timeout.
            if let Some(deadline) = eng.sync_deadline() {
                if Instant::now() < deadline {
                    drop(eng);
                    let mut w = s.wake.lock();
                    w.dirty = false;
                    let wait = deadline.saturating_duration_since(Instant::now());
                    s.cv.wait_for(&mut w, wait);
                    w.dirty = true;
                    continue;
                }
                eng.stop_sync(&mut events);
            }
            {
                let mut w = s.wake.lock();
                w.dirty = false;
                w.force_full = false;
            }
            seq = seq.wrapping_add(1);
            eng.snapshot(&mut out, seq, force_full)
        };
        dispatch(&mut events, emit);
        if sent {
            {
                let mut w = s.wake.lock();
                w.sent = seq;
                w.sent_at = Instant::now();
            }
            emit(TailOut::Frame(out));
        }
        last = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::{Reader, trailer};

    #[test]
    fn trailers_as_claude_writes_them() {
        assert_eq!(
            trailer(b"ok\n\n[exited with code 0]\n", false),
            Some(Some(0))
        );
        assert_eq!(
            trailer(b"ok\r\n\n[exited with code 101]\n", false),
            Some(Some(101))
        );
        assert_eq!(trailer(b"ok\n\n[killed]\n", false), Some(None));
        assert_eq!(trailer(b"\n[exited with code 2]\n", true), Some(Some(2)));
        assert_eq!(trailer(b"\n[killed]\n", true), Some(None));
        assert_eq!(
            trailer(b"x\n\n[exited with code 3221225477]\n", false),
            Some(Some(3221225477u32 as i32))
        );
        assert_eq!(
            trailer(b"x\n\n[exited with code -1]\n", false),
            Some(Some(-1))
        );
        // Not a trailer: no blank line before it, or text after it.
        assert_eq!(trailer(b"x\n[exited with code 0]\n", false), None);
        assert_eq!(trailer(b"\n[exited with code 0]\n", false), None);
        assert_eq!(trailer(b"x\n\n[exited with code 0]\nmore\n", false), None);
        assert_eq!(trailer(b"x\n\n[exited with code 0]", false), None);
        assert_eq!(trailer(b"x\n\n[exited with code x]\n", false), None);
    }

    #[test]
    fn line_feeds_become_crlf_across_chunks() {
        let mut r = Reader::new();
        assert_eq!(r.convert(b"a\nb\r"), b"a\r\nb\r");
        // The CR ended the last chunk: this LF already completes a CRLF.
        assert_eq!(r.convert(b"\nc\r\n\n"), b"\nc\r\n\r\n");
    }

    #[test]
    fn a_tail_from_mid_file_drops_its_partial_line() {
        let mut r = Reader::new();
        r.skip_line = true;
        assert_eq!(r.convert(b"half"), b"");
        assert_eq!(r.convert(b" line\nwhole\n"), b"whole\r\n");
        assert_eq!(r.total, 6);
    }
}
