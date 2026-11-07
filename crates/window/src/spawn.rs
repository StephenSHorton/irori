//! Spawned agents, part 1 (card #445, docs/design/spawned-agents.md §14.1):
//! start a fresh `claude --bg` session for a piece of work, find it again on
//! the roster, and say before starting whether Claude Code will accept the
//! folder at all.
//!
//! What Claude Code 2.1.318 does, measured on the Mac (#445):
//! - a git repo needs its own trust entry; a trusted parent doesn't cover it;
//! - a git worktree (`git worktree add` anywhere) follows its main
//!   repo's entry;
//! - a folder outside any git repo is covered by a trusted parent.
//!
//! The brief never goes on the command line: it is written to a file and the
//! prompt only names that file, so nothing in it (quotes, `&`, `|`, `%`) can
//! reach `claude.cmd`'s `cmd.exe` parsing on Windows, and an 8 KB brief can't
//! run into the command-line limit.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::roster::{TrustSet, copy_capped, find_claude, hide_console, lossy, strip_ansi};

/// How long `claude --bg` may take to print its id (it returns in about a
/// second; the first one also starts Claude Code's background service).
const SPAWN_WAIT: Duration = Duration::from_secs(30);

/// The permission modes a spawn may name. One is always passed: without
/// `--permission-mode` the agent inherits the caller's mode (spike problem 2).
pub const PERMISSION_MODES: &[&str] = &[
    "default",
    "acceptEdits",
    "plan",
    "auto",
    "dontAsk",
    "bypassPermissions",
];

/// One agent to start.
#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    /// The folder it runs in: a repo or a worktree. Never `claude -w`
    /// (card #565): a worktree `claude` makes is one `claude rm` deletes
    /// without Irori's checks, so the Core makes a card's tree itself.
    pub cwd: PathBuf,
    /// Its session name, shown on the roster and in `claude agents`.
    pub name: String,
    /// Everything it is told; written to a file, never put in argv.
    pub brief: String,
    /// Required: one of [`PERMISSION_MODES`].
    pub permission_mode: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// A `--settings` file (the board's allow and deny rules, slice 5).
    pub settings: Option<PathBuf>,
    /// Where the brief file goes.
    pub brief_dir: PathBuf,
    /// A brief the caller already wrote (the Core does, card #446): used as
    /// is, `brief` is ignored, and its folder is the one `--add-dir` names.
    pub brief_file: Option<PathBuf>,
    /// The first prompt; default [`prompt_for`] the brief file. It must name
    /// no shell metacharacter, as it reaches `claude.cmd` on Windows.
    pub prompt: Option<String>,
    /// Set on the `claude` process (card #565).
    pub env: Vec<(String, String)>,
}

/// Characters that never go on a spawn's command line: `cmd.exe` would act
/// on them when `claude` is `claude.cmd`.
const UNSAFE: &[char] = &['"', '%', '&', '|', '^', '<', '>', '\n', '\r', '!'];

/// A started agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    /// The short id `claude --bg` printed: the session id's first 8 hex.
    pub short_id: String,
    pub name: String,
    pub brief_file: PathBuf,
    /// What `claude --bg` printed, colours removed.
    pub output: String,
}

/// The spawner as a seam (like the Core's `ReviveFn`): slice 2 hands one to
/// the Core, and tests replace it with a fake.
pub type SpawnFn = Arc<dyn Fn(&SpawnSpec) -> Result<Spawned, String> + Send + Sync>;

/// The real spawner: [`spawn`].
pub fn spawner() -> SpawnFn {
    Arc::new(spawn)
}

/// Starts `spec` with the `claude` found on PATH.
pub fn spawn(spec: &SpawnSpec) -> Result<Spawned, String> {
    spawn_with(&find_claude()?, spec)
}

/// Starts `spec` with `bin`, checking this device's Claude Code trust.
pub fn spawn_with(bin: &Path, spec: &SpawnSpec) -> Result<Spawned, String> {
    spawn_in(bin, spec, &TrustSet::load())
}

/// Starts `spec` with `bin` (a fake `claude` in tests) under trust `t`.
pub(crate) fn spawn_in(bin: &Path, spec: &SpawnSpec, t: &TrustSet) -> Result<Spawned, String> {
    let args = check(spec)?;
    if !spec.cwd.is_dir() {
        return Err(format!("{} isn't a folder", spec.cwd.display()));
    }
    trusted_in(t, &spec.cwd)?;
    let brief_file = match &spec.brief_file {
        Some(f) if f.is_file() => f.clone(),
        Some(f) => return Err(format!("no brief at {}", f.display())),
        None => {
            std::fs::create_dir_all(&spec.brief_dir)
                .map_err(|e| format!("make {}: {e}", spec.brief_dir.display()))?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let f = spec.brief_dir.join(format!("{}-{stamp}.md", spec.name));
            std::fs::write(&f, &spec.brief).map_err(|e| format!("write {}: {e}", f.display()))?;
            f
        }
    };
    let prompt = spec
        .prompt
        .clone()
        .unwrap_or_else(|| prompt_for(&brief_file));
    if prompt.trim().is_empty() || prompt.starts_with('-') || prompt.contains(UNSAFE) {
        return Err("the first prompt can't go on a command line as it is".into());
    }
    let mut cmd = std::process::Command::new(bin);
    cmd.args(&args)
        .arg(prompt)
        .current_dir(&spec.cwd)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    hide_console(&mut cmd);
    let (ok, text) = run(cmd)?;
    if !ok {
        return Err(format!("claude --bg failed: {text}"));
    }
    match short_id(&text) {
        Some(id) => Ok(Spawned {
            short_id: id,
            name: spec.name.clone(),
            brief_file,
            output: text,
        }),
        None => Err(format!("claude --bg printed no session id: {text}")),
    }
}

/// Ends a spawned session (card #446's `EndFn`): `claude stop`, then
/// `claude rm`, which deletes the session's job and its worktree (one it
/// was started in with `-w`, or one it entered) only when nothing in it is
/// unpushed, and otherwise keeps it and says why. Right after `stop` the
/// worktree's lock can still name the exiting process, so `rm` is retried
/// for a few seconds. `Ok` carries what `rm` printed ("kept …" when it
/// kept the worktree).
///
/// Card #565: `rm` deletes a worktree without Irori's checks (ignored
/// files, reflog-only commits, its own refs, locks), so it is never run
/// while the job's folder ([`ListedAgent::cwd`]) or the worktree its job
/// record names (`worktreePath` in `<claude home>/jobs/<job>/state.json`)
/// is a tree still on disk, nor when either can't be known. Only `stop`
/// runs then, and the report starts with
/// [`irori_core::spawn::STOPPED_ONLY`]; the Core's reap asks again once
/// Irori's own cleanup has removed the tree.
///
/// `id` is the session id Irori knows the agent by, but `stop` and `rm`
/// take the job's short id (card #474: given the session id they said "No
/// job matching", and the end was tried again every sweep for hours), so it
/// is looked up in `claude agents --json --all`. A session that list no
/// longer has was removed already: that is an end, not a failure.
pub fn end(id: &str) -> Result<String, String> {
    let bin = find_claude()?;
    let listed = list_agents_with(&bin);
    end_in(
        &bin,
        id,
        listed,
        jobs_dir().as_deref(),
        Duration::from_secs(8),
    )
}

