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

use irori_core::thread_names::NameAsk;

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

/// What the chat's model is told (cards #783, #784, #785).
pub const SYSTEM: &str = "You are Irori's Office Assistant, the small chat in the corner of the Irori window. \
You answer the person's questions about their work in Irori: cards and the Board, threads, agents, \
commits, pull requests and notes.

Look things up with the irori tools before answering; never guess. Use search to find things by \
words, show to open one item by its ref, board for what is blocked, working or open, overview for \
what is live now, and agents for who is running.

Answer in a few short sentences or a short list (lines starting with \"- \"), in plain everyday words, \
with no jargon, ids or hashes. **Bold** is fine; no headings, tables or code blocks. Cite cards as #123 \
(tag#123 for a space's board) so the person can click them. If nothing matches, say so plainly and \
suggest what to try.

You only read: you cannot change cards, start agents or post anything yourself. Draft a thread only \
for a clear new piece of work the person asks for (to build, fix, change or look into something). Never \
draft for a question, a follow-up, or a tweak to work already in a thread they posted: when this \
conversation's posted thread already covers it, point at that thread with a go line, or say to comment \
on it instead of drafting. When you are not sure whether it is new work, ask one short question instead \
of drafting. For new work, look up any cards it touches, say in one short sentence what you drafted, \
then end your reply with a draft thread for them to post, between these two lines exactly:

<<thread>>
The thread, written as the person would post it to their agents: what they want and why, in a few \
plain sentences, naming related cards as #123.
<</thread>>

The chat shows the draft with a Post thread button; nothing is posted until the person taps it. Write \
at most one draft per reply, and never one for a plain question.

Once the person posts a draft, the conversation says which thread it became, and the chat's own lines \
(it needs them, it is ready for their check, it wants their permission, no agent has started) follow its work. Asked how that \
work is going, show that thread and the cards made from it and answer from what they say now. Asked \
what an agent is doing, use agents, then show the card it is on.

When the person asks where something is, or to show, open or take them to something, the chat can take \
them there and briefly highlight it. Say in a few words where it is, then end your reply with one go \
line in exactly one of these forms:

<<go card #123>> a card or a thread (tag#123 for a space's board)
<<go setting KEY>> a setting, by its KEY from the list below
<<go page NAME>> a page: home, talk, notes, boards, desk (merge desks), notifications, storage, \
profile or held (held messages)
<<go pr URL>> a pull request's page on GitHub, by its full https URL (it opens in the browser)

For an agent, go to the card it is working on, or to setting agents.running when it has none. Write at \
most one go line per reply, only when they asked where something is or to be taken there, never for a \
plain question. The go line is the only place for a URL or a KEY; the chat shows it as a button. \
It goes last, after any draft thread, never inside one.

Settings can't be read, only pointed at: say where a setting is, not how it is set. The places, as \
KEY: what is there:
- stop-here: Stop here, the button in the top bar (not in Settings; it used to be called Pause \
agents): this machine's agents hand off and leave their cards, and nothing starts here until it is \
started again. While it is on it reads Stopped here, and a click starts this machine again.
- stop-everywhere: Stop everywhere, the button in the top bar beside Stop here (it used to be \
called Wind down): every agent on every machine hands off, and nothing starts anywhere until it is \
started again. While it is on, Stop here is hidden.
- notifications: Settings, Notifications
- notifications.show: Show notifications
- notifications.sound: notification sound
- notifications.confetti: Confetti on good news
- notifications.volume: notification volume
- notifications.corner: the corner notifications show in
- appearance: Settings, Appearance: the theme, colours, light or dark, glass
- devices: Settings, Devices and connections: devices, pairing, the home server, relays
- people: Settings, People and invites: who is in your spaces, joining with an invite (your name, colour and picture are on page profile)
- agents: Settings, Agents
- agents.each-card: Personal's switch, in When Irori starts an agent: an agent for each card you post
- agents.spaces: each shared space's switch for your agents, for the cards you make there
- agents.given: cards someone gives or assigns you in a shared space: Irori always asks first in Needs \
you, whatever the switches say
- agents.model: what a new agent starts with: the model and effort for Light, Normal and Heavy cards \
(a card's weight picks the row)
- agents.names: agent name themes
- agents.limits: how many agents at once on each computer, whether agents may start agents
- agents.stops: when Irori stops and asks: idle limit, crashes and failed starts in a row
- agents.weighs: Weigh new cards: Irori setting each new Personal card light, normal or heavy
- agents.running: running agents, with Logs and End
- agents.permissions: what a new agent may do: permissions, allowed and denied tools, pushing to \
main
- agents.trusted: trusted repos for agents
- desks: Settings, Merge desks
- app: Settings, App
- app.login: Launch at login
- app.transcribe: Transcribe what I say in huddles
- app.names: Name new threads: Irori titling the threads you post
- app.updates: updates and Keep this device up to date";

/// A go line's start (card #784): the chat takes the person where it
/// says and highlights it (frontend/src/lib/goto.ts, splitGo).
pub const GO_OPEN: &str = "<<go ";

/// The lines a reply's drafted thread goes between (card #783); the chat
/// shows it with a Post thread button (frontend/src/lib/assistant.ts,
/// splitDraft).
pub const DRAFT_OPEN: &str = "<<thread>>";
pub const DRAFT_CLOSE: &str = "<</thread>>";

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
    /// process's (the person's sign-in), an empty path unsets it (Claude
    /// Code's default dir and sign-in).
    pub claude_config_dir: Option<PathBuf>,
    pub timeout: Duration,
    /// Irori's read tools through `irori mcp` (the chat); `false` gives
    /// the turn no tools at all (naming a thread, card #782).
    pub tools: bool,
}

impl Job {
    /// The window assistant's job for `prompt`, against this Core.
    pub fn assistant(core: &irori_core::Core, prompt: String) -> Result<Job, String> {
        let exe = std::env::current_exe().map_err(|e| format!("no exe path: {e}"))?;
        // Absolute, as the child's cwd is the run dir and its arguments
        // name files in it (a relative IRORI_HOME would not line up).
        let abs = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
        Ok(Job {
            dir: abs(core.assistant_dir()),
            data_dir: abs(core.data_dir().to_path_buf()),
            secret: core.assistant_secret().to_string(),
            exe,
            system: SYSTEM.into(),
            prompt,
            model: MODEL.into(),
            claude_config_dir: dev_claude_config_dir(),
            timeout: TIMEOUT,
            tools: true,
        })
    }

    /// The turn that names a thread (card #782): no tools, its own folder
    /// under the assistant's (so presence leaves it off the roster too,
    /// and a chat run's files are never swapped under it).
    pub fn namer(core: &irori_core::Core, ask: &NameAsk) -> Result<Job, String> {
        let abs = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
        Ok(Job {
            dir: abs(core.assistant_dir().join(NAMES_DIR)),
            data_dir: abs(core.data_dir().to_path_buf()),
            secret: String::new(),
            exe: PathBuf::new(),
            system: ask.system.clone(),
            prompt: ask.prompt.clone(),
            model: MODEL.into(),
            claude_config_dir: dev_claude_config_dir(),
            timeout: NAMES_TIMEOUT,
            tools: false,
        })
    }

    /// The turn that weighs a card (card #965): a naming turn in its own
    /// folder, so neither swaps the other's system prompt.
    pub fn weigher(core: &irori_core::Core, ask: &NameAsk) -> Result<Job, String> {
        let mut job = Job::namer(core, ask)?;
        job.dir = job.dir.with_file_name(WEIGHS_DIR);
        job.timeout = WEIGHS_TIMEOUT;
        Ok(job)
    }

    /// The turn that rewords a draft (card #981): a naming turn in its own
    /// folder, so its system prompt is never swapped under another's.
    pub fn rewriter(core: &irori_core::Core, ask: &NameAsk) -> Result<Job, String> {
        let mut job = Job::namer(core, ask)?;
        job.dir = job.dir.with_file_name(REWRITES_DIR);
        job.timeout = REWRITES_TIMEOUT;
        Ok(job)
    }
}

/// The rewriting turns' folder under the assistant's (card #981).
pub const REWRITES_DIR: &str = "rewrites";
/// How long a rewriting turn may take: the person is waiting on it.
pub const REWRITES_TIMEOUT: Duration = Duration::from_secs(45);

/// The weighing turns' folder under the assistant's (card #965).
pub const WEIGHS_DIR: &str = "weighs";
/// How long a weighing turn may take: a held card starts by then anyway.
pub const WEIGHS_TIMEOUT: Duration = Duration::from_secs(30);

/// The naming turns' folder under the assistant's (card #782).
pub const NAMES_DIR: &str = "names";
/// How long a naming turn may take.
pub const NAMES_TIMEOUT: Duration = Duration::from_secs(60);

/// Debug builds only: `IRORI_ASSISTANT_CLAUDE_CONFIG_DIR`, so a scratch
/// build with an empty `CLAUDE_CONFIG_DIR` can still sign in. `default`
/// unsets it for the child: on a Mac the sign-in's Keychain item is named
/// after the config dir, so naming `~/.claude` outright finds no sign-in
/// (measured, #783). A release build ignores it, like
/// `IRORI_DEV_PORT_OFFSET`.
fn dev_claude_config_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var_os("IRORI_ASSISTANT_CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty())?;
    Some(if v == "default" {
        PathBuf::new()
    } else {
        PathBuf::from(v)
    })
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

/// `claude`'s arguments for `job`: [`args`] for the chat; with no tools,
/// no MCP server at all and nothing allowed.
pub fn job_args(job: &Job) -> Vec<String> {
    if job.tools {
        return args(&job.dir, &job.model);
    }
    let sys = job.dir.join(SYSTEM_FILE).to_string_lossy().into_owned();
    [
        "-p",
        "--model",
        &job.model,
        "--no-session-persistence",
        "--setting-sources",
        "project",
        "--strict-mcp-config",
        "--tools",
        "",
        "--permission-mode",
        "dontAsk",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--append-system-prompt-file",
        &sys,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Writes `data` to `path` readable by this user only (it may hold a
/// secret), replacing it whole.
fn write_private(path: &Path, data: &str) -> std::io::Result<()> {
    // Unique per write: two runs may prepare the same dir at once.
    static WRITES: AtomicU32 = AtomicU32::new(0);
    let tmp = path.with_extension(format!(
        "tmp{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
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
    if job.tools {
        let mcp = job.dir.join(MCP_FILE);
        write_private(&mcp, &mcp_config(&job.exe, &job.data_dir, &job.secret))
            .map_err(|e| format!("write {}: {e}", mcp.display()))?;
    }
    let sys = job.dir.join(SYSTEM_FILE);
    write_private(&sys, &job.system).map_err(|e| format!("write {}: {e}", sys.display()))?;
    Ok(())
}

/// The command for `job` with `claude` at `bin`.
pub fn command(bin: &Path, job: &Job) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(job_args(job))
        .current_dir(&job.dir)
        .env_remove("CLAUDE_CODE_PLUGIN_DIRS")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match &job.claude_config_dir {
        Some(d) if d.as_os_str().is_empty() => {
            cmd.env_remove("CLAUDE_CONFIG_DIR");
        }
        Some(d) => {
            cmd.env("CLAUDE_CONFIG_DIR", d);
        }
        None => {}
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
            kill_tree(c);
        }
    }
    fn ending(&self) -> Ending {
        *self.ending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Kills `c` and what it started. On Windows an npm install's `claude.cmd`
/// runs as `cmd.exe`, which starts `node.exe` (and that `irori mcp`):
/// killing `cmd.exe` alone leaves them running, holding stdout open, so
/// the run would never end. `taskkill /T` ends the whole tree first.
fn kill_tree(c: &mut Child) {
    #[cfg(windows)]
    {
        let mut k = Command::new("taskkill");
        k.args(["/T", "/F", "/PID", &c.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut k);
        let _ = k.status();
    }
    let _ = c.kill();
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
        // A stop or shutdown that came while it was starting found no child
        // to kill: end it here. Under the child's lock, so one coming now
        // finds the child.
        {
            let mut slot = run.child.lock().unwrap_or_else(|e| e.into_inner());
            let closed = *self.closed.lock().unwrap_or_else(|e| e.into_inner());
            if closed || run.ending() != Ending::Running {
                kill_tree(&mut child);
                let _ = child.wait();
                drop(slot);
                forget();
                return fail(STOPPED);
            }
            *slot = Some(child);
        }

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

    /// Runs `job` with `claude` at `bin` to its end and returns the
    /// reply's text (card #782): what a background job asks.
    pub fn run_once(
        &self,
        bin: Result<PathBuf, String>,
        job: Result<Job, String>,
    ) -> Result<String, String> {
        let text: Arc<Mutex<String>> = Arc::default();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let tx = Mutex::new(tx);
        let got = text.clone();
        self.start_with(
            bin,
            job,
            Box::new(move |e: AssistantEvent| {
                match e.kind.as_str() {
                    "delta" => got
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push_str(&e.text),
                    "done" => {
                        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(Ok(()));
                    }
                    "error" => {
                        let _ = tx
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .send(Err(e.text.clone()));
                    }
                    _ => {}
                }
                true
            }),
        );
        rx.recv().map_err(|_| STOPPED.to_string())??;
        Ok(std::mem::take(
            &mut *text.lock().unwrap_or_else(|e| e.into_inner()),
        ))
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
        self.names.shutdown();
        self.weighs.shutdown();
        self.rewrites.shutdown();
    }

    /// One turn that names a thread (card #782), on its own runner so it
    /// never takes the chat's turns; the Core's namer.
    pub fn name_thread(&self, ask: &NameAsk) -> Result<String, String> {
        self.names
            .run_once(find_claude(), Job::namer(&self.core, ask))
    }

    /// One turn that weighs a card (card #965), on its own runner so it
    /// and naming never take each other's slots; the Core's weigher.
    pub fn weigh_card(&self, ask: &NameAsk) -> Result<String, String> {
        self.weighs
            .run_once(find_claude(), Job::weigher(&self.core, ask))
    }

    /// Rewords the person's draft (card #981), on its own runner, and
    /// returns the wording only once its @mentions and #refs check out.
    /// `mentions` are the names the person picked in the box.
    pub fn rewrite_draft(&self, text: &str, mentions: &[String]) -> Result<String, String> {
        let ask = crate::rewrite::ask(text, mentions)?;
        let reply = self
            .rewrites
            .run_once(find_claude(), Job::rewriter(&self.core, &ask))?;
        crate::rewrite::checked(text, mentions, &reply)
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

    // The chat finds a drafted thread by these exact lines (card #783).
    #[test]
    fn system_prompt_names_the_draft_lines() {
        assert!(SYSTEM.contains(&format!("\n{DRAFT_OPEN}\n")));
        assert!(SYSTEM.contains(&format!("\n{DRAFT_CLOSE}\n")));
        assert!(!SYSTEM.contains("ask box"));
        // Card #1000: the chat no longer follows the work line by line.
        assert!(!SYSTEM.contains("an agent took it"));
    }

    // Draft only for new work; reuse a posted thread (card #1016).
    #[test]
    fn system_prompt_drafts_only_new_work() {
        assert!(SYSTEM.contains("Draft a thread only for a clear new piece of work"));
        assert!(SYSTEM.contains("Never draft for a question, a follow-up, or a tweak"));
        assert!(SYSTEM.contains("comment on it instead of drafting"));
        assert!(SYSTEM.contains("ask one short question instead of drafting"));
    }

    // The chat finds a go line by its start (card #784), and the settings
    // list is what frontend/src/lib/goto.ts's settingSpots knows (a
    // frontend test checks the keys).
    #[test]
    fn system_prompt_names_the_go_lines() {
        for kind in ["card #123", "setting KEY", "page NAME", "pr URL"] {
            assert!(SYSTEM.contains(&format!("\n{GO_OPEN}{kind}>>")), "{kind}");
        }
        assert!(SYSTEM.contains("\n- stop-here: "));
        assert!(!SYSTEM.contains("not readable"));
    }

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
            tools: true,
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

    /// A fake `claude` on any platform that keeps its arguments in
    /// `argv.txt` and prints the fixture file `out.txt` (never the real one).
    fn fake_any(dir: &Path, out: &str, fail: bool) -> PathBuf {
        let fixture = dir.join("out.txt");
        std::fs::write(&fixture, out).unwrap();
        let log = dir.join("argv.txt");
        #[cfg(windows)]
        {
            let p = dir.join("claude.cmd");
            let tail = if fail {
                "echo Not logged in 1>&2\r\nexit /b 1\r\n".to_string()
            } else {
                format!("type \"{}\"\r\n", fixture.display())
            };
            std::fs::write(
                &p,
                format!("@echo off\r\n(echo %*) > \"{}\"\r\n{tail}", log.display()),
            )
            .unwrap();
            p
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("claude");
            let tail = if fail {
                "echo 'Not logged in' >&2\nexit 1\n".to_string()
            } else {
                format!("cat '{}'\n", fixture.display())
            };
            std::fs::write(
                &p,
                format!(
                    "#!/bin/sh\ncat >/dev/null\necho \"$@\" > '{}'\n{tail}",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    fn names_job(dir: &Path) -> Job {
        Job {
            dir: dir.join("assistant").join(NAMES_DIR),
            data_dir: dir.to_path_buf(),
            secret: String::new(),
            exe: PathBuf::new(),
            system: "Name it.".into(),
            prompt: "Name this thread.".into(),
            model: MODEL.into(),
            claude_config_dir: None,
            timeout: NAMES_TIMEOUT,
            tools: false,
        }
    }

    #[test]
    fn a_naming_turn_has_no_tools_and_returns_its_reply() {
        let d = tempfile::tempdir().unwrap();
        let bin = fake_any(d.path(), &format!("{DELTA_B}\n{SUCCESS}\n"), false);
        let r = Runner::new();
        let got = r.run_once(Ok(bin), Ok(names_job(d.path()))).unwrap();
        assert_eq!(got, "Two cards are blocked: #12");
        let argv = std::fs::read_to_string(d.path().join("argv.txt")).unwrap();
        for want in [
            "--model haiku",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--permission-mode dontAsk",
        ] {
            assert!(argv.contains(want), "{want} in {argv}");
        }
        for not in [" --mcp-config", "--allowedTools"] {
            assert!(!argv.contains(not), "no {not} in {argv}");
        }
        let run = d.path().join("assistant").join(NAMES_DIR);
        assert!(!run.join(MCP_FILE).exists(), "no MCP config, no secret");
        assert_eq!(
            std::fs::read_to_string(run.join(SYSTEM_FILE)).unwrap(),
            "Name it."
        );
        // The run is let go once the process has exited.
        for _ in 0..200 {
            if r.running() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(r.running(), 0);
    }

    #[test]
    fn a_naming_turn_that_fails_says_why() {
        let d = tempfile::tempdir().unwrap();
        let bin = fake_any(d.path(), "", true);
        let got = Runner::new().run_once(Ok(bin), Ok(names_job(d.path())));
        assert_eq!(got, Err(SIGNED_OUT.to_string()));
        let none = Runner::new().run_once(Err("no".into()), Ok(names_job(d.path())));
        assert_eq!(none, Err(NO_CLAUDE.to_string()));
    }
}
