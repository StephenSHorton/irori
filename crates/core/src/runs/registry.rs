//! The runs registry (card #1249, design §3): every run this device knows,
//! by [`RunId`], saved as `runs.json` in the data dir (through
//! `runs.json.tmp`, 0600) so it outlives an Irori restart: runs are
//! Claude's children, not Irori's.
//!
//! Reports may come out of order (the mod flushes unsent ones on its next
//! hello), so:
//! - an `end` for a run never started makes an ended row;
//! - a `start` for a known run only fills what the row lacks (command,
//!   output path) and never reopens an ended one;
//! - a second `end` keeps the first end's time and only fills a missing
//!   exit code;
//! - a background `start` naming a `toolUseId` drops that call's
//!   foreground row: the call was backgrounded. The run keeps the id, so
//!   a late `fg-start` or `fg` for that call is ignored;
//! - a late `start` sets the kind (an `end` alone may not know it) and an
//!   earlier start time.
//!
//! Kept: every running row, at most [`MAX_RUNNING`] per key (a report
//! starting another is refused); a key's last [`FG_KEEP`] ended foreground
//! rows; ended background rows for [`ENDED_KEEP_MS`] (the mesh fold's
//! window), at most [`MAX_PER_KEY`] of them per key. A foreground call
//! still running [`FG_STALE_MS`] after its start lost its end and goes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{LiveRun, RunId, RunKind, RunReport};
use crate::reset::write_private;

pub const FILE: &str = "runs.json";
/// The longest command kept, in chars.
pub const MAX_CMD: usize = 160;
/// The longest task id (or `toolUseId`) taken, in bytes.
pub const MAX_TASK_ID: usize = 128;
/// The longest output path taken, in bytes.
pub const MAX_OUTPUT: usize = 1024;
/// Ended foreground rows kept per key.
pub const FG_KEEP: usize = 20;
/// How long an ended background row is kept: a day.
pub const ENDED_KEEP_MS: i64 = 24 * 3600 * 1000;
/// Ended background rows kept per key at most (the oldest go first).
pub const MAX_PER_KEY: usize = 100;
/// Running rows a key may have at once; a report starting another is
/// refused, so a runaway caller can't grow `runs.json` without bound.
pub const MAX_RUNNING: usize = 100;
/// A foreground call with no end this long after its start lost its end:
/// Claude Code's shell calls time out (or go to the background) long before.
pub const FG_STALE_MS: i64 = 3600 * 1000;

/// The registry and its file.
pub struct Runs {
    path: PathBuf,
    m: Mutex<BTreeMap<RunId, LiveRun>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Event {
    Start,
    End,
    FgStart,
    Fg,
}

impl Runs {
    /// Loads `<data_dir>/runs.json`; an absent or unreadable file is no
    /// runs. Rows past keeping at `now_ms` are left out.
    pub fn new(data_dir: &Path, now_ms: i64) -> Runs {
        let path = data_dir.join(FILE);
        let rows: Vec<LiveRun> = std::fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default();
        let mut m = BTreeMap::new();
        for r in rows {
            if good_id(&r.key) && good_id(&r.task_id) {
                m.insert(r.id(), r);
            }
        }
        prune(&mut m, now_ms);
        Runs {
            path,
            m: Mutex::new(m),
        }
    }