/// Claude Code's job records: `<CLAUDE_CONFIG_DIR or ~/.claude>/jobs`.
fn jobs_dir() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d).join("jobs")),
        None => std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(|h| PathBuf::from(h).join(".claude").join("jobs")),
    }
}

/// [`end`] with `bin`, the agent list as it came, and the job records in
/// `jobs` (tests pass fakes).
pub(crate) fn end_in(
    bin: &Path,
    id: &str,
    listed: Result<Vec<ListedAgent>, String>,
    jobs: Option<&Path>,
    patience: Duration,
) -> Result<String, String> {
    session_arg(id)?;
    let (job, cwd) = match listed {
        Ok(list) => match list.iter().find(|a| a.session_id == id.trim()) {
            Some(a) if !a.job_id.is_empty() => (a.job_id.clone(), Some(a.cwd.clone())),
            Some(a) => (job_id(id), Some(a.cwd.clone())),
            None => return Ok(format!("{GONE_JOB} {}", id.trim())),
        },
        Err(_) => (job_id(id), None),
    };
    let job = session_arg(&job)?.to_string();
    if let Some(why) = holds_tree(cwd.as_deref(), jobs, &job) {
        return stop_only(bin, &job, &why);
    }
    end_with(bin, &job, patience)
}

/// Why `rm` mustn't run for `job` in `cwd` now (card #565): its folder, or
/// the worktree its job record names, is a tree still on disk, or one of
/// them can't be known. `None` when `rm` can delete nothing of Irori's.
fn holds_tree(cwd: Option<&str>, jobs: Option<&Path>, job: &str) -> Option<String> {
    let Some(cwd) = cwd else {
        return Some("Claude Code's agent list couldn't be read, so its folder isn't known".into());
    };
    if irori_core::treeclean::tree_on_disk(cwd) {
        return Some(format!(
            "its folder {cwd} is a worktree Irori removes itself"
        ));
    }
    let Some(jobs) = jobs else {
        return Some("Claude Code's job folder isn't known".into());
    };
    let state = jobs.join(job).join("state.json");
    let text = match std::fs::read_to_string(&state) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(format!("{}: {e}", state.display())),
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return Some(format!("{}: {e}", state.display())),
    };
    match v.get("worktreePath") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(w)) => {
            let p = Path::new(w);
            (p.exists() || irori_core::treeclean::tree_on_disk(w))
                .then(|| format!("its worktree {w} is still there"))
        }
        Some(_) => Some(format!(
            "{}: a worktreePath that isn't text",
            state.display()
        )),
    }
}

/// `claude stop` alone, for a job `rm` must wait on ([`holds_tree`]).
fn stop_only(bin: &Path, job: &str, why: &str) -> Result<String, String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["stop", job])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    hide_console(&mut cmd);
    let (_, stopped) = run(cmd)?;
    if no_job(&stopped) {
        return Ok(format!("{GONE_JOB} {job}"));
    }
    Ok(format!(
        "{} {why} (stop said: {})",
        irori_core::spawn::STOPPED_ONLY,
        stopped.trim()
    ))
}

/// How [`end`] starts its report for a job Claude Code no longer has.
pub const GONE_JOB: &str = "no job for";

/// The job id `claude stop`, `rm`, `logs` and `attach` take for session
/// `id`: Claude Code names a `--bg` job by its session id's first 8 hex
/// digits, and says "No job matching" to the full session id (card #474).
/// Anything that isn't a full session id is passed on as it is.
pub fn job_id(id: &str) -> String {
    let id = id.trim();
    match id.split_once('-') {
        Some((head, _)) if id.len() == 36 && head.len() == 8 => head.to_string(),
        _ => id.to_string(),
    }
}

/// Whether `claude stop` or `rm` said it has no such job.
fn no_job(text: &str) -> bool {
    text.contains("No job matching")
}

/// A session id as it may go on a command line: 8+ hex digits and dashes,
/// never a flag.
fn session_arg(id: &str) -> Result<&str, String> {
    let id = id.trim();
    if id.len() < 8 || id.starts_with('-') || !id.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
    {
        return Err(format!("not a session id: {id:?}"));
    }
    Ok(id)
}

/// A spawned session's recent terminal output, read only (card #456,
/// sa-2's Logs): `claude logs <id>`, colours removed.
pub fn logs(id: &str) -> Result<String, String> {
    logs_with(&find_claude()?, id)
}

