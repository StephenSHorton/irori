//! The chat's held session (one-assistant card 8, design §8): one `claude
//! -p` kept running on the assistant's computer, with the chat's tools, its
//! stdin left open, one stream-json user message per question. It starts
//! with the first question after Irori starts, so an unused chat costs
//! nothing.
//!
//! The process runs exactly the argv a per-question turn runs
//! ([`crate::assistant::Job::assistant`]: it already reads stream-json), so
//! its events, tools and grants are the same. A turn ends at its `result`
//! line, not at the process's end.
//!
//! Core says which session a question belongs to ([`ChatJob::session`]) and
//! gives two texts: the prompt that seeds a new session from the rows, and
//! the message that goes on with the held one. The process is kept only
//! while the key and the argv (the chat's model and effort) stay the same,
//! the turn ended well, and its context stays under [`RESEED_AT`]. A Stop
//! ends the process, as it ends a per-question run: the next question seeds
//! a new one.
//!
//! [`ChatJob::session`]: irori_core::assistant_chat::ChatJob::session

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::assistant::{
    AssistantEvent, Job, NO_CLAUDE, Parser, STDERR_CAP, STOPPED, Sink, TOO_LONG, command, failure,
    held_result, job_args, kill_tree, prepare,
};

/// Past this much context a held session ends after its turn, and the next
/// question seeds a new one from the rows (§8, as call-agents §7 does).
/// With the chat's tools a session gets here in six to nine questions
/// (measured, #1108).
pub const RESEED_AT: u64 = 60_000;
/// How long an ending session gets to exit on its own once its stdin
/// closes, before it is killed.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Why a turn was ended from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Running,
    Stopped,
    TimedOut,
}

/// One held `claude` process.
struct Proc {
    child: Arc<Mutex<Option<Child>>>,
    /// Messages for the writer thread, which owns stdin; dropped to close
    /// it.
    writer: Option<Sender<String>>,
    lines: Receiver<String>,
    errs: Arc<Mutex<Vec<u8>>>,
    err_reader: Option<JoinHandle<()>>,
    key: u64,
    argv: Vec<String>,
}

impl Proc {
    fn spawn(bin: &std::path::Path, job: &Job, key: u64) -> Result<Proc, String> {
        let mut child = match command(bin, job).spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(NO_CLAUDE.into()),
            Err(e) => return Err(failure(&format!("couldn't start Claude Code: {e}"))),
        };
        let (wtx, wrx) = mpsc::channel::<String>();
        if let Some(mut w) = child.stdin.take() {
            std::thread::spawn(move || {
                for msg in wrx {
                    if w.write_all(msg.as_bytes()).and_then(|_| w.flush()).is_err() {
                        break;
                    }
                }
                // Dropping `w` closes stdin: Claude Code ends at EOF.
            });
        }
        let (tx, rx) = mpsc::channel::<String>();
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        let errs: Arc<Mutex<Vec<u8>>> = Arc::default();
        let err_reader = child.stderr.take().map(|mut r| {
            let errs = errs.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut e = lock(&errs);
                    let room = STDERR_CAP.saturating_sub(e.len());
                    e.extend_from_slice(&buf[..n.min(room)]);
                }
            })
        });
        Ok(Proc {
            child: Arc::new(Mutex::new(Some(child))),
            writer: Some(wtx),
            lines: rx,
            errs,
            err_reader,
            key,
            argv: job_args(job),
        })
    }

    fn alive(&self) -> bool {
        lock(&self.child)
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    fn kill(&self) {
        if let Some(c) = lock(&self.child).as_mut() {
            kill_tree(c);
        }
    }

    /// Waits for the ended process; what stderr said, for a failure.
    fn reap(&mut self) -> String {
        if let Some(c) = lock(&self.child).as_mut() {
            let _ = c.wait();
        }
        if let Some(h) = self.err_reader.take() {
            let _ = h.join();
        }
        String::from_utf8_lossy(&lock(&self.errs))
            .trim()
            .to_string()
    }

    /// Ends it off this thread: closes stdin so Claude Code exits on its
    /// own, and kills it if it hasn't within [`CLOSE_GRACE`].
    fn end(mut self) {
        std::thread::spawn(move || {
            drop(self.writer.take());
            let until = Instant::now() + CLOSE_GRACE;
            while self.alive() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            self.kill();
            self.reap();
        });
    }
}

/// The turn running now, for [`HeldChat::stop`].
#[derive(Clone)]
struct Running {
    child: Arc<Mutex<Option<Child>>>,
    ending: Arc<Mutex<Ending>>,
}

impl Running {
    fn end(&self, why: Ending) {
        {
            let mut e = lock(&self.ending);
            if *e == Ending::Running {
                *e = why;
            }
        }
        if let Some(c) = lock(&self.child).as_mut() {
            kill_tree(c);
        }
    }
}

#[derive(Default)]
struct State {
    /// The held process between turns.
    idle: Option<Proc>,
    running: Option<Running>,
    closed: bool,
    /// Bumped by [`HeldChat::end`]: a turn that began before keeps no
    /// process.
    epoch: u64,
}

/// The chat's held session: at most one process, one turn at a time (Core's
/// worker runs one question at a time).
#[derive(Default)]
pub struct HeldChat {
    st: Arc<Mutex<State>>,
    starts: AtomicU64,
}

impl HeldChat {
    pub fn new() -> HeldChat {
        HeldChat::default()
    }

