//! The tasks-folder scan (card #1249, design §2.2): the runs of sessions
//! with no mod, foreground until proven background, and the re-check of
//! saved runs on Core start.
//!
//! Claude Code writes every run's output to
//! `<tmp>/claude/<project>/<session id>/tasks/<id>.output` (`b*` shell,
//! `a*` agent, `w*` workflow). Every [`TICK`] the scan:
//!
//! 1. Reads each live agent's transcripts forward from a saved offset (the
//!    main one, `AgentCard.transcript`, and its `subagents/**.jsonl`): which
//!    task ids went to the background (`toolUseResult.backgroundTaskId`, or
//!    the "Command running in background with ID: …" text, whose path also
//!    teaches the tasks folder), which ended (`<task-notification>` blocks),
//!    and which shell calls still wait on their result.
//! 2. Maps tasks folders to live agent keys, in order: learned (a run's
//!    reported output path, from the mod or the transcript), process
//!    ancestry (a held file's holder is a child of the agent's claude),
//!    then the folder's name equal to a live key's session id. A resumed
//!    session writes under its ORIGINAL session id while its transcript is
//!    filed under the new one, so the name alone never wins over the other
//!    two. Unmapped folders are never read.
//! 3. Follows the files of mapped folders created after their agent's
//!    session started: a `b*` file is a foreground row until the transcript
//!    proves it background or it outlives its call; `a*`/`w*` are
//!    background rows; a row ends on a notification, on its file vanishing,
//!    or on the `[exited with code N]` / `[killed]` last line once the file
//!    stopped growing and nothing holds it.
//!
//! On the first tick (Core start) every saved run still open is re-checked:
//! a trailer with no holders, a vanished file, a tree that is gone, or an
//! agent that is gone ends it.
//!
//! Nothing here reads a run's output beyond its last [`TRAILER_TAIL`]
//! bytes, and nothing from a file goes into a row.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{MAX_CMD, RunId, RunKind, RunReport, Runs};
use crate::logf;
use crate::service::Core;

/// How often the scan looks.
pub const TICK: Duration = Duration::from_secs(5);
/// How much of a main transcript's end the first read takes.
const FIRST_READ: u64 = 64 << 20;
/// The most of one transcript read in one tick.
const MAX_READ: u64 = 64 << 20;
/// The longest transcript line parsed (whole prompts are skipped).
const MAX_LINE: usize = 4 << 20;
/// How much of a file's end is read for its trailer.
const TRAILER_TAIL: u64 = 96;
/// How often one unmapped folder may be tried by ancestry.
const ANCESTRY_EVERY_MS: i64 = 30_000;
/// Unmapped folders tried by ancestry per tick at most.
const ANCESTRY_PER_TICK: usize = 4;
/// Looks with nothing holding a file, no growth and no trailer before its
/// run counts as gone.
const QUIET_TICKS: u32 = 3;
/// Runs of gone agents are re-checked every this many ticks.
const GONE_EVERY: u64 = 6;
/// Ids remembered per session at most (background, ended, commands).
const MAX_IDS: usize = 4096;
/// A shell call with no result after this many ticks is forgotten (an
/// interrupted call may never get one).
const PENDING_TICKS: u64 = 720;
/// How long the thread waits for presence's first scan before its first
/// tick.
const PRESENCE_WAIT: Duration = Duration::from_secs(60);

/// Starts the scan thread. Nothing on the phone: no Claude Code there.
pub(crate) fn start(core: &Core) {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return;
    }
    let s = core.s.clone();
    let stop = core.stop.clone();
    core.spawn("runs-scan", move || {
        // The first tick ends runs whose agent is gone: wait for presence's
        // first scan so "not listed" means gone.
        let waited = std::time::Instant::now();
        while !s.presence.watching() && waited.elapsed() < PRESENCE_WAIT {
            if stop.sleep(Duration::from_millis(250)) {
                return;
            }
        }
        let mut sc = Scanner::new(OsHost, base_roots());
        loop {
            let live = Live {
                agents: live_agents(&s.presence.snapshot()),
                known: s.presence.watching(),
            };
            let now = (s.now)().unix_milli_value();
            let scrub = |c: &str| s.scrubber.string(c);
            let keys = sc.tick(&live, &s.runs, &scrub, now);
            s.publish_runs(keys.into_iter().collect());
            if stop.sleep(TICK) {
                return;
            }
        }
    });
}

/// One live agent session on this device, as the scan needs it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Agent {
    pub key: String,
    pub session_id: String,
    /// Its claude process.
    pub pid: u32,
    /// When its session started, Unix ms; 0 when unknown.
    pub started_ms: i64,
    /// Its main transcript, when presence found one.
    pub transcript: Option<PathBuf>,
    /// Claude Code's status ("shell" while a shell call runs).
    pub status: String,
}

/// The live agents, and whether that list can be trusted to say who is
/// gone (presence has finished its first scan).
#[derive(Debug, Clone, Default)]
pub(crate) struct Live {
    pub agents: Vec<Agent>,
    pub known: bool,
}

fn live_agents(cards: &[presence::AgentCard]) -> Vec<Agent> {
    cards
        .iter()
        .filter(|a| a.status != "gone" && a.pid > 0 && !a.key.is_empty())
        .map(|a| Agent {
            key: a.key.clone(),
            session_id: a.session_id.clone(),
            pid: u32::try_from(a.pid).unwrap_or(0),
            started_ms: if a.started_at.is_zero() {
                0
            } else {
                a.started_at.unix_milli_value()
            },
            transcript: (!a.transcript.is_empty()).then(|| PathBuf::from(&a.transcript)),
            status: a.status.clone(),
        })
        .collect()
}

/// What the scan asks the OS about processes. Faked in tests.
pub(crate) trait Host {
    /// Whether [`Host::holders`] can tell (an empty answer means none).
    fn supported(&self) -> bool;
    /// The processes holding `file` open, as `(pid, start ticks)`.
    fn holders(&self, file: &Path) -> Vec<(u32, u64)>;
    /// Each pid's parent pid, for the pids the OS knows.
    fn parents(&self, pids: &[u32]) -> HashMap<u32, u32>;
    /// When `pid` started, as FILETIME ticks.
    fn start_ticks(&self, pid: u32) -> Option<u64>;
    /// Whether the process `(pid, start ticks)` still runs.
    fn alive(&self, member: (u32, u64)) -> bool {
        self.start_ticks(member.0)
            .is_some_and(|t| t.abs_diff(member.1) <= START_SLACK)
    }
}

/// Start times read twice may differ by rounding: 1 ms.
const START_SLACK: u64 = 10_000;

/// The real OS: Restart Manager holders (sys::runmeter), a hidden
/// `powershell`/`ps` for parents.
pub(crate) struct OsHost;

impl Host for OsHost {
    fn supported(&self) -> bool {
        sys::runmeter::SUPPORTED
    }
    fn holders(&self, file: &Path) -> Vec<(u32, u64)> {
        sys::runmeter::holders(file)
    }
    fn parents(&self, pids: &[u32]) -> HashMap<u32, u32> {
        os_parents(pids)
    }
    fn start_ticks(&self, pid: u32) -> Option<u64> {
        if !sys::proc::alive(i64::from(pid)) {
            return None;
        }
        sys::proc::start_time(i64::from(pid)).map(sys::proc::filetime_ticks)
    }
}

/// Parents of `pids`. Core has no process API of its own for another
/// process's parent, so this asks once per mapping attempt (never per
/// tick): `Get-CimInstance` on Windows, `ps` elsewhere.
fn os_parents(pids: &[u32]) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    if pids.is_empty() {
        return out;
    }
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let filter = pids
            .iter()
            .map(|p| format!("ProcessId={p}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let mut c = std::process::Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process -Filter '{filter}' | ForEach-Object {{ \"$($_.ProcessId) $($_.ParentProcessId)\" }}"
            ),
        ]);
        // CREATE_NO_WINDOW, as prwatch's gh: Irori is a GUI app.
        c.creation_flags(0x0800_0000);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut c = std::process::Command::new("ps");
        c.args(["-o", "pid=,ppid=", "-p", &list]);
        c
    };
    let Ok(o) = cmd.stdin(std::process::Stdio::null()).output() else {
        return out;
    };
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        let mut it = line.split_whitespace().map(str::parse::<u32>);
        if let (Some(Ok(p)), Some(Ok(pp))) = (it.next(), it.next()) {
            out.insert(p, pp);
        }
    }
    out
}

