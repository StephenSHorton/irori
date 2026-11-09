//! The mesh summary (card #1249, design §7): this device's `agent.run`
//! rows and the fold of other devices' ones.
//!
//! **Writer.** Only the device running a run writes its rows, and only
//! for background runs (`bash`, `agent`, `workflow`; never a foreground
//! call): one `start` and one `end` per run, never CPU on a timer. It
//! wakes on [`CoreEvent::Runs`] and writes what this device's runs lack.
//! "Once each" outlives a restart: the rows this device already wrote are
//! read back from the log at start (they are in the fold's `written`).
//! A run that ended more than [`KEEP_MS`] ago is never written.
//!
//! **Reader.** [`Fold`] folds other devices' rows, copying the roster's
//! shape (`crates/coord/src/roster.rs`): replay the log, then follow the
//! store. A row counts only when its writer is the run's own device
//! (`device_of(key) == ev.device_id`); this device's own rows only teach
//! the writer what is written. It does **not** clear on `device.started`:
//! runs outlive an Irori restart. It keeps open runs and runs ended in the
//! last [`KEEP_MS`] (the registry's [`ENDED_KEEP_MS`]): an end older than
//! that drops its run, start and end alike, so a replay of the whole log
//! never grows past that. Folded runs go into the [`Runs`](super::Runs)
//! registry under their own (remote) key, where the window's join on read
//! finds them, and each change is a [`CoreEvent::Runs`].
//!
//! Never in a row: the output path, the members, live CPU or memory.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use irori_identity::owner::device_of;
use serde::{Deserialize, Serialize};
use store::{Event, Filter, kinds};

use super::{ENDED_KEEP_MS, LiveRun, MAX_PER_KEY, MAX_RUNNING, MAX_TASK_ID, RunId, RunKind};
use crate::events::{CoreEvent, Topic};
use crate::logf;
use crate::service::{Core, Services};

/// The longest command a row carries, in chars.
pub const MAX_ROW_CMD: usize = 120;
/// How long an ended run stays in the fold (and is still written).
pub const KEEP_MS: i64 = ENDED_KEEP_MS;
/// Bytes in the rows' `peakMemMb`.
const MB: u64 = 1024 * 1024;

/// Which half of a run a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Start,
    End,
}

/// One `agent.run` row's payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    /// The agent's Irori key (`<device>/<session>`), never a folder's
    /// session id.
    pub key: String,
    /// Claude's task id.
    pub run: String,
    /// `bash`, `agent` or `workflow`.
    pub kind: RunKind,
    pub op: Op,
    /// Scrubbed, at most [`MAX_ROW_CMD`] chars.
    #[serde(default)]
    pub cmd: String,
    /// Unix ms.
    #[serde(default)]
    pub started_at: i64,
    /// Unix ms; on an end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// The person pressed Stop on it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
    /// The run's peak CPU in cores (two decimals), on an end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_cores: Option<f64>,
    /// The run's peak memory in MiB, on an end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_mem_mb: Option<u64>,
    /// When the row was written, Unix ms.
    #[serde(default)]
    pub at: i64,
}

impl RunRow {
    /// `run`'s `op` row as this device writes it; `scrub` redacts the
    /// command once more.
    pub fn of(run: &LiveRun, op: Op, scrub: impl Fn(&str) -> String, now_ms: i64) -> RunRow {
        let end = op == Op::End;
        RunRow {
            key: run.key.clone(),
            run: run.task_id.clone(),
            kind: run.kind,
            op,
            cmd: cap(&scrub(&run.cmd), MAX_ROW_CMD),
            started_at: run.started_ms,
            ended_at: if end { run.ended_ms } else { None },
            exit: if end { run.exit } else { None },
            stopped: end && run.stopped_by_person,
            peak_cores: (end && run.peak_cores > 0.0)
                .then(|| (f64::from(run.peak_cores) * 100.0).round() / 100.0),
            peak_mem_mb: (end && run.peak_mem > 0).then(|| run.peak_mem.div_ceil(MB)),
            at: now_ms,
        }
    }
}

