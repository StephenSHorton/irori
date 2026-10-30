//! A Rust port of I:internal/capture/harness_test.go: a capture service
//! over a temp data dir and a fake `~/.claude`, a settable roster, a
//! settable clock, real temp git repos, and a synthetic transcript writer.
//! Everything is synthetic.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use capture::{AgentCard, Capture, MemBlobs, NoScrub, Options, Roster, Scrubber};
use gocompat::time::GoTime;
use serde_json::{Value as J, json};
use store::{Filter, MAX_LIST_LIMIT, Store};
use sys::cancel::Cancel;

pub const DEVICE: &str = "dev1";
pub const SID_A: &str = "aaaaaaaa-1111-2222-3333-444444444444";
pub const SID_B: &str = "bbbbbbbb-1111-2222-3333-444444444444";

/// A settable presence roster. While held, `agents` blocks.
#[derive(Default)]
pub struct FakeRoster {
    cards: Mutex<Vec<AgentCard>>,
    held: Mutex<bool>,
    gate: std::sync::Condvar,
}

impl FakeRoster {
    pub fn set(&self, cards: Vec<AgentCard>) {
        *self.cards.lock().unwrap() = cards;
    }

    /// Makes `agents` block until the returned hold is released (or
    /// dropped).
    pub fn hold(&self) -> Hold<'_> {
        *self.held.lock().unwrap() = true;
        Hold(self)
    }
}

/// Releases a [`FakeRoster::hold`]; releasing twice is fine.
pub struct Hold<'a>(&'a FakeRoster);

impl Hold<'_> {
    pub fn release(&self) {
        *self.0.held.lock().unwrap() = false;
        self.0.gate.notify_all();
    }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

impl Roster for FakeRoster {
    fn agents(&self) -> Vec<AgentCard> {
        let mut held = self.held.lock().unwrap();
        while *held {
            held = self.gate.wait(held).unwrap();
        }
        drop(held);
        self.cards.lock().unwrap().clone()
    }
}

pub fn live_card(sid: &str, cwd: &str, status: &str) -> AgentCard {
    AgentCard {
        key: format!("{DEVICE}/{sid}"),
        session_id: sid.into(),
        cwd: cwd.into(),
        status: status.into(),
        ..AgentCard::default()
    }
}

/// A clock offset from the real one.
#[derive(Default)]
pub struct Clock {
    off_ns: Mutex<i128>,
}

impl Clock {
    pub fn now(&self) -> GoTime {
        capture::gotime::add_nanos(&capture::gotime::now(), *self.off_ns.lock().unwrap())
    }
    pub fn advance(&self, d: Duration) {
        *self.off_ns.lock().unwrap() += d.as_nanos() as i128;
    }
}

/// A scrubber replacing exact values with `[redacted:exact]` (the real
/// scrubber's exact-value rule; the pattern rules are the scrub port's).
pub struct ExactScrub(pub Vec<String>);

impl Scrubber for ExactScrub {
    fn string(&self, s: &str) -> String {
        let mut out = s.to_string();
        for v in &self.0 {
            out = out.replace(v.as_str(), "[redacted:exact]");
        }
        out
    }
    fn skip_file(&self, path: &str) -> bool {
        let base = capture::gopath::base(path).to_ascii_lowercase();
        base == ".env" || base.ends_with(".pem") || base.ends_with(".key")
    }
}

/// Canonical absolute path of a dir (Go `filepath.EvalSymlinks`).
pub fn canon(p: &Path) -> String {
    let c = std::fs::canonicalize(p).unwrap();
    let s = c.to_string_lossy().into_owned();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

pub struct Rig {
    _tmp: Vec<tempfile::TempDir>,
    pub data: String,
    pub claude: String,
    pub st: Arc<Store>,
    pub c: Capture,
    pub roster: Arc<FakeRoster>,
    pub clock: Arc<Clock>,
    pub opts: Options,
    pub blobs: Arc<MemBlobs>,
}

pub fn tempdir() -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let p = canon(d.path());
    (d, p)
}