pub(crate) fn logs_with(bin: &Path, id: &str) -> Result<String, String> {
    let job = job_id(session_arg(id)?);
    let id = job.as_str();
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["logs", id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    hide_console(&mut cmd);
    let (ok, text) = run(cmd)?;
    if !ok {
        return Err(format!("claude logs {id}: {text}"));
    }
    Ok(text)
}

/// The argv a card's terminal tab runs to attach to a spawned session
/// (card #456, sa-2's Attach): `claude attach <id>`. A `claude.cmd` goes
/// through `cmd.exe /c`, as a pty can't start a batch file itself; the id
/// is plain hex, so nothing in it reaches cmd's parsing.
pub fn attach_argv(id: &str) -> Result<Vec<String>, String> {
    attach_argv_with(&find_claude()?, id)
}

pub(crate) fn attach_argv_with(bin: &Path, id: &str) -> Result<Vec<String>, String> {
    let job = job_id(session_arg(id)?);
    let id = job.as_str();
    let b = bin.to_string_lossy().into_owned();
    let batch = bin
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let mut argv = Vec::new();
    if batch {
        argv.extend(["cmd.exe".to_string(), "/c".to_string()]);
    }
    argv.extend([b, "attach".to_string(), id.to_string()]);
    Ok(argv)
}

pub(crate) fn end_with(bin: &Path, id: &str, patience: Duration) -> Result<String, String> {
    let id = session_arg(id)?;
    let call = |verb: &str| {
        let mut cmd = std::process::Command::new(bin);
        cmd.args([verb, id])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        hide_console(&mut cmd);
        run(cmd)
    };
    let (_, stopped) = call("stop")?;
    let deadline = Instant::now() + patience;
    loop {
        let (ok, text) = call("rm")?;
        let locked = text.contains("still running");
        if ok && !(locked && Instant::now() < deadline) {
            return Ok(text);
        }
        if !locked && text.contains("kept") {
            return Ok(text);
        }
        if !ok && no_job(&text) {
            // Removed already (by hand, or an earlier end): nothing left.
            return Ok(format!("{GONE_JOB} {id}"));
        }
        if !ok && !locked {
            return Err(format!("claude rm {id}: {text} (stop said: {stopped})"));
        }
        if Instant::now() >= deadline {
            return Ok(text);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Claude Code's agents on this device, finished ones included (card
/// #447's restart cleanup): `claude agents --json --all`. Each is its
/// session id and whether it still runs.
pub fn list_agents() -> Result<Vec<ListedAgent>, String> {
    list_agents_with(&find_claude()?)
}

/// One of Claude Code's agents as `claude agents --json --all` lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListedAgent {
    pub session_id: String,
    /// The short id `claude stop` and `rm` take; `""` for an interactive
    /// session, which has none.
    pub job_id: String,
    /// Whether it still runs.
    pub live: bool,
    /// The folder it runs in.
    pub cwd: String,
}

/// How long `claude agents` gets before it counts as hung (card #565
/// finding 12): a sweep waits on it, and a removal needs its answer.
pub const LIST_PATIENCE: Duration = Duration::from_secs(30);

fn list_agents_with(bin: &Path) -> Result<Vec<ListedAgent>, String> {
    list_agents_within(bin, &["agents", "--json", "--all"], LIST_PATIENCE)
}

/// [`list_agents_with`] running `bin args`: ended, and an `Err`, once
/// `patience` is up.
pub(crate) fn list_agents_within(
    bin: &Path,
    args: &[&str],
    patience: Duration,
) -> Result<Vec<ListedAgent>, String> {
    use std::io::Read;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    hide_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("claude agents: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("claude agents: no stdout")?;
    // Read on a thread, so a full pipe can't stall the wait; on a hang it
    // is left behind with the pipe (a grandchild the kill missed holds it).
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = stdout.read_to_end(&mut buf).map(|_| buf);
        let _ = tx.send(r);
    });
    let deadline = Instant::now() + patience;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "claude agents gave no answer in {}s",
                    patience.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("claude agents: {e}")),
        }
    };
    if !status.success() {
        return Err(format!("claude agents exited with {status}"));
    }
    let left = deadline.saturating_duration_since(Instant::now()) + Duration::from_secs(1);
    let out = match rx.recv_timeout(left) {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => return Err(format!("claude agents: {e}")),
        Err(_) => return Err("claude agents: its output never ended".into()),
    };
    parse_agents(&String::from_utf8_lossy(&out))
}

