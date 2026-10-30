//! `irori-dev probe` (architecture.md 5.2, 8 M8; tools/rehearse): the real
//! `irori` exe run headless on a copy of a data dir, with no network, and
//! asked from outside whether every API route answers and what `irori mcp`
//! lists.
//!
//!   irori-dev probe --irori <exe> --home <copy> --claude <empty dir>
//!                   [--go-tools <Go's fallback_tools.json>]
//!                   [--go-lockfile <a copy of the running Go Core's irori.json>]
//!
//! 1. `irori serve --headless` starts with `IRORI_HOME=<copy>`,
//!    `CLAUDE_CONFIG_DIR=<empty dir>` (capture reads no real session) and
//!    `IRORI_DEV_OFFLINE=1` (`irori_core::offline`: the copy holds this
//!    device's real key, so its Core never binds a socket). Its start line
//!    must say `offline`, or the probe stops it before asking anything.
//! 2. Every route the Go API serves (I:internal/api server.go,
//!    routes_v02.go, routes_v03.go, routes_board.go, routes_events.go,
//!    routes_pair.go) is asked once, with the token and no identity header,
//!    and with bodies that fail validation, so nothing is written. A route
//!    answers when its status is below 500 and it is not the router's own
//!    "no such route" 404. `/v1/nope` must get that 404, which proves the
//!    check can fail.
//! 3. `irori mcp` (same env) is asked `initialize` and `tools/list`; its
//!    names must be `irori_` + each `GET /v1/tools?client=grok` name, and,
//!    with `--go-tools`, `irori_` + each name in Go's fallback list.
//! 4. With `--go-lockfile`, `GET /v1/devices` of the Rust Core equals the
//!    running Go Core's, row by row, in what a copy keeps: deviceId, name,
//!    role, os, this, revoked. State, last seen, warnings and actions are
//!    live (the copy is offline) and only counted. The lockfile's port and
//!    token are read in code and never printed.
//!
//! Output is routes, statuses and tool names; never the token, the device
//! id, paths inside the copy or any row.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::http::{self, Lock};

pub struct Opts {
    irori: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    go_tools: Option<PathBuf>,
    go_lockfile: Option<PathBuf>,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let (mut irori, mut home, mut claude, mut go_tools, mut go_lockfile) =
            (None, None, None, None, None);
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let (flag, inline) = match a.split_once('=') {
                Some((f, v)) => (f, Some(v.to_string())),
                None => (a.as_str(), None),
            };
            let mut val = || {
                inline
                    .clone()
                    .or_else(|| it.next().cloned())
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag.trim_start_matches('-') {
                "irori" => irori = Some(PathBuf::from(val()?)),
                "home" => home = Some(PathBuf::from(val()?)),
                "claude" => claude = Some(PathBuf::from(val()?)),
                "go-tools" => go_tools = Some(PathBuf::from(val()?)),
                "go-lockfile" => go_lockfile = Some(PathBuf::from(val()?)),
                _ => return Err(format!("unknown flag {a}")),
            }
        }
        match (irori, home, claude) {
            (Some(irori), Some(home), Some(claude)) => Ok(Opts {
                irori,
                home,
                claude,
                go_tools,
                go_lockfile,
            }),
            _ => Err("give --irori, --home and --claude".into()),
        }
    }
}

/// How a route is asked.
struct Ask {
    method: &'static str,
    path: &'static str,
    body: Option<&'static [u8]>,
    /// A stream: read the head only.
    stream: bool,
}

const fn get(path: &'static str) -> Ask {
    Ask {
        method: "GET",
        path,
        body: None,
        stream: false,
    }
}

const fn post(path: &'static str) -> Ask {
    // Not JSON: every POST handler rejects it before it acts.
    Ask {
        method: "POST",
        path,
        body: Some(b"not json"),
        stream: false,
    }
}

const fn stream(path: &'static str) -> Ask {
    Ask {
        method: "GET",
        path,
        body: None,
        stream: true,
    }
}

