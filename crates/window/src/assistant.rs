//! The window's assistant (card #783): the small chat in the corner. Each
//! question is one headless `claude -p` turn on Haiku, through the
//! person's own Claude Code sign-in, with only Irori's read tools: `irori
//! mcp` started with the Core's assistant secret, which the API reads as
//! the person through `tools::ASSISTANT_TOOLS` only.
//!
//! It is no agent: it runs in `<data dir>/assistant`, which presence leaves
//! off the roster (so it never counts toward limits or gets cards), with
//! `--no-session-persistence` (no transcript for capture to index) and no
//! built-in tools.
//!
//! The runner ([`Runner`], [`Job`]) knows nothing of the chat: card #782's
//! background jobs run through it too. The parser ([`Parser`]) and the
//! status lines ([`tool_status`]) are pure, for tests.
//!
//! What Claude Code 2.1.319 does, measured (#783): `--tools ""` removes
//! the built-in tools but keeps `--mcp-config`'s; a variadic flag swallows a
//! trailing prompt, so the prompt goes on stdin; with `--output-format
//! stream-json --verbose --include-partial-messages` the reply's text comes
//! as `stream_event` `text_delta`s, tool calls as `assistant` messages'
//! `tool_use` blocks, and the run ends with one `result` line; signed out,
//! the result says "Not logged in · Please run /login".

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::roster::{find_claude, hide_console};

/// One turn of the chat so far.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AssistantTurn {
    /// `"person"` or `"assistant"`.
    pub role: String,
    pub text: String,
}

/// What a run says, in order: `delta`s of reply text, `tool` status lines
/// (each replaces the last; not part of the reply), then exactly one
/// `done` (text empty) or `error` (one plain sentence for the person).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AssistantEvent {
    /// `"delta"`, `"tool"`, `"done"` or `"error"`.
    pub kind: String,
    pub text: String,
}

impl AssistantEvent {
    pub fn delta(t: &str) -> AssistantEvent {
        AssistantEvent::new("delta", t)
    }
    pub fn tool(t: &str) -> AssistantEvent {
        AssistantEvent::new("tool", t)
    }
    pub fn done() -> AssistantEvent {
        AssistantEvent::new("done", "")
    }
    pub fn error(t: &str) -> AssistantEvent {
        AssistantEvent::new("error", t)
    }
    fn new(kind: &str, text: &str) -> AssistantEvent {
        AssistantEvent {
            kind: kind.into(),
            text: text.into(),
        }
    }
    /// Whether this event ends the run (`done` or `error`).
    pub fn ends(&self) -> bool {
        self.kind == "done" || self.kind == "error"
    }
}

/// No Claude Code on this device (a phone, or a computer without it).
pub const NO_CLAUDE: &str =
    "The assistant can't run on this device yet: it needs Claude Code, which runs on a computer.";
/// Claude Code is there but signed out.
pub const SIGNED_OUT: &str = "The assistant needs Claude Code signed in on this computer.";
/// The run went past [`TIMEOUT`].
pub const TOO_LONG: &str = "The assistant took too long. Try again.";
/// The person stopped it.
pub const STOPPED: &str = "Stopped.";
/// [`MAX_RUNS`] already run.
pub const BUSY: &str = "The assistant is still answering.";

/// How long one run may take, all in.
pub const TIMEOUT: Duration = Duration::from_secs(120);
/// How many runs at once.
pub const MAX_RUNS: usize = 2;
/// How many earlier turns go into the prompt.
pub const HISTORY_TURNS: usize = 12;
/// The model: quick and cheap, enough for looking things up.
pub const MODEL: &str = "haiku";
/// The MCP server's name in the config, so its tools are
/// `mcp__irori__irori_<name>`.
const SERVER: &str = "irori";

/// What the chat's model is told (card #783).
pub const SYSTEM: &str = "You are Irori's assistant, the small chat in the corner of the Irori window. \
You answer the person's questions about their work in Irori: cards and the Board, threads, agents, \
commits, pull requests and notes.

Look things up with the irori tools before answering; never guess. Use search to find things by \
words, show to open one item by its ref, board for what is blocked, working or open, overview for \
what is live now, and agents for who is running.