/// Parses `claude agents --json --all`: an array of agents, each with a
/// `sessionId`; a `--bg` one has an `id` (its job), and a running one a
/// `pid`. A `state` of done with a `pid` is a session still running, idle
/// after its turn (card #474: card-396-2 sat like that holding its card),
/// so only the `pid` says whether it runs.
///
/// Card #565 made it a safety gate (no tree goes while an agent is in it),
/// so it fails closed (finding 11): a shape it doesn't know is an `Err`,
/// not an empty list, and so is an entry that isn't an object, or a live
/// one with no `sessionId` or a `cwd` that isn't text. A finished entry
/// with no `sessionId` is skipped, as before.
pub fn parse_agents(json: &str) -> Result<Vec<ListedAgent>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| format!("claude agents: {e}"))?;
    let list = match &v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(o) => o
            .get("agents")
            .and_then(|a| a.as_array())
            .ok_or("claude agents: an object with no agents list")?,
        _ => return Err("claude agents: not a list".into()),
    };
    let mut out = Vec::new();
    for a in list {
        let o = a
            .as_object()
            .ok_or("claude agents: an entry that isn't an object")?;
        let live = o.get("pid").is_some_and(|p| !p.is_null());
        let text = |k: &str| o.get(k).and_then(|s| s.as_str()).map(str::to_string);
        let Some(session_id) = text("sessionId") else {
            if live {
                return Err("claude agents: a running agent with no sessionId".into());
            }
            continue;
        };
        if live && o.get("cwd").is_some_and(|c| !c.is_string() && !c.is_null()) {
            return Err("claude agents: a running agent whose cwd isn't text".into());
        }
        out.push(ListedAgent {
            session_id,
            job_id: text("id").unwrap_or_default(),
            live,
            cwd: text("cwd").unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The fixed first prompt: it names the brief file and nothing else.
pub fn prompt_for(brief_file: &Path) -> String {
    format!(
        "Your brief is in the file {}. Read it with the Read tool and do what it says.",
        brief_file.display()
    )
}

/// The flags for `spec`, after checking every value that reaches argv.
fn check(spec: &SpawnSpec) -> Result<Vec<String>, String> {
    if !plain_word(&spec.name, 64) {
        return Err(format!(
            "an agent name is 1-64 letters, digits, '.', '_' or '-', not {:?}",
            spec.name
        ));
    }
    if !PERMISSION_MODES.contains(&spec.permission_mode.as_str()) {
        return Err(format!(
            "a spawn always names its permission mode ({}), not {:?}",
            PERMISSION_MODES.join(", "),
            spec.permission_mode
        ));
    }
    let mut a: Vec<String> = vec![
        "--bg".into(),
        "--name".into(),
        spec.name.clone(),
        "--permission-mode".into(),
        spec.permission_mode.clone(),
    ];
    for (flag, v) in [("--model", &spec.model), ("--effort", &spec.effort)] {
        if let Some(v) = v {
            if !plain_word(v, 80) {
                return Err(format!("{flag} takes a plain word, not {v:?}"));
            }
            a.push(flag.into());
            a.push(v.clone());
        }
    }
    if let Some(s) = &spec.settings {
        let t = s.to_string_lossy();
        if !s.is_file() || t.contains(UNSAFE) {
            return Err(format!("not a usable settings file: {}", s.display()));
        }
        a.push("--settings".into());
        a.push(t.into_owned());
    }
    let dir = match &spec.brief_file {
        Some(f) => f.parent().map(Path::to_path_buf).unwrap_or_default(),
        None => spec.brief_dir.clone(),
    };
    let b = dir.to_string_lossy();
    if b.is_empty() || b.contains(UNSAFE) {
        return Err(format!(
            "the brief folder {b:?} can't be named on a command line"
        ));
    }
    // Outside the agent's folder a Read asks permission, so in `default`
    // mode the agent would stop at a prompt before reading its brief (#445).
    // `--add-dir` takes several folders, so it goes first: the next flag ends
    // it. Last, it swallowed the prompt.
    a.splice(0..0, ["--add-dir".to_string(), b.into_owned()]);
    Ok(a)
}

fn plain_word(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && !s.starts_with(['-', '.'])
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Runs `cmd` to its end (or [`SPAWN_WAIT`]), returning success and its
/// output. Never kills it: a slow exit may be the session starting.
fn run(mut cmd: std::process::Command) -> Result<(bool, String), String> {
    let mut child = cmd.spawn().map_err(|e| format!("start claude: {e}"))?;
    let out = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut readers = Vec::new();
    for pipe in [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let out = out.clone();
        readers.push(std::thread::spawn(move || copy_capped(pipe, &out)));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    let status = rx.recv_timeout(SPAWN_WAIT);
    let deadline = Instant::now() + Duration::from_secs(3);
    for r in readers {
        while !r.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let text = strip_ansi(&lossy(&out)).trim().to_string();
    match status {
        Ok(Ok(s)) => Ok((s.success(), text)),
        Ok(Err(e)) => Err(format!("claude --bg: {e}")),
        Err(_) => Ok((true, text)),
    }
}

/// The short id in `claude --bg`'s "backgrounded · <id> · <name>" line.
pub fn short_id(out: &str) -> Option<String> {
    for line in out.lines() {
        let Some(rest) = line.trim().strip_prefix("backgrounded") else {
            continue;
        };
        let id = rest
            .trim_start_matches(|c: char| c.is_whitespace() || c == '·')
            .split(|c: char| c.is_whitespace() || c == '·')
            .next()
            .unwrap_or("");
        if id.len() >= 8 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(id.to_ascii_lowercase());
        }
    }
    None
}

/// The spawned session among the roster's cards: the one with its name whose
/// session id starts with its short id. The name alone could be an older
/// session's, and the prefix alone is only 8 hex.
pub fn find_session<'a>(
    cards: impl IntoIterator<Item = &'a presence::AgentCard>,
    name: &str,
    short_id: &str,
) -> Option<&'a presence::AgentCard> {
    let id = short_id.to_ascii_lowercase();
    if id.len() < 8 {
        return None;
    }
    cards.into_iter().find(|c| {
        c.agent == "claude" && c.name == name && c.session_id.to_ascii_lowercase().starts_with(&id)
    })
}

/// The folder whose trust entry Claude Code checks for `dir` when `dir` is in
/// a git repo: the main repo's root, also for a worktree. `None` outside git.
pub fn trust_root(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(p) = d {
        let dot = p.join(".git");
        if dot.is_dir() {
            return Some(p.to_path_buf());
        }
        if dot.is_file() {
            return main_root(p, &dot).or_else(|| Some(p.to_path_buf()));
        }
        d = p.parent();
    }
    None
}

/// A worktree's main repo root from its `.git` file: `gitdir: <common>/worktrees/<n>`,
/// whose `commondir` names the common `.git`.
fn main_root(wt: &Path, dot: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(dot).ok()?;
    let g = raw.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let gitdir = if Path::new(g).is_absolute() {
        PathBuf::from(g)
    } else {
        wt.join(g)
    };
    let common = match std::fs::read_to_string(gitdir.join("commondir")) {
        Ok(c) => {
            let c = c.trim();
            if Path::new(c).is_absolute() {
                PathBuf::from(c)
            } else {
                gitdir.join(c)
            }
        }
        // A submodule's gitdir has no commondir: it is its own repo.
        Err(_) => return Some(wt.to_path_buf()),
    };
    let common = std::fs::canonicalize(&common).map(plain).unwrap_or(common);
    common.parent().map(Path::to_path_buf)
}

/// A canonical path without Windows' verbatim prefix: `\\?\C:\x` is `C:\x`
/// and `\\?\UNC\s\x` is `\\s\x`. Git refuses verbatim paths, and Claude
/// Code's trust keys never have one (PC run on #445).
pub fn plain(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

/// Whether Claude Code will start a `--bg` session in `dir`, by its own
/// trust entries; `Err` is the reason to show, with the way to fix it.
pub fn trusted(dir: &Path) -> Result<(), String> {
    trusted_in(&TrustSet::load(), dir)
}

/// Whether this computer has the `claude` command (card #541).
pub fn claude_installed() -> bool {
    find_claude().is_ok()
}

/// [`trusted`] as a yes or no, for the Core's `Spawner { trusted }` seam
/// (card #446): the Core checks before it spawns and writes the reason.
pub fn trust_ok(dir: &Path) -> bool {
    trusted(dir).is_ok()
}

pub(crate) fn trusted_in(t: &TrustSet, dir: &Path) -> Result<(), String> {
    let (ok, folder) = match trust_root(dir) {
        Some(root) => (t.has(&root.to_string_lossy()), root),
        None => (t.covers(&dir.to_string_lossy()), dir.to_path_buf()),
    };
    if ok {
        return Ok(());
    }
    Err(format!(
        "Can't start an agent: {} isn't trusted for Claude Code on this device. \
         To trust it, run `cd \"{}\" && claude`, accept the trust prompt, then type /exit.",
        folder.display(),
        folder.display()
    ))
}

#[cfg(test)]
mod tests {

    #[test]
    fn claude_agents_json_gives_sessions_and_whether_they_run() {
        let got = parse_agents(
            r#"[
  {"id":"31d4b3cd","cwd":"/r","kind":"background","startedAt":1,"sessionId":"31d4b3cd-ded7-4c48-8f15-a7bbd8b90af9","name":"probe","state":"failed"},
  {"pid":12301,"cwd":"/h","kind":"interactive","startedAt":2,"sessionId":"e0e01949-5ac0-4312-a44c-85938569a94e","name":"salvia","status":"idle"},
  {"pid":12302,"kind":"background","sessionId":"aa","state":"working","status":"busy"},
  {"kind":"background","sessionId":"bb","state":"done"},
  {"pid":34422,"id":"28b6946e","kind":"background","sessionId":"cc","status":"idle","state":"done"},
  {"name":"no session"}
]"#,
        )
        .unwrap();
        assert_eq!(got[0].cwd, "/r");
        assert_eq!(got[2].cwd, "", "none listed");
        let got: Vec<(&str, &str, bool)> = got
            .iter()
            .map(|a| (a.session_id.as_str(), a.job_id.as_str(), a.live))
            .collect();
        assert_eq!(
            got,
            [
                ("31d4b3cd-ded7-4c48-8f15-a7bbd8b90af9", "31d4b3cd", false),
                ("e0e01949-5ac0-4312-a44c-85938569a94e", "", true),
                ("aa", "", true),
                ("bb", "", false),
                // Done, but still running, idle: not to be stopped.
                ("cc", "28b6946e", true),
            ]
        );
        assert_eq!(job_id("6fa86d9b-cbdd-4fd2-9cc1-3252d7f87606"), "6fa86d9b");
        assert_eq!(job_id("0a1b2c3d"), "0a1b2c3d");
        assert!(parse_agents("not json").is_err());
    }

    /// Card #565 finding 11: what it can't read is an error, never "nobody
    /// is running".
    #[test]
    fn claude_agents_json_it_cant_read_is_an_error_not_an_empty_list() {
        for bad in [
            r#"{"jobs":[]}"#,
            r#"{"agents":{}}"#,
            r#""none""#,
            "42",
            "null",
            "[1]",
            r#"[{"pid":5,"cwd":"/x"}]"#,
            r#"[{"pid":5,"sessionId":"a","cwd":7}]"#,
        ] {
            assert!(parse_agents(bad).is_err(), "{bad}");
        }
        let ok = parse_agents(r#"{"agents":[{"sessionId":"a","pid":1,"cwd":"/w"}]}"#).unwrap();
        assert_eq!(ok[0].cwd, "/w");
        assert!(ok[0].live);
        assert!(parse_agents("[]").unwrap().is_empty());
    }

    /// Card #565 finding 12: a `claude agents` that hangs is ended and is
    /// an error, so one hang doesn't stop every sweep after it.
    #[test]
    fn claude_agents_that_hangs_is_ended_and_an_error() {
        let (bin, args): (&str, &[&str]) = if cfg!(windows) {
            ("ping", &["-n", "60", "127.0.0.1"])
        } else {
            ("sleep", &["60"])
        };
        let t0 = Instant::now();
        let r = list_agents_within(Path::new(bin), args, Duration::from_secs(1));
        assert!(r.as_ref().is_err_and(|e| e.contains("no answer")), "{r:?}");
        assert!(t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
    }

    #[test]
    fn claude_agents_that_answers_in_time_is_read() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("agents.json");
        std::fs::write(&f, r#"[{"sessionId":"a","pid":1,"cwd":"/w"}]"#).unwrap();
        let f = f.to_string_lossy().into_owned();
        let (bin, args): (&str, Vec<&str>) = if cfg!(windows) {
            ("cmd", vec!["/c", "type", &f])
        } else {
            ("cat", vec![&f])
        };
        let got = list_agents_within(Path::new(bin), &args, Duration::from_secs(20)).unwrap();
        assert_eq!(got[0].session_id, "a");
        assert_eq!(got[0].cwd, "/w");
    }
    use super::*;

    fn cfg(dir: &Path, trusted: &[&Path]) -> TrustSet {
        let mut projects = serde_json::Map::new();
        for p in trusted {
            projects.insert(
                p.to_string_lossy().into_owned(),
                serde_json::json!({ "hasTrustDialogAccepted": true }),
            );
        }
        let f = dir.join(".claude.json");
        std::fs::write(&f, serde_json::json!({ "projects": projects }).to_string()).unwrap();
        TrustSet::load_file(&f)
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn repo_under_trusted_parent_is_not_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let repo = base.join("projects").join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let plain = base.join("projects").join("notes");
        std::fs::create_dir_all(&plain).unwrap();
        let t = cfg(&base, &[&base.join("projects")]);
        let e = trusted_in(&t, &repo).unwrap_err();
        assert!(e.contains("isn't trusted") && e.contains("claude"), "{e}");
        assert!(
            trusted_in(&t, &plain).is_ok(),
            "a plain folder follows its parent"
        );
        let t = cfg(&base, &[&repo]);
        assert!(trusted_in(&t, &repo).is_ok());
        assert!(trusted_in(&t, &repo.join("sub")).is_ok() || !repo.join("sub").exists());
    }

    /// Card #731, checked live on Claude Code 2.1.319: a folder outside git
    /// follows its nearest trusted ancestor, however deep; a git repo (and
    /// any folder in it) doesn't; nothing counts in an untrusted tree.
    #[test]
    fn ancestors_count_outside_git_only() {
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let home = base.join("home");
        let deep = home.join("a").join("b").join("c");
        let repo = home.join("projects").join("rune");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let other = base.join("elsewhere").join("x");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(base.join("elsewhere").join("r").join(".git")).unwrap();

        let t = cfg(&base, &[&home]);
        assert!(
            trusted_in(&t, &deep).is_ok(),
            "a plain folder deep under a trusted one"
        );
        for d in [&repo, &repo.join("src")] {
            let e = trusted_in(&t, d).unwrap_err();
            assert!(
                e.contains(&repo.display().to_string()),
                "names the repo: {e}"
            );
        }
        assert!(trusted_in(&t, &other).is_err(), "an untrusted tree");
        assert!(trusted_in(&t, &base.join("elsewhere").join("r")).is_err());

        let t = cfg(&base, &[&home, &repo]);
        assert!(trusted_in(&t, &repo).is_ok());
        assert!(
            trusted_in(&t, &repo.join("src")).is_ok(),
            "inside a trusted repo"
        );
        assert!(trusted_in(&TrustSet::load_file(&base.join("none.json")), &deep).is_err());
    }

    /// Windows keys in `.claude.json` (`C:/...`, or backslashes) match the
    /// folder however it is written, and a parent counts for a plain folder.
    #[test]
    fn windows_style_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join(".claude.json");
        std::fs::write(
            &f,
            r#"{"projects":{"C:/Users/me":{"hasTrustDialogAccepted":true},
                "C:\\work\\api":{"hasTrustDialogAccepted":true},
                "C:/work/web":{"hasTrustDialogAccepted":false}}}"#,
        )
        .unwrap();
        let t = TrustSet::load_file(&f);
        assert!(t.has("C:/work/api") && t.has("C:/work/api/"));
        assert!(t.has(r"C:\Users\me\") && t.has("C:/Users/me"));
        assert!(!t.has("C:/work/web"), "false isn't trust");
        assert!(!t.has("C:/Users/me/projects/rune"), "has is exact");
        assert!(t.covers("C:/Users/me/projects/notes"));
        assert!(!t.covers("C:/work/web/x") && !t.covers("D:/Users/me"));
        if cfg!(windows) {
            assert!(t.covers(r"C:\Users\me\projects\notes"));
            assert!(t.has(r"c:\work\API"), "Windows paths ignore case");
        }
    }

    #[test]
    fn worktree_follows_its_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "i",
            ],
        );
        let wt = base.join("repo-wt");
        git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);
        assert_eq!(trust_root(&wt).as_deref(), Some(repo.as_path()));
        assert_eq!(trust_root(&wt.join("x")).as_deref(), Some(repo.as_path()));
        assert!(trusted_in(&cfg(&base, &[&repo]), &wt).is_ok());
        assert!(
            trusted_in(&cfg(&base, &[&wt]), &wt).is_err(),
            "the repo's entry counts, not the worktree's"
        );
        assert_eq!(trust_root(&base), None);
    }

    #[test]
    fn short_ids() {
        assert_eq!(
            short_id("Starting background service…\nbackgrounded · f517b512\n  claude agents"),
            Some("f517b512".into())
        );
        assert_eq!(
            short_id("backgrounded · 6A8E564A · irori-445-permprobe"),
            Some("6a8e564a".into())
        );
        assert_eq!(short_id("Workspace not trusted."), None);
    }

    #[test]
    fn spec_checks() {
        let ok = SpawnSpec {
            name: "pawpaw".into(),
            permission_mode: "acceptEdits".into(),
            model: Some("sonnet".into()),
            brief_dir: PathBuf::from("/tmp/briefs"),
            ..Default::default()
        };
        let a = check(&ok).unwrap();
        assert_eq!(
            a,
            [
                "--add-dir",
                "/tmp/briefs",
                "--bg",
                "--name",
                "pawpaw",
                "--permission-mode",
                "acceptEdits",
                "--model",
                "sonnet"
            ]
        );
        let no_mode = SpawnSpec {
            permission_mode: String::new(),
            ..ok.clone()
        };
        assert!(check(&no_mode).unwrap_err().contains("permission mode"));
        for bad in ["", "-x", "a b", "a&b", "x\"y", "%PATH%"] {
            let s = SpawnSpec {
                name: bad.into(),
                ..ok.clone()
            };
            assert!(check(&s).is_err(), "name {bad:?}");
        }
        let m = SpawnSpec {
            model: Some("opus & calc".into()),
            ..ok.clone()
        };
        assert!(check(&m).is_err());
    }

    /// The arguments the fake recorded: one per line off Windows; on Windows
    /// the raw `%*` line, split the way Rust quoted it (spaces outside quotes).
    fn args_of(raw: &str) -> Vec<String> {
        if !cfg!(windows) {
            return raw.lines().map(str::to_string).collect();
        }
        let mut out = Vec::new();
        let (mut cur, mut quoted, mut any) = (String::new(), false, false);
        for c in raw.trim_end_matches(['\r', '\n']).chars() {
            match c {
                '"' => {
                    quoted = !quoted;
                    any = true;
                }
                ' ' if !quoted => {
                    if any || !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                    any = false;
                }
                _ => cur.push(c),
            }
        }
        if any || !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    #[test]
    fn verbatim_prefix_is_stripped() {
        assert_eq!(
            plain(PathBuf::from(r"\\?\C:\Users\x")),
            PathBuf::from(r"C:\Users\x")
        );
        assert_eq!(
            plain(PathBuf::from(r"\\?\UNC\srv\share\x")),
            PathBuf::from(r"\\srv\share\x")
        );
        assert_eq!(plain(PathBuf::from("/Users/x")), PathBuf::from("/Users/x"));
    }

    /// A fake `claude` that writes each argument on its own line to
    /// `argv.txt` beside it and prints what `claude --bg` prints.
    fn fake_claude(dir: &Path) -> PathBuf {
        #[cfg(windows)]
        {
            let p = dir.join("claude.cmd");
            let log = dir.join("argv.txt");
            std::fs::write(
                &p,
                format!(
                    "@echo off\r\n(echo %*) > \"{}\"\r\necho backgrounded · 0a1b2c3d · fake\r\n",
                    log.display()
                ),
            )
            .unwrap();
            p
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("claude");
            let log = dir.join("argv.txt");
            std::fs::write(
                &p,
                format!(
                    "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\necho 'Starting background service…'\necho 'backgrounded · 0a1b2c3d · fake'\n",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    #[test]
    fn spawns_with_a_fake_claude_and_keeps_the_brief_off_argv() {
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let bin = fake_claude(&base);
        let mut brief =
            String::from("Card #445. \"quoted\" & ampersand | pipe %PATH% ^caret <lt> >gt\n");
        while brief.len() < 9 * 1024 {
            brief.push_str("more text with & and | and % and \" in it.\n");
        }
        let spec = SpawnSpec {
            cwd: repo.clone(),
            name: "pawpaw".into(),
            brief: brief.clone(),
            permission_mode: "acceptEdits".into(),
            effort: Some("low".into()),
            brief_dir: base.join("briefs"),
            ..Default::default()
        };
        let untrusted = cfg(&base, &[&base]);
        assert!(
            spawn_in(&bin, &spec, &untrusted)
                .unwrap_err()
                .contains("isn't trusted")
        );
        assert!(
            !base.join("argv.txt").exists(),
            "nothing ran for an untrusted repo"
        );

        let got = spawn_in(&bin, &spec, &cfg(&base, &[&repo])).unwrap();
        assert_eq!(got.short_id, "0a1b2c3d");
        assert_eq!(std::fs::read_to_string(&got.brief_file).unwrap(), brief);
        let raw = std::fs::read_to_string(base.join("argv.txt")).unwrap();
        let lines = args_of(&raw);
        assert!(
            !lines.iter().any(|l| l == "-w" || l == "--worktree"),
            "never claude -w (card #565): {lines:?}"
        );
        let argv = lines.join("\n");
        for bad in ["&", "|", "%", "\"", "quoted", "ampersand"] {
            assert!(!argv.contains(bad), "{bad:?} reached argv: {argv}");
        }
        assert!(argv.len() < 1024, "argv is short: {} bytes", argv.len());
        assert_eq!(lines[0], "--add-dir");
        assert_eq!(
            &lines[2..9],
            [
                "--bg",
                "--name",
                "pawpaw",
                "--permission-mode",
                "acceptEdits",
                "--effort",
                "low"
            ]
        );
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("Your brief is in the file "),
            "the prompt is last: {argv}"
        );
        assert!(
            !lines[..lines.len() - 1]
                .iter()
                .any(|l| l.starts_with("Your brief")),
            "{argv}"
        );

        // The roster then has the session under its name and id prefix.
        let cards = vec![
            card("pawpaw", "99999999-0000-4000-8000-000000000001"),
            card("pawpaw", "0a1b2c3d-1111-4111-8111-000000000002"),
        ];
        let c = find_session(&cards, &got.name, &got.short_id).unwrap();
        assert_eq!(c.session_id, "0a1b2c3d-1111-4111-8111-000000000002");
    }

    /// Live, by hand: `IRORI_LIVE_SPAWN_REPO=<a trusted repo> cargo test -p
    /// irori-window --lib live_spawn -- --ignored --nocapture`. Starts a real
    /// agent in the repo's own folder that only replies; stop and rm it after
    /// with the printed id.
    #[test]
    #[ignore]
    fn live_spawn() {
        let repo =
            PathBuf::from(std::env::var("IRORI_LIVE_SPAWN_REPO").expect("IRORI_LIVE_SPAWN_REPO"));
        let briefs = tempfile::tempdir().unwrap();
        let got = spawn(&SpawnSpec {
            cwd: repo,
            name: "irori-live-spawn-probe".into(),
            brief: "Reply with exactly: spawn-probe-ok. Do nothing else: no tools, no files."
                .into(),
            permission_mode: "default".into(),
            model: Some("sonnet".into()),
            effort: Some("low".into()),
            brief_dir: briefs.keep(),
            ..Default::default()
        })
        .unwrap();
        println!(
            "spawned {} ({}); brief {}",
            got.short_id,
            got.name,
            got.brief_file.display()
        );
        println!("{}", got.output);
    }

    #[test]
    fn a_ready_brief_is_used_as_is_and_its_folder_added() {
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let briefs = base.join("core-briefs");
        std::fs::create_dir_all(&briefs).unwrap();
        let f = briefs.join("quince.md");
        std::fs::write(&f, "written by the Core").unwrap();
        let bin = fake_claude(&base);
        let spec = SpawnSpec {
            cwd: repo.clone(),
            name: "quince".into(),
            permission_mode: "default".into(),
            brief_file: Some(f.clone()),
            prompt: Some(format!("Read {} and do what it says.", f.display())),
            ..Default::default()
        };
        let got = spawn_in(&bin, &spec, &cfg(&base, &[&repo])).unwrap();
        assert_eq!(got.brief_file, f);
        let lines = args_of(&std::fs::read_to_string(base.join("argv.txt")).unwrap());
        assert_eq!(&lines[..2], ["--add-dir", briefs.to_str().unwrap()]);
        assert_eq!(
            lines.last().unwrap(),
            &format!("Read {} and do what it says.", f.display())
        );
        let bad = SpawnSpec {
            prompt: Some("do it & calc".into()),
            ..spec.clone()
        };
        assert!(spawn_in(&bin, &bad, &cfg(&base, &[&repo])).is_err());
    }

    /// `end`: stop, then rm, retried while the lock still names the exiting
    /// process, and "kept" (a dirty worktree) is not a failure.
    #[cfg(unix)]
    #[test]
    fn end_stops_then_removes_and_retries_a_locked_rm() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let bin = base.join("claude");
        let log = base.join("calls.txt");
        let n = base.join("n");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$1 $2\" >> '{log}'\nif [ \"$1\" = rm ]; then\n  if [ ! -f '{n}' ]; then touch '{n}'; echo 'kept x — A Claude Code lock on the worktree names a process that is still running'; exit 1; fi\n  echo 'removed x'\nfi\nexit 0\n",
                log = log.display(),
                n = n.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = end_with(&bin, "0a1b2c3d", Duration::from_secs(5)).unwrap();
        assert!(out.contains("removed"), "{out}");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            ["stop 0a1b2c3d", "rm 0a1b2c3d", "rm 0a1b2c3d"]
        );
        assert!(
            end_with(&bin, "--help", Duration::ZERO).is_err(),
            "never a flag"
        );
    }

    /// Card #474: a job Claude Code no longer has ("No job matching" from
    /// stop and rm) is ended, not a failure to try again.
    #[cfg(unix)]
    #[test]
    fn end_of_a_job_already_removed_is_an_end() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let bin = base.join("claude");
        std::fs::write(&bin, "#!/bin/sh\necho \"No job matching '$2'\"\nexit 1\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = end_with(&bin, "6fa86d9b", Duration::from_secs(5)).unwrap();
        assert!(out.starts_with(GONE_JOB), "{out}");
    }

    /// A fake `claude` that appends `<verb> <id>` to `calls.txt` and says
    /// "removed" to `rm`, on any platform.
    fn verb_logger(base: &Path) -> (PathBuf, PathBuf) {
        let log = base.join("calls.txt");
        #[cfg(windows)]
        let bin = {
            let bin = base.join("claude.cmd");
            std::fs::write(
                &bin,
                format!(
                    "@echo off\r\n>> \"{}\" echo %1 %2\r\nif \"%1\"==\"rm\" echo removed x\r\nexit /b 0\r\n",
                    log.display()
                ),
            )
            .unwrap();
            bin
        };
        #[cfg(unix)]
        let bin = {
            use std::os::unix::fs::PermissionsExt;
            let bin = base.join("claude");
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\necho \"$1 $2\" >> '{}'\nif [ \"$1\" = rm ]; then echo 'removed x'; fi\nexit 0\n",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            bin
        };
        (bin, log)
    }

    fn calls(log: &Path) -> Vec<String> {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log);
        let text = std::fs::read_to_string(log).unwrap();
        let out = text.lines().map(|l| l.trim().to_string()).collect();
        std::fs::write(log, "").unwrap();
        out
    }

    /// Card #565 finding A: `claude rm` deletes a job's worktree without
    /// Irori's checks, so it never runs while the job's folder, or the
    /// worktree its job record names, is a tree on disk, nor when the
    /// folder can't be known. Only `stop` runs, the tree stays, and once
    /// the tree is gone `rm` drops the job.
    #[test]
    fn end_never_runs_rm_while_the_jobs_tree_is_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let base = plain(std::fs::canonicalize(tmp.path()).unwrap());
        let (bin, log) = verb_logger(&base);
        let jobs = base.join("jobs");
        let main = base.join("repo");
        let tree = main.join(".claude/worktrees/card-7-1");
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::write(tree.join(".env"), "SECRET=1\n").unwrap();
        let sid = "0a1b2c3d-1111-4111-8111-000000000001";
        let agent = |cwd: &Path| {
            Ok(vec![ListedAgent {
                session_id: sid.into(),
                job_id: "0a1b2c3d".into(),
                live: false,
                cwd: cwd.to_string_lossy().into_owned(),
            }])
        };
        let patience = Duration::from_secs(2);

        // In the tree, or below it: stop only.
        for cwd in [tree.clone(), tree.join("src")] {
            let out = end_in(&bin, sid, agent(&cwd), Some(&jobs), patience).unwrap();
            assert!(out.starts_with(irori_core::spawn::STOPPED_ONLY), "{out}");
            assert!(!out.contains("kept"), "{out}");
            assert_eq!(calls(&log), ["stop 0a1b2c3d"]);
        }
        assert!(tree.join(".env").is_file(), "the tree stays");

        // The list couldn't be read: the folder isn't known, so stop only.
        let out = end_in(&bin, sid, Err("hung".into()), Some(&jobs), patience).unwrap();
        assert!(out.starts_with(irori_core::spawn::STOPPED_ONLY), "{out}");
        assert_eq!(calls(&log), ["stop 0a1b2c3d"]);

        // Started in the main checkout, but its job record names a
        // worktree it entered that is still there: stop only.
        std::fs::create_dir_all(jobs.join("0a1b2c3d")).unwrap();
        let state = jobs.join("0a1b2c3d/state.json");
        std::fs::write(
            &state,
            serde_json::json!({"cwd": main, "worktreePath": tree}).to_string(),
        )
        .unwrap();
        let out = end_in(&bin, sid, agent(&main), Some(&jobs), patience).unwrap();
        assert!(out.starts_with(irori_core::spawn::STOPPED_ONLY), "{out}");
        assert_eq!(calls(&log), ["stop 0a1b2c3d"]);
        // A record Irori can't read: stop only.
        std::fs::write(&state, "{not json").unwrap();
        let out = end_in(&bin, sid, agent(&main), Some(&jobs), patience).unwrap();
        assert!(out.starts_with(irori_core::spawn::STOPPED_ONLY), "{out}");
        assert_eq!(calls(&log), ["stop 0a1b2c3d"]);
        // Aside for its removal still counts as there.
        std::fs::remove_file(&state).unwrap();
        let aside = irori_core::treeclean::aside_path(&tree);
        std::fs::rename(&tree, &aside).unwrap();
        let out = end_in(&bin, sid, agent(&tree), Some(&jobs), patience).unwrap();
        assert!(out.starts_with(irori_core::spawn::STOPPED_ONLY), "{out}");
        assert_eq!(calls(&log), ["stop 0a1b2c3d"]);

        // Once the tree is gone, `rm` drops the job.
        std::fs::remove_dir_all(&aside).unwrap();
        let out = end_in(&bin, sid, agent(&tree), Some(&jobs), patience).unwrap();
        assert!(out.contains("removed"), "{out}");
        assert_eq!(calls(&log), ["stop 0a1b2c3d", "rm 0a1b2c3d"]);
        // So it does for a job in a main checkout with no worktree.
        let out = end_in(&bin, sid, agent(&main), Some(&jobs), patience).unwrap();
        assert!(out.contains("removed"), "{out}");
        assert_eq!(calls(&log), ["stop 0a1b2c3d", "rm 0a1b2c3d"]);
        // A job the list no longer has is gone: nothing runs.
        let out = end_in(&bin, sid, Ok(Vec::new()), Some(&jobs), patience).unwrap();
        assert!(out.starts_with(GONE_JOB), "{out}");
        assert!(calls(&log).is_empty());
    }

    #[test]
    fn logs_runs_claude_logs_with_the_session_id_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let log = dir.join("argv.txt");
        #[cfg(windows)]
        let bin = {
            let p = dir.join("claude.cmd");
            std::fs::write(
                &p,
                format!(
                    "@echo off
(echo %*) > \"{}\"
echo Read src/world/lantern.rs
echo test result: ok
",
                    log.display()
                ),
            )
            .unwrap();
            p
        };
        #[cfg(not(windows))]
        let bin = {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("claude");
            std::fs::write(
                &p,
                format!(
                    "#!/bin/sh
echo \"$@\" > '{}'
echo 'Read src/world/lantern.rs'
echo 'test result: ok'
",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let out = logs_with(&bin, "0a1b2c3d-1111").unwrap();
        assert!(
            out.contains("lantern.rs") && out.contains("test result: ok"),
            "{out}"
        );
        let argv = std::fs::read_to_string(&log).unwrap();
        assert_eq!(argv.trim(), "logs 0a1b2c3d-1111");
        assert!(logs_with(&bin, "--help").is_err(), "never a flag");
        assert!(
            logs_with(&bin, "abc & calc").is_err(),
            "never a metacharacter"
        );
    }

    #[test]
    fn attach_runs_claude_attach_and_a_batch_file_through_cmd() {
        assert_eq!(
            attach_argv_with(Path::new("/usr/bin/claude"), "0a1b2c3d").unwrap(),
            ["/usr/bin/claude", "attach", "0a1b2c3d"]
        );
        // A full session id goes on as its job id (card #474).
        assert_eq!(
            attach_argv_with(
                Path::new("/usr/bin/claude"),
                "6fa86d9b-cbdd-4fd2-9cc1-3252d7f87606"
            )
            .unwrap(),
            ["/usr/bin/claude", "attach", "6fa86d9b"]
        );
        assert_eq!(
            attach_argv_with(Path::new("C:/npm/claude.CMD"), "0a1b2c3d").unwrap(),
            ["cmd.exe", "/c", "C:/npm/claude.CMD", "attach", "0a1b2c3d"]
        );
        assert!(attach_argv_with(Path::new("claude"), "-x").is_err());
    }

    fn card(name: &str, sid: &str) -> presence::AgentCard {
        presence::AgentCard {
            key: format!("dev/{sid}"),
            agent: "claude".into(),
            session_id: sid.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn finds_session_by_name_and_prefix() {
        let cards = vec![
            card("pawpaw", "aaaaaaaa-0000-4000-8000-000000000001"),
            card("other", "f517b512-0000-4000-8000-000000000002"),
            card("pawpaw", "f517b512-7b0d-48ec-a73f-d6f5ef31b3c5"),
        ];
        let c = find_session(&cards, "pawpaw", "F517B512").unwrap();
        assert_eq!(c.session_id, "f517b512-7b0d-48ec-a73f-d6f5ef31b3c5");
        assert!(find_session(&cards, "quince", "f517b512").is_none());
        assert!(find_session(&cards, "pawpaw", "f517").is_none());
    }
}