/// What this device already wrote for one of its runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Written {
    start: bool,
    /// The end row's end time, once written.
    end: Option<i64>,
}

/// The fold of the `agent.run` rows: other devices' runs, and what this
/// device wrote for its own.
#[derive(Debug, Default)]
pub struct Fold {
    me: String,
    /// Other devices' runs: open, or ended within [`KEEP_MS`].
    remote: BTreeMap<RunId, LiveRun>,
    /// This device's runs that have rows in the log.
    written: BTreeMap<RunId, Written>,
    /// Rows refused (bad payload, or written by a device that isn't the
    /// run's own).
    ignored: usize,
}

impl Fold {
    /// An empty fold for the device `me`.
    pub fn new(me: &str) -> Fold {
        Fold {
            me: me.to_string(),
            ..Fold::default()
        }
    }

    /// Folds one row at `now_ms`. Returns the run whose remote row
    /// changed (its key is the [`CoreEvent::Runs`] to publish); `None`
    /// for this device's own rows, other kinds and refused rows.
    pub fn apply(&mut self, ev: &Event, now_ms: i64) -> Option<RunId> {
        if ev.kind != kinds::AGENT_RUN || ev.is_resync() {
            return None;
        }
        let Some(row) = serde_json::from_slice::<RunRow>(&ev.payload)
            .ok()
            .filter(|r| good_id(&r.key) && good_id(&r.run) && r.kind != RunKind::Fg)
        else {
            self.ignored += 1;
            return None;
        };
        let dev = device_of(&row.key);
        let id = RunId::new(&row.key, &row.run);
        if ev.device_id == self.me {
            if dev == self.me {
                let w = self.written.entry(id).or_default();
                match row.op {
                    Op::Start => w.start = true,
                    Op::End => w.end = Some(row.ended_at.unwrap_or(row.at)),
                }
            }
            return None;
        }
        // Only the run's own device writes its rows.
        if dev.is_empty() || dev != ev.device_id {
            self.ignored += 1;
            return None;
        }
        let at = |t: i64| if t > 0 { t } else { row.at.max(1) };
        let cmd = cap(&row.cmd, MAX_ROW_CMD);
        let before = self.remote.get(&id).cloned();
        let r = self
            .remote
            .entry(id.clone())
            .or_insert_with(|| LiveRun::new(&row.key, &row.run, row.kind, at(row.started_at)));
        r.kind = row.kind;
        if r.cmd.is_empty() {
            r.cmd = cmd;
        }
        if row.started_at > 0 && row.started_at < r.started_ms {
            r.started_ms = row.started_at;
        }
        if row.op == Op::End {
            // A second end keeps the first one's time and fills the rest.
            if r.ended_ms.is_none() {
                r.ended_ms = Some(at(row.ended_at.unwrap_or(0)).max(r.started_ms));
            }
            if r.exit.is_none() {
                r.exit = row.exit;
            }
            r.stopped_by_person |= row.stopped;
            if let Some(c) = row.peak_cores {
                r.peak_cores = r.peak_cores.max(c as f32);
            }
            if let Some(mb) = row.peak_mem_mb {
                r.peak_mem = r.peak_mem.max(mb.saturating_mul(MB));
            }
            // An end past keeping drops the run, start and end alike.
            if r.ended_ms.is_some_and(|e| now_ms - e > KEEP_MS) {
                self.remote.remove(&id);
            }
        }
        let after = self.remote.get(&id);
        (before.as_ref() != after).then_some(id)
    }

