//! `irori-dev agent-check`: what a real Claude agent does with Irori's
//! tools (card #92). The goldens pin what the tools return; this checks
//! what an agent does with them, the gap #68, #69, #71, #74 and #82 fell
//! through. It spends API tokens, so it is run by hand before a release
//! (`sh scripts/agent-check.sh`, docs/releasing.md), never by the full check.
//!
//! A real Core runs in-process on a scratch --home, with its loopback API
//! and lockfile as `irori serve` has them, offline (no mesh, no home
//! server) and with the file keystore (no Keychain item). The person acts
//! through the window's own backend (`irori_window::Window`), so an ask or
//! a comment goes out exactly as the app sends it. Each scenario starts a
//! fresh `claude -p` session in stream-json mode with the mod (--mod, a
//! copy of `mod/`) pointed at --irori, so the session attaches to this
//! Core and is woken through the mod as a real session is. A turn ends
//! when claude prints its `result` line; the check reads the board after
//! each turn, never the agent's text. The --turn-wait is only a ceiling
//! for a turn that never ends.
//!
//! Scenarios:
//!   1 ask: the person asks the room; the agent takes the ask and closes it
//!     with an outcome.
//!   2 needs-person: the person asks for something with a choice only they
//!     can make; the agent raises a blocker instead of asking in its reply.
//!   3 purpose: the person's comment turns the agent's card into other
//!     work; the agent renames the card and redoes its steps.
//!   4 answered: the person answers the agent's blocker; the agent
//!     resolves it in the turn it acts on the answer (card #204), so the
//!     card stops reading as waiting on the person.
//!   5 stop (card #207): the person stops an agent in the middle of a long
//!     task; its turn is cut, it answers the person first and leaves the
//!     task unfinished.
//!   6 wind down (card #220): the person presses Wind down all while an
//!     agent, holding a card and a lease, is busy on a long task; it is not
//!     cut, and at its safe point it posts a hand-off and leaves the card,
//!     releases the lease and answers "wound down".
//!   7 not done (card #476): the agent's done check waits on the person,
//!     who says Not done with a reason; the agent answers them on that
//!     check's thread, and any new check it raises carries that round (as
//!     the window shows it) with proof that changed.
//!
//! The Core's claude dir is --claude, a scratch dir holding only the
//! sessions this run starts, so it never sees the real sessions; claude
//! itself keeps the real config dir (its sign-in). It runs `--restricted`
//! with `--strict-mcp-config` and no claude.ai connectors: no settings
//! files, no other plugin or MCP server, not the installed mod
//! (CLAUDE_CODE_PLUGIN_DIRS unset), and only the file tools, confined to
//! its scratch cwd under --work, plus Irori's.
//!
//! Prints `PASS <scenario>` or `FAIL <scenario>: why`; the exit code is the
//! number of failures (100 on a setup error).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use irori_core::{Core, Options};
use irori_window::{Config, Window};
use tokio_util::sync::CancellationToken;

/// The most turns one scenario may take before it is judged.
const MAX_TURNS: usize = 4;

const ASK_TEXT: &str =
    "What's the difference between a mutex and a semaphore? A couple of sentences is plenty.";

const LICENSE_ASK: &str = "Add a license file to this project.";

const S3_TITLE: &str = "Add a dark mode toggle to the settings page";
const S3_OBJECTIVE: &str =
    "Settings gets a toggle that switches the app between light and dark themes.";
const S3_STEPS: [&str; 3] = [
    "Add the toggle to the settings page",
    "Write the dark palette",
    "Remember the choice between runs",
];
const S4_TITLE: &str = "Add a license file to this folder";
const S4_QUESTION: &str = "Which license should the LICENSE file use?";
const S4_OPTIONS: [&str; 2] = ["MIT", "Apache-2.0"];
/// 5 (card #207): a long task, then the person's stop with a question.
const S5_FILES: usize = 40;
const S5_TASK: &str = "Read every file in this folder (note-01.txt to note-40.txt) one at a time with the Read tool, in order, one call each. When all 40 are read, write SUMMARY.md listing each file's first line. Don't stop early.";
const S6_TITLE: &str = "Summarize the notes folder";
const S6_LEASE: &str = "notes/summary";
const S6_FILES: usize = 12;
const S6_TASK: &str = "Read every file in this folder (note-01.txt to note-12.txt) one at a time with the Read tool, in order, one call each, then write SUMMARY.md listing each file's first line. This is the work on your card; don't stop early.";
const S5_STOP: &str = "Which note file were you reading when I stopped you? Answer me with the irori send tool (to: \"user\"), then wait: don't go back to the notes.";
const S7_TITLE: &str = "Write NOTES.md for the release";
const S7_PROOF: &str =
    "Wrote NOTES.md with one line: 'Release notes: the boat no longer clips the dock.'";