/// Where Claude Code keeps tasks folders: `%TEMP%\claude` on Windows,
/// `/private/tmp/claude-<uid>` on macOS (`/tmp/claude-<uid>` on Linux).
/// Roots of reported output paths are added as they are learned.
fn base_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    v.push(std::env::temp_dir().join("claude"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let me = std::fs::metadata(std::env::temp_dir())
            .map(|m| m.uid())
            .ok();
        for base in ["/private/tmp", "/tmp"] {
            let Ok(rd) = std::fs::read_dir(base) else {
                continue;
            };
            for e in rd.flatten() {
                let ours = e
                    .metadata()
                    .is_ok_and(|m| m.is_dir() && Some(m.uid()) == me);
                if ours && e.file_name().to_string_lossy().starts_with("claude-") {
                    let p = e.path();
                    v.push(std::fs::canonicalize(&p).unwrap_or(p));
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v
}

// ---- transcripts --------------------------------------------------------

/// How a notification said a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ended {
    /// `completed`, `failed`, `killed`, or "".
    pub status: String,
    pub exit: Option<i64>,
}

/// What one live agent's transcripts said, kept across ticks.
#[derive(Debug, Default)]
struct Told {
    /// Transcript file → offset read to.
    offsets: HashMap<PathBuf, u64>,
    /// Shell calls with no result yet: tool_use id → tick first seen.
    pending: HashMap<String, u64>,
    /// The tick a shell call's result last landed.
    result_tick: u64,
    /// Task ids proven background.
    bg: HashSet<String>,
    /// Task ids a notification ended.
    ended: HashMap<String, Ended>,
    /// Shell tool_use id → its command.
    cmds: HashMap<String, String>,
    /// Bash/PowerShell tool_use ids: only their results' text can prove a
    /// run background (any other tool's output is untrusted text).
    shells: HashSet<String>,
    /// Background task id → the tool_use id that started it.
    tool_of: HashMap<String, String>,
    /// Tasks folders the transcript named.
    folders: BTreeSet<PathBuf>,
    /// TaskStop calls with no result yet.
    stops: HashSet<String>,
}

impl Told {
    fn cap(&mut self) {
        if self.bg.len() > MAX_IDS {
            self.bg.clear();
        }
        if self.ended.len() > MAX_IDS {
            self.ended.clear();
        }
        if self.cmds.len() > MAX_IDS {
            self.cmds.clear();
        }
        if self.shells.len() > MAX_IDS {
            self.shells.clear();
        }
        if self.tool_of.len() > MAX_IDS {
            self.tool_of.clear();
        }
        if self.folders.len() > MAX_IDS {
            self.folders.clear();
        }
        if self.pending.len() > MAX_IDS {
            self.pending.clear();
        }
        if self.stops.len() > MAX_IDS {
            self.stops.clear();
        }
    }

    /// Takes one transcript line.
    fn line(&mut self, line: &str, tick: u64) {
        // Cheap filters first: api-request rows carry whole prompts.
        if line.len() > MAX_LINE
            || line.starts_with("{\"type\":\"api-request")
            || !(line.contains("tool_use")
                || line.contains("tool_result")
                || line.contains("<task-notification>"))
        {
            return;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or_default();
        if ty.starts_with("api-request") {
            return;
        }
        match ty {
            "assistant" => {
                for c in content(&v) {
                    if c.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        continue;
                    }
                    let name = c.get("name").and_then(|t| t.as_str()).unwrap_or_default();
                    let Some(id) = c.get("id").and_then(|t| t.as_str()) else {
                        continue;
                    };
                    if name == "TaskStop" {
                        self.stops.insert(id.to_string());
                    }
                    if name == "Bash" || name == "PowerShell" {
                        self.pending.insert(id.to_string(), tick);
                        self.shells.insert(id.to_string());
                        let input = c.get("input");
                        let cmd = ["command", "description"]
                            .iter()
                            .filter_map(|k| input.and_then(|i| i.get(*k)).and_then(|s| s.as_str()))
                            .map(str::trim)
                            .find(|s| !s.is_empty());
                        if let Some(cmd) = cmd {
                            self.cmds.insert(id.to_string(), cmd.to_string());
                        }
                    }
                }
            }
            "user" => {
                let bg_field = v
                    .get("toolUseResult")
                    .and_then(|r| r.get("backgroundTaskId"))
                    .and_then(|t| t.as_str());
                for c in content(&v) {
                    if c.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                        continue;
                    }
                    let tu = c
                        .get("tool_use_id")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default();
                    if self.pending.remove(tu).is_some() {
                        self.result_tick = tick;
                    }
                    // Only a shell call's result says it went to the
                    // background: another tool (an MCP server, a file it
                    // printed) could print the same text or field.
                    let shell = self.shells.remove(tu);
                    let text = text_of(c.get("content"));
                    let is_error = c.get("is_error").and_then(|e| e.as_bool()) == Some(true);
                    if self.stops.remove(tu) && !is_error {
                        // TaskStop ended a run: no notification follows.
                        let id = v
                            .get("toolUseResult")
                            .and_then(|r| r.get("task_id"))
                            .and_then(|t| t.as_str())
                            .map(str::to_string)
                            .or_else(|| {
                                let rest = text
                                    .trim_start()
                                    .strip_prefix("Successfully stopped task: ")?;
                                rest.split_whitespace().next().map(str::to_string)
                            });
                        if let Some(id) = id.filter(|i| good_id(i)) {
                            self.ended.entry(id).or_insert(Ended {
                                status: "killed".into(),
                                exit: None,
                            });
                        }
                    }
                    // `run_in_background`'s text, or any backgrounded call
                    // (Ctrl+B, the timeout's auto-background) by its field.
                    let found = shell
                        .then(|| {
                            bg_from_text(&text).or_else(|| {
                                let id = bg_field?;
                                Some((id.to_string(), bg_path(&text, id).unwrap_or_default()))
                            })
                        })
                        .flatten();
                    if let Some((id, path)) = found.filter(|(i, _)| good_id(i)) {
                        if !tu.is_empty() {
                            self.tool_of.insert(id.clone(), tu.to_string());
                        }
                        if let Some(dir) = tasks_folder(Path::new(&path), &id) {
                            self.folders.insert(dir);
                        }
                        self.bg.insert(id);
                    }
                }
                if origin_kind(&v) == Some("task-notification") {
                    for c in content(&v) {
                        self.notifications(c.get("text").and_then(|t| t.as_str()).unwrap_or(""));
                    }
                    if let Some(s) = v
                        .get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        self.notifications(s);
                    }
                }
            }
            "queue-operation" => {
                if v.get("operation").and_then(|o| o.as_str()) == Some("enqueue")
                    && let Some(s) = v.get("content").and_then(|c| c.as_str())
                    && s.trim_start().starts_with("<task-notification>")
                {
                    self.notifications(s);
                }
            }
            "attachment" => {
                let a = v.get("attachment");
                let mode = a
                    .and_then(|a| a.get("commandMode"))
                    .and_then(|m| m.as_str());
                let kind = a
                    .and_then(|a| a.get("origin"))
                    .and_then(|o| o.get("kind"))
                    .and_then(|k| k.as_str());
                if (mode == Some("task-notification") || kind == Some("task-notification"))
                    && let Some(s) = a.and_then(|a| a.get("prompt")).and_then(|p| p.as_str())
                {
                    self.notifications(s);
                }
            }
            _ => {}
        }
    }

    fn notifications(&mut self, text: &str) {
        for n in parse_notifications(text) {
            if let Some(dir) = n
                .output
                .as_deref()
                .and_then(|p| tasks_folder(Path::new(p), &n.id))
            {
                self.folders.insert(dir);
            }
            // The first notification for an id wins (one can arrive as a
            // queue row and again as a prompt row).
            self.ended.entry(n.id).or_insert(n.end);
        }
    }

    /// Reads `path` forward from its offset. The first read starts at most
    /// `first_tail` from its end, on a line start.
    fn read(&mut self, path: &Path, first_tail: Option<u64>, tick: u64) {
        let Ok(mut f) = std::fs::File::open(path) else {
            return;
        };
        let Ok(len) = f.metadata().map(|m| m.len()) else {
            return;
        };
        let mut skip_partial = false;
        let off = match self.offsets.get(path) {
            Some(&o) if o <= len => o,
            Some(_) => 0, // replaced or truncated
            None => match first_tail {
                Some(t) if len > t => {
                    skip_partial = true;
                    len - t
                }
                _ => 0,
            },
        };
        if off == len {
            self.offsets.insert(path.to_path_buf(), off);
            return;
        }
        let want = (len - off).min(MAX_READ);
        let mut buf = Vec::with_capacity(want as usize);
        if f.seek(SeekFrom::Start(off)).is_err() || f.take(want).read_to_end(&mut buf).is_err() {
            return;
        }
        // Only whole lines; the rest is read next tick.
        let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
            if want == MAX_READ {
                // One line longer than a read: skip it.
                self.offsets.insert(path.to_path_buf(), off + want);
            } else {
                self.offsets.insert(path.to_path_buf(), off);
            }
            return;
        };
        let mut start = 0;
        if skip_partial {
            start = buf
                .iter()
                .position(|&b| b == b'\n')
                .map_or(buf.len(), |i| i + 1);
        }
        if start <= last_nl {
            for line in buf[start..last_nl].split(|&b| b == b'\n') {
                if let Ok(s) = std::str::from_utf8(line) {
                    self.line(s.trim_end_matches('\r'), tick);
                }
            }
        }
        self.offsets
            .insert(path.to_path_buf(), off + last_nl as u64 + 1);
    }
}