Answer in a few short sentences or a short list (lines starting with \"- \"), in plain everyday words, \
with no jargon, ids or hashes. **Bold** is fine; no headings, tables or code blocks. Cite cards as #123 \
(tag#123 for a space's board) so the person can click them. If nothing matches, say so plainly and \
suggest what to try.

You only read: you cannot change cards, start agents or post anything. If the person asks for work \
to be done, say that the way to get work done is to post it as a thread from the ask box, and offer \
to find related cards. Irori's settings are not readable yet: say so if asked.";

// ---- the prompt -------------------------------------------------------------

/// The prompt for one question: the last [`HISTORY_TURNS`] turns as a
/// plain transcript, then the new question.
pub fn render_prompt(history: &[AssistantTurn], text: &str) -> String {
    let skip = history.len().saturating_sub(HISTORY_TURNS);
    let mut b = String::new();
    let turns: Vec<&AssistantTurn> = history
        .iter()
        .skip(skip)
        .filter(|t| !t.text.trim().is_empty())
        .collect();
    if !turns.is_empty() {
        b.push_str("The conversation so far:\n\n");
        for t in turns {
            let who = if t.role == "assistant" {
                "You"
            } else {
                "The person"
            };
            b.push_str(&format!("{who}: {}\n\n", t.text.trim()));
        }
        b.push_str("Now answer the person's new message.\n\n");
    }
    b.push_str(&format!("The person: {}\n", text.trim()));
    b
}

// ---- the stream-json parser -----------------------------------------------

/// Reads `claude -p --output-format stream-json` lines into events. Pure:
/// it only remembers whether reply text came before a tool call, so the
/// text after it starts a new paragraph.
#[derive(Debug, Default)]
pub struct Parser {
    said: bool,
    broke: bool,
}

impl Parser {
    pub fn new() -> Parser {
        Parser::default()
    }

    /// The event one stdout line makes, if any.
    pub fn line(&mut self, line: &str) -> Option<AssistantEvent> {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        match v.get("type")?.as_str()? {
            "stream_event" => {
                let ev = v.get("event")?;
                if ev.get("type")?.as_str()? != "content_block_delta" {
                    return None;
                }
                let d = ev.get("delta")?;
                if d.get("type")?.as_str()? != "text_delta" {
                    return None;
                }
                let t = d.get("text")?.as_str()?;
                if t.is_empty() {
                    return None;
                }
                let mut out = String::new();
                if self.broke {
                    self.broke = false;
                    if self.said {
                        out.push_str("\n\n");
                    }
                }
                out.push_str(t);
                self.said = true;
                Some(AssistantEvent::delta(&out))
            }
            "assistant" => {
                let content = v.get("message")?.get("content")?.as_array()?;
                let tu = content
                    .iter()
                    .rev()
                    .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))?;
                self.broke = true;
                let name = tu.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let null = serde_json::Value::Null;
                Some(AssistantEvent::tool(&tool_status(
                    name,
                    tu.get("input").unwrap_or(&null),
                )))
            }
            "result" => {
                let is_error = v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false);
                let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
                let text = v.get("result").and_then(|s| s.as_str()).unwrap_or("");
                if !is_error && subtype == "success" {
                    return Some(AssistantEvent::done());
                }
                let reason = if text.trim().is_empty() {
                    subtype
                } else {
                    text
                };
                Some(AssistantEvent::error(&failure(reason)))
            }
            _ => None,
        }
    }
}

/// A short plain status for a tool call: `mcp__irori__irori_search` with
/// `{"query":"renaming"}` is “Searching for “renaming””.
pub fn tool_status(name: &str, input: &serde_json::Value) -> String {
    let tool = name.rsplit("__").next().unwrap_or(name);
    let bare = tool.strip_prefix("irori_").unwrap_or(tool);
    let arg = |k: &str| {
        input
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| short(s, 60))
            .unwrap_or_default()
    };
    let quoted = |head: &str, k: &str| {
        let a = arg(k);
        if a.is_empty() {
            head.to_string()
        } else {
            format!("{head} for \u{201c}{a}\u{201d}")
        }
    };
    match bare {
        "search" => quoted("Searching", "query"),
        "notes_search" => quoted("Searching the notes", "q"),
        "show" => {
            let r = arg("ref");
            let r = r.strip_prefix("task:").unwrap_or(&r);
            if r.is_empty() {
                "Opening a card".into()
            } else {
                format!("Opening {r}")
            }
        }
        "board" => "Reading the Board".into(),
        "overview" => "Looking at what's live now".into(),
        "agents" => "Checking who is running".into(),
        "agents_started" => "Checking the agents Irori started".into(),
        "commits" => "Looking at commits".into(),
        "prs" => "Looking at pull requests".into(),
        "timeline" => "Reading the timeline".into(),
        "why" => "Looking into why a line changed".into(),
        "leases" => "Checking what is held".into(),
        "waits" => "Checking waits".into(),
        "events" => "Reading recent activity".into(),
        _ => "Looking things up".into(),
    }
}

