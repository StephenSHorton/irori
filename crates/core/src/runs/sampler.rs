//! Process metering, tail tabs and Stop (card #1249, design §4, §5): the
//! `irori-runs` sampler thread, the read-only tail a background shell run
//! opens on its card, and [`Core::stop_run`].
//!
//! The thread wakes on every [`Topic::Runs`] and [`Topic::Board`] event
//! and at least every [`EVERY`] while this device has a run going (once a
//! minute otherwise). Each wake it:
//! - samples the process tree of every **running** run of this device that
//!   names an output file ([`sys::runmeter::TreeSampler`]), at most once
//!   per [`EVERY`]: ended runs are never sampled (a finished run with no
//!   cached holders costs a Restart Manager call per sample);
//! - keeps each run's heavy verdict ([`Heavy`]) and publishes
//!   [`crate::events::CoreEvent::Runs`] when it flips;
//! - opens a read-only tail ([`term::Terms::open_tail`]) on the card a
//!   running background shell run's agent holds, for a `b*.output` file
//!   under Claude Code's tasks folders only ([`tail_path_ok`]);
//! - closes a tail whose run has ended once its card is done, closed or
//!   gone.
//!
//! Cards are joined read-only ([`Core::run_cards`]): the private log and
//! every ready board's log through [`Core::private_task`], never a call
//! that writes an `agent.card`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{LiveRun, RunId, RunKind};
use crate::events::Topic;
use crate::service::Core;

/// How often a running run is sampled.
pub const EVERY: Duration = Duration::from_secs(2);
/// How long the thread sleeps with nothing running (events still wake it).
pub const IDLE: Duration = Duration::from_secs(60);
/// A run is heavy at this many cores on average over [`AVG_MS`] ...
pub const HEAVY_CORES: f32 = 1.0;
/// ... or this much private memory ...
pub const HEAVY_MEM: u64 = 1 << 30;
/// ... held this long.
pub const HEAVY_HOLD_MS: i64 = 10_000;
/// It stops being heavy after this long below half of both thresholds.
pub const LIGHT_HOLD_MS: i64 = 30_000;
/// The window CPU is averaged over.
pub const AVG_MS: i64 = 10_000;
/// How long a run with no card to tail on waits before looking again.
const CARD_RETRY: Duration = Duration::from_secs(10);

/// One run's heavy verdict with its hysteresis (design §6): heavy once
/// its [`AVG_MS`] CPU average is at least [`HEAVY_CORES`] or its memory at
/// least [`HEAVY_MEM`] for [`HEAVY_HOLD_MS`]; light again only after
/// [`LIGHT_HOLD_MS`] below half of both, so a chip doesn't flicker.
#[derive(Debug, Default, Clone)]
pub struct Heavy {
    /// `(at ms, cores)` readings inside the averaging window.
    cpu: Vec<(i64, f32)>,
    since: Option<i64>,
    pub heavy: bool,
}