fn content(v: &serde_json::Value) -> Vec<&serde_json::Value> {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn origin_kind(v: &serde_json::Value) -> Option<&str> {
    v.get("origin")
        .or_else(|| v.get("message").and_then(|m| m.get("origin")))
        .and_then(|o| o.get("kind"))
        .and_then(|k| k.as_str())
}

fn text_of(c: Option<&serde_json::Value>) -> String {
    match c {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// A shell result's "Command running in background with ID: X. Output is
/// being written to: P.output" as `(X, P)`.
pub(crate) fn bg_from_text(text: &str) -> Option<(String, String)> {
    const HEAD: &str = "Command running in background with ID: ";
    const MID: &str = ". Output is being written to: ";
    let rest = text.trim_start().strip_prefix(HEAD)?;
    let j = rest.find(MID)?;
    let id = &rest[..j];
    if !good_id(id) {
        return None;
    }
    let p = &rest[j + MID.len()..];
    let k = p.find(".output")?;
    Some((id.to_string(), p[..k + ".output".len()].to_string()))
}

/// The "Output is being written to: P.output" path in `text` when it is
/// `id`'s file.
fn bg_path(text: &str, id: &str) -> Option<String> {
    const MARK: &str = "Output is being written to: ";
    let p = &text[text.find(MARK)? + MARK.len()..];
    let p = &p[..p.find(".output")? + ".output".len()];
    tasks_folder(Path::new(p), id).map(|_| p.to_string())
}

/// `path`'s tasks folder when it is `<…>/tasks/<id>.output`.
fn tasks_folder(path: &Path, id: &str) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let dir = path.parent()?;
    (stem == id && path.extension()? == "output" && dir.file_name()? == "tasks")
        .then(|| dir.to_path_buf())
}

/// One `<task-notification>` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub id: String,
    pub end: Ended,
    pub output: Option<String>,
}

/// Every `<task-notification>` block in `text`.
pub(crate) fn parse_notifications(text: &str) -> Vec<Notice> {
    const OPEN: &str = "<task-notification>";
    const CLOSE: &str = "</task-notification>";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(OPEN) {
        let after = &rest[i + OPEN.len()..];
        let (block, next) = match after.find(CLOSE) {
            Some(j) => (&after[..j], &after[j + CLOSE.len()..]),
            None => (after, ""),
        };
        rest = next;
        let Some(id) = tag(block, "task-id").filter(|i| good_id(i)) else {
            continue;
        };
        let summary = tag(block, "summary").unwrap_or_default();
        out.push(Notice {
            id: id.to_string(),
            end: Ended {
                status: tag(block, "status").unwrap_or_default().to_string(),
                exit: exit_in(summary),
            },
            output: tag(block, "output-file").map(str::to_string),
        });
    }
    out
}

fn tag<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let i = block.find(&open)? + open.len();
    let j = block[i..].find(&close)?;
    Some(block[i..i + j].trim())
}

/// `N` from "… exit code N …".
fn exit_in(summary: &str) -> Option<i64> {
    let i = summary.find("exit code ")? + "exit code ".len();
    let s = &summary[i..];
    let end = s
        .char_indices()
        .find(|&(k, c)| !(c.is_ascii_digit() || (k == 0 && c == '-')))
        .map_or(s.len(), |(k, _)| k);
    s[..end].parse().ok()
}

fn good_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= super::MAX_TASK_ID && s.bytes().all(|b| b.is_ascii_graphic())
}

// ---- files --------------------------------------------------------------

/// A finished shell file's last line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trailer {
    Exit(i64),
    Killed,
}

/// The trailer on the last line of `tail` (a file's end; `whole` when it is
/// the whole file). Only the last line counts: a program can print the
/// same text earlier.
pub(crate) fn parse_trailer(tail: &[u8], whole: bool) -> Option<Trailer> {
    let s = tail.strip_suffix(b"\n")?;
    let s = s.strip_suffix(b"\r").unwrap_or(s);
    let line = match s.iter().rposition(|&b| b == b'\n') {
        Some(i) => &s[i + 1..],
        None if whole => s,
        None => return None,
    };
    let line = std::str::from_utf8(line).ok()?;
    if line == "[killed]" {
        return Some(Trailer::Killed);
    }
    let n = line.strip_prefix("[exited with code ")?.strip_suffix(']')?;
    n.parse().ok().map(Trailer::Exit)
}

fn read_trailer(path: &Path) -> Option<Trailer> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(TRAILER_TAIL);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    f.take(TRAILER_TAIL).read_to_end(&mut buf).ok()?;
    parse_trailer(&buf, from == 0)
}

fn ms_of(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A file's length and creation time (its mtime where the OS has no
/// creation time); `None` when it is gone.
fn stat(path: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.created().or_else(|_| m.modified()).map_or(0, ms_of);
    Some((m.len(), t))
}

/// The kind a task file's prefix names.
fn kind_of(id: &str) -> Option<RunKind> {
    match id.as_bytes().first()? {
        b'b' => Some(RunKind::Bash),
        b'a' => Some(RunKind::Agent),
        b'w' => Some(RunKind::Workflow),
        _ => None,
    }
}

fn kind_name(k: RunKind) -> &'static str {
    match k {
        RunKind::Bash => "bash",
        RunKind::Agent => "agent",
        RunKind::Workflow => "workflow",
        RunKind::Fg => "fg",
    }
}

/// One task file the scan follows.
#[derive(Debug, Clone)]
struct Followed {
    key: String,
    id: String,
    /// Its prefix's kind.
    kind: RunKind,
    created_ms: i64,
    /// Its length at the last look.
    len: Option<u64>,
    /// The tick it was first seen.
    seen_tick: u64,
    /// Looks in a row with no growth, no holders and no trailer.
    quiet: u32,
    /// Nothing more to do (ended, ignored or old).
    done: bool,
}

#[derive(Debug, Clone, Default)]
struct Listed {
    mtime: Option<SystemTime>,
    names: Vec<String>,
}

// ---- the scanner --------------------------------------------------------

pub(crate) struct Scanner<H: Host> {
    host: H,
    roots: BTreeSet<PathBuf>,
    tick: u64,
    told: HashMap<String, Told>,
    /// tasks folder → key, by process ancestry.
    ancestry: HashMap<PathBuf, String>,
    /// tasks folder → last ancestry try (Unix ms) and what it asked about
    /// (the holders and the live agents' pids, hashed): the same question
    /// is not asked of the OS again.
    tried: HashMap<PathBuf, (i64, u64)>,
    /// project folder → (mtime, session folders).
    projects: HashMap<PathBuf, (Option<SystemTime>, Vec<PathBuf>)>,
    /// tasks folder → its last listing.
    listed: HashMap<PathBuf, Listed>,
    /// task file → what the scan knows of it.
    files: HashMap<PathBuf, Followed>,
    /// Lengths of saved runs' files at the last gone-agent check.
    row_lens: HashMap<RunId, u64>,
    /// When a key was first seen with no session start, Unix ms.
    first_seen: HashMap<String, i64>,
}

type Scrub<'a> = &'a dyn Fn(&str) -> String;

impl<H: Host> Scanner<H> {
    pub(crate) fn new(host: H, roots: Vec<PathBuf>) -> Scanner<H> {
        Scanner {
            host,
            roots: roots.into_iter().collect(),
            tick: 0,
            told: HashMap::new(),
            ancestry: HashMap::new(),
            tried: HashMap::new(),
            projects: HashMap::new(),
            listed: HashMap::new(),
            files: HashMap::new(),
            row_lens: HashMap::new(),
            first_seen: HashMap::new(),
        }
    }

    /// One look. The first one is the Core-start re-check. Returns the keys
    /// whose runs changed.
    pub(crate) fn tick(
        &mut self,
        live: &Live,
        runs: &Runs,
        scrub: Scrub<'_>,
        now_ms: i64,
    ) -> BTreeSet<String> {
        self.tick += 1;
        let first = self.tick == 1;
        let mut changed = BTreeSet::new();
        let agents: BTreeMap<&str, &Agent> =
            live.agents.iter().map(|a| (a.key.as_str(), a)).collect();
        self.first_seen
            .retain(|k, _| agents.contains_key(k.as_str()));
        for a in &live.agents {
            self.first_seen.entry(a.key.clone()).or_insert(now_ms);
        }

        // 1. Transcripts, before any file is looked at.
        self.told.retain(|k, _| agents.contains_key(k.as_str()));
        for a in &live.agents {
            self.read_transcripts(a);
        }

        // 2. Folders.
        let rows = runs.all();
        let mut learned: HashMap<PathBuf, String> = HashMap::new();
        for r in &rows {
            if r.kind == RunKind::Fg || !agents.contains_key(r.key.as_str()) {
                continue;
            }
            if let Some(dir) = r
                .output_path
                .as_deref()
                .and_then(|p| tasks_folder(p, &r.task_id))
            {
                learned.insert(dir, r.key.clone());
            }
        }
        for (key, t) in &self.told {
            for dir in &t.folders {
                learned.insert(dir.clone(), key.clone());
            }
        }
        for dir in learned.keys() {
            // `<root>/<project>/<session>/tasks`, where Claude Code's root
            // is `claude` or `claude-<uid>`: a reported path elsewhere
            // never makes the scan list another tree.
            if let Some(root) = dir.parent().and_then(Path::parent).and_then(Path::parent)
                && root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "claude" || n.starts_with("claude-"))
            {
                self.roots.insert(root.to_path_buf());
            }
        }
        self.ancestry.retain(|_, k| agents.contains_key(k.as_str()));
        let folders = if live.agents.is_empty() {
            Vec::new()
        } else {
            self.folders()
        };
        let by_session: HashMap<&str, &str> = live
            .agents
            .iter()
            .map(|a| (a.session_id.as_str(), a.key.as_str()))
            .collect();
        let mut mapped: Vec<(PathBuf, String)> = Vec::new();
        let mut unmapped: Vec<PathBuf> = Vec::new();
        for dir in folders {
            let key = learned
                .get(&dir)
                .or_else(|| self.ancestry.get(&dir))
                .cloned()
                .or_else(|| {
                    let name = dir.parent()?.file_name()?.to_str()?;
                    by_session.get(name).map(|k| k.to_string())
                });
            match key {
                Some(k) => mapped.push((dir, k)),
                None => unmapped.push(dir),
            }
        }
        if self.host.supported() {
            self.try_ancestry(&unmapped, &live.agents, now_ms);
        }