    /// Takes one report from `key`'s session at `now_ms` and saves it.
    /// `scrub` redacts the command. `Ok(true)` when a row changed;
    /// `Err` is why the report was refused (the 400's text).
    pub fn record(
        &self,
        key: &str,
        rep: &RunReport,
        scrub: impl Fn(&str) -> String,
        now_ms: i64,
    ) -> Result<bool, String> {
        if !good_id(key) {
            return Err("no agent key".into());
        }
        let ev = match rep.event.as_str() {
            "start" => Event::Start,
            "end" => Event::End,
            "fg-start" => Event::FgStart,
            "fg" => Event::Fg,
            e => return Err(format!("event {e:?} is not start, end, fg-start or fg")),
        };
        let kind = match (ev, rep.kind.as_str()) {
            (Event::FgStart | Event::Fg, "" | "fg") => RunKind::Fg,
            (Event::FgStart | Event::Fg, k) => {
                return Err(format!("kind {k:?} on a foreground event"));
            }
            // An end that names no kind ends whatever was started.
            (Event::End, "") => RunKind::Bash,
            (_, k) => match RunKind::parse(k) {
                Some(RunKind::Fg) | None => {
                    return Err(format!("kind {k:?} is not bash, agent or workflow"));
                }
                Some(k) => k,
            },
        };
        let fg = kind == RunKind::Fg;
        let task_id = if fg {
            rep.tool_use_id.as_deref().or(rep.task_id.as_deref())
        } else {
            rep.task_id.as_deref()
        }
        .map(str::trim)
        .unwrap_or_default();
        if !good_id(task_id) {
            return Err(format!(
                "{} is not 1-{MAX_TASK_ID} printable ASCII",
                if fg { "toolUseId" } else { "taskId" }
            ));
        }
        let exit = rep.exit_code.and_then(exit_code);
        // Verbatim: the path is never rebuilt or tidied (design §2.1).
        let output = match rep.output_file.as_deref() {
            None => None,
            Some(p) if p.trim().is_empty() => None,
            Some(p) if p.len() > MAX_OUTPUT || p.contains('\0') => {
                return Err(format!("outputFile is not 1-{MAX_OUTPUT} bytes"));
            }
            Some(p) => Some(PathBuf::from(p)),
        };
        let tool_use = rep
            .tool_use_id
            .as_deref()
            .map(str::trim)
            .filter(|t| good_id(t));
        let cmd = cap(
            &scrub(
                [&rep.command, &rep.workflow_name, &rep.description]
                    .into_iter()
                    .flatten()
                    .map(|s| s.trim())
                    .find(|s| !s.is_empty())
                    .unwrap_or_default(),
            ),
            MAX_CMD,
        );
        let at = |t: i64| if t > 0 && t <= now_ms { t } else { now_ms };
        let id = RunId::new(key, task_id);

        let mut m = self.lock();
        // A backgrounded call's foreground reports are that run's, not a row.
        if fg
            && m.values().any(|r| {
                r.key == key && r.kind != RunKind::Fg && r.tool_use_id.as_deref() == Some(task_id)
            })
        {
            return Ok(false);
        }
        if matches!(ev, Event::Start | Event::FgStart)
            && !m.contains_key(&id)
            && m.values().filter(|r| r.key == key && r.running()).count() >= MAX_RUNNING
        {
            return Err(format!("already {MAX_RUNNING} runs running"));
        }
        let mut changed = false;
        if ev == Event::Start
            && let Some(tu) = tool_use
            && tu != task_id
        {
            let fg_id = RunId::new(key, tu);
            if m.get(&fg_id).is_some_and(|r| r.kind == RunKind::Fg) {
                m.remove(&fg_id);
                changed = true;
            }
        }
        match (ev, m.get_mut(&id)) {
            (Event::Start | Event::FgStart, None) => {
                let mut r = LiveRun::new(key, task_id, kind, at(rep.started_at));
                r.cmd = cmd;
                r.output_path = output;
                if !fg {
                    r.tool_use_id = tool_use.map(str::to_string);
                }
                m.insert(id, r);
                changed = true;
            }
            (Event::Start | Event::FgStart, Some(r)) => {
                // A start knows the kind; an end that came first may not.
                if ev == Event::Start && r.kind != RunKind::Fg && r.kind != kind {
                    r.kind = kind;
                    changed = true;
                }
                if rep.started_at > 0 && at(rep.started_at) < r.started_ms {
                    r.started_ms = at(rep.started_at);
                    changed = true;
                }
                if !fg && r.tool_use_id.is_none() && tool_use.is_some() {
                    r.tool_use_id = tool_use.map(str::to_string);
                    changed = true;
                }
                if r.cmd.is_empty() && !cmd.is_empty() {
                    r.cmd = cmd;
                    changed = true;
                }
                if r.output_path.is_none() && output.is_some() {
                    r.output_path = output;
                    changed = true;
                }
            }
            (Event::End | Event::Fg, None) => {
                let ended = at(rep.ended_at);
                let started = if rep.started_at > 0 {
                    at(rep.started_at).min(ended)
                } else {
                    ended
                };
                let mut r = LiveRun::new(key, task_id, kind, started);
                r.cmd = cmd;
                r.output_path = output;
                r.ended_ms = Some(ended);
                r.exit = exit;
                if !fg {
                    r.tool_use_id = tool_use.map(str::to_string);
                }
                m.insert(id, r);
                changed = true;
            }
            (Event::End | Event::Fg, Some(r)) => {
                if r.ended_ms.is_none() {
                    r.ended_ms = Some(at(rep.ended_at).max(r.started_ms));
                    changed = true;
                }
                if r.exit.is_none() && exit.is_some() {
                    r.exit = exit;
                    changed = true;
                }
                if r.cmd.is_empty() && !cmd.is_empty() {
                    r.cmd = cmd;
                    changed = true;
                }
            }
        }
        if changed {
            prune(&mut m, now_ms);
            self.save(&m);
        }
        Ok(changed)
    }