/// `s` on one line, at most `n` characters.
fn short(s: &str, n: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= n {
        return one;
    }
    let mut t: String = one.chars().take(n.saturating_sub(1)).collect();
    t.push('\u{2026}');
    t
}

/// Whether Claude Code's words say it is signed out.
fn signed_out(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("not logged in")
        || l.contains("/login")
        || l.contains("invalid api key")
        || l.contains("oauth token has expired")
}

/// One plain sentence for a failed run, from what Claude Code said.
pub fn failure(reason: &str) -> String {
    if signed_out(reason) {
        return SIGNED_OUT.into();
    }
    let r = short(reason, 160);
    let r = r.trim_end_matches('.');
    if r.is_empty() {
        "The assistant couldn't answer. Try again.".into()
    } else {
        format!("The assistant couldn't answer: {r}.")
    }
}

// ---- one run ----------------------------------------------------------------

/// One headless turn to run.
#[derive(Debug, Clone)]
pub struct Job {
    /// The run dir: its cwd, where its MCP config and system prompt go.
    /// Presence leaves sessions here off the roster.
    pub dir: PathBuf,
    /// The Irori data dir `irori mcp` dials.
    pub data_dir: PathBuf,
    /// The Core's assistant secret.
    pub secret: String,
    /// The `irori` exe to start as `irori mcp`.
    pub exe: PathBuf,
    /// Appended to Claude Code's system prompt.
    pub system: String,
    /// Written to stdin, then stdin is closed.
    pub prompt: String,
    pub model: String,
    /// The `CLAUDE_CONFIG_DIR` to give the child; `None` keeps this
    /// process's (the person's sign-in).
    pub claude_config_dir: Option<PathBuf>,
    pub timeout: Duration,
}

impl Job {
    /// The window assistant's job for `prompt`, against this Core.
    pub fn assistant(core: &irori_core::Core, prompt: String) -> Result<Job, String> {
        let exe = std::env::current_exe().map_err(|e| format!("no exe path: {e}"))?;
        Ok(Job {
            dir: core.assistant_dir(),
            data_dir: core.data_dir().to_path_buf(),
            secret: core.assistant_secret().to_string(),
            exe,
            system: SYSTEM.into(),
            prompt,
            model: MODEL.into(),
            claude_config_dir: dev_claude_config_dir(),
            timeout: TIMEOUT,
        })
    }
}