    /// Runs one question: `message` on the held process when it belongs to
    /// session `key` and runs `job`'s argv, otherwise a new process seeded
    /// with `job.prompt` (both one stream-json line). `None` always seeds.
    /// Returns at once; the events, ending with one `done` or `error`, come
    /// to `sink`, the end only once the process is kept or ended, so the
    /// next question finds it.
    pub fn start_turn(
        &self,
        bin: Result<PathBuf, String>,
        job: Result<Job, String>,
        key: u64,
        message: Option<String>,
        sink: Sink,
    ) {
        let fail = |s: &str| {
            sink(AssistantEvent::error(s));
        };
        let Ok(bin) = bin else {
            return fail(NO_CLAUDE);
        };
        let job = match job {
            Ok(j) => j,
            Err(e) => return fail(&failure(&e)),
        };
        let (old, epoch) = {
            let mut st = lock(&self.st);
            if st.closed {
                drop(st);
                return fail(STOPPED);
            }
            (st.idle.take(), st.epoch)
        };
        let argv = job_args(&job);
        let held = match (old, message) {
            (Some(p), Some(m)) if p.key == key && p.argv == argv && p.alive() => Some((p, m)),
            (Some(p), _) => {
                p.end();
                None
            }
            (None, _) => None,
        };
        let (mut proc, line) = match held {
            Some(pm) => pm,
            None => {
                if let Err(e) = prepare(&job) {
                    return fail(&failure(&e));
                }
                match Proc::spawn(&bin, &job, key) {
                    Ok(p) => {
                        self.starts.fetch_add(1, Ordering::SeqCst);
                        (p, job.prompt.clone())
                    }
                    Err(e) => return fail(&e),
                }
            }
        };
        let run = Running {
            child: proc.child.clone(),
            ending: Arc::new(Mutex::new(Ending::Running)),
        };
        {
            let mut st = lock(&self.st);
            if st.closed || st.epoch != epoch {
                drop(st);
                proc.end();
                return fail(STOPPED);
            }
            st.running = Some(run.clone());
        }
        let st = self.st.clone();
        let look = job.look.clone();
        let timeout = job.timeout;
        std::thread::spawn(move || {
            // Nothing of an earlier turn is read as this one's.
            while proc.lines.try_recv().is_ok() {}
            let sent = proc.writer.as_ref().is_some_and(|w| w.send(line).is_ok());
            let deadline = Instant::now() + timeout;
            let mut p = Parser::looking(&look);
            let mut end: Option<AssistantEvent> = None;
            let mut context = 0u64;
            loop {
                // Not written: the process has gone, and why is said below.
                if !sent {
                    break;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                match proc.lines.recv_timeout(left) {
                    Ok(l) => {
                        if *lock(&run.ending) != Ending::Running {
                            continue; // drain to its end
                        }
                        let Some(ev) = p.line(&l) else { continue };
                        if ev.ends() {
                            if let Some(Ok(look)) = held_result(&l) {
                                context = look.context;
                            }
                            end = Some(ev);
                            break;
                        }
                        if !sink(ev) {
                            run.end(Ending::Stopped);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        run.end(Ending::TimedOut);
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            let ending = *lock(&run.ending);
            let done = end.as_ref().is_some_and(|e| e.kind == "done");
            let keep = done && ending == Ending::Running && context < RESEED_AT && proc.alive();
            let last = match end {
                Some(ev) if ending == Ending::Running => ev,
                _ => AssistantEvent::error(&match ending {
                    Ending::Stopped => STOPPED.to_string(),
                    Ending::TimedOut => TOO_LONG.to_string(),
                    Ending::Running => {
                        proc.kill();
                        let e = proc.reap();
                        failure(if e.is_empty() { "it stopped" } else { &e })
                    }
                }),
            };
            // Kept or ended before the end is told, so the next question
            // finds the process.
            {
                let mut s = lock(&st);
                if s.running
                    .as_ref()
                    .is_some_and(|r| Arc::ptr_eq(&r.ending, &run.ending))
                {
                    s.running = None;
                }
                if keep && !s.closed && s.epoch == epoch && s.idle.is_none() {
                    s.idle = Some(proc);
                } else {
                    drop(s);
                    proc.end();
                }
            }
            sink(last);
        });
    }

    /// Stops the running turn: its process ends (the next question seeds a
    /// new one) and the turn ends with [`STOPPED`].
    pub fn stop(&self) {
        if let Some(r) = lock(&self.st).running.clone() {
            r.end(Ending::Stopped);
        }
    }

    /// Ends the held process, and stops a running turn (this computer is no
    /// longer the assistant's).
    pub fn end(&self) {
        let (idle, running) = {
            let mut st = lock(&self.st);
            st.epoch += 1;
            (st.idle.take(), st.running.clone())
        };
        if let Some(r) = running {
            r.end(Ending::Stopped);
        }
        if let Some(p) = idle {
            p.end();
        }
    }

    /// Ends it and starts none after (the app is quitting).
    pub fn shutdown(&self) {
        lock(&self.st).closed = true;
        self.end();
    }

    /// How many processes it has started (tests).
    pub fn starts(&self) -> u64 {
        self.starts.load(Ordering::SeqCst)
    }

    /// Whether a process is held between turns.
    pub fn held(&self) -> bool {
        lock(&self.st).idle.is_some()
    }
}