    /// Adds or replaces one row (the scan's runs) and saves.
    pub fn insert(&self, run: LiveRun, now_ms: i64) {
        let mut m = self.lock();
        m.insert(run.id(), run);
        prune(&mut m, now_ms);
        self.save(&m);
    }

    /// Changes one row in place with no save (the sampler's 2 s figures);
    /// `false` when there is no such row. Call [`Runs::save`] after a
    /// change that must outlive a restart (an end, a stop).
    pub fn update(&self, id: &RunId, f: impl FnOnce(&mut LiveRun)) -> bool {
        match self.lock().get_mut(id) {
            Some(r) => {
                f(r);
                true
            }
            None => false,
        }
    }

    /// Writes the rows to `runs.json` now.
    pub fn save_now(&self) {
        let m = self.lock();
        self.save(&m);
    }

    /// One row.
    pub fn get(&self, id: &RunId) -> Option<LiveRun> {
        self.lock().get(id).cloned()
    }

    /// `key`'s rows, oldest start first.
    pub fn by_key(&self, key: &str) -> Vec<LiveRun> {
        let mut v: Vec<LiveRun> = self
            .lock()
            .values()
            .filter(|r| r.key == key)
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.started_ms, &a.task_id).cmp(&(b.started_ms, &b.task_id)));
        v
    }

    /// Every row, by key and task id.
    pub fn all(&self) -> Vec<LiveRun> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<RunId, LiveRun>> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, m: &BTreeMap<RunId, LiveRun>) {
        let rows: Vec<&LiveRun> = m.values().collect();
        let Ok(raw) = serde_json::to_vec_pretty(&rows) else {
            return;
        };
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if let Err(e) = write_private(&tmp, &raw) {
            crate::logf(format_args!("runs: {e}"));
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            crate::logf(format_args!("runs: {e}"));
        }
    }
}

