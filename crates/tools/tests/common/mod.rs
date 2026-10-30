//! The tools_test.go harness (I:internal/tools/tools_test.go): a fake
//! Backend over a real store, inbox and coord; the clock fixed at
//! 2026-10-26T09:00Z and moved by hand; alpha, beta and gamma (Claude Code,
//! bypass mode) and grokker on dev1.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use coord::Coord;
use gocompat::time::GoTime;
use inbox::{Inbox, Sender};
use tools::{Backend, Caller, Set};

pub const KEY_A: &str = "dev1/aaaaaaaa-1111-4111-8111-111111111111";
pub const KEY_B: &str = "dev1/bbbbbbbb-2222-4222-8222-222222222222";
pub const KEY_C: &str = "dev1/cccccccc-3333-4333-8333-333333333333";
pub const KEY_G: &str = "dev1/grok-gggggggg";

#[derive(Clone)]
pub struct Clock(pub Arc<Mutex<GoTime>>);

impl Clock {
    pub fn now(&self) -> GoTime {
        *self.0.lock().unwrap()
    }
    pub fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        let n = i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos) + d.as_nanos() as i128;
        *t = GoTime::unix(
            n.div_euclid(1_000_000_000) as i64,
            n.rem_euclid(1_000_000_000) as i64,
        );
    }
}

/// The roster every service reads: presence cards by key.
#[derive(Default)]
pub struct Roster(pub Mutex<BTreeMap<String, presence::AgentCard>>);