/// Go's routes, one ask each (I:internal/api at be67a86).
const ROUTES: &[Ask] = &[
    // server.go
    get("/v1/health"),
    get("/v1/device"),
    get("/v1/agents"),
    get("/v1/devices"),
    // routes_events.go
    get("/v1/events?limit=5"),
    stream("/v1/events/stream"),
    // routes_v02.go
    stream("/v1/attach"),
    post("/v1/members"),
    get("/v1/policy"),
    post("/v1/policy"),
    get("/v1/inbox"),
    get("/v1/tools?client=grok"),
    post("/v1/tools/irori_inbox"),
    post("/v1/deliveries/delivered"),
    post("/v1/deliveries/engaged"),
    post("/v1/deliveries/turn-ended"),
    // routes_v03.go
    get("/v1/timeline"),
    get("/v1/edits"),
    get("/v1/commits"),
    get("/v1/commits/0000000"),
    get("/v1/blobs/0000000000000000000000000000000000000000000000000000000000000000"),
    get("/v1/decisions"),
    get("/v1/prs"),
    get("/v1/capture/status"),
    get("/v1/why"),
    // routes_board.go
    get("/v1/board"),
    get("/v1/tasks/rehearsal-none"),
    get("/v1/search?q=a"),
    get("/v1/show?ref=task:rehearsal-none"),
    get("/v1/overview"),
    // routes_pair.go
    get("/v1/pair/status"),
    post("/v1/pair/join"),
    post("/v1/pair/key"),
    post("/v1/pair/cancel"),
];

/// The router's own miss (`http::error(404, "no such route")`).
fn no_such_route(status: u16, body: &[u8]) -> bool {
    status == 404 && String::from_utf8_lossy(body).contains("no such route")
}

struct Serve {
    child: Child,
    stderr: Arc<Mutex<Vec<String>>>,
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn env(c: &mut Command, o: &Opts) {
    c.env("IRORI_HOME", &o.home)
        .env("CLAUDE_CONFIG_DIR", &o.claude)
        .env("IRORI_DEV_OFFLINE", "1")
        .env_remove("IRORI_DEV_SHELL_LOG")
        .env_remove("IRORI_DEV_RUN_KEY");
}

fn start_serve(o: &Opts) -> Result<Serve, String> {
    let mut c = Command::new(&o.irori);
    c.args(["serve", "--headless"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    env(&mut c, o);
    let mut child = c.spawn().map_err(|e| format!("start irori serve: {e}"))?;
    let lines = Arc::new(Mutex::new(Vec::new()));
    if let Some(err) = child.stderr.take() {
        let lines = lines.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                lines.lock().expect("stderr lines").push(l);
            }
        });
    }
    Ok(Serve {
        child,
        stderr: lines,
    })
}