/// An exit code as an `i32`. Windows can report a crash's NTSTATUS as an
/// unsigned number (`3221225477` for 0xC0000005): that is its `i32` bits.
/// Anything else out of range is no code, and the end still counts.
fn exit_code(c: i64) -> Option<i32> {
    i32::try_from(c)
        .ok()
        .or_else(|| u32::try_from(c).ok().map(|u| u as i32))
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

/// Drops the rows past keeping (the module's rules).
fn prune(m: &mut BTreeMap<RunId, LiveRun>, now_ms: i64) {
    m.retain(|_, r| match (r.kind, r.ended_ms) {
        (RunKind::Fg, None) => now_ms - r.started_ms <= FG_STALE_MS,
        (RunKind::Fg, Some(_)) | (_, None) => true,
        (_, Some(e)) => now_ms - e <= ENDED_KEEP_MS,
    });
    let mut ended: BTreeMap<(&str, bool), Vec<(i64, RunId)>> = BTreeMap::new();
    for r in m.values() {
        if let Some(e) = r.ended_ms {
            ended
                .entry((r.key.as_str(), r.kind == RunKind::Fg))
                .or_default()
                .push((e, r.id()));
        }
    }
    let mut drop: Vec<RunId> = Vec::new();
    for ((_, fg), mut v) in ended {
        let keep = if fg { FG_KEEP } else { MAX_PER_KEY };
        if v.len() > keep {
            v.sort();
            let n = v.len() - keep;
            drop.extend(v.into_iter().take(n).map(|(_, id)| id));
        }
    }
    for id in drop {
        m.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const K: &str = "dev/sess-a";

    fn rep(json: serde_json::Value) -> RunReport {
        serde_json::from_value(json).unwrap()
    }

    fn rec(r: &Runs, json: serde_json::Value, now: i64) -> Result<bool, String> {
        r.record(K, &rep(json), |s| s.replace("sekrit", "[redacted]"), now)
    }

    fn fresh() -> (tempfile::TempDir, Runs) {
        let d = tempfile::tempdir().unwrap();
        let r = Runs::new(d.path(), NOW);
        (d, r)
    }

    #[test]
    fn a_start_then_an_end_reads_back_and_reloads() {
        let (d, r) = fresh();
        let start = serde_json::json!({"event":"start","kind":"bash","taskId":"be9ihtlp2",
            "toolUseId":"toolu_1","command":"cargo test --token sekrit",
            "outputFile":"C:\\tmp\\tasks\\be9ihtlp2.output","startedAt":NOW - 5000});
        assert_eq!(rec(&r, start.clone(), NOW), Ok(true));
        assert_eq!(rec(&r, start, NOW), Ok(false), "a repeat changes nothing");
        let got = r.by_key(K);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].cmd, "cargo test --token [redacted]");
        assert_eq!(got[0].started_ms, NOW - 5000);
        assert!(got[0].running());
        assert_eq!(
            got[0].output_path.as_deref(),
            Some(Path::new("C:\\tmp\\tasks\\be9ihtlp2.output"))
        );
        let end = serde_json::json!({"event":"end","taskId":"be9ihtlp2","status":"failed","exitCode":101});
        assert_eq!(rec(&r, end, NOW + 1000), Ok(true));
        let got = r.get(&RunId::new(K, "be9ihtlp2")).unwrap();
        assert_eq!((got.ended_ms, got.exit), (Some(NOW + 1000), Some(101)));
        // Another key's rows are its own.
        assert!(r.by_key("dev/other").is_empty());
        let again = Runs::new(d.path(), NOW + 2000);
        assert_eq!(again.all(), r.all());
    }

    #[test]
    fn out_of_order_reports_settle() {
        let (_d, r) = fresh();
        // An end first makes an ended row; the late start never reopens it.
        rec(&r, serde_json::json!({"event":"end","kind":"agent","taskId":"a1","exitCode":0,"endedAt":NOW - 10}), NOW).unwrap();
        rec(&r, serde_json::json!({"event":"start","kind":"agent","taskId":"a1","description":"review"}), NOW).unwrap();
        let a = r.get(&RunId::new(K, "a1")).unwrap();
        assert_eq!(
            (a.kind, a.ended_ms, a.exit, a.cmd.as_str()),
            (RunKind::Agent, Some(NOW - 10), Some(0), "review")
        );
        // A kill then the notification: the first end's time stays, the code fills in.
        rec(
            &r,
            serde_json::json!({"event":"start","kind":"bash","taskId":"b2","command":"sleep 30"}),
            NOW,
        )
        .unwrap();
        rec(
            &r,
            serde_json::json!({"event":"end","taskId":"b2","status":"killed"}),
            NOW + 5,
        )
        .unwrap();
        rec(
            &r,
            serde_json::json!({"event":"end","taskId":"b2","exitCode":137}),
            NOW + 9,
        )
        .unwrap();
        let b = r.get(&RunId::new(K, "b2")).unwrap();
        assert_eq!((b.ended_ms, b.exit), (Some(NOW + 5), Some(137)));
        // A future clock on the wire is now.
        rec(&r, serde_json::json!({"event":"start","kind":"workflow","taskId":"w3","workflowName":"build","startedAt":NOW + 99_999}), NOW).unwrap();
        let w = r.get(&RunId::new(K, "w3")).unwrap();
        assert_eq!((w.started_ms, w.cmd.as_str()), (NOW, "build"));
    }

    #[test]
    fn a_backgrounded_call_drops_its_foreground_row() {
        let (_d, r) = fresh();
        rec(
            &r,
            serde_json::json!({"event":"fg-start","toolUseId":"toolu_9","command":"cargo build"}),
            NOW,
        )
        .unwrap();
        assert_eq!(r.by_key(K)[0].kind, RunKind::Fg);
        rec(&r, serde_json::json!({"event":"start","kind":"bash","taskId":"bx","toolUseId":"toolu_9","command":"cargo build"}), NOW + 1).unwrap();
        let rows = r.by_key(K);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].kind, rows[0].task_id.as_str()),
            (RunKind::Bash, "bx")
        );
        // A finished foreground call with no start seen is one ended row.
        rec(&r, serde_json::json!({"event":"fg","toolUseId":"toolu_10","startedAt":NOW - 50,"endedAt":NOW,"isError":true}), NOW).unwrap();
        let f = r.get(&RunId::new(K, "toolu_10")).unwrap();
        assert_eq!(
            (f.kind, f.started_ms, f.ended_ms),
            (RunKind::Fg, NOW - 50, Some(NOW))
        );
    }

    #[test]
    fn bad_reports_are_refused() {
        let (_d, r) = fresh();
        for bad in [
            serde_json::json!({"event":"begin","kind":"bash","taskId":"b1"}),
            serde_json::json!({"event":"start","kind":"shell","taskId":"b1"}),
            serde_json::json!({"event":"start","kind":"fg","taskId":"b1"}),
            serde_json::json!({"event":"start","taskId":"b1"}),
            serde_json::json!({"event":"start","kind":"bash"}),
            serde_json::json!({"event":"start","kind":"bash","taskId":"has space"}),
            serde_json::json!({"event":"start","kind":"bash","taskId":"x".repeat(MAX_TASK_ID + 1)}),
            serde_json::json!({"event":"fg-start","kind":"bash","toolUseId":"t"}),
            serde_json::json!({"event":"fg","kind":"fg"}),
            serde_json::json!({"event":"start","kind":"bash","taskId":"b1","outputFile":"x".repeat(MAX_OUTPUT + 1)}),
        ] {
            assert!(rec(&r, bad.clone(), NOW).is_err(), "{bad}");
        }
        assert!(r.all().is_empty());
        assert!(
            r.record(
                "",
                &rep(serde_json::json!({"event":"start","kind":"bash","taskId":"b1"})),
                |s| s.into(),
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn long_commands_are_capped_on_a_char_boundary() {
        let (_d, r) = fresh();
        let long = "é".repeat(MAX_CMD + 10);
        rec(
            &r,
            serde_json::json!({"event":"start","kind":"bash","taskId":"b1","command":long}),
            NOW,
        )
        .unwrap();
        assert_eq!(r.by_key(K)[0].cmd.chars().count(), MAX_CMD);
    }

    #[test]
    fn old_rows_are_pruned() {
        let (d, r) = fresh();
        for i in 0..(FG_KEEP + 5) {
            let id = format!("toolu_{i:03}");
            rec(&r, serde_json::json!({"event":"fg","toolUseId":id,"startedAt":NOW + i as i64,"endedAt":NOW + i as i64}), NOW + 100).unwrap();
        }
        rec(
            &r,
            serde_json::json!({"event":"fg-start","toolUseId":"toolu_live"}),
            NOW + 100,
        )
        .unwrap();
        rec(
            &r,
            serde_json::json!({"event":"end","kind":"bash","taskId":"old","endedAt":NOW}),
            NOW + 100,
        )
        .unwrap();
        rec(
            &r,
            serde_json::json!({"event":"start","kind":"bash","taskId":"running","startedAt":NOW}),
            NOW + 100,
        )
        .unwrap();
        let fg: Vec<_> = r
            .by_key(K)
            .into_iter()
            .filter(|x| x.kind == RunKind::Fg)
            .collect();
        assert_eq!(
            fg.len(),
            FG_KEEP + 1,
            "the last {FG_KEEP} ended plus the running one"
        );
        assert!(r.get(&RunId::new(K, "toolu_000")).is_none());
        assert!(r.get(&RunId::new(K, "toolu_live")).is_some());
        // A day later the ended background row goes; the running one stays.
        let later = Runs::new(d.path(), NOW + ENDED_KEEP_MS + 1000);
        assert!(later.get(&RunId::new(K, "old")).is_none());
        assert!(later.get(&RunId::new(K, "running")).is_some());
    }

    #[test]
    fn a_backgrounded_call_s_late_foreground_reports_make_no_row() {
        let (_d, r) = fresh();
        // The start lands before its fg-start (a flush after a failed post).
        rec(&r, serde_json::json!({"event":"start","kind":"bash","taskId":"bq","toolUseId":"toolu_5","command":"sleep 99"}), NOW).unwrap();
        assert_eq!(
            rec(
                &r,
                serde_json::json!({"event":"fg-start","kind":"fg","toolUseId":"toolu_5","command":"sleep 99"}),
                NOW
            ),
            Ok(false)
        );
        assert_eq!(
            rec(
                &r,
                serde_json::json!({"event":"fg","kind":"fg","toolUseId":"toolu_5","endedAt":NOW}),
                NOW
            ),
            Ok(false)
        );
        let rows = r.by_key(K);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(
            (rows[0].kind, rows[0].tool_use_id.as_deref()),
            (RunKind::Bash, Some("toolu_5"))
        );
        assert!(rows[0].running());
        // Another key's call with the same id is its own.
        let other = r.record(
            "dev/sess-b",
            &rep(serde_json::json!({"event":"fg-start","toolUseId":"toolu_5"})),
            |s| s.into(),
            NOW,
        );
        assert_eq!(other, Ok(true));
    }

    #[test]
    fn a_late_start_sets_the_kind_and_the_start_time() {
        let (_d, r) = fresh();
        // TaskStop's end for an id with no known prefix names no kind.
        rec(
            &r,
            serde_json::json!({"event":"end","taskId":"x7","status":"killed","endedAt":NOW}),
            NOW,
        )
        .unwrap();
        assert_eq!(r.by_key(K)[0].kind, RunKind::Bash);
        rec(&r, serde_json::json!({"event":"start","kind":"agent","taskId":"x7","startedAt":NOW - 900,"description":"review"}), NOW).unwrap();
        let x = r.get(&RunId::new(K, "x7")).unwrap();
        assert_eq!(
            (x.kind, x.started_ms, x.ended_ms, x.cmd.as_str()),
            (RunKind::Agent, NOW - 900, Some(NOW), "review")
        );
    }

    #[test]
    fn windows_crash_codes_are_kept_and_others_never_lose_the_end() {
        let (_d, r) = fresh();
        rec(&r, serde_json::json!({"event":"end","kind":"bash","taskId":"b1","exitCode":3_221_225_477_i64}), NOW).unwrap();
        assert_eq!(
            r.get(&RunId::new(K, "b1")).unwrap().exit,
            Some(0xC000_0005_u32 as i32)
        );
        rec(
            &r,
            serde_json::json!({"event":"end","kind":"bash","taskId":"b2","exitCode":-1}),
            NOW,
        )
        .unwrap();
        assert_eq!(r.get(&RunId::new(K, "b2")).unwrap().exit, Some(-1));
        rec(&r, serde_json::json!({"event":"end","kind":"bash","taskId":"b3","exitCode":5_000_000_000_i64}), NOW).unwrap();
        let b3 = r.get(&RunId::new(K, "b3")).unwrap();
        assert_eq!((b3.running(), b3.exit), (false, None));
    }

    #[test]
    fn the_output_path_is_kept_verbatim() {
        let (_d, r) = fresh();
        let p = "/private/tmp/claude-501/x/s/tasks/b1.output ";
        rec(
            &r,
            serde_json::json!({"event":"start","kind":"bash","taskId":"b1","outputFile":p}),
            NOW,
        )
        .unwrap();
        assert_eq!(r.by_key(K)[0].output_path.as_deref(), Some(Path::new(p)));
        rec(
            &r,
            serde_json::json!({"event":"start","kind":"bash","taskId":"b2","outputFile":"  "}),
            NOW,
        )
        .unwrap();
        assert_eq!(r.get(&RunId::new(K, "b2")).unwrap().output_path, None);
    }

    #[test]
    fn running_rows_are_capped_and_lost_foreground_calls_expire() {
        let (d, r) = fresh();
        for i in 0..MAX_RUNNING {
            rec(
                &r,
                serde_json::json!({"event":"start","kind":"bash","taskId":format!("b{i}")}),
                NOW,
            )
            .unwrap();
        }
        assert!(
            rec(
                &r,
                serde_json::json!({"event":"start","kind":"bash","taskId":"one-more"}),
                NOW
            )
            .is_err()
        );
        assert!(
            rec(
                &r,
                serde_json::json!({"event":"fg-start","toolUseId":"toolu_x"}),
                NOW
            )
            .is_err()
        );
        // A known run still takes reports, and an end frees a place.
        assert_eq!(
            rec(
                &r,
                serde_json::json!({"event":"end","taskId":"b0","exitCode":0}),
                NOW
            ),
            Ok(true)
        );
        assert_eq!(
            rec(
                &r,
                serde_json::json!({"event":"fg-start","toolUseId":"toolu_x"}),
                NOW
            ),
            Ok(true)
        );
        assert!(r.get(&RunId::new(K, "one-more")).is_none());
        // The call's end never came: past FG_STALE_MS it goes; background runs stay.
        let later = Runs::new(d.path(), NOW + FG_STALE_MS + 1);
        assert!(later.get(&RunId::new(K, "toolu_x")).is_none());
        assert!(later.get(&RunId::new(K, "b1")).is_some_and(|b| b.running()));
    }
}