        // 3. Files of mapped folders.
        let keep: HashSet<&PathBuf> = mapped.iter().map(|(d, _)| d).collect();
        self.listed.retain(|d, _| keep.contains(d));
        self.files.retain(|p, f| {
            agents.contains_key(f.key.as_str())
                && p.parent().is_some_and(|d| keep.contains(&d.to_path_buf()))
        });
        for (dir, key) in &mapped {
            let Some(a) = agents.get(key.as_str()) else {
                continue;
            };
            let start = if a.started_ms > 0 {
                a.started_ms
            } else {
                self.first_seen.get(key).copied().unwrap_or(now_ms)
            };
            for name in self.list(dir) {
                let path = dir.join(&name);
                if self.files.contains_key(&path) {
                    continue;
                }
                let Some(id) = name.strip_suffix(".output").filter(|i| good_id(i)) else {
                    continue;
                };
                let Some(kind) = kind_of(id) else {
                    continue;
                };
                let Some((_, created)) = stat(&path) else {
                    continue;
                };
                self.files.insert(
                    path,
                    Followed {
                        key: key.clone(),
                        id: id.to_string(),
                        kind,
                        created_ms: created,
                        len: None,
                        seen_tick: self.tick,
                        quiet: 0,
                        // Files from before the session started are not its.
                        done: created < start,
                    },
                );
            }
        }
        let mods: HashSet<String> = rows
            .iter()
            .filter(|r| r.kind == RunKind::Fg && r.task_id.starts_with("toolu_"))
            .map(|r| r.key.clone())
            .collect();
        // A file set aside (older than its session, or no row yet) comes
        // back while a row for it runs: a run from before a resume, or a
        // start reported after the file was first seen.
        let open: HashSet<RunId> = rows
            .iter()
            .filter(|r| r.running())
            .map(|r| r.id())
            .collect();
        let paths: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(_, f)| !f.done || open.contains(&RunId::new(&f.key, &f.id)))
            .map(|(p, _)| p.clone())
            .collect();
        let mut followed: HashSet<RunId> = HashSet::new();
        for p in paths {
            let Some(f) = self.files.get(&p).cloned() else {
                continue;
            };
            followed.insert(RunId::new(&f.key, &f.id));
            let a = agents.get(f.key.as_str()).copied();
            let (f, ch) = self.follow(f, &p, a, &mods, runs, scrub, now_ms);
            if ch {
                changed.insert(f.key.clone());
            }
            self.files.insert(p, f);
        }

        // 4. Saved runs no followed file covers: gone agents, and the
        //    Core-start re-check.
        if first || self.tick.is_multiple_of(GONE_EVERY) {
            changed
                .extend(self.recheck(&rows, &followed, live, &agents, runs, scrub, now_ms, first));
        }
        changed
    }

    fn read_transcripts(&mut self, a: &Agent) {
        let Some(main) = a.transcript.clone() else {
            return;
        };
        let tick = self.tick;
        let t = self.told.entry(a.key.clone()).or_default();
        t.read(&main, Some(FIRST_READ), tick);
        // A subagent's or a workflow agent's runs write to this session's
        // tasks folder; their results are in `<id>/subagents/**.jsonl`.
        let subs = main.with_extension("").join("subagents");
        let mut stack = vec![(subs, 0)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                let Ok(ft) = e.file_type() else {
                    continue;
                };
                if ft.is_dir() {
                    if depth < 3 {
                        stack.push((p, depth + 1));
                    }
                } else if p.extension().is_some_and(|x| x == "jsonl") {
                    let fresh = !t.offsets.contains_key(&p);
                    let old = a.started_ms > 0
                        && e.metadata()
                            .and_then(|m| m.modified())
                            .is_ok_and(|m| ms_of(m) < a.started_ms);
                    if fresh && old {
                        // Last written before this session: not its runs.
                        let len = e.metadata().map_or(0, |m| m.len());
                        t.offsets.insert(p, len);
                        continue;
                    }
                    t.read(&p, Some(FIRST_READ), tick);
                }
            }
        }
        t.pending
            .retain(|_, at| tick.saturating_sub(*at) <= PENDING_TICKS);
        t.cap();
    }

    /// Every `<root>/<project>/<session>/tasks` folder, re-listing a
    /// project only when its mtime moved.
    fn folders(&mut self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for root in self.roots.clone() {
            let Ok(rd) = std::fs::read_dir(&root) else {
                continue;
            };
            for e in rd.flatten() {
                if !e.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let proj = e.path();
                let mtime = e.metadata().and_then(|m| m.modified()).ok();
                let fresh = self
                    .projects
                    .get(&proj)
                    .is_none_or(|(m, _)| *m != mtime || mtime.is_none());
                if fresh {
                    let sessions: Vec<PathBuf> = std::fs::read_dir(&proj)
                        .map(|rd| {
                            rd.flatten()
                                .filter(|s| s.file_type().is_ok_and(|t| t.is_dir()))
                                .map(|s| s.path().join("tasks"))
                                .filter(|t| t.is_dir())
                                .collect()
                        })
                        .unwrap_or_default();
                    self.projects.insert(proj.clone(), (mtime, sessions));
                }
                seen.insert(proj.clone());
                if let Some((_, s)) = self.projects.get(&proj) {
                    out.extend(s.iter().cloned());
                }
            }
        }
        self.projects.retain(|p, _| seen.contains(p));
        out.sort();
        out.dedup();
        out
    }

    /// A tasks folder's file names, listed again only when its mtime moved.
    fn list(&mut self, dir: &Path) -> Vec<String> {
        let mtime = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if let Some(l) = self.listed.get(dir)
            && l.mtime == mtime
            && mtime.is_some()
        {
            return l.names.clone();
        }
        let names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        self.listed.insert(
            dir.to_path_buf(),
            Listed {
                mtime,
                names: names.clone(),
            },
        );
        names
    }

    /// Maps unmapped folders by their newest held `b*` file: a holder whose
    /// parent is a live agent's claude (started no later than the holder)
    /// joins the folder to that agent. Never reads a file.
    fn try_ancestry(&mut self, unmapped: &[PathBuf], agents: &[Agent], now_ms: i64) {
        let earliest = agents
            .iter()
            .map(|a| a.started_ms)
            .filter(|&s| s > 0)
            .min()
            .unwrap_or(0);
        let by_pid: HashMap<u32, &Agent> = agents.iter().map(|a| (a.pid, a)).collect();
        self.tried.retain(|d, _| unmapped.contains(d));
        let mut tries = 0;
        for dir in unmapped {
            if tries >= ANCESTRY_PER_TICK {
                break;
            }
            let moved = std::fs::metadata(dir)
                .and_then(|m| m.modified())
                .is_ok_and(|m| ms_of(m) >= earliest);
            let last = self.tried.get(dir).copied();
            if !moved || last.is_some_and(|(t, _)| now_ms - t < ANCESTRY_EVERY_MS) {
                continue;
            }
            let newest = std::fs::read_dir(dir).ok().and_then(|rd| {
                rd.flatten()
                    .filter(|e| {
                        let n = e.file_name();
                        let n = n.to_string_lossy();
                        n.starts_with('b') && n.ends_with(".output")
                    })
                    .filter_map(|e| stat(&e.path()).map(|(_, c)| (c, e.path())))
                    .filter(|(c, _)| *c >= earliest)
                    .max()
            });
            let Some((_, file)) = newest else {
                continue;
            };
            tries += 1;
            let holders = self.host.holders(&file);
            let asked = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                let mut hs = holders.clone();
                hs.sort_unstable();
                let mut pids: Vec<u32> = agents.iter().map(|a| a.pid).collect();
                pids.sort_unstable();
                (&file, hs, pids).hash(&mut h);
                h.finish()
            };
            self.tried.insert(dir.clone(), (now_ms, asked));
            // Nothing held, or the same holders and agents as last time (a
            // session Irori doesn't list): no parents to ask for.
            if holders.is_empty() || last.is_some_and(|(_, a)| a == asked) {
                continue;
            }
            let pids: Vec<u32> = holders.iter().map(|h| h.0).collect();
            let parents = self.host.parents(&pids);
            let hit = holders.iter().find_map(|&(pid, start)| {
                let a = by_pid.get(parents.get(&pid)?)?;
                // A reused pid would have started after its "child".
                let ps = self.host.start_ticks(a.pid)?;
                (ps <= start).then(|| a.key.clone())
            });
            if let Some(key) = hit {
                self.ancestry.insert(dir.clone(), key);
            }
        }
    }

    /// One followed file's turn. Returns it updated and whether a row
    /// changed.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &self,
        mut f: Followed,
        path: &Path,
        agent: Option<&Agent>,
        mods: &HashSet<String>,
        runs: &Runs,
        scrub: Scrub<'_>,
        now_ms: i64,
    ) -> (Followed, bool) {
        let id = RunId::new(&f.key, &f.id);
        let told = self.told.get(&f.key);
        let ended = told.and_then(|t| t.ended.get(&f.id));
        let proven = told.is_some_and(|t| t.bg.contains(&f.id));
        let now_len = stat(path).map(|(l, _)| l);
        let grew = matches!((f.len, now_len), (Some(a), Some(b)) if b > a);
        let stable = f.len.is_some() && f.len == now_len;
        let first_look = f.len.is_none();
        f.len = now_len;
        let row = runs.get(&id);
        let mut ch = false;

        // An L1 end with no kind made a Bash row: the prefix says better.
        if let Some(r) = &row
            && r.kind == RunKind::Bash
            && f.kind != RunKind::Bash
        {
            let k = f.kind;
            runs.update(&id, |r| r.kind = k);
            runs.save_now();
            ch = true;
        }
        let Some(row) = row else {
            // No row yet.
            if ended.is_some() || now_len.is_none() {
                // Ended before it was seen, or gone.
                f.done = true;
                return (f, ch);
            }
            if f.kind == RunKind::Bash
                && read_trailer(path).is_some()
                && self.host.holders(path).is_empty()
            {
                // Finished with no row: a call too short to see, or a
                // mod session's foreground call. A row that shows up later
                // brings the file back (see `tick`).
                f.done = true;
                return (f, ch);
            }
            let cmd = told
                .and_then(|t| t.tool_of.get(&f.id))
                .and_then(|tu| told?.cmds.get(tu).cloned());
            let tool_use = told.and_then(|t| t.tool_of.get(&f.id).cloned());
            let rep = if f.kind != RunKind::Bash || proven {
                RunReport {
                    event: "start".into(),
                    kind: kind_name(f.kind).into(),
                    task_id: Some(f.id.clone()),
                    tool_use_id: tool_use,
                    command: cmd,
                    output_file: Some(path.to_string_lossy().into_owned()),
                    started_at: f.created_ms,
                    ..RunReport::default()
                }
            } else if mods.contains(&f.key) {
                // The mod reports this session's foreground calls itself,
                // and its background starts: a row for this file, if one
                // comes, brings it back (see `tick`).
                f.done = true;
                return (f, ch);
            } else {
                RunReport {
                    event: "fg-start".into(),
                    kind: "fg".into(),
                    task_id: Some(f.id.clone()),
                    output_file: Some(path.to_string_lossy().into_owned()),
                    started_at: f.created_ms,
                    ..RunReport::default()
                }
            };
            ch |= record(runs, &f.key, &rep, scrub, now_ms);
            return (f, ch);
        };
        if !row.running() {
            f.done = true;
            return (f, ch);
        }

        // Proven background now? (A foreground row only.)
        let mut kind = row.kind;
        // Its call is over: the transcript has its result and no shell call
        // waits, or (no transcript) the status left "shell".
        let call_done = match told.filter(|_| agent.is_some_and(|a| a.transcript.is_some())) {
            Some(t) => t.pending.is_empty() && t.result_tick > f.seen_tick,
            None => agent.is_some_and(|a| a.status != "shell"),
        };
        if kind == RunKind::Fg {
            let outlived = !first_look
                && self.tick > f.seen_tick + 1
                && now_len.is_some()
                && call_done
                && (grew || !self.host.holders(path).is_empty());
            // A §2.1 start for this id (it named the call) skips the wait.
            let reported = row.tool_use_id.is_some();
            if proven || reported || ended.is_some() || outlived {
                let tool_use = told.and_then(|t| t.tool_of.get(&f.id).cloned());
                let cmd = tool_use.as_ref().and_then(|tu| told?.cmds.get(tu).cloned());
                runs.update(&id, |r| r.kind = RunKind::Bash);
                record(
                    runs,
                    &f.key,
                    &RunReport {
                        event: "start".into(),
                        kind: "bash".into(),
                        task_id: Some(f.id.clone()),
                        tool_use_id: tool_use,
                        command: cmd,
                        ..RunReport::default()
                    },
                    scrub,
                    now_ms,
                );
                runs.save_now();
                ch = true;
                kind = RunKind::Bash;
            }
        }

        // Ended?
        let end = if let Some(e) = ended {
            Some(end_report(kind, &f.id, &e.status, e.exit))
        } else if now_len.is_none() {
            Some(end_report(kind, &f.id, "", None))
        } else if (stable || (first_look && self.host.supported())) && f.kind == RunKind::Bash {
            let trailer = read_trailer(path);
            let free = self.host.holders(path).is_empty();
            match trailer {
                Some(t) if free => Some(match t {
                    Trailer::Exit(n) => end_report(kind, &f.id, "", Some(n)),
                    Trailer::Killed => end_report(kind, &f.id, "killed", None),
                }),
                // Nothing holds it, it stopped growing and no trailer came
                // (a kill with no TaskStop): its processes are gone.
                // A foreground call whose call is over and whose file
                // stopped: ended (no trailer is written for one). The only
                // end where holders can't be told.
                None if free && kind == RunKind::Fg && !grew && !first_look && call_done => {
                    Some(end_report(kind, &f.id, "", None))
                }
                None if free
                    && self.host.supported()
                    && !row.members.iter().any(|&m| self.host.alive(m)) =>
                {
                    f.quiet += 1;
                    (f.quiet >= QUIET_TICKS).then(|| end_report(kind, &f.id, "", None))
                }
                _ => {
                    f.quiet = 0;
                    None
                }
            }
        } else {
            f.quiet = 0;
            None
        };
        if let Some(rep) = end {
            ch |= record(runs, &f.key, &rep, scrub, now_ms);
            f.done = true;
        }
        (f, ch)
    }

    /// Saved open runs no followed file covers: on the first tick every
    /// one; later, those of gone agents.
    #[allow(clippy::too_many_arguments)]
    fn recheck(
        &mut self,
        rows: &[super::LiveRun],
        followed: &HashSet<RunId>,
        live: &Live,
        agents: &BTreeMap<&str, &Agent>,
        runs: &Runs,
        scrub: Scrub<'_>,
        now_ms: i64,
        first: bool,
    ) -> BTreeSet<String> {
        let mut changed = BTreeSet::new();
        let mut lens = HashMap::new();
        for r in rows.iter().filter(|r| r.running()) {
            let id = r.id();
            if followed.contains(&id) {
                continue;
            }
            let gone = live.known && !agents.contains_key(r.key.as_str());
            if !gone && !first {
                continue;
            }
            let told_end = self.told.get(&r.key).and_then(|t| t.ended.get(&r.task_id));
            let rep = if let Some(e) = told_end {
                Some(end_report(r.kind, &r.task_id, &e.status, e.exit))
            } else {
                match (r.kind, r.output_path.as_deref()) {
                    // A foreground call or an agent/workflow (inside
                    // claude) ends with its agent.
                    (RunKind::Fg | RunKind::Agent | RunKind::Workflow, _)
                    | (RunKind::Bash, None) => {
                        gone.then(|| end_report(r.kind, &r.task_id, "", None))
                    }
                    (RunKind::Bash, Some(p)) => match stat(p) {
                        None => Some(end_report(r.kind, &r.task_id, "", None)),
                        Some((len, _)) => {
                            let prev = self.row_lens.get(&id).copied();
                            lens.insert(id.clone(), len);
                            // Holders are only trusted where they can be told;
                            // elsewhere the file must have stopped growing.
                            let still = if self.host.supported() {
                                self.host.holders(p).is_empty()
                            } else {
                                prev == Some(len)
                            };
                            let members_gone = !r.members.iter().any(|&m| self.host.alive(m));
                            match read_trailer(p) {
                                Some(Trailer::Exit(n)) if still => {
                                    Some(end_report(r.kind, &r.task_id, "", Some(n)))
                                }
                                Some(Trailer::Killed) if still => {
                                    Some(end_report(r.kind, &r.task_id, "killed", None))
                                }
                                None if still
                                    && members_gone
                                    && (gone || self.host.supported()) =>
                                {
                                    // Its processes are gone and no trailer
                                    // will come.
                                    Some(end_report(r.kind, &r.task_id, "", None))
                                }
                                _ => None,
                            }
                        }
                    },
                }
            };
            if let Some(rep) = rep
                && record(runs, &r.key, &rep, scrub, now_ms)
            {
                changed.insert(r.key.clone());
            }
        }
        self.row_lens = lens;
        changed
    }
}