/// Waits for the start line and the lockfile; checks the start line says
/// offline.
fn wait_ready(s: &mut Serve, home: &Path) -> Result<Lock, String> {
    let start = Instant::now();
    loop {
        if let Some(st) = s.child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "irori serve exited before it was ready ({st}); its last line: {}",
                last_line(&s.stderr)
            ));
        }
        let started = s
            .stderr
            .lock()
            .expect("stderr lines")
            .iter()
            .find(|l| l.contains("serving headless"))
            .cloned();
        if let Some(line) = started {
            // app/src/serve.rs: "... (device …, data …), offline".
            if !line.trim_end().ends_with("), offline") {
                return Err("irori serve did not start offline (an exe without IRORI_DEV_OFFLINE?); stopped it".into());
            }
            let lf = home.join("irori.json");
            if lf.exists()
                && let Ok(l) = Lock::read(&lf)
            {
                return Ok(l);
            }
        }
        if start.elapsed() > Duration::from_secs(180) {
            return Err("irori serve was not ready within 180 s".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The last stderr line, with anything after "device " cut (it names the
/// device and the copy's path).
fn last_line(lines: &Arc<Mutex<Vec<String>>>) -> String {
    let l = lines
        .lock()
        .expect("stderr lines")
        .last()
        .cloned()
        .unwrap_or_default();
    match l.find("(device ") {
        Some(i) => format!("{}(…)", &l[..i]),
        None => l,
    }
}

pub fn run(o: &Opts) -> ExitCode {
    if let Err(e) = crate::verify::refuse_live(&o.home) {
        eprintln!("irori-dev: {e}");
        return ExitCode::from(2);
    }
    match probe(o) {
        Ok(true) => {
            println!("probe: passed");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("probe: FAILED");
            ExitCode::FAILURE
        }
        Err(e) => {
            println!("FAIL {e}");
            ExitCode::FAILURE
        }
    }
}

fn probe(o: &Opts) -> Result<bool, String> {
    let mut serve = start_serve(o)?;
    let lock = wait_ready(&mut serve, &o.home)?;
    println!("serve: started offline on 127.0.0.1");
    let mut ok = true;

    // The check itself can fail: an unknown path is the router's 404.
    match http::get(&lock, "/v1/nope") {
        Ok((st, body)) if no_such_route(st, &body) => {}
        Ok((st, _)) => {
            println!("FAIL /v1/nope answered {st}, not the router's 404");
            ok = false;
        }
        Err(e) => {
            println!("FAIL /v1/nope: {e}");
            ok = false;
        }
    }
    let mut answered = 0;
    for a in ROUTES {
        let res = http::request(&lock, a.method, a.path, &[], a.body, a.stream);
        match res {
            Ok((st, body)) if st < 500 && !no_such_route(st, &body) => {
                answered += 1;
                println!("ok   {st} {} {}", a.method, a.path);
            }
            Ok((st, body)) => {
                ok = false;
                println!("FAIL {st} {} {}: {}", a.method, a.path, error_text(&body));
            }
            Err(e) => {
                ok = false;
                println!("FAIL {} {}: {e}", a.method, a.path);
            }
        }
    }
    println!("routes: {answered} of {} answered", ROUTES.len());

    // The tool list three ways: the Core's, irori mcp's, Go's.
    let core_names = core_tools(&lock)?;
    let mcp_names = match mcp_tools(o) {
        Ok(n) => n,
        Err(e) => {
            println!("FAIL irori mcp: {e}");
            return Ok(false);
        }
    };
    let want: Vec<String> = core_names.iter().map(|n| format!("irori_{n}")).collect();
    if sorted(&mcp_names) == sorted(&want) {
        println!(
            "ok   irori mcp lists {} tools, the Core's /v1/tools set",
            mcp_names.len()
        );
    } else {
        ok = false;
        println!(
            "FAIL irori mcp lists {:?}, the Core offers {:?}",
            missing(&mcp_names, &want),
            missing(&want, &mcp_names)
        );
    }
    if let Some(p) = &o.go_tools {
        let go = go_tools(p)?;
        let go: Vec<String> = go.iter().map(|n| format!("irori_{n}")).collect();
        if sorted(&mcp_names) == sorted(&go) {
            println!(
                "ok   the same {} names as Go's fallback_tools.json",
                go.len()
            );
        } else {
            ok = false;
            println!(
                "FAIL against Go's fallback list: extra {:?}, missing {:?}",
                missing(&mcp_names, &go),
                missing(&go, &mcp_names)
            );
        }
    }
    if let Some(p) = &o.go_lockfile {
        ok &= same_devices(&lock, p);
    }
    drop(serve);
    Ok(ok)
}

/// What a row of `/v1/devices` keeps on a copy, and what is live.
const DEVICE_KEPT: &[&str] = &["deviceId", "name", "role", "os", "this", "revoked"];
const DEVICE_LIVE: &[&str] = &["state", "lastSeen", "warning", "action"];

/// Step 4: the Rust Core's `/v1/devices` against the Go Core's. Prints row
/// indexes, roles and field names only.
fn same_devices(rust: &Lock, go_lockfile: &Path) -> bool {
    let rows = |lock: &Lock, who: &str| -> Result<Vec<serde_json::Value>, String> {
        let (st, body) = http::get(lock, "/v1/devices")?;
        if st != 200 {
            return Err(format!("{who} GET /v1/devices answered {st}"));
        }
        match serde_json::from_slice(&body) {
            Ok(serde_json::Value::Array(v)) => Ok(v),
            _ => Err(format!("{who} /v1/devices is not a JSON array")),
        }
    };
    let got = Lock::read(go_lockfile).and_then(|go| Ok((rows(rust, "Rust")?, rows(&go, "Go")?)));
    let (r, g) = match got {
        Ok(x) => x,
        Err(e) => {
            println!("FAIL devices: {e}");
            return false;
        }
    };
    let mut ok = r.len() == g.len();
    println!("devices: Rust {} rows, Go {} rows", r.len(), g.len());
    let field = |v: &serde_json::Value, k: &str| v.get(k).cloned().unwrap_or_default();
    for (i, (a, b)) in r.iter().zip(&g).enumerate() {
        let differ: Vec<&str> = DEVICE_KEPT
            .iter()
            .copied()
            .filter(|k| field(a, k) != field(b, k))
            .collect();
        let live: Vec<&str> = DEVICE_LIVE
            .iter()
            .copied()
            .filter(|k| field(a, k) != field(b, k))
            .collect();
        let role = field(a, "role");
        let role = role.as_str().unwrap_or("");
        if differ.is_empty() {
            println!("ok   devices[{i}] ({role}) the same; live fields that differ: {live:?}");
        } else {
            ok = false;
            println!("FAIL devices[{i}] ({role}) differs in {differ:?}");
        }
    }
    ok
}

fn error_text(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| format!("{} bytes", body.len()))
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v
}

/// What `a` has that `b` lacks.
fn missing(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|n| !b.contains(n)).cloned().collect()
}

fn core_tools(lock: &Lock) -> Result<Vec<String>, String> {
    let (st, body) = http::get(lock, "/v1/tools?client=grok")?;
    if st != 200 {
        return Err(format!("GET /v1/tools answered {st}"));
    }
    names(&body)
}

fn go_tools(p: &Path) -> Result<Vec<String>, String> {
    let raw = std::fs::read(p).map_err(|e| format!("read {}: {e}", p.display()))?;
    names(&raw)
}

fn names(json: &[u8]) -> Result<Vec<String>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("tool list: {e}"))?;
    let list = v
        .as_array()
        .or_else(|| v.get("tools").and_then(|t| t.as_array()))
        .ok_or("tool list is not a list")?;
    Ok(list
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect())
}