impl Rig {
    pub fn new(modify: impl FnOnce(&mut Options)) -> Rig {
        let (d1, data) = tempdir();
        let (d2, claude) = tempdir();
        let roster = Arc::new(FakeRoster::default());
        let clock = Arc::new(Clock::default());
        let blobs = Arc::new(MemBlobs::default());
        let mut opts = Options::new(&claude, &data, DEVICE, Arc::new(NoScrub), blobs.clone());
        let ck = clock.clone();
        opts.now = Arc::new(move || ck.now());
        opts.scan_interval = Duration::from_millis(50);
        opts.repo_interval = Duration::from_millis(50);
        // Go's 1.5 s pass budget cuts a pass short when parallel tests load
        // the machine (Go's own tests flake on it); tests that are about the
        // budget set it themselves.
        opts.repo_budget = Duration::from_secs(60);
        // The repo-key resolver's 2 s git limit (Go's too) gives up on a
        // root whose git runs crawl, and the root is then not noted: its
        // edits have no repo and its commits are never read. Spawning git
        // takes seconds on a loaded machine, so tests wait as long as a
        // capture git run may (GIT_TIMEOUT).
        let resolver = Arc::new(real_resolver());
        opts.repo_key = Arc::new(move |d: &str| resolver.key(d));
        modify(&mut opts);
        let st = open_store(&data);
        let c = Capture::new(
            st.clone(),
            Some(roster.clone() as Arc<dyn Roster>),
            opts.clone(),
        )
        .unwrap();
        Rig {
            _tmp: vec![d1, d2],
            data,
            claude,
            st,
            c,
            roster,
            clock,
            opts,
            blobs,
        }
    }

    /// Opens a fresh service over the same dirs and store file.
    pub fn restart(&mut self) {
        self.st.close().unwrap();
        self.st = open_store(&self.data);
        self.c = Capture::new(
            self.st.clone(),
            Some(self.roster.clone() as Arc<dyn Roster>),
            self.opts.clone(),
        )
        .unwrap();
    }

    pub fn tail(&self) {
        self.c.tail_pass(&Cancel::new());
    }

    pub fn repos(&self) {
        self.c.repo_pass(&Cancel::new());
    }

    /// The logged payloads of one kind, as JSON.
    pub fn rows(&self, kind: &str) -> Vec<J> {
        self.st
            .list(&Filter {
                kinds: vec![kind.into()],
                limit: MAX_LIST_LIMIT,
                ..Filter::default()
            })
            .unwrap()
            .iter()
            .map(|e| serde_json::from_slice(&e.payload).unwrap())
            .collect()
    }

    pub fn count(&self, kind: &str) -> usize {
        self.rows(kind).len()
    }