const S7_REASON: &str =
    "Not done: NOTES.md must also say who the release is for (the beta testers), on a second line.";
const S3_COMMENT: &str = "Change of plan: forget dark mode. What we actually need here is a high-contrast mode for people with low vision, toggled from the same settings page. Dark mode can wait.";

pub struct Opts {
    home: PathBuf,
    claude: PathBuf,
    work: PathBuf,
    irori: PathBuf,
    module: PathBuf,
    claude_bin: String,
    model: String,
    turn_wait: Duration,
    only: Vec<u32>,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let mut o = Opts {
            home: PathBuf::new(),
            claude: PathBuf::new(),
            work: PathBuf::new(),
            irori: PathBuf::new(),
            module: PathBuf::new(),
            claude_bin: "claude".into(),
            model: String::new(),
            turn_wait: Duration::from_secs(600),
            only: Vec::new(),
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
            match a.as_str() {
                "--home" => o.home = val()?.into(),
                "--claude" => o.claude = val()?.into(),
                "--work" => o.work = val()?.into(),
                "--irori" => o.irori = val()?.into(),
                "--mod" => o.module = val()?.into(),
                "--claude-bin" => o.claude_bin = val()?,
                "--model" => o.model = val()?,
                "--turn-wait" => {
                    let v = val()?;
                    let n: u64 = v.parse().map_err(|_| format!("--turn-wait {v}: seconds"))?;
                    o.turn_wait = Duration::from_secs(n);
                }
                "--only" => {
                    for p in val()?.split(',').filter(|p| !p.is_empty()) {
                        let n: u32 = p.parse().map_err(|_| format!("--only {p}: 1 to 7"))?;
                        if !(1..=7).contains(&n) {
                            return Err(format!("--only {p}: 1 to 7"));
                        }
                        o.only.push(n);
                    }
                }
                _ => return Err(format!("unknown flag {a}")),
            }
        }
        for (flag, p) in [
            ("--home", &o.home),
            ("--claude", &o.claude),
            ("--work", &o.work),
            ("--irori", &o.irori),
            ("--mod", &o.module),
        ] {
            if p.as_os_str().is_empty() {
                return Err(format!("{flag} is required"));
            }
        }
        Ok(o)
    }

    fn runs(&self, n: u32) -> bool {
        self.only.is_empty() || self.only.contains(&n)
    }
}

fn empty_dir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut rd = std::fs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if rd.next().is_some() {
        return Err(format!(
            "{} is not empty; give a fresh scratch dir",
            p.display()
        ));
    }
    Ok(())
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("agent-check: no randomness");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// The Core and its API, hosted as `irori serve --headless` hosts them.
struct Host {
    core: Arc<Core>,
    window: Window,
    stop: CancellationToken,
    rt: tokio::runtime::Runtime,
}

fn host(o: &Opts) -> Result<Host, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let core = Arc::new(
        Core::start(Options {
            data_dir: o.home.clone(),
            claude_dir: o.claude.clone(),
            version: "agent-check".into(),
            headless: true,
            runtime: Some(rt.handle().clone()),
            keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
            pair_net: Some(irori_core::offline::offline_net()),
            no_home: true,
            ..Options::default()
        })
        .map_err(|e| format!("start: {e}"))?,
    );
    let stop = CancellationToken::new();
    rt.block_on(async {
        let server = irori_api::Server::new(core.clone(), core.token())
            .with_tools(Arc::new(irori_api::CoreTools(core.clone())));
        let ln = irori_api::listen().await.map_err(|e| e.to_string())?;
        let port = ln.local_addr().map_err(|e| e.to_string())?.port();
        let s = stop.clone();
        tokio::spawn(async move { server.serve(ln, s).await });
        core.write_lockfile(port).map_err(|e| e.to_string())
    })?;
    let claude_dir = o.claude.clone();
    let downloads = o.work.join("downloads");
    let window = Window::new(
        core.clone(),
        Window::default_seams(&core),
        Config {
            claude_dir,
            downloads: Box::new(move || Ok(downloads.clone())),
        },
    );
    Ok(Host {
        core,
        window,
        stop,
        rt,
    })
}