/// `initialize`, `notifications/initialized`, `tools/list` over stdio.
fn mcp_tools(o: &Opts) -> Result<Vec<String>, String> {
    let mut c = Command::new(&o.irori);
    c.arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    env(&mut c, o);
    let mut child = c.spawn().map_err(|e| format!("start: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for l in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let send = |stdin: &mut std::process::ChildStdin, v: serde_json::Value| {
        let mut b = serde_json::to_vec(&v).expect("json");
        b.push(b'\n');
        stdin.write_all(&b).map_err(|e| format!("write: {e}"))
    };
    let result = (|| {
        send(
            &mut stdin,
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"irori-rust-dev","version":"0"}}}),
        )?;
        wait_id(&rx, 1)?;
        send(
            &mut stdin,
            serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
        )?;
        send(
            &mut stdin,
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        )?;
        let v = wait_id(&rx, 2)?;
        let tools = v
            .get("result")
            .and_then(|r| r.get("tools"))
            .ok_or("tools/list has no result.tools")?;
        names(tools.to_string().as_bytes())
    })();
    drop(stdin);
    let start = Instant::now();
    while child.try_wait().ok().flatten().is_none() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn wait_id(rx: &std::sync::mpsc::Receiver<String>, id: i64) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx
            .recv_timeout(left)
            .map_err(|_| format!("no answer to request {id}"))?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("id").and_then(|i| i.as_i64()) == Some(id) {
            if let Some(e) = v.get("error") {
                return Err(format!("request {id}: {e}"));
            }
            return Ok(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_miss_only() {
        assert!(no_such_route(404, br#"{"error":"no such route"}"#));
        assert!(!no_such_route(404, br#"{"error":"no such task"}"#));
        assert!(!no_such_route(200, b"no such route"));
    }

    #[test]
    fn tool_names_from_either_shape() {
        assert_eq!(
            names(br#"[{"name":"a"},{"name":"b"}]"#).unwrap(),
            ["a", "b"]
        );
        assert_eq!(names(br#"{"tools":[{"name":"x"}]}"#).unwrap(), ["x"]);
        assert!(names(b"{}").is_err());
    }

    #[test]
    fn flags() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(Opts::parse(&a(&["--irori", "x", "--home", "h", "--claude", "c"])).is_ok());
        assert!(Opts::parse(&a(&["--irori", "x"])).is_err());
        assert!(Opts::parse(&a(&["--bogus", "x"])).is_err());
    }
}