    /// Drops ended runs past [`KEEP_MS`] at `now_ms`, and past the
    /// registry's per-key caps (the oldest first); returns the keys whose
    /// runs went.
    pub fn prune(&mut self, now_ms: i64) -> Vec<String> {
        let mut gone: Vec<RunId> = self
            .remote
            .values()
            .filter(|r| r.ended_ms.is_some_and(|e| now_ms - e > KEEP_MS))
            .map(LiveRun::id)
            .collect();
        let mut by_key: BTreeMap<(&str, bool), Vec<(i64, RunId)>> = BTreeMap::new();
        for r in self.remote.values() {
            let t = r.ended_ms.unwrap_or(r.started_ms);
            by_key
                .entry((r.key.as_str(), r.running()))
                .or_default()
                .push((t, r.id()));
        }
        for ((_, running), mut v) in by_key {
            let keep = if running { MAX_RUNNING } else { MAX_PER_KEY };
            if v.len() > keep {
                v.sort();
                let n = v.len() - keep;
                gone.extend(v.into_iter().take(n).map(|(_, id)| id));
            }
        }
        let mut keys: Vec<String> = Vec::new();
        for id in gone {
            if self.remote.remove(&id).is_some() && !keys.contains(&id.key) {
                keys.push(id.key);
            }
        }
        // What this device wrote for runs that ended long ago is never
        // asked again (the writer skips them).
        self.written
            .retain(|_, w| w.end.is_none_or(|e| now_ms - e <= KEEP_MS));
        keys
    }

    /// Other devices' runs the fold keeps.
    pub fn remote(&self) -> impl Iterator<Item = &LiveRun> {
        self.remote.values()
    }

    /// Rows refused so far.
    #[cfg(test)]
    pub fn ignored(&self) -> usize {
        self.ignored
    }

    /// The rows this device still owes for `run` (its own, background):
    /// `(start, end)`.
    pub fn owed(&self, run: &LiveRun, now_ms: i64) -> (bool, bool) {
        if run.kind == RunKind::Fg
            || device_of(&run.key) != self.me
            || run.ended_ms.is_some_and(|e| now_ms - e > KEEP_MS)
        {
            return (false, false);
        }
        let w = self.written.get(&run.id()).copied().unwrap_or_default();
        (!w.start, run.ended_ms.is_some() && w.end.is_none())
    }

    /// Marks `run`'s `op` row as written.
    pub fn wrote(&mut self, run: &LiveRun, op: Op) {
        let w = self.written.entry(run.id()).or_default();
        match op {
            Op::Start => w.start = true,
            Op::End => w.end = Some(run.ended_ms.unwrap_or_default()),
        }
    }
}

/// The fold shared by the reader and the writer threads.
type Shared = Arc<Mutex<Fold>>;