impl Roster {
    pub fn put(&self, c: presence::AgentCard) {
        self.0.lock().unwrap().insert(c.key.clone(), c);
    }
    pub fn remove(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
    pub fn set_status(&self, key: &str, status: &str) {
        if let Some(c) = self.0.lock().unwrap().get_mut(key) {
            c.status = status.into();
        }
    }
    pub fn cards(&self) -> Vec<presence::AgentCard> {
        self.0.lock().unwrap().values().cloned().collect()
    }
}

struct InboxRoster(Arc<Roster>);
struct CoordRoster(Arc<Roster>);

impl inbox::Roster for InboxRoster {
    fn agents(&self) -> Vec<inbox::AgentCard> {
        self.0
            .cards()
            .into_iter()
            .map(|c| inbox::AgentCard {
                key: c.key,
                device_id: c.device_id,
                session_id: c.session_id,
                agent: c.agent,
                name: c.name,
                status: c.status,
                repo_key: c.repo_key,
                permission_mode: c.permission_mode,
            })
            .collect()
    }
}

impl coord::roster::Agents for CoordRoster {
    fn agents(&self) -> Vec<coord::roster::AgentCard> {
        self.0
            .cards()
            .into_iter()
            .map(|c| coord::roster::AgentCard {
                key: c.key,
                device_id: c.device_id,
                agent: c.agent,
                session_id: c.session_id,
                pid: c.pid,
                name: c.name,
                title: c.title,
                name_source: c.name_source,
                ai_title: c.ai_title,
                cwd: c.cwd,
                status: c.status,
                session_kind: c.session_kind,
                entrypoint: c.entrypoint,
                version: c.version,
                peer_features: c.peer_features,
                started_at: c.started_at,
                updated_at: c.updated_at,
                last_activity: c.last_activity,
                last_text: c.last_text,
                transcript: c.transcript,
                repo_key: c.repo_key,
                permission_mode: c.permission_mode,
                device_state: c.device_state,
            })
            .collect()
    }
}

/// The fake Backend.
pub struct TestBackend {
    pub roster: Arc<Roster>,
    pub st: Arc<store::Store>,
    pub ib: Arc<Inbox>,
    pub co: Arc<Coord>,
    pub notifier: Option<Arc<dyn tools::Notifier>>,
    /// Unset: capture is off (Go's nil `cp` until withCapture).
    pub cp: OnceLock<capture::Capture>,
    /// Unset: attachments are off (Go's attachingBackend sets it).
    pub attacher: OnceLock<Arc<dyn tools::Attacher>>,
    /// Unset: no shared boards (Go's nil router).
    pub router: OnceLock<Arc<dyn tools::TaskRouter>>,
}

impl Backend for TestBackend {
    fn agents(&self) -> Vec<presence::AgentCard> {
        self.roster.cards()
    }
    fn events(&self, f: &store::Filter) -> Result<Vec<store::Event>, String> {
        self.st.list(f).map_err(|e| e.to_string())
    }
    fn inbox(&self) -> &Inbox {
        &self.ib
    }
    fn coord(&self) -> &Arc<Coord> {
        &self.co
    }
    fn notifier(&self) -> Option<&dyn tools::Notifier> {
        self.notifier.as_deref()
    }
    fn capture(&self) -> Option<&capture::Capture> {
        self.cp.get()
    }
    fn attacher(&self) -> Option<&dyn tools::Attacher> {
        self.attacher.get().map(|a| a.as_ref())
    }
    fn router(&self) -> Option<&dyn tools::TaskRouter> {
        self.router.get().map(|r| r.as_ref())
    }
}

pub struct Harness {
    pub _dir: tempfile::TempDir,
    pub clk: Clock,
    pub b: Arc<TestBackend>,
    pub set: Set,
    pub owners: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

pub fn card(key: &str, name: &str, repo: &str, mode: &str) -> presence::AgentCard {
    let sess = key.split_once('/').map(|(_, s)| s).unwrap_or("");
    presence::AgentCard {
        key: key.into(),
        device_id: "dev1".into(),
        session_id: sess.into(),
        agent: "claude".into(),
        name: name.into(),
        status: "idle".into(),
        repo_key: repo.into(),
        permission_mode: mode.into(),
        cwd: format!("C:\\src\\{name}"),
        ..presence::AgentCard::default()
    }
}

pub fn claude(key: &str, name: &str) -> Caller {
    Caller {
        sender: Sender::new(key, name, "claude"),
        path: inbox::path::MOD.into(),
    }
}

pub fn grok_caller() -> Caller {
    Caller {
        sender: Sender::new(KEY_G, "grokker", "grok"),
        path: inbox::path::CHANNEL.into(),
    }
}

pub fn call_a() -> Caller {
    claude(KEY_A, "alpha")
}
pub fn call_b() -> Caller {
    claude(KEY_B, "beta")
}
pub fn call_c() -> Caller {
    claude(KEY_C, "gamma")
}

pub fn new_harness() -> Harness {
    new_harness_with(None)
}

pub fn new_harness_with(notifier: Option<Arc<dyn tools::Notifier>>) -> Harness {
    let clk = Clock(Arc::new(Mutex::new(
        GoTime::parse_rfc3339("2026-10-26T09:00:00Z").unwrap(),
    )));
    let dir = tempfile::tempdir().unwrap();
    let c2 = clk.clone();
    let hc = hlc::Clock::with_now(Box::new(move || c2.now().to_system()));
    let st =
        Arc::new(store::Store::open(dir.path().join("irori.db"), "dev1", Arc::new(hc)).unwrap());
    let roster = Arc::new(Roster::default());
    let owners: Arc<Mutex<HashMap<String, Vec<String>>>> = Arc::default();
    let co_cell: Arc<OnceLock<Arc<Coord>>> = Arc::default();
    let now: Arc<dyn Fn() -> GoTime + Send + Sync> = {
        let c = clk.clone();
        Arc::new(move || c.now())
    };
    let gone = {
        let cell = co_cell.clone();
        Arc::new(move |m: &inbox::Message, to: &str| {
            tools::notice::record_gone(cell.get().expect("coord"), m, to)
        })
    };
    let passed = {
        let cell = co_cell.clone();
        Arc::new(move |m: &inbox::Message, to: &str| {
            tools::notice::record_passed(cell.get().expect("coord"), m, to)
        })
    };
    let ow = owners.clone();
    let ib = Arc::new(
        Inbox::new(
            st.clone(),
            Arc::new(InboxRoster(roster.clone())),
            inbox::Options {
                device_id: "dev1".into(),
                now: Some(now.clone()),
                pr_owners: Some(Arc::new(move |k: &str| {
                    ow.lock().unwrap().get(k).cloned().unwrap_or_default()
                })),
                on_recipient_gone: Some(gone),
                on_passed: Some(passed),
                ..inbox::Options::default()
            },
        )
        .unwrap(),
    );
    let co = Arc::new(
        Coord::new(
            st.clone(),
            Some(Arc::new(CoordRoster(roster.clone()))),
            coord::Options {
                now: Some(now.clone()),
                ..coord::Options::default()
            },
        )
        .unwrap(),
    );
    let _ = co_cell.set(co.clone());
    roster.put(card(
        KEY_A,
        "alpha",
        "remote:github.com/acme/app",
        inbox::BYPASS_MODE,
    ));
    roster.put(card(
        KEY_B,
        "beta",
        "remote:github.com/acme/app",
        inbox::BYPASS_MODE,
    ));
    roster.put(card(
        KEY_C,
        "gamma",
        "remote:github.com/acme/other",
        inbox::BYPASS_MODE,
    ));
    let mut g = card(KEY_G, "grokker", "", "");
    g.agent = "grok".into();
    roster.put(g);
    let b = Arc::new(TestBackend {
        roster,
        st,
        ib,
        co,
        notifier,
        cp: OnceLock::new(),
        attacher: OnceLock::new(),
        router: OnceLock::new(),
    });
    let set = Set::new(b.clone(), Some(now));
    Harness {
        _dir: dir,
        clk,
        b,
        set,
        owners,
    }
}

impl Harness {
    /// A tool call; args are raw JSON.
    pub fn call(&self, c: &Caller, name: &str, args: &str) -> String {
        let out = self
            .set
            .call(c, name, args.as_bytes())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            out.len() <= tools::MAX_RESULT,
            "{name}: {} bytes",
            out.len()
        );
        out
    }