/// One `claude -p` session attached to the Core through the mod.
struct Agent {
    key: String,
    child: Child,
    _stdin: ChildStdin,
    lines: Receiver<String>,
    log: PathBuf,
    turns: usize,
}

impl Agent {
    fn start(o: &Opts, h: &Host, name: &str) -> Result<Agent, String> {
        let session = uuid_v4();
        let cwd = o.work.join(name);
        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
        let settings = serde_json::json!({
            "pluginConfigs": {"irori@inline": {"options": {"iroriPath": o.irori.to_string_lossy()}}}
        });
        let mut cmd = Command::new(&o.claude_bin);
        cmd.args([
            "-p",
            "--verbose",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--session-id",
            &session,
            "--restricted",
            "--strict-mcp-config",
            "--tools",
            "Read,Glob,Grep,Edit,Write,ToolSearch",
            "--permission-mode",
            "acceptEdits",
            "--allowedTools",
            "mcp__irori",
        ]);
        cmd.arg("--plugin-dir").arg(&o.module);
        cmd.arg("--settings").arg(settings.to_string());
        if !o.model.is_empty() {
            cmd.args(["--model", &o.model]);
        }
        cmd.current_dir(&cwd)
            .env("IRORI_HOME", &o.home)
            .env_remove("CLAUDE_CODE_PLUGIN_DIRS")
            .env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_ENTRYPOINT")
            .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("can't run {}: {e}", o.claude_bin))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let log = o.work.join(format!("{name}.log"));
        let (tx, lines) = channel();
        {
            let out = child.stdout.take().ok_or("no stdout")?;
            let mut f = std::fs::File::create(&log).map_err(|e| e.to_string())?;
            std::thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = writeln!(f, "{l}");
                    let _ = tx.send(l);
                }
            });
        }
        {
            let err = child.stderr.take().ok_or("no stderr")?;
            let mut f = std::fs::File::create(o.work.join(format!("{name}.stderr")))
                .map_err(|e| e.to_string())?;
            std::thread::spawn(move || {
                for l in BufReader::new(err).lines().map_while(Result::ok) {
                    let _ = writeln!(f, "{l}");
                }
            });
        }
        // Presence lists the session from a file in the Core's claude dir,
        // as Claude Code writes one in ~/.claude/sessions; `irori attach`
        // is refused until it does.
        let pid = child.id();
        let sessions = o.claude.join("sessions");
        std::fs::create_dir_all(&sessions).map_err(|e| e.to_string())?;
        let m = serde_json::json!({
            "pid": pid, "sessionId": session, "cwd": cwd.to_string_lossy(),
            "name": name, "status": "idle", "kind": "interactive", "entrypoint": "cli",
            "version": "2.1.311", "startedAt": unix_ms(), "updatedAt": unix_ms(),
        });
        std::fs::write(sessions.join(format!("{pid}.json")), m.to_string())
            .map_err(|e| e.to_string())?;
        let key = format!("{}/{session}", h.core.device_id());
        let a = Agent {
            key,
            child,
            _stdin: stdin,
            lines,
            log,
            turns: 0,
        };
        // Attached: the mod's stream is open, so a wake reaches it.
        let deadline = Instant::now() + Duration::from_secs(120);
        while !h.core.inbox().reachable(&a.key).0 {
            if Instant::now() > deadline {
                return Err(format!(
                    "the session never attached through the mod (see {} and its .stderr)",
                    a.log.display()
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        // A terminal session is handed asks and unheld cards only once the
        // person turns on "Offer it cards" for it (card #453): this one
        // stands in for an agent they did, as scenarios 1 and 2 need.
        h.core.set_agent_offer(&a.key, true)?;
        Ok(a)
    }

    /// Waits until the running turn has made `n` tool calls (card #207).
    fn wait_tool_uses(&mut self, n: usize, wait: Duration) -> Result<(), String> {
        let deadline = Instant::now() + wait;
        let mut seen = 0;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    let v: serde_json::Value = serde_json::from_str(&l).unwrap_or_default();
                    if v["type"] == "result" {
                        return Err("the turn ended first".into());
                    }
                    if v["type"] == "assistant"
                        && let Some(c) = v["message"]["content"].as_array()
                    {
                        seen += c.iter().filter(|x| x["type"] == "tool_use").count();
                        if seen >= n {
                            return Ok(());
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{seen} tool call(s) within {} s", wait.as_secs()));
                }
                Err(RecvTimeoutError::Disconnected) => return Err("claude exited".into()),
            }
        }
    }

    /// Waits for the end of the agent's next turn: claude's `result` line.
    fn turn(&mut self, wait: Duration) -> Result<(), String> {
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    let v: serde_json::Value = serde_json::from_str(&l).unwrap_or_default();
                    if v["type"] == "result" {
                        self.turns += 1;
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("no turn ended within {} s", wait.as_secs()));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("claude exited".into());
                }
            }
        }
    }

    /// Open deliveries to this agent (a wake still to come).
    fn unread(&self, h: &Host) -> usize {
        h.core.inbox().for_agent(&self.key, false, 100).len()
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs turns until `judge` passes, the agent has nothing more coming, or
/// MAX_TURNS. `judge` returns Ok(detail) on a pass, Err(why) otherwise.
fn play(
    o: &Opts,
    h: &Host,
    a: &mut Agent,
    judge: impl Fn(&Host, &Agent) -> Result<String, String>,
) -> Result<String, String> {
    let mut why = String::from("no turn ran");
    while a.turns < MAX_TURNS {
        if let Err(e) = a.turn(o.turn_wait) {
            return Err(format!("{e}; {why}"));
        }
        match judge(h, a) {
            Ok(d) => return Ok(d),
            Err(e) => why = e,
        }
        if a.unread(h) == 0 {
            break;
        }
    }
    Err(format!("{why} (after {} turn(s))", a.turns))
}

fn tasks(h: &Host) -> Vec<coord::Task> {
    h.core.coord().list_tasks(&coord::TaskFilter::default())
}

struct Tally {
    failed: u32,
}

impl Tally {
    fn report(&mut self, name: &str, r: Result<String, String>, log: Option<&Path>) {
        match r {
            Ok(d) if d.is_empty() => println!("PASS {name}"),
            Ok(d) => println!("PASS {name}: {d}"),
            Err(e) => {
                self.failed += 1;
                match log {
                    Some(l) => println!("FAIL {name}: {e} (transcript {})", l.display()),
                    None => println!("FAIL {name}: {e}"),
                }
            }
        }
    }
}

/// 1: the person asks the room; the agent takes the ask and closes it.
fn ask(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s1-ask") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    let id = match h.window.ask(ASK_TEXT, "", true, &[], "", &[]) {
        Ok(r) => r.task.id,
        Err(e) => return (Err(format!("the person's ask: {e}")), Some(a.log.clone())),
    };
    let r = play(o, h, &mut a, |h, a| {
        let t = h
            .core
            .coord()
            .get_task(&id)
            .ok_or("the ask's card is gone")?;
        let ask = t.ask.as_ref().ok_or("the card is no longer an ask")?;
        if ask.taker != a.key {
            return Err("the agent never took the ask".into());
        }
        match &ask.outcome {
            Some(out) if !out.kind.is_empty() => Ok(format!("closed as {}", out.kind)),
            _ => Err("the agent took the ask but left it open".into()),
        }
    });
    let log = a.log.clone();
    a.stop();
    (r, Some(log))
}