    pub fn session(&self, sid: &str, cwd: &str) -> Tw {
        let slug: String = cwd
            .chars()
            .map(|c| {
                if matches!(c, '\\' | '/' | ':') {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        Tw {
            path: capture::gopath::join(&[
                &self.claude,
                "projects",
                &slug,
                &format!("{sid}.jsonl"),
            ]),
            sid: sid.into(),
            cwd: cwd.into(),
            agent_id: String::new(),
            n: Mutex::new(0),
        }
    }

    pub fn subagent(&self, sid: &str, cwd: &str, agent: &str) -> Tw {
        let mut w = self.session(sid, cwd);
        let stem = w.path.strip_suffix(".jsonl").unwrap().to_string();
        w.path = capture::gopath::join(&[&stem, "subagents", &format!("agent-{agent}.jsonl")]);
        w.agent_id = agent.into();
        w
    }
}

/// The repo-key resolver over the real git, with capture's own git limit
/// instead of the resolver's 2 s (see [`Rig::new`]).
pub fn real_resolver() -> capture::repo::Resolver {
    let mut r = capture::repo::Resolver::new();
    r.set_timeout(capture::GIT_TIMEOUT);
    r
}

/// The time now, taken early enough in its wall-clock second that a row
/// stamped up to 1.2 s later is within [`capture::COMMIT_SLACK`] of a
/// commit made after it (git commit times are whole seconds: a commit at
/// 10.97 is 10, and an edit stamped 1.1 s after 10.95 is past 10 + 2).
pub fn now_early_in_second() -> GoTime {
    loop {
        let t = capture::gotime::now();
        if t.utc().nanos < 800_000_000 {
            return t;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub fn open_store(data: &str) -> Arc<Store> {
    let p = capture::gopath::join(&[data, "irori.db"]);
    Arc::new(Store::open(p, DEVICE, Arc::new(hlc::Clock::new())).unwrap())
}

// --- git ---

pub fn git(dir: &str, args: &[&str]) -> String {
    let mut full = vec![
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "init.defaultBranch=main",
    ];
    full.extend_from_slice(args);
    let mut cmd = Command::new("git");
    cmd.args(&full).current_dir(dir);
    for (k, _) in std::env::vars() {
        if k.to_ascii_uppercase().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo with one commit and core.autocrlf set as given.
pub fn new_repo(autocrlf: &str) -> (tempfile::TempDir, String) {
    let (d, dir) = tempdir();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "core.autocrlf", autocrlf]);
    write_file(&capture::gopath::join(&[&dir, "README.md"]), "hello\n");
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    (d, dir)
}

pub fn write_file(path: &str, content: &str) {
    std::fs::create_dir_all(Path::new(path).parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn head(dir: &str) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

// --- transcripts ---

pub fn ts(at: &GoTime) -> String {
    let u = at.utc();
    let (y, mo, d, h, mi, s) = u.civil();
    format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:03}Z",
        u.nanos / 1_000_000
    )
}

/// One synthetic transcript file.
pub struct Tw {
    pub path: String,
    pub sid: String,
    pub cwd: String,
    pub agent_id: String,
    n: Mutex<usize>,
}

impl Tw {
    fn uuid(&self) -> String {
        let mut n = self.n.lock().unwrap();
        *n += 1;
        format!(
            "{}-u000{}{}",
            &self.sid[..8],
            char::from(b'a' + (*n % 26) as u8),
            *n
        )
    }

    pub fn common(&self, typ: &str, at: &GoTime, extra: J) -> J {
        let mut m = json!({
            "parentUuid": null, "isSidechain": !self.agent_id.is_empty(), "type": typ,
            "uuid": self.uuid(), "timestamp": ts(at), "userType": "external", "entrypoint": "cli",
            "cwd": self.cwd, "sessionId": self.sid, "version": "2.1.300", "gitBranch": "main",
        });
        if !self.agent_id.is_empty() {
            m["agentId"] = J::String(self.agent_id.clone());
        }
        if let J::Object(e) = extra {
            for (k, v) in e {
                m[k] = v;
            }
        }
        m
    }

    pub fn prompt(&self, prompt_id: &str, text: &str, at: &GoTime) -> J {
        self.common(
            "user",
            at,
            json!({"promptId": prompt_id, "message": {"role": "user", "content": text}}),
        )
    }

    pub fn tool_use(&self, id: &str, name: &str, input: J, at: &GoTime) -> J {
        self.common(
            "assistant",
            at,
            json!({"message": {"id": format!("msg_{id}"), "role": "assistant", "type": "message",
                "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}}),
        )
    }

    pub fn reply(&self, text: &str, at: &GoTime) -> J {
        self.common(
            "assistant",
            at,
            json!({"message": {"id": "msg_r", "role": "assistant", "type": "message",
                "content": [{"type": "text", "text": text}]}}),
        )
    }

    pub fn result(&self, tool_id: &str, tur: J, at: &GoTime) -> J {
        self.common(
            "user",
            at,
            json!({"message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": tool_id, "is_error": false, "content": "ok"}]},
                "toolUseResult": tur}),
        )
    }

    pub fn done(&self, at: &GoTime) -> J {
        self.common(
            "system",
            at,
            json!({"subtype": "turn_duration", "durationMs": 1000}),
        )
    }

    /// A Write call and its result; also writes the file to disk.
    pub fn write_tool(
        &self,
        id: &str,
        path: &str,
        content: &str,
        original: Option<&str>,
        at: &GoTime,
    ) -> Vec<J> {
        let mut tur = json!({"type": "create", "filePath": path, "content": content,
            "structuredPatch": [], "originalFile": null});
        if let Some(o) = original {
            tur["type"] = json!("update");
            tur["originalFile"] = json!(o);
            tur["structuredPatch"] = json!([{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1,
                "lines": [format!("-{}", o.lines().next().unwrap_or("")),
                          format!("+{}", content.lines().next().unwrap_or(""))]}]);
        }
        write_file(path, content);
        vec![
            self.tool_use(
                id,
                "Write",
                json!({"file_path": path, "content": content}),
                at,
            ),
            self.result(
                id,
                tur,
                &capture::gotime::add(at, Duration::from_millis(100)),
            ),
        ]
    }

    /// A Bash or PowerShell call and its result.
    pub fn shell(
        &self,
        id: &str,
        tool: &str,
        command: &str,
        stdout: &str,
        git_op: Option<J>,
        at: &GoTime,
    ) -> Vec<J> {
        let mut tur =
            json!({"stdout": stdout, "stderr": "", "interrupted": false, "isImage": false});
        if let Some(g) = git_op {
            tur["gitOperation"] = g;
        }
        vec![
            self.tool_use(
                id,
                tool,
                json!({"command": command, "description": "run"}),
                at,
            ),
            self.result(id, tur, at),
        ]
    }

    pub fn append_bytes(&self, b: &[u8]) {
        use std::io::Write;
        std::fs::create_dir_all(Path::new(&self.path).parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)
            .unwrap();
        f.write_all(b).unwrap();
    }

    pub fn write(&self, rows: &[J]) {
        let mut buf = Vec::new();
        for r in rows {
            buf.extend(serde_json::to_vec(r).unwrap());
            buf.push(b'\n');
        }
        self.append_bytes(&buf);
    }
}

pub fn flat(groups: Vec<Vec<J>>) -> Vec<J> {
    groups.into_iter().flatten().collect()
}

pub fn at_secs(base: &GoTime, secs: i64) -> GoTime {
    capture::gotime::add_nanos(base, i128::from(secs) * 1_000_000_000)
}