impl Heavy {
    /// Takes one reading at `now` ms (`cores` `None` on a run's first
    /// sample); true when the verdict flipped.
    pub fn step(&mut self, now: i64, cores: Option<f32>, mem: u64) -> bool {
        if let Some(c) = cores {
            self.cpu.push((now, c));
        }
        self.cpu.retain(|&(t, _)| now - t < AVG_MS);
        let avg = if self.cpu.is_empty() {
            0.0
        } else {
            self.cpu.iter().map(|&(_, c)| c).sum::<f32>() / self.cpu.len() as f32
        };
        let leaning = if self.heavy {
            avg < HEAVY_CORES / 2.0 && mem < HEAVY_MEM / 2
        } else {
            avg >= HEAVY_CORES || mem >= HEAVY_MEM
        };
        if !leaning {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        let hold = if self.heavy {
            LIGHT_HOLD_MS
        } else {
            HEAVY_HOLD_MS
        };
        if now - since >= hold {
            self.heavy = !self.heavy;
            self.since = None;
            return true;
        }
        false
    }
}

/// Every Core's heavy runs, by the address of its services: the verdict
/// lives with the sampler thread, and the registry rows (L2's) carry no
/// field for it. A Core's entry goes when its thread ends.
static HEAVY: Mutex<BTreeMap<usize, BTreeSet<RunId>>> = Mutex::new(BTreeMap::new());

fn heavy_lock() -> std::sync::MutexGuard<'static, BTreeMap<usize, BTreeSet<RunId>>> {
    HEAVY.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether `p` is a file a tail may open (design §5.2, the lead's rule):
/// a background shell's `b<id>.output` (Bash and PowerShell alike) in a
/// `tasks` folder under Claude Code's temp folder (`…\claude\…` on
/// Windows, `/private/tmp/claude-<uid>/…` on macOS). Anything else, a
/// reported path included, is refused.
pub fn tail_path_ok(p: &Path) -> bool {
    if !p.is_absolute() {
        return false;
    }
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(id) = name
        .strip_prefix('b')
        .and_then(|n| n.strip_suffix(".output"))
    else {
        return false;
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let Some(dir) = p.parent() else { return false };
    if dir.file_name().and_then(|n| n.to_str()) != Some("tasks") {
        return false;
    }
    let mut parts = dir.components().map(|c| c.as_os_str().to_string_lossy());
    // No `..` anywhere, and some folder above is Claude Code's.
    let all: Vec<_> = parts.by_ref().collect();
    !all.iter().any(|c| c == "..")
        && all
            .iter()
            .rev()
            .skip(1)
            .any(|c| c == "claude" || c.starts_with("claude-"))
}

/// A card an agent holds, as [`Core::run_cards`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCard {
    pub id: String,
    /// The card is Working: a run shows on its holder's Working card when
    /// it has one, else on every card it holds (design §3).
    pub working: bool,
}

/// Whether agent `key` runs on device `me` (`<device>/<session>`).
fn here(key: &str, me: &str) -> bool {
    key.split_once('/').is_some_and(|(d, _)| d == me)
}

/// Picks the cards a key's runs show on: its Working ones, else all.
pub fn show_on(cards: &[RunCard]) -> Vec<String> {
    let working: Vec<String> = cards
        .iter()
        .filter(|c| c.working)
        .map(|c| c.id.clone())
        .collect();
    if working.is_empty() {
        cards.iter().map(|c| c.id.clone()).collect()
    } else {
        working
    }
}

/// Starts the sampler thread.
pub(crate) fn start(core: &Core) {
    let feed = core.feed(&[Topic::Runs, Topic::Board]);
    let q = feed.queue().clone();
    let reg = core.stop.on_cancel(move || q.close());
    let me = Sampler {
        s: core.s.clone(),
        terms: core.terms.clone(),
        heavy: HashMap::new(),
        sampler: sys::runmeter::TreeSampler::new(),
        sampled: None,
        no_card: HashMap::new(),
    };
    let stop = core.stop.clone();
    // The thread needs a Core's read-only card join; it holds the
    // services and builds that join from them.
    core.spawn("runs", move || {
        let _reg = reg;
        let mut me = me;
        let mut wait = Duration::ZERO;
        loop {
            match feed.recv_timeout(wait) {
                Ok(None) => break,
                Ok(Some(_)) | Err(()) => {}
            }
            // Drain a burst; one pass covers it.
            while feed.try_recv().is_some() {}
            if stop.is_canceled() {
                break;
            }
            wait = me.tick();
        }
        heavy_lock().remove(&me.key());
    });
}

struct Sampler {
    s: Arc<super::Services>,
    terms: Arc<term::Terms>,
    heavy: HashMap<RunId, Heavy>,
    sampler: sys::runmeter::TreeSampler,
    sampled: Option<Instant>,
    /// Runs whose agent held no card when last looked, and when.
    no_card: HashMap<RunId, Instant>,
}

impl Sampler {
    fn key(&self) -> usize {
        Arc::as_ptr(&self.s) as usize
    }

    /// One wake; returns how long to wait for the next.
    fn tick(&mut self) -> Duration {
        let me_dev = self.s.device.id.clone();
        let running: Vec<LiveRun> = self
            .s
            .runs
            .all()
            .into_iter()
            .filter(|r| r.running() && here(&r.key, &me_dev))
            .collect();
        let due = self.sampled.is_none_or(|t| t.elapsed() >= EVERY);
        if due {
            self.sample(&running);
        }
        self.open_tails(&running);
        self.close_tails();
        if running.is_empty() {
            IDLE
        } else {
            EVERY.saturating_sub(self.sampled.map_or(EVERY, |t| t.elapsed()))
        }
    }

    fn sample(&mut self, running: &[LiveRun]) {
        self.sampled = Some(Instant::now());
        let pids: HashMap<String, u32> = self
            .s
            .presence
            .snapshot()
            .into_iter()
            .map(|a| (a.key, u32::try_from(a.pid).unwrap_or(0)))
            .collect();
        // Only running shell runs (and foreground calls) have a tree of
        // their own; an agent or workflow run is claude itself.
        let mut by_path: HashMap<std::path::PathBuf, RunId> = HashMap::new();
        let mut keys = Vec::new();
        for r in running {
            if matches!(r.kind, RunKind::Agent | RunKind::Workflow) {
                continue;
            }
            let Some(p) = &r.output_path else { continue };
            if by_path.insert(p.clone(), r.id()).is_none() {
                keys.push(sys::runmeter::RunKey {
                    output: p.clone(),
                    session_pid: pids.get(&r.key).copied().unwrap_or(0),
                });
            }
        }
        let now = (self.s.now)().unix_milli_value();
        let samples = if keys.is_empty() {
            // Forgets the trees of runs that ended.
            self.sampler.sample(&[])
        } else {
            self.sampler.sample(&keys)
        };
        let mut flipped = Vec::new();
        for (path, t) in samples {
            let Some(id) = by_path.get(&path) else {
                continue;
            };
            self.s.runs.update(id, |r| {
                r.members = t.members.clone();
                r.mem_bytes = t.mem_bytes;
                r.peak_mem = r.peak_mem.max(t.mem_bytes);
                // A run's first sample only primes its CPU: no reading yet.
                if let Some(c) = t.cores {
                    r.cpu_cores = c;
                    r.peak_cores = r.peak_cores.max(c);
                    r.sampled_ms = now;
                }
            });
            if self
                .heavy
                .entry(id.clone())
                .or_default()
                .step(now, t.cores, t.mem_bytes)
            {
                flipped.push(id.key.clone());
            }
        }
        // Verdicts of runs no longer running go; one that was heavy is a
        // change too (its chip leaves).
        let live: HashSet<RunId> = running.iter().map(LiveRun::id).collect();
        self.heavy.retain(|id, h| {
            let keep = live.contains(id);
            if !keep && h.heavy {
                flipped.push(id.key.clone());
            }
            keep
        });
        let set: BTreeSet<RunId> = self
            .heavy
            .iter()
            .filter(|(_, h)| h.heavy)
            .map(|(id, _)| id.clone())
            .collect();
        heavy_lock().insert(self.key(), set);
        flipped.sort();
        flipped.dedup();
        self.s.publish_runs(flipped);
    }

    /// Opens a tail for every running background shell run of this device
    /// that has none yet, on the cards its agent holds.
    fn open_tails(&mut self, running: &[LiveRun]) {
        self.no_card
            .retain(|id, at| at.elapsed() < CARD_RETRY && running.iter().any(|r| &r.id() == id));
        let want: Vec<&LiveRun> = running
            .iter()
            .filter(|r| r.kind == RunKind::Bash && !self.no_card.contains_key(&r.id()))
            .filter(|r| r.output_path.as_deref().is_some_and(tail_path_ok))
            .collect();
        if want.is_empty() || !self.terms.supported() {
            return;
        }
        let tailed = tails(&self.terms);
        let want: Vec<&LiveRun> = want
            .into_iter()
            .filter(|r| !tailed.iter().any(|(_, _, run, _)| run == &r.task_id))
            .collect();
        if want.is_empty() {
            return;
        }
        let keys: BTreeSet<String> = want.iter().map(|r| r.key.clone()).collect();
        let cards = run_cards(&self.s, &keys);
        let mut opened = Vec::new();
        for r in want {
            let on = cards.get(&r.key).map(|c| show_on(c)).unwrap_or_default();
            if on.is_empty() {
                self.no_card.insert(r.id(), Instant::now());
                continue;
            }
            let Some(path) = r.output_path.clone() else {
                continue;
            };
            let label = if r.cmd.is_empty() {
                "a run".to_string()
            } else {
                r.cmd.clone()
            };
            for card in on {
                let spec = term::TailSpec {
                    scope: term::Scope::Card(card),
                    path: path.clone(),
                    label: label.clone(),
                    run: r.task_id.clone(),
                };
                match self.terms.open_tail(spec) {
                    Ok(_) => opened.push(r.key.clone()),
                    Err(e) => crate::logf(format_args!("runs: tail {}: {e}", r.task_id)),
                }
            }
        }
        opened.sort();
        opened.dedup();
        // The card's Terminal section lists its terms again on board:changed.
        self.s.publish_runs(opened);
    }

    /// Closes the tails whose run ended on a card that is done, closed or
    /// gone. A running tail stays until its run ends.
    fn close_tails(&self) {
        for (id, card, _, running) in tails(&self.terms) {
            if running {
                continue;
            }
            let open = self.s.route(&card).ok().and_then(|(_, co)| {
                co.get_task(&card)
                    .map(|t| !coord::is_terminal(coord::effective_state(&t)))
            });
            if open != Some(true) {
                let _ = self.terms.close(id);
            }
        }
    }
}

/// Every tail on a card: `(term, card, run, still running)`.
fn tails(terms: &term::Terms) -> Vec<(term::TermId, String, String, bool)> {
    let scopes: BTreeSet<String> = terms
        .list()
        .into_iter()
        .filter_map(|(_, s, _)| match s {
            term::Scope::Card(c) => Some(c),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for card in scopes {
        for t in terms.in_scope(&term::Scope::Card(card.clone())) {
            if let Some(tail) = t.tail {
                out.push((t.id, card.clone(), tail.run, t.running));
            }
        }
    }
    out
}

/// The open cards each of `keys` holds (as holder, co-holder or on the
/// Where line), on the private log and every ready shared board, with the
/// board's keys as this device knows them. Read only: never a call that
/// writes an `agent.card`.
fn run_cards(s: &super::Services, keys: &BTreeSet<String>) -> BTreeMap<String, Vec<RunCard>> {
    let mut out: BTreeMap<String, Vec<RunCard>> = BTreeMap::new();
    if keys.is_empty() {
        return out;
    }
    let open = coord::TaskFilter {
        open: true,
        ..Default::default()
    };
    let mut add = |t: &coord::Task| {
        let working = coord::effective_state(t) == coord::WORKING;
        let mut seen: Vec<&str> = Vec::new();
        let who = t.where_.as_ref().map(|w| w.who.as_str());
        for k in who.into_iter().chain(
            t.holders
                .iter()
                .chain(&t.co_holders)
                .map(String::as_str),
        ) {
            if keys.contains(k) && !seen.contains(&k) {
                seen.push(k);
                out.entry(k.to_string()).or_default().push(RunCard {
                    id: t.id.clone(),
                    working,
                });
            }
        }
    };
    for t in s.coord.list_tasks(&open) {
        add(&t);
    }
    for b in s.board_list() {
        if !b.ready {
            continue;
        }
        let Ok(co) = s.board(&b.board_id).map(|l| l.coord().clone()) else {
            continue;
        };
        for t in co.list_tasks(&open) {
            if t.withdrawn {
                continue;
            }
            add(&s.private_task(t));
        }
    }
    out
}

impl Core {
    /// The open cards each of `keys` holds, read only (design §3): the
    /// private log and every ready board's, its `agent:<cardId>` holders as
    /// this device's keys. Never writes an `agent.card`.
    pub fn run_cards(&self, keys: &BTreeSet<String>) -> BTreeMap<String, Vec<RunCard>> {
        run_cards(&self.s, keys)
    }

    /// This device's runs the sampler holds heavy now (design §6).
    pub fn heavy_runs(&self) -> BTreeSet<RunId> {
        heavy_lock()
            .get(&(Arc::as_ptr(&self.s) as usize))
            .cloned()
            .unwrap_or_default()
    }

    /// The person stops one of this device's runs (the window's
    /// `runs_stop`, design §5.3): marks it stopped, marks its tails
    /// Stopped, then ends the processes of its tree as last sampled (a
    /// run not sampled yet is sampled once now). Never a bare pid, never
    /// Irori or the claude session. Whatever end follows keeps "Stopped".
    pub fn stop_run(&self, key: &str, task_id: &str) -> Result<(), String> {
        let id = RunId::new(key, task_id);
        let Some(r) = self.s.runs.get(&id) else {
            return Err(format!("no run {task_id}"));
        };
        if !here(key, self.device_id()) {
            return Err("that run is on another device".into());
        }
        if !r.running() {
            return Err("that run has already ended".into());
        }
        if matches!(r.kind, RunKind::Agent | RunKind::Workflow) {
            return Err("only a shell run can be stopped here".into());
        }
        let mut members = r.members.clone();
        if members.is_empty()
            && let Some(p) = &r.output_path
        {
            let pid = self
                .s
                .presence
                .snapshot()
                .into_iter()
                .find(|a| a.key == key)
                .and_then(|a| u32::try_from(a.pid).ok())
                .unwrap_or(0);
            let mut s = sys::runmeter::TreeSampler::new();
            members = s
                .sample(&[sys::runmeter::RunKey {
                    output: p.clone(),
                    session_pid: pid,
                }])
                .into_iter()
                .next()
                .map(|(_, t)| t.members)
                .unwrap_or_default();
        }
        self.s.runs.update(&id, |r| r.stopped_by_person = true);
        self.s.runs.save_now();
        for (term, _, run, _) in tails(&self.terms) {
            if run == task_id {
                let _ = self.terms.mark_stopped(term);
            }
        }
        self.s.publish_runs(vec![key.to_string()]);
        sys::runmeter::end_tree(&members)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_turns_heavy_after_ten_seconds_and_light_after_thirty() {
        let mut h = Heavy::default();
        // First sample: no CPU reading, light memory.
        assert!(!h.step(0, None, 10));
        let mut t = 0;
        let mut flips = Vec::new();
        while t <= 60_000 {
            t += 2000;
            if h.step(t, Some(4.0), 10) {
                flips.push(t);
            }
        }
        assert_eq!(flips, vec![12_000], "heavy once over for 10 s");
        assert!(h.heavy);
        // A dip under the threshold but over half keeps it heavy.
        for _ in 0..30 {
            t += 2000;
            assert!(!h.step(t, Some(0.8), 10));
        }
        // Below half for 30 s (the 10 s average falls first).
        let mut light = None;
        for _ in 0..40 {
            t += 2000;
            if h.step(t, Some(0.0), 10) {
                light = Some(t);
                break;
            }
        }
        assert!(!h.heavy);
        assert!(light.is_some());
    }

    #[test]
    fn memory_alone_makes_a_run_heavy() {
        let mut h = Heavy::default();
        let mut t = 0;
        let mut flipped = false;
        for _ in 0..6 {
            t += 2000;
            flipped |= h.step(t, None, 2 << 30);
        }
        assert!(flipped && h.heavy);
    }

    #[test]
    fn a_short_spike_is_not_heavy() {
        let mut h = Heavy::default();
        assert!(!h.step(2000, Some(8.0), 0));
        assert!(!h.step(4000, Some(8.0), 0));
        for t in (6000..40_000).step_by(2000) {
            assert!(!h.step(t, Some(0.1), 0));
        }
        assert!(!h.heavy);
    }

    #[test]
    fn only_b_output_files_under_claudes_tasks_folders_are_tailed() {
        let ok = if cfg!(windows) {
            vec![
                r"C:\Users\x\AppData\Local\Temp\claude\284f1fee44e7a201\sess\tasks\be9ihtlp2.output",
                r"C:\Users\x\AppData\Local\Temp\claude\C--Users-x\sess\tasks\b1.output",
            ]
        } else {
            vec![
                "/private/tmp/claude-501/-Users-x-proj/sess/tasks/be9ihtlp2.output",
                "/tmp/claude/proj/sess/tasks/bx.output",
            ]
        };
        for p in ok {
            assert!(tail_path_ok(Path::new(p)), "{p}");
        }
        let root = if cfg!(windows) { r"C:\t" } else { "/t" };
        let sep = std::path::MAIN_SEPARATOR;
        let bad = [
            // An agent's or a workflow's file.
            "claude/p/s/tasks/aagent1.output",
            "claude/p/s/tasks/wflow1.output",
            // Not in a tasks folder, or not under claude's.
            "claude/p/s/other/b1.output",
            "elsewhere/p/s/tasks/b1.output",
            // Not an output file, or a name with more in it.
            "claude/p/s/tasks/b1.txt",
            "claude/p/s/tasks/b.output",
            "claude/p/s/tasks/b1.x.output",
            "claude/p/../s/tasks/b1.output",
        ];
        for b in bad {
            let p = format!("{root}{sep}{}", b.replace('/', &sep.to_string()));
            assert!(!tail_path_ok(Path::new(&p)), "{p}");
        }
        // A relative path is never taken.
        assert!(!tail_path_ok(Path::new("claude/p/s/tasks/b1.output")));
    }

    #[test]
    fn a_run_shows_on_its_working_card_else_on_all() {
        let c = |id: &str, working| RunCard {
            id: id.into(),
            working,
        };
        assert_eq!(show_on(&[c("a", false), c("b", true)]), vec!["b"]);
        assert_eq!(show_on(&[c("a", false), c("b", false)]), vec!["a", "b"]);
        assert!(show_on(&[]).is_empty());
    }
}