/// 2: a choice only the person can make; the agent raises a blocker.
fn needs_person(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s2-needs-person") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    if let Err(e) = h.window.ask(LICENSE_ASK, "", true, &[], "", &[]) {
        return (Err(format!("the person's ask: {e}")), Some(a.log.clone()));
    }
    let r = play(o, h, &mut a, |h, a| {
        for t in tasks(h) {
            if let Some(b) = t
                .blockers
                .iter()
                .find(|b| b.raised_by == a.key && b.resolved.is_none())
            {
                return Ok(format!("blocker {:?} on #{}", b.question, t.short));
            }
        }
        Err("the agent raised no blocker for the person's choice".into())
    });
    let log = a.log.clone();
    a.stop();
    (r, Some(log))
}

/// 3: the person's comment changes what the agent's card is for.
fn purpose(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s3-purpose") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    let log = Some(a.log.clone());
    // The agent's card, made as the agent: what it was doing before.
    let caller = tools::Caller {
        sender: inbox::Sender::new(&a.key, "s3-purpose", "claude"),
        path: String::new(),
    };
    let steps: Vec<serde_json::Value> = S3_STEPS
        .iter()
        .map(|s| serde_json::json!({"text": s}))
        .collect();
    let args = serde_json::json!({"title": S3_TITLE, "objective": S3_OBJECTIVE, "steps": steps});
    let raw = serde_json::to_vec(&args).unwrap_or_default();
    if let Err(e) = h.core.tools().call(&caller, "board_create", &raw) {
        return (Err(format!("the agent's card: {e}")), log);
    }
    let Some(t) = tasks(h).into_iter().find(|t| t.title == S3_TITLE) else {
        return (Err("the agent's card was not made".into()), log);
    };
    if !t.holders.contains(&a.key) {
        // Take it as the agent if creating it did not.
        let raw = serde_json::to_vec(&serde_json::json!({"task_id": format!("task:{}", t.id)}))
            .unwrap_or_default();
        if let Err(e) = h.core.tools().call(&caller, "board_join", &raw) {
            return (Err(format!("the agent's card: {e}")), log);
        }
    }
    let before: Vec<String> = S3_STEPS.iter().map(|s| s.to_string()).collect();
    if let Err(e) = h.window.comment(&t.id, S3_COMMENT, "wake", &[], &[]) {
        return (Err(format!("the person's comment: {e}")), log);
    }
    let id = t.id.clone();
    let r = play(o, h, &mut a, |h, _| {
        let t = h.core.coord().get_task(&id).ok_or("the card is gone")?;
        let steps: Vec<String> = t.steps.iter().map(|s| s.text.clone()).collect();
        let renamed = t.title != S3_TITLE;
        let restepped = steps != before;
        match (renamed, restepped) {
            (true, true) => Ok(format!("now {:?}", t.title)),
            (false, true) => Err("the steps changed but the card kept its old title".into()),
            (true, false) => Err("renamed, but the old steps were left".into()),
            (false, false) => Err("the card kept its old title and steps".into()),
        }
    });
    a.stop();
    (r, log)
}