fn lock(f: &Shared) -> MutexGuard<'_, Fold> {
    f.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the writer and the fold: replays the log here (so the writer
/// knows what is written before it writes), then follows the store and
/// the hub on two threads that [`Core::close`] stops.
pub(crate) fn start(core: &Core) {
    let s = core.s.clone();
    let fold: Shared = Arc::new(Mutex::new(Fold::new(&s.device.id)));
    // The subscription is taken before the replay so nothing falls between.
    let sub = Arc::new(s.store.subscribe());
    let keys = rebuild(&s, &fold);
    s.publish_runs(keys);
    {
        let reg = core.on_stop(&sub, |x| x.cancel());
        let (s, fold) = (s.clone(), fold.clone());
        core.spawn("runs-fold", move || {
            let _reg = reg;
            while let Some(ev) = sub.recv() {
                let keys = if ev.is_resync() {
                    rebuild(&s, &fold)
                } else if ev.kind == kinds::AGENT_RUN {
                    fold_one(&s, &fold, &ev)
                } else {
                    continue;
                };
                s.publish_runs(keys);
            }
        });
    }
    {
        let feed = Arc::new(s.hub.subscribe(&[Topic::Runs]));
        let reg = core.on_stop(&feed, |f| f.close());
        core.spawn("runs-mesh", move || {
            let _reg = reg;
            write_own(&s, &fold, None);
            while let Some(ev) = feed.recv() {
                match ev {
                    CoreEvent::Runs { keys } => write_own(&s, &fold, Some(&keys)),
                    CoreEvent::Resync => write_own(&s, &fold, None),
                    _ => {}
                }
            }
        });
    }
}

fn now_ms(s: &Services) -> i64 {
    (s.now)().unix_milli_value()
}

/// Replays every `agent.run` row into a fresh fold and mirrors the kept
/// remote runs into the registry; returns their keys.
fn rebuild(s: &Services, fold: &Shared) -> Vec<String> {
    let now = now_ms(s);
    let mut fresh = Fold::new(&s.device.id);
    let mut f = Filter {
        kinds: vec![kinds::AGENT_RUN.into()],
        limit: store::MAX_LIST_LIMIT,
        ..Filter::default()
    };
    loop {
        let evs = match s.store.list(&f) {
            Ok(evs) => evs,
            Err(e) => {
                logf(format_args!("runs: reading agent.run rows: {e}"));
                return Vec::new();
            }
        };
        for ev in &evs {
            fresh.apply(ev, now);
        }
        if (evs.len() as i64) < f.limit {
            break;
        }
        f.after = evs[evs.len() - 1].cursor();
    }
    fresh.prune(now);
    let mut keys: Vec<String> = Vec::new();
    for r in fresh.remote() {
        mirror(s, r, now);
        if !keys.contains(&r.key) {
            keys.push(r.key.clone());
        }
    }
    *lock(fold) = fresh;
    keys
}

/// Folds one row; returns the keys to publish.
fn fold_one(s: &Services, fold: &Shared, ev: &Event) -> Vec<String> {
    let now = now_ms(s);
    let mut f = lock(fold);
    let Some(id) = f.apply(ev, now) else {
        return Vec::new();
    };
    let mut keys = f.prune(now);
    if let Some(r) = f.remote.get(&id) {
        mirror(s, r, now);
    }
    if !keys.contains(&id.key) {
        keys.push(id.key);
    }
    keys
}

/// Puts a remote run into the registry when it differs.
fn mirror(s: &Services, r: &LiveRun, now: i64) {
    if s.runs.get(&r.id()).as_ref() != Some(r) {
        s.runs.insert(r.clone(), now);
    }
}

/// Writes the rows this device's runs (of `keys`, or all) still owe.
fn write_own(s: &Services, fold: &Shared, keys: Option<&[String]>) {
    let me = s.device.id.as_str();
    let rows: Vec<LiveRun> = match keys {
        Some(keys) => keys
            .iter()
            .filter(|k| device_of(k) == me)
            .flat_map(|k| s.runs.by_key(k))
            .collect(),
        None => s
            .runs
            .all()
            .into_iter()
            .filter(|r| device_of(&r.key) == me)
            .collect(),
    };
    for r in rows {
        let now = now_ms(s);
        // Only this thread writes, so the lock needn't span the append.
        let (start, end) = lock(fold).owed(&r, now);
        for (owed, op) in [(start, Op::Start), (end, Op::End)] {
            if !owed {
                continue;
            }
            let row = RunRow::of(&r, op, |c| s.scrubber.string(c), now);
            match s.store.append("irori", kinds::AGENT_RUN, &row) {
                Ok(_) => lock(fold).wrote(&r, op),
                Err(e) => {
                    logf(format_args!("runs: writing agent.run: {e}"));
                    return;
                }
            }
        }
    }
}

impl Core {
    /// The name of the device a run's agent is on, for a run on another
    /// device (`on Mac`); `None` for this device's own runs.
    pub fn run_device(&self, key: &str) -> Option<String> {
        let dev = device_of(key);
        (!dev.is_empty() && dev != self.s.device.id).then(|| self.device_label(dev))
    }
}

fn good_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_TASK_ID && s.bytes().all(|b| b.is_ascii_graphic())
}