fn end_report(kind: RunKind, id: &str, status: &str, exit: Option<i64>) -> RunReport {
    let fg = kind == RunKind::Fg;
    RunReport {
        event: if fg { "fg" } else { "end" }.into(),
        kind: kind_name(kind).into(),
        task_id: Some(id.to_string()),
        status: (!status.is_empty()).then(|| status.to_string()),
        exit_code: exit,
        ..RunReport::default()
    }
}

fn record(runs: &Runs, key: &str, rep: &RunReport, scrub: Scrub<'_>, now_ms: i64) -> bool {
    let cap = |s: &str| {
        let s = scrub(s);
        match s.char_indices().nth(MAX_CMD) {
            Some((i, _)) => s[..i].to_string(),
            None => s,
        }
    };
    match runs.record(key, rep, cap, now_ms) {
        Ok(c) => c,
        Err(e) => {
            logf(format_args!("runs scan {key}: {e}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const KEY: &str = "dev/new-sess";
    const SID: &str = "new-sess";

    #[derive(Default)]
    struct Fake {
        supported: bool,
        holders: HashMap<PathBuf, Vec<(u32, u64)>>,
        parents: HashMap<u32, u32>,
        starts: HashMap<u32, u64>,
        /// How many times parents were asked for.
        asks: std::cell::Cell<u32>,
    }

    impl Host for Fake {
        fn supported(&self) -> bool {
            self.supported
        }
        fn holders(&self, file: &Path) -> Vec<(u32, u64)> {
            self.holders.get(file).cloned().unwrap_or_default()
        }
        fn parents(&self, pids: &[u32]) -> HashMap<u32, u32> {
            self.asks.set(self.asks.get() + 1);
            pids.iter()
                .filter_map(|p| self.parents.get(p).map(|pp| (*p, *pp)))
                .collect()
        }
        fn start_ticks(&self, pid: u32) -> Option<u64> {
            self.starts.get(&pid).copied()
        }
    }

    struct Fix {
        _d: tempfile::TempDir,
        root: PathBuf,
        projects: PathBuf,
        runs: Runs,
    }

    fn now() -> i64 {
        ms_of(SystemTime::now()) + 1000
    }

    fn fix() -> Fix {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("tmp").join("claude");
        let projects = d.path().join("projects").join("C--x");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::create_dir_all(d.path().join("data")).unwrap();
        let runs = Runs::new(&d.path().join("data"), now());
        Fix {
            _d: d,
            root,
            projects,
            runs,
        }
    }

    impl Fix {
        /// `<root>/proj/<session>/tasks`.
        fn tasks(&self, session: &str) -> PathBuf {
            let t = self.root.join("proj").join(session).join("tasks");
            std::fs::create_dir_all(&t).unwrap();
            t
        }
        fn transcript(&self, sid: &str) -> PathBuf {
            self.projects.join(format!("{sid}.jsonl"))
        }
        fn agent(&self, transcript: bool) -> Agent {
            Agent {
                key: KEY.into(),
                session_id: SID.into(),
                pid: 100,
                started_ms: now() - 60_000,
                transcript: transcript.then(|| self.transcript(SID)),
                status: "busy".into(),
            }
        }
        fn scanner(&self, host: Fake) -> Scanner<Fake> {
            Scanner::new(host, vec![self.root.clone()])
        }
        fn row(&self, id: &str) -> Option<super::super::LiveRun> {
            self.runs.get(&RunId::new(KEY, id))
        }
        fn start(&self, id: &str, kind: &str, out: Option<&Path>) {
            let mut j = serde_json::json!({"event":"start","kind":kind,"taskId":id});
            if let Some(o) = out {
                j["outputFile"] = o.to_string_lossy().into();
            }
            self.report(j);
        }
        fn report(&self, j: serde_json::Value) {
            let rep: RunReport = serde_json::from_value(j).unwrap();
            self.runs.record(KEY, &rep, |s| s.into(), now()).unwrap();
        }
    }

    fn live(agents: Vec<Agent>) -> Live {
        Live {
            agents,
            known: true,
        }
    }

    fn tick<H: Host>(sc: &mut Scanner<H>, l: &Live, runs: &Runs) -> BTreeSet<String> {
        sc.tick(l, runs, &|s: &str| s.replace("sekrit", "[redacted]"), now())
    }

    fn append(p: &Path, s: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .unwrap();
        f.write_all(s.as_bytes()).unwrap();
    }

    fn log(p: &Path, v: serde_json::Value) {
        append(p, &format!("{v}\n"));
    }

    fn tool_use(p: &Path, id: &str, cmd: &str) {
        log(
            p,
            serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"tool_use","id":id,"name":"Bash","input":{"command":cmd}}]}}),
        );
    }

    fn result(p: &Path, tu: &str, bg: Option<(&str, &Path)>) {
        let (text, extra) = match bg {
            Some((id, out)) => (
                format!(
                    "Command running in background with ID: {id}. Output is being written to: {}. You will be notified.",
                    out.display()
                ),
                serde_json::json!({"backgroundTaskId": id}),
            ),
            None => ("ok".to_string(), serde_json::json!({"stdout":"ok"})),
        };
        log(
            p,
            serde_json::json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":tu,"content":text}]},"toolUseResult":extra}),
        );
    }

    fn notification(p: &Path, id: &str, status: &str, summary: &str) {
        log(
            p,
            serde_json::json!({"type":"queue-operation","operation":"enqueue","content":format!(
                "<task-notification>\n<task-id>{id}</task-id>\n<status>{status}</status>\n<summary>{summary}</summary>\n</task-notification>")}),
        );
    }

    #[test]
    fn a_shell_file_stays_foreground_until_the_transcript_says_background() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        tool_use(&t, "toolu_1", "cargo test --token sekrit");
        let out = tasks.join("b1.output");
        append(&out, "compiling\n");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);

        assert!(tick(&mut sc, &l, &fx.runs).contains(KEY));
        assert_eq!(fx.row("b1").unwrap().kind, RunKind::Fg);
        // Growing with the call still waiting on its result: still foreground.
        for _ in 0..3 {
            append(&out, "more\n");
            tick(&mut sc, &l, &fx.runs);
            assert_eq!(fx.row("b1").unwrap().kind, RunKind::Fg);
        }
        result(&t, "toolu_1", Some(("b1", &out)));
        assert!(tick(&mut sc, &l, &fx.runs).contains(KEY));
        let r = fx.row("b1").unwrap();
        assert_eq!(
            (
                r.kind,
                r.tool_use_id.as_deref(),
                r.cmd.as_str(),
                r.running()
            ),
            (
                RunKind::Bash,
                Some("toolu_1"),
                "cargo test --token [redacted]",
                true
            )
        );
        assert_eq!(r.output_path.as_deref(), Some(out.as_path()));
        // The trailer ends it once the file stopped growing (no holders known).
        append(&out, "\n[exited with code 0]\n");
        tick(&mut sc, &l, &fx.runs);
        assert!(fx.row("b1").unwrap().running(), "grew this tick");
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b1").unwrap();
        assert_eq!((r.running(), r.exit), (false, Some(0)));
        // Nothing new on disk: nothing to publish.
        assert!(tick(&mut sc, &l, &fx.runs).is_empty());
        assert!(tick(&mut sc, &l, &fx.runs).is_empty());
    }

    #[test]
    fn printed_trailer_text_and_held_files_do_not_end_a_run() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        let out = tasks.join("b2.output");
        append(&out, "start\n");
        tool_use(&t, "toolu_2", "sh x");
        result(&t, "toolu_2", Some(("b2", &out)));
        let mut sc = fx.scanner(Fake {
            supported: true,
            ..Fake::default()
        });
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b2").unwrap().kind, RunKind::Bash);
        // A program prints the text, then more: not the last line.
        append(&out, "[exited with code 1]\nstill going\n");
        tick(&mut sc, &l, &fx.runs);
        tick(&mut sc, &l, &fx.runs);
        assert!(fx.row("b2").unwrap().running());
        // The last line, but something still holds the file.
        append(&out, "[exited with code 1]\n");
        sc.host.holders.insert(out.clone(), vec![(7, 1)]);
        tick(&mut sc, &l, &fx.runs);
        tick(&mut sc, &l, &fx.runs);
        assert!(fx.row("b2").unwrap().running());
        sc.host.holders.clear();
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b2").unwrap();
        assert_eq!((r.running(), r.exit), (false, Some(1)));
    }

    #[test]
    fn a_notification_proves_background_and_ends_the_run() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        tool_use(&t, "toolu_3", "make");
        let out = tasks.join("b3.output");
        append(&out, "x\n");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b3").unwrap().kind, RunKind::Fg);
        notification(
            &t,
            "b3",
            "failed",
            "Background command \"make\" failed with exit code 3",
        );
        // The same id again (a queue row, then a prompt row): the first wins.
        notification(
            &t,
            "b3",
            "failed",
            "Background command \"make\" failed with exit code 9",
        );
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b3").unwrap();
        assert_eq!(
            (r.kind, r.running(), r.exit),
            (RunKind::Bash, false, Some(3))
        );
    }

    #[test]
    fn a_file_that_outlives_its_call_is_background() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        tool_use(&t, "toolu_4", "npm run dev");
        let out = tasks.join("b4.output");
        append(&out, "x\n");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        append(&out, "y\n");
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b4").unwrap().kind, RunKind::Fg);
        // Its call's result lands with no background id (a subagent's run,
        // whose result is elsewhere), and the file keeps growing.
        result(&t, "toolu_4", None);
        append(&out, "z\n");
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b4").unwrap().kind, RunKind::Bash);

        // With no transcript: the status leaving "shell" with the file held.
        let fx = fix();
        let tasks = fx.tasks(SID);
        let out = tasks.join("b5.output");
        append(&out, "x\n");
        let mut a = fx.agent(false);
        a.status = "shell".into();
        let mut sc = fx.scanner(Fake {
            supported: true,
            ..Fake::default()
        });
        sc.host.holders.insert(out.clone(), vec![(9, 1)]);
        for _ in 0..3 {
            tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        }
        assert_eq!(fx.row("b5").unwrap().kind, RunKind::Fg);
        a.status = "idle".into();
        tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        assert_eq!(fx.row("b5").unwrap().kind, RunKind::Bash);
    }

    #[test]
    fn a_foreground_file_that_vanishes_ends_its_row() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let out = tasks.join("b6.output");
        append(&out, "x\n");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        std::fs::remove_file(&out).unwrap();
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b6").unwrap();
        assert_eq!((r.kind, r.running()), (RunKind::Fg, false));
    }

    #[test]
    fn unmapped_and_older_files_are_ignored() {
        let fx = fix();
        // Another session's folder: never mapped (no live key, no ancestry).
        let other = fx.tasks("someone-else");
        append(&other.join("b7.output"), "x\n");
        // This session's folder, but the file predates the session.
        let tasks = fx.tasks(SID);
        append(&tasks.join("b8.output"), "x\n");
        let mut a = fx.agent(false);
        a.started_ms = now() + 60_000;
        let mut sc = fx.scanner(Fake {
            supported: true,
            ..Fake::default()
        });
        for _ in 0..2 {
            tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        }
        assert!(fx.runs.all().is_empty(), "{:?}", fx.runs.all());
    }

    #[test]
    fn a_resumed_session_joins_by_ancestry() {
        let fx = fix();
        // The resumed session writes under its original id.
        let tasks = fx.tasks("old-sess");
        let out = tasks.join("b9.output");
        append(&out, "x\n");
        let host = |claude_start: u64| Fake {
            supported: true,
            holders: HashMap::from([(out.clone(), vec![(200, 5_000)])]),
            parents: HashMap::from([(200, 100)]),
            starts: HashMap::from([(100, claude_start)]),
            ..Fake::default()
        };
        // A reused pid (claude "started" after its child) never maps.
        let mut sc = fx.scanner(host(6_000));
        let l = live(vec![fx.agent(false)]);
        for _ in 0..3 {
            tick(&mut sc, &l, &fx.runs);
        }
        assert!(fx.runs.all().is_empty());

        let mut sc = fx.scanner(host(4_000));
        tick(&mut sc, &l, &fx.runs);
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b9").unwrap().kind, RunKind::Fg);
    }

    #[test]
    fn a_resumed_session_joins_by_the_learned_map() {
        let fx = fix();
        let tasks = fx.tasks("old-sess");
        // The transcript (filed under the new id) names the old folder.
        let t = fx.transcript(SID);
        let out = tasks.join("b10.output");
        append(&out, "x\n");
        tool_use(&t, "toolu_10", "cargo build");
        result(&t, "toolu_10", Some(("b10", &out)));
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b10").unwrap();
        assert_eq!((r.kind, r.cmd.as_str()), (RunKind::Bash, "cargo build"));

        // A mod's start teaches the folder too.
        let fx = fix();
        let tasks = fx.tasks("older-sess");
        let reported = tasks.join("b11.output");
        let fg = tasks.join("b12.output");
        append(&reported, "x\n");
        append(&fg, "y\n");
        fx.start("b11", "bash", Some(&reported));
        let mut sc = fx.scanner(Fake::default());
        tick(&mut sc, &live(vec![fx.agent(false)]), &fx.runs);
        assert_eq!(fx.row("b12").unwrap().kind, RunKind::Fg);
    }

    #[test]
    fn runs_that_ended_while_irori_was_down_end_on_start() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let done = tasks.join("b20.output");
        append(&done, "ok\n\n[exited with code 4]\n");
        fx.start("b20", "bash", Some(&done));
        let killed = tasks.join("b21.output");
        append(&killed, "x\n\n[killed]\n");
        fx.start("b21", "bash", Some(&killed));
        fx.start("b22", "bash", Some(&tasks.join("b22.output"))); // vanished
        let dead = tasks.join("b23.output");
        append(&dead, "no trailer\n");
        fx.start("b23", "bash", Some(&dead));
        fx.runs
            .update(&RunId::new(KEY, "b23"), |r| r.members = vec![(300, 1)]);
        let held = tasks.join("b24.output");
        append(&held, "busy\n");
        fx.start("b24", "bash", Some(&held));
        fx.start("a25", "agent", None);
        fx.report(serde_json::json!({"event":"fg-start","toolUseId":"toolu_26"}));

        // The agent itself is gone; b24's tree still runs (it outlives claude).
        let mut sc = fx.scanner(Fake {
            supported: true,
            holders: HashMap::from([(held.clone(), vec![(400, 1)])]),
            ..Fake::default()
        });
        let changed = tick(&mut sc, &live(vec![]), &fx.runs);
        assert!(changed.contains(KEY));
        let got = |id: &str| {
            let r = fx.row(id).unwrap();
            (r.running(), r.exit)
        };
        assert_eq!(got("b20"), (false, Some(4)));
        assert_eq!(got("b21"), (false, None));
        assert_eq!(got("b22"), (false, None));
        assert_eq!(got("b23"), (false, None));
        assert_eq!(got("b24"), (true, None));
        assert_eq!(got("a25"), (false, None));
        assert_eq!(got("toolu_26"), (false, None));

        // With the agent live: a finished file ends on the first tick, a held
        // run and an agent run stay open.
        let fx = fix();
        let tasks = fx.tasks(SID);
        let held = tasks.join("b30.output");
        append(&held, "busy\n");
        fx.start("b30", "bash", Some(&held));
        fx.start("a31", "agent", None);
        let done = tasks.join("b32.output");
        append(&done, "ok\n\n[exited with code 0]\n");
        fx.start("b32", "bash", Some(&done));
        let mut sc = fx.scanner(Fake {
            supported: true,
            holders: HashMap::from([(held.clone(), vec![(400, 1)])]),
            ..Fake::default()
        });
        tick(&mut sc, &live(vec![fx.agent(false)]), &fx.runs);
        assert!(fx.row("b30").unwrap().running());
        assert!(fx.row("a31").unwrap().running());
        assert_eq!(fx.row("b32").unwrap().exit, Some(0));
        // Presence not ready: nobody counts as gone.
        let mut sc = fx.scanner(Fake::default());
        tick(&mut sc, &Live::default(), &fx.runs);
        assert!(fx.row("a31").unwrap().running());
    }

    #[test]
    fn l1_quirks_are_tolerated() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        // An end with no kind for an id never started, twice: one Bash row.
        let end = serde_json::json!({"event":"end","taskId":"a40","status":"killed"});
        fx.report(end.clone());
        fx.report(end);
        append(&tasks.join("a40.output"), "");
        // A workflow file with no row gets a running workflow row.
        append(&tasks.join("w41.output"), "{}");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(false)]);
        tick(&mut sc, &l, &fx.runs);
        tick(&mut sc, &l, &fx.runs);
        let a = fx.row("a40").unwrap();
        assert_eq!(
            (a.kind, a.running()),
            (RunKind::Agent, false),
            "kind fixed, never reopened"
        );
        let w = fx.row("w41").unwrap();
        assert_eq!((w.kind, w.running()), (RunKind::Workflow, true));
        assert_eq!(fx.runs.by_key(KEY).len(), 2);
    }

    #[test]
    fn a_mod_session_gets_no_scanned_foreground_rows() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        append(&tasks.join("b50.output"), "x\n");
        fx.report(serde_json::json!({"event":"fg-start","toolUseId":"toolu_50"}));
        let mut sc = fx.scanner(Fake::default());
        tick(&mut sc, &live(vec![fx.agent(false)]), &fx.runs);
        assert!(fx.row("b50").is_none());
        assert_eq!(fx.runs.by_key(KEY).len(), 1);
    }

    #[test]
    fn trailers_are_the_last_line_only() {
        assert_eq!(
            parse_trailer(b"\n[exited with code 1]\n", true),
            Some(Trailer::Exit(1))
        );
        assert_eq!(
            parse_trailer(b"out\n\n[exited with code -2]\r\n", false),
            Some(Trailer::Exit(-2))
        );
        assert_eq!(
            parse_trailer(b"x\n\n[killed]\n", false),
            Some(Trailer::Killed)
        );
        assert_eq!(parse_trailer(b"[exited with code 0]\nmore\n", true), None);
        assert_eq!(parse_trailer(b"x\n[exited with code 0]", true), None);
        assert_eq!(parse_trailer(b"[exited with code 0]\n", false), None);
        assert_eq!(parse_trailer(b"x\n[exited with code N]\n", false), None);
    }

    #[test]
    fn notifications_and_background_texts_parse() {
        let text = "<system-reminder>\n<task-notification>\n<task-id>bqff5z00r</task-id>\n\
            <tool-use-id>toolu_1</tool-use-id>\n<output-file>C:\\t\\s\\tasks\\bqff5z00r.output</output-file>\n\
            <status>completed</status>\n<summary>Background command \"x\" completed (exit code 0)</summary>\n\
            </task-notification><task-notification><task-id>a1</task-id><status>killed</status>\
            <summary>stopped</summary></task-notification>";
        let n = parse_notifications(text);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].id, "bqff5z00r");
        assert_eq!(
            n[0].end,
            Ended {
                status: "completed".into(),
                exit: Some(0)
            }
        );
        assert_eq!(
            n[0].output.as_deref(),
            Some("C:\\t\\s\\tasks\\bqff5z00r.output")
        );
        assert_eq!(
            n[1].end,
            Ended {
                status: "killed".into(),
                exit: None
            }
        );
        assert_eq!(exit_in("failed with exit code 101"), Some(101));
        assert_eq!(exit_in("(exit code -1)"), Some(-1));

        let (id, p) = bg_from_text(
            "Command running in background with ID: be9ihtlp2. Output is being written to: /tmp/claude-501/h/s/tasks/be9ihtlp2.output. You will be notified",
        )
        .unwrap();
        assert_eq!(
            (id.as_str(), p.as_str()),
            ("be9ihtlp2", "/tmp/claude-501/h/s/tasks/be9ihtlp2.output")
        );
        assert_eq!(
            tasks_folder(Path::new(&p), &id),
            Some(PathBuf::from("/tmp/claude-501/h/s/tasks"))
        );
        assert!(
            bg_from_text(
                "cat says Command running in background with ID: x. Output is being written to: y.output"
            )
            .is_none()
        );
        assert!(tasks_folder(Path::new("/x/notasks/b1.output"), "b1").is_none());
    }

    #[test]
    fn a_task_stop_ends_and_a_timed_out_call_is_background() {
        let fx = fix();
        let tasks = fx.tasks("old-sess");
        let t = fx.transcript(SID);
        let out = tasks.join("b60.output");
        append(
            &out, "slow
",
        );
        tool_use(&t, "toolu_60", "cargo test");
        // The 2-minute timeout moved it to the background: the field, and a
        // text other than run_in_background's (which still names the folder).
        log(
            &t,
            serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result",
                "tool_use_id":"toolu_60","content":format!(
                "Command did not complete within its 120s timeout and was moved to the background (ID: b60). Output is being written to: {}. You will be notified.",
                out.display())}]},"toolUseResult":{"backgroundTaskId":"b60"}}),
        );
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b60").unwrap();
        assert_eq!((r.kind, r.running()), (RunKind::Bash, true));
        // TaskStop: no trailer, no notification.
        log(
            &t,
            serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"tool_use","id":"toolu_61","name":"TaskStop","input":{"task_id":"b60"}}]}}),
        );
        log(
            &t,
            serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result",
                "tool_use_id":"toolu_61","content":"Successfully stopped task: b60 (cargo test)"}]},
                "toolUseResult":{"task_id":"b60","task_type":"local_bash","command":"cargo test"}}),
        );
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b60").unwrap();
        assert_eq!((r.running(), r.exit), (false, None));
    }

    #[test]
    fn a_run_whose_processes_are_gone_ends_with_no_trailer() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        let out = tasks.join("b70.output");
        append(
            &out, "x
",
        );
        tool_use(&t, "toolu_70", "sleep 99");
        result(&t, "toolu_70", Some(("b70", &out)));
        let mut sc = fx.scanner(Fake {
            supported: true,
            ..Fake::default()
        });
        sc.host.holders.insert(out.clone(), vec![(5, 1)]);
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        for _ in 0..4 {
            assert!(tick(&mut sc, &l, &fx.runs).is_empty(), "steady: no change");
        }
        assert!(fx.row("b70").unwrap().running(), "held: still going");
        sc.host.holders.clear();
        for _ in 0..QUIET_TICKS - 1 {
            tick(&mut sc, &l, &fx.runs);
        }
        assert!(fx.row("b70").unwrap().running());
        tick(&mut sc, &l, &fx.runs);
        assert!(!fx.row("b70").unwrap().running());
    }

    #[test]
    fn another_tools_output_never_proves_background() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        let out = tasks.join("b80.output");
        append(&out, "x\n");
        tool_use(&t, "toolu_80", "sleep 5");
        // An MCP tool's result prints the shell's text and field, naming
        // this file and a folder somewhere else.
        let elsewhere = fx.root.join("p").join("s").join("tasks").join("b81.output");
        log(
            &t,
            serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"tool_use","id":"toolu_m","name":"mcp__x__y","input":{}}]}}),
        );
        result(&t, "toolu_m", Some(("b80", &out)));
        result(&t, "toolu_m2", Some(("b81", &elsewhere)));
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        append(&out, "y\n");
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b80").unwrap().kind, RunKind::Fg);
        let told = &sc.told[KEY];
        assert!(told.bg.is_empty() && told.folders.is_empty() && told.tool_of.is_empty());
    }

    #[test]
    fn a_finished_foreground_call_ends_where_holders_cannot_be_told() {
        // No holders on this OS, no trailer for a foreground call: the
        // transcript's result ends the row once the file stopped.
        let fx = fix();
        let tasks = fx.tasks(SID);
        let t = fx.transcript(SID);
        tool_use(&t, "toolu_82", "ls");
        let out = tasks.join("b82.output");
        append(&out, "");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(true)]);
        tick(&mut sc, &l, &fx.runs);
        tick(&mut sc, &l, &fx.runs);
        assert!(fx.row("b82").unwrap().running(), "its call still waits");
        result(&t, "toolu_82", None);
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b82").unwrap();
        assert_eq!((r.kind, r.running()), (RunKind::Fg, false));

        // No transcript: the status leaving "shell".
        let fx = fix();
        let tasks = fx.tasks(SID);
        let out = tasks.join("b83.output");
        append(&out, "x\n");
        let mut a = fx.agent(false);
        a.status = "shell".into();
        let mut sc = fx.scanner(Fake::default());
        for _ in 0..3 {
            tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        }
        assert!(fx.row("b83").unwrap().running());
        a.status = "idle".into();
        tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        assert!(!fx.row("b83").unwrap().running());
    }

    #[test]
    fn a_mod_start_after_the_scan_skips_the_wait() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let out = tasks.join("b84.output");
        append(&out, "x\n");
        let mut sc = fx.scanner(Fake::default());
        let mut a = fx.agent(false);
        a.status = "shell".into();
        let l = live(vec![a]);
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b84").unwrap().kind, RunKind::Fg);
        fx.report(
            serde_json::json!({"event":"start","kind":"bash","taskId":"b84",
            "toolUseId":"toolu_84","outputFile":out.to_string_lossy()}),
        );
        tick(&mut sc, &l, &fx.runs);
        let r = fx.row("b84").unwrap();
        assert_eq!((r.kind, r.running()), (RunKind::Bash, true));
    }

    #[test]
    fn set_aside_files_come_back_while_their_row_runs() {
        // A mod session: its foreground files are set aside, then a start
        // reported late for one brings it back so its trailer ends it.
        let fx = fix();
        let tasks = fx.tasks(SID);
        fx.report(serde_json::json!({"event":"fg-start","toolUseId":"toolu_85"}));
        let out = tasks.join("b85.output");
        append(&out, "x\n");
        let mut sc = fx.scanner(Fake::default());
        let l = live(vec![fx.agent(false)]);
        tick(&mut sc, &l, &fx.runs);
        assert!(sc.files[&out].done);
        fx.start("b85", "bash", Some(&out));
        append(&out, "\n[exited with code 2]\n");
        tick(&mut sc, &l, &fx.runs);
        tick(&mut sc, &l, &fx.runs);
        assert_eq!(fx.row("b85").unwrap().exit, Some(2));

        // A file older than the (resumed) session whose run still ran.
        let fx = fix();
        let tasks = fx.tasks(SID);
        let out = tasks.join("b86.output");
        append(&out, "x\n");
        fx.start("b86", "bash", Some(&out));
        let mut a = fx.agent(false);
        a.started_ms = now() + 60_000;
        let mut sc = fx.scanner(Fake::default());
        tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        assert!(fx.row("b86").unwrap().running());
        append(&out, "\n[exited with code 0]\n");
        tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        tick(&mut sc, &live(vec![a.clone()]), &fx.runs);
        assert_eq!(fx.row("b86").unwrap().exit, Some(0));
    }

    #[test]
    fn a_finished_file_with_no_row_is_set_aside_only_when_free() {
        let fx = fix();
        let tasks = fx.tasks(SID);
        let out = tasks.join("b87.output");
        append(&out, "[exited with code 0]\n");
        let mut sc = fx.scanner(Fake {
            supported: true,
            holders: HashMap::from([(out.clone(), vec![(9, 1)])]),
            ..Fake::default()
        });
        let l = live(vec![fx.agent(false)]);
        tick(&mut sc, &l, &fx.runs);
        assert!(fx.row("b87").unwrap().running(), "printed text, still held");
    }

    #[test]
    fn a_reported_path_outside_claudes_tree_adds_no_root() {
        let fx = fix();
        let odd = fx
            .root
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("x")
            .join("y")
            .join("tasks");
        std::fs::create_dir_all(&odd).unwrap();
        fx.start("b88", "bash", Some(&odd.join("b88.output")));
        let mut sc = fx.scanner(Fake::default());
        tick(&mut sc, &live(vec![fx.agent(false)]), &fx.runs);
        assert_eq!(sc.roots.len(), 1, "{:?}", sc.roots);
    }

    #[test]
    fn ancestry_asks_the_same_question_once() {
        let fx = fix();
        let tasks = fx.tasks("someone-else");
        let out = tasks.join("b89.output");
        append(&out, "x\n");
        let mut sc = fx.scanner(Fake {
            supported: true,
            holders: HashMap::from([(out.clone(), vec![(500, 1)])]),
            parents: HashMap::from([(500, 999)]),
            ..Fake::default()
        });
        let l = live(vec![fx.agent(false)]);
        let mut t = now();
        for _ in 0..4 {
            sc.tick(&l, &fx.runs, &|s: &str| s.into(), t);
            t += ANCESTRY_EVERY_MS + 1;
        }
        assert_eq!(sc.host.asks.get(), 1);
        assert!(fx.runs.all().is_empty());
    }
}