/// 4: the person answers the agent's blocker; the agent resolves it in
/// the turn it acts on the answer (card #204).
fn answered(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s4-answered") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    let log = Some(a.log.clone());
    let caller = tools::Caller {
        sender: inbox::Sender::new(&a.key, "s4-answered", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let raw = serde_json::to_vec(&args).unwrap_or_default();
        h.core
            .tools()
            .call(&caller, tool, &raw)
            .map_err(|e| format!("the agent's {tool}: {e}"))
    };
    // The agent's card and its question, made as the agent.
    let args = serde_json::json!({
        "title": S4_TITLE,
        "objective": "Write a LICENSE file in the working folder once the person picks the license.",
        "steps": [{"text": "Ask which license"}, {"text": "Write LICENSE"}],
    });
    if let Err(e) = call("board_create", args) {
        return (Err(e), log);
    }
    let Some(t) = tasks(h).into_iter().find(|t| t.title == S4_TITLE) else {
        return (Err("the agent's card was not made".into()), log);
    };
    let task = format!("task:{}", t.id);
    if !t.holders.contains(&a.key)
        && let Err(e) = call("board_join", serde_json::json!({ "task_id": task }))
    {
        return (Err(e), log);
    }
    let options: Vec<serde_json::Value> = S4_OPTIONS
        .iter()
        .map(|o| serde_json::json!({ "answer": o }))
        .collect();
    let args = serde_json::json!({"task_id": task, "question": S4_QUESTION, "options": options});
    if let Err(e) = call("board_block", args) {
        return (Err(e), log);
    }
    let Some(bid) = h.core.coord().get_task(&t.id).and_then(|t| {
        t.blockers
            .iter()
            .find(|b| b.resolved.is_none())
            .map(|b| b.id.clone())
    }) else {
        return (Err("the agent's blocker was not raised".into()), log);
    };
    // The person picks MIT in the window.
    if let Err(e) = h.window.answer(&t.id, &bid, 0, "", true, &[]) {
        return (Err(format!("the person's answer: {e}")), log);
    }
    let id = t.id.clone();
    let r = play(o, h, &mut a, |h, _| {
        let t = h.core.coord().get_task(&id).ok_or("the card is gone")?;
        let b = t
            .blockers
            .iter()
            .find(|b| b.id == bid)
            .ok_or("the blocker is gone")?;
        match &b.resolved {
            Some(r) => Ok(format!("resolved: {:?}", r.note)),
            None => Err(format!(
                "the agent left the answered blocker open (it reads {}; card {})",
                b.status, t.state
            )),
        }
    });
    a.stop();
    (r, log)
}

/// 5 (card #207): the person stops an agent in the middle of a long task;
/// it answers the person first and doesn't finish the task.
fn stop(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s5-stop") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    let log = Some(a.log.clone());
    let cwd = o.work.join("s5-stop");
    for i in 1..=S5_FILES {
        let body = format!("note {i:02}: {}\n", "lorem ipsum ".repeat(40));
        if let Err(e) = std::fs::write(cwd.join(format!("note-{i:02}.txt")), body) {
            return (Err(format!("the notes: {e}")), log);
        }
    }
    let person = h.core.user_sender();
    let sent = h.core.inbox().send_with(
        &person,
        &inbox::Address::new(inbox::addr::AGENT, &a.key),
        S5_TASK,
        inbox::SendOptions {
            mode: inbox::mode::WAKE.into(),
            ..inbox::SendOptions::default()
        },
    );
    if let Err(e) = sent {
        return (Err(format!("the person's task: {e}")), log);
    }
    // Stop once the long turn is under way: its third tool call.
    if let Err(e) = a.wait_tool_uses(3, o.turn_wait) {
        return (Err(format!("the long turn never got going: {e}")), log);
    }
    let since = h.core.inbox().for_agent(&person.key, true, 500).len();
    if let Err(e) = h.window.agent_stop(&a.key, S5_STOP) {
        return (Err(format!("the person's stop: {e}")), log);
    }
    let summary = cwd.join("SUMMARY.md");
    let r = play(o, h, &mut a, |h, a| {
        let to_me = h
            .core
            .inbox()
            .for_agent(&h.core.user_sender().key, true, 500);
        let answered = to_me.len() > since
            && to_me
                .iter()
                .any(|d| d.message.from.key == a.key && d.message.text.contains("note"));
        match (answered, summary.exists()) {
            (true, false) => Ok("answered the person, the task stayed cut".into()),
            (true, true) => Err("answered, but went back and finished the task".into()),
            (false, true) => Err("finished the task instead of answering the stop".into()),
            (false, false) => Err("no answer to the person yet".into()),
        }
    });
    a.stop();
    (r, log)
}

/// 6 (card #220): Wind down all while an agent holding a card and a lease
/// is busy; at its safe point it hands the card back with a hand-off,
/// releases the lease and answers.
fn wind_down(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s6-wind-down") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    let log = Some(a.log.clone());
    let cwd = o.work.join("s6-wind-down");
    for i in 1..=S6_FILES {
        let body = format!("note {i:02}: {}\n", "lorem ipsum ".repeat(40));
        if let Err(e) = std::fs::write(cwd.join(format!("note-{i:02}.txt")), body) {
            return (Err(format!("the notes: {e}")), log);
        }
    }
    let caller = tools::Caller {
        sender: inbox::Sender::new(&a.key, "s6-wind-down", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let raw = serde_json::to_vec(&args).unwrap_or_default();
        h.core
            .tools()
            .call(&caller, tool, &raw)
            .map_err(|e| format!("the agent's {tool}: {e}"))
    };
    // The agent's card and lease, made as the agent.
    let args = serde_json::json!({
        "title": S6_TITLE,
        "objective": "Read the notes in the working folder and write SUMMARY.md.",
        "steps": [{"text": "Read the notes", "doing": true}, {"text": "Write SUMMARY.md"}],
    });
    if let Err(e) = call("board_create", args) {
        return (Err(e), log);
    }
    let Some(t) = tasks(h).into_iter().find(|t| t.title == S6_TITLE) else {
        return (Err("the agent's card was not made".into()), log);
    };
    let task = format!("task:{}", t.id);
    if !t.holders.contains(&a.key)
        && let Err(e) = call("board_join", serde_json::json!({ "task_id": task }))
    {
        return (Err(e), log);
    }
    if let Err(e) = call("lease", serde_json::json!({ "resource": S6_LEASE })) {
        return (Err(e), log);
    }
    let person = h.core.user_sender();
    let sent = h.core.inbox().send_with(
        &person,
        &inbox::Address::new(inbox::addr::AGENT, &a.key),
        S6_TASK,
        inbox::SendOptions {
            mode: inbox::mode::WAKE.into(),
            ref_: task.clone(),
            ..inbox::SendOptions::default()
        },
    );
    if let Err(e) = sent {
        return (Err(format!("the person's task: {e}")), log);
    }
    // Wind down once the long turn is under way: its third tool call.
    if let Err(e) = a.wait_tool_uses(3, o.turn_wait) {
        return (Err(format!("the long turn never got going: {e}")), log);
    }
    let run = match h.window.wind_down_all(None) {
        Ok(r) => r,
        Err(e) => return (Err(format!("the person's wind down: {e}")), log),
    };
    if !run.rows.iter().any(|r| r.key == a.key) {
        return (
            Err(format!("the run missed the agent: {:?}", run.rows)),
            log,
        );
    }
    let id = t.id.clone();
    let r = play(o, h, &mut a, |h, a| {
        let t = h.core.coord().get_task(&id).ok_or("the card is gone")?;
        let handoff = h
            .core
            .coord()
            .comments(&id)
            .unwrap_or_default()
            .iter()
            .any(|c| c.by == a.key);
        let leased = h
            .core
            .coord()
            .leases()
            .iter()
            .any(|l| l.resource == S6_LEASE);
        let st = h.window.wind_down_status();
        let row = st
            .rows
            .iter()
            .find(|r| r.key == a.key)
            .map(|r| r.state.clone())
            .unwrap_or_default();
        let mut missing = Vec::new();
        if t.holders.contains(&a.key) {
            missing.push("still holds the card");
        }
        if !handoff {
            missing.push("no hand-off comment");
        }
        if leased {
            missing.push("the lease is still held");
        }
        if row != irori_window::wind_down::row::DONE {
            missing.push("no answer to the wind-down");
        }
        if missing.is_empty() {
            Ok(format!(
                "card unheld with a hand-off, lease released, answered; left_off {:?}; Wind down {}",
                t.left_off
                    .as_ref()
                    .map(|l| l.text.clone())
                    .unwrap_or_default(),
                if st.on { "on" } else { "off" }
            ))
        } else {
            Err(missing.join(", "))
        }
    });
    a.stop();
    (r, log)
}

/// 7 (card #476): the person sends the agent's done check back with a
/// reason; the agent answers them on that check, and a new check it raises
/// shows that round with changed proof.
fn not_done(o: &Opts, h: &Host) -> (Result<String, String>, Option<PathBuf>) {
    let mut a = match Agent::start(o, h, "s7-not-done") {
        Ok(a) => a,
        Err(e) => return (Err(e), None),
    };
    let log = Some(a.log.clone());
    let caller = tools::Caller {
        sender: inbox::Sender::new(&a.key, "s7-not-done", "claude"),
        path: String::new(),
    };
    let call = |tool: &str, args: serde_json::Value| {
        let raw = serde_json::to_vec(&args).unwrap_or_default();
        h.core
            .tools()
            .call(&caller, tool, &raw)
            .map_err(|e| format!("the agent's {tool}: {e}"))
    };
    // The agent's card, its work and its done check, made as the agent.
    let notes = o.work.join("s7-not-done").join("NOTES.md");
    if let Err(e) = std::fs::write(
        &notes,
        "Release notes: the boat no longer clips the dock.\n",
    ) {
        return (Err(format!("NOTES.md: {e}")), log);
    }
    let args = serde_json::json!({
        "title": S7_TITLE,
        "objective": "NOTES.md in the working folder says what the release changes.",
        "steps": [{"text": "Write NOTES.md", "done": true}],
    });
    if let Err(e) = call("board_create", args) {
        return (Err(e), log);
    }
    let Some(t) = tasks(h).into_iter().find(|t| t.title == S7_TITLE) else {
        return (Err("the agent's card was not made".into()), log);
    };
    let task = format!("task:{}", t.id);
    if !t.holders.contains(&a.key)
        && let Err(e) = call("board_join", serde_json::json!({ "task_id": task }))
    {
        return (Err(e), log);
    }
    if let Err(e) = call(
        "board_status",
        serde_json::json!({"task_id": task, "state": "completed", "proof": S7_PROOF}),
    ) {
        return (Err(e), log);
    }
    let Some(bid) = h.core.coord().get_task(&t.id).and_then(|t| {
        t.blockers
            .iter()
            .find(|b| b.done_check && b.open())
            .map(|b| b.id.clone())
    }) else {
        return (Err("the done check was not raised".into()), log);
    };
    // The person sends it back with a reason, in the window, woken.
    if let Err(e) = h.window.answer(&t.id, &bid, -1, S7_REASON, true, &[]) {
        return (Err(format!("the person's Not done: {e}")), log);
    }
    let id = t.id.clone();
    let r = play(o, h, &mut a, |h, a| {
        // What the window shows: the check's thread, or the next check's
        // earlier round.
        let d = h.window.card(&id).map_err(|e| format!("the card: {e}"))?;
        let reply_on = |items: &[irori_window::cards::CardThreadItem]| {
            items
                .iter()
                .filter(|it| it.kind == "comment" && it.who.key == a.key)
                .map(|it| it.text.clone())
                .next_back()
        };
        let old = d
            .blockers
            .iter()
            .find(|b| b.id == bid)
            .ok_or("the sent-back check is gone")?;
        let next = d
            .blockers
            .iter()
            .find(|b| b.done_check && b.id != bid && b.resolved.is_none());
        let Some(said) = reply_on(&old.thread) else {
            return Err(format!(
                "no answer from the agent on {bid}'s thread (card {}, {} check(s) open)",
                d.summary.state,
                d.blockers
                    .iter()
                    .filter(|b| b.done_check && b.resolved.is_none())
                    .count()
            ));
        };
        let Some(n) = next else {
            return Ok(format!("answered on {bid}: {said:?}"));
        };
        let p = n
            .previous
            .as_ref()
            .ok_or(format!("the new check {} doesn't show the round", n.id))?;
        if p.id != bid || reply_on(&p.thread).is_none() {
            return Err(format!("the new check's round lacks the answer: {p:?}"));
        }
        if p.same_proof || n.context == S7_PROOF {
            return Err("the new check has the same proof".into());
        }
        Ok(format!(
            "answered on {bid}: {said:?}; new check {} shows round {} with new proof {:?}",
            n.id, p.round, n.context
        ))
    });
    a.stop();
    (r, log)
}

pub fn run(o: &Opts) -> ExitCode {
    for d in [&o.home, &o.claude, &o.work] {
        if let Err(e) = empty_dir(d) {
            println!("FAIL setup: {e}");
            return ExitCode::from(100);
        }
    }
    let h = match host(o) {
        Ok(h) => h,
        Err(e) => {
            println!("FAIL setup: {e}");
            return ExitCode::from(100);
        }
    };
    let mut tally = Tally { failed: 0 };
    type Scenario = fn(&Opts, &Host) -> (Result<String, String>, Option<PathBuf>);
    let all: [(u32, &str, Scenario); 7] = [
        (1, "1 ask: taken and closed with an outcome", ask),
        (
            2,
            "2 needs the person: a blocker, not a question in the reply",
            needs_person,
        ),
        (
            3,
            "3 purpose changed: card renamed and steps redone",
            purpose,
        ),
        (
            4,
            "4 answered: the agent resolves the blocker as it acts on the answer",
            answered,
        ),
        (
            5,
            "5 stop: a busy agent's turn is cut and it answers the person first",
            stop,
        ),
        (
            6,
            "6 wind down: a busy agent hands its card back, releases its lease and answers",
            wind_down,
        ),
        (
            7,
            "7 not done: the agent answers on the check and the next one shows what changed",
            not_done,
        ),
    ];
    for (n, name, f) in all {
        if !o.runs(n) {
            continue;
        }
        println!("agent-check: running {name}");
        // Every scenario shares one Core: start each with Pause all and
        // Wind down off, since 6's wind down holds agent starts until it is
        // turned off (cards #669, #787). Reset any other shared state a new
        // scenario changes too, and run a full pass, not just --only N, since
        // a scenario run alone never meets what the others leave behind.
        let off = h
            .window
            .inbox_set_paused(false)
            .and_then(|()| h.window.wind_down_off().map(|_| ()));
        let (r, log) = match off {
            Ok(()) => f(o, &h),
            Err(e) => (Err(format!("Pause all and Wind down off: {e}")), None),
        };
        tally.report(name, r, log.as_deref());
    }
    h.stop.cancel();
    let Host {
        core, window, rt, ..
    } = h;
    drop(window);
    drop(rt);
    let _ = core.close();
    let n = tally.failed;
    if n == 0 {
        println!("agent-check: every scenario passed");
    } else {
        println!("agent-check: {n} scenario(s) failed");
    }
    ExitCode::from(n.min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags() {
        let o = Opts::parse(&args(&[
            "--home",
            "h",
            "--claude",
            "c",
            "--work",
            "w",
            "--irori",
            "i",
            "--mod",
            "m",
            "--only",
            "1,3",
            "--turn-wait",
            "30",
        ]))
        .unwrap();
        assert!(o.runs(1) && !o.runs(2) && o.runs(3));
        assert_eq!(o.turn_wait, Duration::from_secs(30));
        assert!(Opts::parse(&args(&["--home", "h"])).is_err());
        assert!(
            Opts::parse(&args(&[
                "--home", "h", "--claude", "c", "--work", "w", "--irori", "i", "--mod", "m",
                "--only", "8"
            ]))
            .is_err()
        );
    }

    #[test]
    fn uuid_shape() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
    }
}