/// Debug builds only: `IRORI_ASSISTANT_CLAUDE_CONFIG_DIR`, so a scratch
/// build with an empty `CLAUDE_CONFIG_DIR` can still sign in. A release
/// build ignores it, like `IRORI_DEV_PORT_OFFSET`.
fn dev_claude_config_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("IRORI_ASSISTANT_CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The MCP config's file name in the run dir.
pub const MCP_FILE: &str = "mcp.json";
/// The system prompt's file name in the run dir.
pub const SYSTEM_FILE: &str = "system.md";

/// The MCP config: `irori mcp` as the person's read-only assistant.
pub fn mcp_config(exe: &Path, data_dir: &Path, secret: &str) -> String {
    serde_json::json!({
        "mcpServers": {
            SERVER: {
                "command": exe.to_string_lossy(),
                "args": ["mcp"],
                "env": {
                    "IRORI_HOME": data_dir.to_string_lossy(),
                    "IRORI_ASSISTANT_SECRET": secret,
                }
            }
        }
    })
    .to_string()
}

/// `claude`'s arguments for a run whose files are in `dir`. The prompt is
/// not among them: it goes on stdin. The system prompt is a file, so none
/// of it reaches `claude.cmd`'s `cmd.exe` parsing on Windows.
pub fn args(dir: &Path, model: &str) -> Vec<String> {
    let p = |f: &str| dir.join(f).to_string_lossy().into_owned();
    [
        "-p",
        "--model",
        model,
        "--no-session-persistence",
        "--setting-sources",
        "project",
        "--mcp-config",
        &p(MCP_FILE),
        "--strict-mcp-config",
        "--tools",
        "",
        // dontAsk refuses whatever isn't allowed: the irori server's tools
        // are (and the API lets only the read ones through).
        "--allowedTools",
        &format!("mcp__{SERVER}"),
        "--permission-mode",
        "dontAsk",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--append-system-prompt-file",
        &p(SYSTEM_FILE),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Writes `data` to `path` readable by this user only (it may hold a
/// secret), replacing it whole.
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(data.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// Makes the run dir and writes its MCP config and system prompt.
pub fn prepare(job: &Job) -> Result<(), String> {
    std::fs::create_dir_all(&job.dir).map_err(|e| format!("make {}: {e}", job.dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&job.dir, std::fs::Permissions::from_mode(0o700));
    }
    let mcp = job.dir.join(MCP_FILE);
    write_private(&mcp, &mcp_config(&job.exe, &job.data_dir, &job.secret))
        .map_err(|e| format!("write {}: {e}", mcp.display()))?;
    let sys = job.dir.join(SYSTEM_FILE);
    write_private(&sys, &job.system).map_err(|e| format!("write {}: {e}", sys.display()))?;
    Ok(())
}

/// The command for `job` with `claude` at `bin`.
pub fn command(bin: &Path, job: &Job) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(args(&job.dir, &job.model))
        .current_dir(&job.dir)
        .env_remove("CLAUDE_CODE_PLUGIN_DIRS")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(d) = &job.claude_config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", d);
    }
    hide_console(&mut cmd);
    cmd
}

// ---- the runner -------------------------------------------------------------

/// Where a run's events go; `false` means no one listens any more.
pub type Sink = Box<dyn Fn(AssistantEvent) -> bool + Send + Sync>;

/// Why a run was ended from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Running,
    Stopped,
    TimedOut,
}

struct Run {
    child: Mutex<Option<Child>>,
    ending: Mutex<Ending>,
}

impl Run {
    /// Kills the child (once), noting why.
    fn end(&self, why: Ending) {
        {
            let mut e = self.ending.lock().unwrap_or_else(|e| e.into_inner());
            if *e == Ending::Running {
                *e = why;
            }
        }
        if let Some(c) = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = c.kill();
        }
    }
    fn ending(&self) -> Ending {
        *self.ending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Runs headless turns, at most [`MAX_RUNS`] at once, each stoppable by
/// its id.
#[derive(Default)]
pub struct Runner {
    next: AtomicU32,
    runs: Arc<Mutex<HashMap<u32, Arc<Run>>>>,
    closed: Mutex<bool>,
}

/// How much of stderr is kept for an error.
const STDERR_CAP: usize = 16 << 10;

impl Runner {
    pub fn new() -> Runner {
        Runner::default()
    }

    /// Starts `job` with the `claude` found on this device. Returns its id
    /// at once; every outcome, a failure to start included, comes to
    /// `sink` as events ending with one `done` or `error`.
    pub fn start(&self, job: Result<Job, String>, sink: Sink) -> u32 {
        self.start_with(find_claude(), job, sink)
    }

    /// [`Runner::start`] with `claude` at `bin` (a fake in tests).
    pub fn start_with(
        &self,
        bin: Result<PathBuf, String>,
        job: Result<Job, String>,
        sink: Sink,
    ) -> u32 {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let fail = |s: &str| {
            sink(AssistantEvent::error(s));
            id
        };
        let Ok(bin) = bin else {
            return fail(NO_CLAUDE);
        };
        let job = match job {
            Ok(j) => j,
            Err(e) => return fail(&failure(&e)),
        };
        if *self.closed.lock().unwrap_or_else(|e| e.into_inner()) {
            return fail(STOPPED);
        }
        let run = Arc::new(Run {
            child: Mutex::new(None),
            ending: Mutex::new(Ending::Running),
        });
        {
            let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
            if runs.len() >= MAX_RUNS {
                drop(runs);
                return fail(BUSY);
            }
            runs.insert(id, run.clone());
        }
        let forget = {
            let runs = self.runs.clone();
            move || {
                runs.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            }
        };
        if let Err(e) = prepare(&job) {
            forget();
            return fail(&failure(&e));
        }
        let mut child = match command(&bin, &job).spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                forget();
                return fail(NO_CLAUDE);
            }
            Err(e) => {
                forget();
                return fail(&failure(&format!("couldn't start Claude Code: {e}")));
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *run.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);

        // stdin on its own thread: a long prompt can fill the pipe before
        // the child reads it.
        let prompt = job.prompt.clone();
        std::thread::spawn(move || {
            if let Some(mut w) = stdin {
                let _ = w.write_all(prompt.as_bytes());
            }
        });
        let errs: Arc<Mutex<Vec<u8>>> = Arc::default();
        let err_reader = stderr.map(|mut r| {
            let errs = errs.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut e = errs.lock().unwrap_or_else(|e| e.into_inner());
                    let room = STDERR_CAP.saturating_sub(e.len());
                    e.extend_from_slice(&buf[..n.min(room)]);
                }
            })
        });
        // The watchdog: ends the run at its timeout unless it ended first.
        let (fin_tx, fin_rx) = std::sync::mpsc::channel::<()>();
        {
            let run = run.clone();
            let timeout = job.timeout;
            std::thread::spawn(move || {
                if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    fin_rx.recv_timeout(timeout)
                {
                    run.end(Ending::TimedOut);
                }
            });
        }
        std::thread::spawn(move || {
            let mut p = Parser::new();
            let mut ended = false;
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    if ended || run.ending() != Ending::Running {
                        continue; // drain to EOF
                    }
                    if let Some(ev) = p.line(&line) {
                        ended = ev.ends();
                        if !sink(ev) {
                            run.end(Ending::Stopped);
                        }
                    }
                }
            }
            let status = run
                .child
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .map(|c| c.wait());
            if let Some(h) = err_reader {
                let _ = h.join();
            }
            drop(fin_tx);
            forget();
            if ended {
                return;
            }
            let last = match run.ending() {
                Ending::Stopped => STOPPED.to_string(),
                Ending::TimedOut => TOO_LONG.to_string(),
                Ending::Running => {
                    let e = errs.lock().unwrap_or_else(|e| e.into_inner());
                    let e = String::from_utf8_lossy(&e).trim().to_string();
                    let code = match status {
                        Some(Ok(s)) => s.to_string(),
                        _ => "it stopped".into(),
                    };
                    failure(if e.is_empty() { &code } else { &e })
                }
            };
            sink(AssistantEvent::error(&last));
        });
        id
    }

    /// Ends run `id`; its sink then gets error [`STOPPED`] (or nothing more
    /// when it had ended). Unknown or finished ids are fine.
    pub fn stop(&self, id: u32) {
        let run = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        if let Some(r) = run {
            r.end(Ending::Stopped);
        }
    }

    /// Ends every run and starts none after (the app is quitting).
    pub fn shutdown(&self) {
        *self.closed.lock().unwrap_or_else(|e| e.into_inner()) = true;
        let runs: Vec<Arc<Run>> = self
            .runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        for r in runs {
            r.end(Ending::Stopped);
        }
    }

    /// How many runs are going.
    pub fn running(&self) -> usize {
        self.runs.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl crate::Window {
    /// Asks the assistant `text` after `history` (card #783); events come
    /// to `sink`.
    pub fn assistant_ask(&self, history: &[AssistantTurn], text: &str, sink: Sink) -> u32 {
        let prompt = render_prompt(history, text);
        self.assistant
            .start(Job::assistant(&self.core, prompt), sink)
    }

    /// Stops assistant run `id`.
    pub fn assistant_stop(&self, id: u32) {
        self.assistant.stop(id);
    }

    /// Ends the assistant's runs as the app quits.
    pub fn assistant_shutdown(&self) {
        self.assistant.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Vec<AssistantEvent> {
        let mut p = Parser::new();
        lines.iter().filter_map(|l| p.line(l)).collect()
    }

    const DELTA_A: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me look."}}}"#;
    const DELTA_B: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Two cards are blocked: #12"}}}"#;
    const TOOL_SEARCH: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Let me look."},{"type":"tool_use","id":"t1","name":"mcp__irori__irori_search","input":{"query":"renaming  the\nboard"}}]}}"#;
    const TOOL_START: &str = r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","name":"mcp__irori__irori_search","input":{}}}}"#;
    const SUCCESS: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Two cards are blocked: #12"}"#;

    #[test]
    fn text_deltas_tool_status_and_done() {
        let evs = parse(&[
            r#"{"type":"system","subtype":"init","tools":[]}"#,
            DELTA_A,
            TOOL_START,
            TOOL_SEARCH,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"..."}]}}"#,
            DELTA_B,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Two cards are blocked: #12"}]}}"#,
            SUCCESS,
            "not json",
        ]);
        assert_eq!(
            evs,
            vec![
                AssistantEvent::delta("Let me look."),
                AssistantEvent::tool("Searching for \u{201c}renaming the board\u{201d}"),
                AssistantEvent::delta("\n\nTwo cards are blocked: #12"),
                AssistantEvent::done(),
            ]
        );
    }

    #[test]
    fn statuses_in_plain_words() {
        let j = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert_eq!(
            tool_status("mcp__irori__irori_show", &j(r#"{"ref":"task:#12"}"#)),
            "Opening #12"
        );
        assert_eq!(
            tool_status("mcp__irori__irori_board", &j("{}")),
            "Reading the Board"
        );
        assert_eq!(
            tool_status("mcp__irori__irori_search", &j("{}")),
            "Searching"
        );
        assert_eq!(
            tool_status("mcp__irori__irori_notes_search", &j(r#"{"q":"deploy"}"#)),
            "Searching the notes for \u{201c}deploy\u{201d}"
        );
        assert_eq!(
            tool_status("mcp__other__thing", &j("{}")),
            "Looking things up"
        );
        let long = "x".repeat(200);
        let s = tool_status(
            "mcp__irori__irori_search",
            &serde_json::json!({ "query": long }),
        );
        assert!(s.chars().count() < 90, "{s}");
    }

    #[test]
    fn signed_out_and_other_errors() {
        let evs = parse(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Not logged in · Please run /login"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}"#,
        ]);
        assert_eq!(evs, vec![AssistantEvent::error(SIGNED_OUT)]);
        let evs = parse(&[
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":""}"#,
        ]);
        assert_eq!(
            evs,
            vec![AssistantEvent::error(
                "The assistant couldn't answer: error_max_turns."
            )]
        );
        let evs = parse(&[
            r#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: 529 Overloaded."}"#,
        ]);
        assert_eq!(
            evs,
            vec![AssistantEvent::error(
                "The assistant couldn't answer: API Error: 529 Overloaded."
            )]
        );
    }

    #[test]
    fn prompt_keeps_the_last_turns_then_the_question() {
        let mut h = vec![];
        for i in 0..20 {
            h.push(AssistantTurn {
                role: if i % 2 == 0 { "person" } else { "assistant" }.into(),
                text: format!("turn {i}"),
            });
        }
        let p = render_prompt(&h, " what's blocked? ");
        assert!(!p.contains("turn 7\n"), "{p}");
        assert!(p.contains("The person: turn 8\n"), "{p}");
        assert!(p.contains("You: turn 19\n"), "{p}");
        assert!(p.ends_with("The person: what's blocked?\n"), "{p}");
        assert_eq!(render_prompt(&[], "hi"), "The person: hi\n");
    }

    #[test]
    fn args_keep_the_prompt_off_the_command_line() {
        let a = args(Path::new("/d/assistant"), "haiku");
        assert_eq!(a[0], "-p");
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i + 1], "");
        assert!(a.iter().any(|x| x.ends_with(SYSTEM_FILE)));
        assert!(!a.iter().any(|x| x == "--append-system-prompt"));
        // Nothing for cmd.exe to act on.
        for x in &a {
            assert!(!x.contains(['"', '%', '&', '|', '^', '<', '>', '!']), "{x}");
        }
        let c: serde_json::Value = serde_json::from_str(&mcp_config(
            Path::new("/bin/irori"),
            Path::new("/data"),
            "s3cret",
        ))
        .unwrap();
        let s = &c["mcpServers"]["irori"];
        assert_eq!(s["args"][0], "mcp");
        assert_eq!(s["env"]["IRORI_ASSISTANT_SECRET"], "s3cret");
        assert_eq!(s["env"]["IRORI_HOME"], "/data");
    }

    fn collect() -> (Sink, Arc<Mutex<Vec<AssistantEvent>>>) {
        let got: Arc<Mutex<Vec<AssistantEvent>>> = Arc::default();
        let g = got.clone();
        (
            Box::new(move |e| {
                g.lock().unwrap().push(e);
                true
            }),
            got,
        )
    }

    #[test]
    fn no_claude_is_one_plain_error() {
        let (sink, got) = collect();
        let r = Runner::new();
        r.start_with(Err("no claude".into()), Err("unused".into()), sink);
        assert_eq!(*got.lock().unwrap(), vec![AssistantEvent::error(NO_CLAUDE)]);
        assert_eq!(r.running(), 0);
    }

    /// A fake `claude` that prints fixture lines (never the real one).
    #[cfg(unix)]
    fn fake(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("claude");
        std::fs::write(&p, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    fn job(dir: &Path, timeout: Duration) -> Job {
        Job {
            dir: dir.join("assistant"),
            data_dir: dir.to_path_buf(),
            secret: "s".into(),
            exe: PathBuf::from("/bin/irori"),
            system: SYSTEM.into(),
            prompt: "The person: hi\n".into(),
            model: MODEL.into(),
            claude_config_dir: None,
            timeout,
        }
    }

    #[cfg(unix)]
    fn wait_end(got: &Arc<Mutex<Vec<AssistantEvent>>>) -> Vec<AssistantEvent> {
        for _ in 0..200 {
            if got.lock().unwrap().iter().any(|e| e.ends()) {
                std::thread::sleep(Duration::from_millis(50));
                return got.lock().unwrap().clone();
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("no end: {:?}", got.lock().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_fake_claude_to_done_and_writes_a_private_config() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let script = format!("echo '{DELTA_B}'\necho '{SUCCESS}'");
        let bin = fake(d.path(), &script);
        let (sink, got) = collect();
        let r = Runner::new();
        r.start_with(Ok(bin), Ok(job(d.path(), TIMEOUT)), sink);
        let evs = wait_end(&got);
        assert_eq!(
            evs,
            vec![
                AssistantEvent::delta("Two cards are blocked: #12"),
                AssistantEvent::done()
            ]
        );
        let m = std::fs::metadata(d.path().join("assistant").join(MCP_FILE)).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        assert_eq!(r.running(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn stop_and_timeout_end_a_run_with_one_error() {
        let d = tempfile::tempdir().unwrap();
        let bin = fake(d.path(), "exec sleep 30");
        let (sink, got) = collect();
        let r = Runner::new();
        let id = r.start_with(Ok(bin.clone()), Ok(job(d.path(), TIMEOUT)), sink);
        std::thread::sleep(Duration::from_millis(200));
        r.stop(id);
        r.stop(id);
        assert_eq!(wait_end(&got), vec![AssistantEvent::error(STOPPED)]);

        let (sink, got) = collect();
        r.start_with(
            Ok(bin.clone()),
            Ok(job(d.path(), Duration::from_millis(300))),
            sink,
        );
        assert_eq!(wait_end(&got), vec![AssistantEvent::error(TOO_LONG)]);

        // A third at once is refused.
        let (s1, _g1) = collect();
        let (s2, _g2) = collect();
        let (s3, g3) = collect();
        let a = r.start_with(Ok(bin.clone()), Ok(job(d.path(), TIMEOUT)), s1);
        let b = r.start_with(Ok(bin.clone()), Ok(job(d.path(), TIMEOUT)), s2);
        r.start_with(Ok(bin), Ok(job(d.path(), TIMEOUT)), s3);
        assert_eq!(*g3.lock().unwrap(), vec![AssistantEvent::error(BUSY)]);
        r.stop(a);
        r.stop(b);
    }

    #[cfg(unix)]
    #[test]
    fn a_silent_failure_says_what_stderr_said() {
        let d = tempfile::tempdir().unwrap();
        let bin = fake(d.path(), "echo 'Not logged in' >&2\nexit 1");
        let (sink, got) = collect();
        Runner::new().start_with(Ok(bin), Ok(job(d.path(), TIMEOUT)), sink);
        assert_eq!(wait_end(&got), vec![AssistantEvent::error(SIGNED_OUT)]);
    }
}