    pub fn ok(&self, c: &Caller, name: &str, args: &str) -> String {
        let out = self.call(c, name, args);
        assert!(!out.starts_with("Error: "), "{name}: {out}");
        out
    }

    pub fn fails(&self, c: &Caller, name: &str, args: &str, want: &[&str]) -> String {
        let out = self.call(c, name, args);
        assert!(
            out.starts_with("Error: "),
            "{name}: want an error, got {out:?}"
        );
        for w in want {
            assert!(
                out.contains(w),
                "{name}: error {out:?} should mention {w:?}"
            );
        }
        out
    }
}

pub fn has(out: &str, want: &[&str]) {
    for w in want {
        assert!(out.contains(w), "output should contain {w:?}:\n{out}");
    }
}

pub fn has_not(out: &str, bad: &[&str]) {
    for w in bad {
        assert!(!out.contains(w), "output must not contain {w:?}:\n{out}");
    }
}

/// Go's `field`: the token after `label `.
pub fn field(out: &str, label: &str) -> String {
    let pat = format!("{label} ");
    let i = out
        .find(&pat)
        .unwrap_or_else(|| panic!("no {label:?} in {out:?}"));
    let rest = &out[i + pat.len()..];
    let end = rest.find([' ', ';', ',', ')', '\n']).unwrap_or(rest.len());
    rest[..end].to_string()
}

// ---- JSON args, cards and notices (Go's map[string]any calls) ----

pub use serde_json::json;
pub type J = serde_json::Value;

impl Harness {
    pub fn callj(&self, c: &Caller, name: &str, args: J) -> String {
        self.call(c, name, &args.to_string())
    }
    pub fn okj(&self, c: &Caller, name: &str, args: J) -> String {
        self.ok(c, name, &args.to_string())
    }
    pub fn failsj(&self, c: &Caller, name: &str, args: J, want: &[&str]) -> String {
        self.fails(c, name, &args.to_string(), want)
    }

    /// Go `sendTo`: the message id and the delivery id.
    pub fn send_to(&self, c: &Caller, to: &str, text: &str) -> (String, String) {
        let out = self.okj(c, "send", json!({"to": to, "text": text}));
        (field(&out, "message"), field(&out, "(delivery"))
    }

    /// Go `newCard`: puts a card up as `c` and returns its id.
    pub fn new_card(&self, c: &Caller, args: J) -> String {
        created_id(&self.okj(c, "board_create", args))
    }

    /// Go `ref`: a card's short ref ("#1").
    pub fn short(&self, id: &str) -> String {
        let t = self
            .b
            .co
            .get_task(id)
            .unwrap_or_else(|| panic!("no task {id}"));
        assert!(!t.short.is_empty(), "no short ref for {id}");
        t.short_ref()
    }