/// `s` cut to at most `max` chars.
fn cap(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const ME: &str = "pc";
    const MAC: &str = "mac";

    fn row(key: &str, run: &str, op: Op, ended: Option<i64>, exit: Option<i32>) -> RunRow {
        RunRow {
            key: key.into(),
            run: run.into(),
            kind: RunKind::Bash,
            op,
            cmd: "cargo test".into(),
            started_at: NOW - 60_000,
            ended_at: ended,
            exit,
            stopped: false,
            peak_cores: (op == Op::End).then_some(12.5),
            peak_mem_mb: (op == Op::End).then_some(5800),
            at: ended.unwrap_or(NOW - 60_000),
        }
    }

    fn ev(device: &str, seq: i64, r: &RunRow) -> Event {
        Event {
            device_id: device.into(),
            seq,
            kind: kinds::AGENT_RUN.into(),
            actor: "irori".into(),
            payload: serde_json::to_vec(r).unwrap(),
            ..Event::default()
        }
    }

    #[test]
    fn a_remote_start_then_end_folds_with_its_code_and_peaks() {
        let mut f = Fold::new(ME);
        let k = "mac/sess-1";
        let id = f.apply(&ev(MAC, 1, &row(k, "b1", Op::Start, None, None)), NOW);
        assert_eq!(id, Some(RunId::new(k, "b1")));
        let r = f.remote().next().unwrap();
        assert!(r.running());
        assert_eq!((r.cmd.as_str(), r.started_ms), ("cargo test", NOW - 60_000));
        // A repeat changes nothing.
        assert_eq!(
            f.apply(&ev(MAC, 2, &row(k, "b1", Op::Start, None, None)), NOW),
            None
        );
        f.apply(
            &ev(MAC, 3, &row(k, "b1", Op::End, Some(NOW), Some(101))),
            NOW,
        );
        let r = f.remote().next().unwrap();
        assert_eq!((r.ended_ms, r.exit), (Some(NOW), Some(101)));
        assert_eq!(r.peak_cores, 12.5);
        assert_eq!(r.peak_mem, 5800 * MB);
        assert!(r.output_path.is_none());
    }

    #[test]
    fn a_row_from_the_wrong_device_is_ignored() {
        let mut f = Fold::new(ME);
        // The Mac writes a row for a run on a third device, and for one of
        // this device's own runs.
        for (seq, k) in [(1, "phone/s"), (2, "pc/s"), (3, "irori"), (4, "mac")] {
            assert_eq!(
                f.apply(&ev(MAC, seq, &row(k, "b1", Op::Start, None, None)), NOW),
                None,
                "{k}"
            );
        }
        assert_eq!(f.remote().count(), 0);
        // A key with no device is refused too; the bad ones are counted.
        assert_eq!(f.ignored(), 4);
        // A broken payload and a foreground kind are refused.
        let mut bad = ev(MAC, 5, &row("mac/s", "b1", Op::Start, None, None));
        bad.payload = b"{\"key\":1}".to_vec();
        assert_eq!(f.apply(&bad, NOW), None);
        let mut fg = row("mac/s", "b1", Op::Start, None, None);
        fg.kind = RunKind::Fg;
        assert_eq!(f.apply(&ev(MAC, 6, &fg), NOW), None);
        assert_eq!(f.remote().count(), 0);
    }

    #[test]
    fn own_rows_teach_the_writer_and_are_not_folded() {
        let mut f = Fold::new(ME);
        let mut run = LiveRun::new("pc/s", "b1", RunKind::Bash, NOW - 1000);
        assert_eq!(f.owed(&run, NOW), (true, false));
        f.apply(&ev(ME, 1, &row("pc/s", "b1", Op::Start, None, None)), NOW);
        assert_eq!(f.remote().count(), 0);
        assert_eq!(f.owed(&run, NOW), (false, false));
        run.ended_ms = Some(NOW);
        assert_eq!(f.owed(&run, NOW), (false, true));
        f.wrote(&run, Op::End);
        assert_eq!(f.owed(&run, NOW), (false, false));
        // Foreground calls, other devices' runs and long-ended runs owe nothing.
        let fg = LiveRun::new("pc/s", "toolu_1", RunKind::Fg, NOW);
        assert_eq!(f.owed(&fg, NOW), (false, false));
        let theirs = LiveRun::new("mac/s", "b9", RunKind::Bash, NOW);
        assert_eq!(f.owed(&theirs, NOW), (false, false));
        let mut old = LiveRun::new("pc/s", "b2", RunKind::Bash, NOW - KEEP_MS - 9);
        old.ended_ms = Some(NOW - KEEP_MS - 1);
        assert_eq!(f.owed(&old, NOW), (false, false));
    }

    #[test]
    fn old_ended_runs_are_dropped_and_open_ones_stay() {
        let mut f = Fold::new(ME);
        let k = "mac/s";
        // A replay of an old pair never keeps it.
        let mut old = row(k, "old", Op::Start, None, None);
        old.started_at = NOW - KEEP_MS - 9_000;
        f.apply(&ev(MAC, 1, &old), NOW);
        assert_eq!(f.remote().count(), 1, "open until its end");
        old.op = Op::End;
        old.ended_at = Some(NOW - KEEP_MS - 1);
        old.exit = Some(0);
        f.apply(&ev(MAC, 2, &old), NOW);
        assert_eq!(f.remote().count(), 0, "an old end drops the pair");
        // A recent end and a long-open run stay.
        f.apply(&ev(MAC, 3, &row(k, "recent", Op::Start, None, None)), NOW);
        f.apply(
            &ev(
                MAC,
                4,
                &row(k, "recent", Op::End, Some(NOW - 1000), Some(1)),
            ),
            NOW,
        );
        let mut open = row(k, "open", Op::Start, None, None);
        open.started_at = NOW - 10 * KEEP_MS;
        f.apply(&ev(MAC, 5, &open), NOW);
        f.prune(NOW);
        let ids: Vec<&str> = f.remote().map(|r| r.task_id.as_str()).collect();
        assert_eq!(ids, ["open", "recent"]);
        // A day on, the recent one goes too and its key is reported.
        assert_eq!(f.prune(NOW + KEEP_MS), vec![k.to_string()]);
        let ids: Vec<&str> = f.remote().map(|r| r.task_id.as_str()).collect();
        assert_eq!(ids, ["open"]);
        // This device's written marks for long-ended runs go as well.
        let mut mine = LiveRun::new("pc/s", "b1", RunKind::Bash, NOW - 5000);
        mine.ended_ms = Some(NOW - 4000);
        f.wrote(&mine, Op::Start);
        f.wrote(&mine, Op::End);
        f.prune(NOW + KEEP_MS);
        assert!(f.written.is_empty());
    }

    #[test]
    fn per_key_caps_hold() {
        let mut f = Fold::new(ME);
        let k = "mac/s";
        for i in 0..(MAX_RUNNING + 3) {
            let mut r = row(k, &format!("b{i:03}"), Op::Start, None, None);
            r.started_at = NOW - 10_000 + i as i64;
            f.apply(&ev(MAC, i as i64 + 1, &r), NOW);
        }
        f.prune(NOW);
        assert_eq!(f.remote().count(), MAX_RUNNING);
        assert!(!f.remote().any(|r| r.task_id == "b000"), "the oldest went");
    }

    #[test]
    fn the_row_carries_no_path_and_a_capped_scrubbed_command() {
        let mut run = LiveRun::new("pc/s", "b1", RunKind::Bash, NOW - 5000);
        run.cmd = format!("deploy --token sekrit {}", "x".repeat(200));
        run.output_path = Some("C:\\tmp\\tasks\\b1.output".into());
        run.ended_ms = Some(NOW);
        run.exit = Some(0);
        run.stopped_by_person = true;
        run.peak_cores = 3.456;
        run.peak_mem = 3 * MB + 1;
        let start = RunRow::of(&run, Op::Start, |c| c.replace("sekrit", "[r]"), NOW);
        assert_eq!(start.cmd.chars().count(), MAX_ROW_CMD);
        assert!(start.cmd.starts_with("deploy --token [r] "));
        assert_eq!(
            (start.ended_at, start.exit, start.stopped),
            (None, None, false)
        );
        assert_eq!((start.peak_cores, start.peak_mem_mb), (None, None));
        let end = RunRow::of(&run, Op::End, |c| c.into(), NOW);
        assert_eq!(
            (
                end.ended_at,
                end.exit,
                end.stopped,
                end.peak_cores,
                end.peak_mem_mb
            ),
            (Some(NOW), Some(0), true, Some(3.46), Some(4))
        );
        let json = serde_json::to_string(&end).unwrap();
        assert!(!json.contains("output"), "{json}");
        assert!(json.contains("\"op\":\"end\""), "{json}");
        assert!(json.contains("\"peakMemMb\":4"), "{json}");
    }
}