    /// Go `notices`: the open deliveries to `key` that carry Board notices.
    pub fn notices(&self, key: &str) -> Vec<inbox::Delivery> {
        self.b
            .ib
            .for_agent(key, true, 0)
            .into_iter()
            .filter(|d| d.message.notice)
            .collect()
    }
}

/// Go `createdID`: the id in board_create's "Created #1 (task:<id>)."
pub fn created_id(out: &str) -> String {
    let rest = out
        .split_once("(task:")
        .unwrap_or_else(|| panic!("no task id in {out:?}"))
        .1;
    let id = rest.split(')').next().unwrap().to_string();
    assert!(!id.is_empty(), "no task id in {out:?}");
    id
}

/// `t` moved by `secs` seconds.
pub fn plus(t: GoTime, secs: i64) -> GoTime {
    GoTime::unix(t.secs + secs, i64::from(t.nanos))
}

// ---- capture on (Go capture_test.go withCapture) ----

/// Canonical absolute path of a dir (Go `filepath.EvalSymlinks`).
pub fn canon(p: &std::path::Path) -> String {
    let c = std::fs::canonicalize(p).unwrap();
    let s = c.to_string_lossy().into_owned();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

/// Go `gitRun`: git with a fixed identity and no GIT_* from the
/// environment.
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
        "-c",
        "core.autocrlf=false",
    ];
    full.extend_from_slice(args);
    let mut cmd = std::process::Command::new("git");
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

pub fn write_file(path: &str, content: &str) {
    std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn join(dir: &str, name: &str) -> String {
    std::path::Path::new(dir)
        .join(name)
        .to_string_lossy()
        .into_owned()
}

pub const FOO_GO: &str = "package foo\n\nfunc A() {}\n\nfunc B() {}\n";

struct CaptureRoster(Arc<Roster>);

impl capture::Roster for CaptureRoster {
    fn agents(&self) -> Vec<capture::AgentCard> {
        self.0
            .cards()
            .into_iter()
            .map(|c| capture::AgentCard {
                key: c.key,
                session_id: c.session_id,
                cwd: c.cwd,
                status: c.status,
                name: c.name,
                title: c.title,
                repo_key: c.repo_key,
            })
            .collect()
    }
}

/// A harness with capture on over a temp repo that alpha works in.
pub struct CapRig {
    pub h: Harness,
    /// The repo, alpha's cwd.
    pub dir: String,
    pub repo_key: String,
    /// The commit that added foo.go.
    pub sha: String,
    pub edit_id: String,
    _tmp: Vec<tempfile::TempDir>,
}

impl std::ops::Deref for CapRig {
    type Target = Harness;
    fn deref(&self) -> &Harness {
        &self.h
    }
}

/// Go `withCapture`: a temp repo with foo.go committed, alpha's card
/// pointed at it, a turn, its edit and the linked commit seeded (plus
/// whatever `seed` adds), then capture started over the harness's store.
pub fn with_capture(seed: impl FnOnce(&CapRig)) -> CapRig {
    let h = new_harness();
    let tmp = tempfile::tempdir().unwrap();
    let dir = canon(tmp.path());
    git(&dir, &["init", "-q"]);
    write_file(&join(&dir, "foo.go"), FOO_GO);
    git(&dir, &["add", "foo.go"]);
    git(&dir, &["commit", "-q", "-m", "add foo"]);
    // capture's git limit, not the resolver's 2 s: spawning git can take
    // that long on a loaded machine.
    let mut resolver = capture::repo::Resolver::new();
    resolver.set_timeout(capture::GIT_TIMEOUT);
    let repo_key = resolver.key(&dir);
    assert!(!repo_key.is_empty(), "no repo key for {dir}");
    let sha = git(&dir, &["rev-parse", "HEAD"]);
    let (claude_dir, data_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut r = CapRig {
        h,
        dir: dir.clone(),
        repo_key: repo_key.clone(),
        sha: sha.clone(),
        edit_id: "e-foo1".into(),
        _tmp: vec![tmp],
    };
    let mut a = card(KEY_A, "alpha", &repo_key, inbox::BYPASS_MODE);
    a.cwd = dir.clone();
    r.b.roster.put(a);

    let t0 = plus(r.clk.now(), -2 * 3600);
    r.turn(
        KEY_A,
        "p1",
        "add foo please",
        "added foo.go",
        t0,
        &["foo.go"],
    );
    r.append(
        capture::KIND_EDIT,
        &capture::EditRow {
            id: r.edit_id.clone(),
            key: KEY_A.into(),
            turn_id: "p1".into(),
            tool_use_id: "w1".into(),
            tool: "Write".into(),
            repo_key: repo_key.clone(),
            root: dir.clone(),
            path: "foo.go".into(),
            create: true,
            adds: 5,
            fidelity: capture::transcript::EXACT.into(),
            at: plus(t0, 1),
            ..Default::default()
        },
    );
    r.append(
        capture::KIND_COMMIT_SEEN,
        &capture::CommitRow {
            repo_key: repo_key.clone(),
            root: dir.clone(),
            op: "commit".into(),
            branch: "main".into(),
            commit: capture::gitlink::Commit {
                sha: sha.clone(),
                at: plus(t0, 60),
                author: "Test".into(),
                subject: "add foo".into(),
                files: Some(vec![capture::gitlink::FileChange {
                    path: "foo.go".into(),
                    ..Default::default()
                }]),
                ..Default::default()
            },
        },
    );
    r.link(std::slice::from_ref(&r.edit_id), &["p1"], plus(t0, 60));
    seed(&r);
    let mut opts = capture::Options::new(
        &canon(claude_dir.path()),
        &canon(data_dir.path()),
        "dev1",
        Arc::new(capture::NoScrub),
        Arc::new(capture::MemBlobs::default()),
    );
    let clk = r.clk.clone();
    opts.now = Arc::new(move || clk.now());
    opts.repo_key = Arc::new(move |d: &str| resolver.key(d));
    let cp = capture::Capture::new(
        r.b.st.clone(),
        Some(Arc::new(CaptureRoster(r.b.roster.clone()))),
        opts,
    )
    .unwrap();
    cp.add_source(r.b.co.clone());
    cp.add_source(r.b.ib.clone());
    assert!(r.b.cp.set(cp).is_ok());
    r._tmp.push(claude_dir);
    r._tmp.push(data_dir);
    r
}

impl CapRig {
    pub fn append<P: serde::Serialize>(&self, kind: &str, payload: &P) {
        self.b.st.append("irori", kind, payload).unwrap();
    }

    /// Go `capRig.turn`: a human turn with two Edits and a Bash.
    pub fn turn(&self, key: &str, id: &str, prompt: &str, reply: &str, at: GoTime, edits: &[&str]) {
        let sess = key.split_once('/').map(|(_, s)| s).unwrap_or("");
        let tool = |s: &str, name: &str| capture::transcript::ToolCall {
            id: format!("{id}{s}"),
            name: name.into(),
            ..Default::default()
        };
        self.append(
            capture::KIND_TURN,
            &capture::TurnRow {
                id: format!("t-{key}{id}"),
                key: key.into(),
                repo_key: self.repo_key.clone(),
                root: self.dir.clone(),
                turn_id: id.into(),
                turn: capture::transcript::Turn {
                    session_id: sess.into(),
                    origin: "human".into(),
                    prompt: prompt.into(),
                    reply: reply.into(),
                    cwd: self.dir.clone(),
                    started_at: at,
                    ended_at: plus(at, 30),
                    end_reason: "turn_duration".into(),
                    edit_paths: edits.iter().map(|s| s.to_string()).collect(),
                    tools: vec![tool("a", "Edit"), tool("b", "Edit"), tool("c", "Bash")],
                    ..Default::default()
                },
            },
        );
    }

    /// A commit.linked row for the seeded commit, committed by alpha in p1.
    pub fn link(&self, edits: &[String], turns: &[&str], at: GoTime) {
        self.append(
            capture::KIND_COMMIT_LINK,
            &capture::CommitLink {
                repo_key: self.repo_key.clone(),
                sha: self.sha.clone(),
                committer: Some(capture::Committer {
                    key: KEY_A.into(),
                    turn_id: "p1".into(),
                }),
                files: Some(vec![capture::gitlink::FileLink {
                    path: "foo.go".into(),
                    link: capture::gitlink::LINK_EXACT.into(),
                    edits: edits.to_vec(),
                }]),
                sessions: vec![KEY_A.into()],
                turns: turns.iter().map(|s| s.to_string()).collect(),
                at,
            },
        );
    }

    /// Go `grokNoCwd`: the Grok card as a real member has it.
    pub fn grok_no_cwd(&self) {
        let mut g = card(KEY_G, "grokker", "", "");
        g.agent = "grok".into();
        g.cwd = String::new();
        self.b.roster.put(g);
    }
}

/// Go `moreBefore`: the before= value of a (more: ...) hint.
pub fn more_before(out: &str) -> String {
    let rest = out
        .split_once("(more: before=")
        .unwrap_or_else(|| panic!("no more hint in {out:?}"))
        .1;
    rest.split(';').next().unwrap().to_string()
}
