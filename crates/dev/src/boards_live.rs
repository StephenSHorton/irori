//! `irori-dev boards-live`: one side of the live two-person run (card #48
//! s3, v0.5-shared-boards §11 "Live"). Each side starts a real Core
//! in-process on a fresh scratch data dir, as a made-up person with one
//! made-up Claude session, and steps through the run as that person:
//!
//!   host (person A, the PC):  home server on, create a board, invite, ask
//!       for the five words the guest's screen shows, then cards both
//!       ways, a comment to B's agent, share then withdraw, a board ask, a
//!       review request to B's agent, and B's leave.
//!   guest (person B, the Mac): join from the link and show the words, then
//!       the same steps from B's side, ending with Leave.
//!
//! The two sides keep in step only by what each sees on the board, with a
//! timeout per step. Every step prints `PASS <step>` or `FAIL <step>: why`,
//! and the exit code is the number of failures (capped at 100).
//!
//! It writes nothing outside --home and --claude, both scratch dirs the
//! scripts make and delete. The keys are sealed with the file keystore,
//! so no Keychain item or DPAPI blob is made. It refuses a --home that
//! already holds anything.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_core::{Core, Options};
use store::kinds::PERSON_ACTOR_PREFIX;

const SESSION: &str = "11111111-2222-4333-8444-555555555555";
const TAG: &str = "live";
const PR: &str = "https://github.com/example-org/example-app/pull/1";
const CARD_A: &str = "live run: from A";
const CARD_B: &str = "live run: from B";
const CARD_SHARED: &str = "live run: A's private card, shared";
const HELLO_AGENT: &str = "live run: hello, B's agent";
const SEEN_SHARE: &str = "live run: B sees the shared card";
const ASK_TEXT: &str = "live run: a question for the room on the shared board";

pub struct Opts {
    host: bool,
    home: PathBuf,
    claude: PathBuf,
    link: String,
    link_out: Option<PathBuf>,
    step_wait: Duration,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let host = match args.first().map(String::as_str) {
            Some("host") => true,
            Some("guest") => false,
            _ => return Err("say host or guest".into()),
        };
        let mut o = Opts {
            host,
            home: PathBuf::new(),
            claude: PathBuf::new(),
            link: String::new(),
            link_out: None,
            step_wait: Duration::from_secs(300),
        };
        let mut it = args[1..].iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
            match a.as_str() {
                "--home" => o.home = val()?.into(),
                "--claude" => o.claude = val()?.into(),
                "--link" => o.link = val()?,
                "--link-out" => o.link_out = Some(val()?.into()),
                "--step-wait" => {
                    let s: u64 = val()?.parse().map_err(|_| "--step-wait is seconds")?;
                    o.step_wait = Duration::from_secs(s);
                }
                other => return Err(format!("unknown flag {other}")),
            }
        }
        if o.home.as_os_str().is_empty() || o.claude.as_os_str().is_empty() {
            return Err("--home and --claude are required".into());
        }
        if !o.host && o.link.is_empty() {
            return Err("the guest needs --link".into());
        }
        Ok(o)
    }
}

fn empty_dir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut it = std::fs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if it.next().is_some() {
        return Err(format!(
            "{} is not empty: boards-live only runs on a fresh scratch dir",
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

/// A live Claude session file for this process, so presence lists one
/// agent the run can act as.
fn write_session(claude: &Path, name: &str) -> Result<(), String> {
    let sessions = claude.join("sessions");
    let cwd = claude.join("work");
    std::fs::create_dir_all(&sessions).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
    let mut m = serde_json::json!({
        "pid": std::process::id(), "sessionId": SESSION, "cwd": cwd.to_string_lossy(),
        "name": name, "status": "idle", "kind": "bg", "entrypoint": "cli",
        "version": "2.1.311", "startedAt": unix_ms(), "updatedAt": unix_ms(),
    });
    if cfg!(windows)
        && let Some(st) = sys::proc::start_time(i64::from(std::process::id()))
    {
        m["procStart"] = sys::proc::filetime_ticks(st).to_string().into();
    }
    std::fs::write(
        sessions.join("900001.json"),
        serde_json::to_vec(&m).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

struct Run {
    core: Core,
    agent: String,
    wait: Duration,
    failed: u32,
    board: String,
}

impl Run {
    fn pass(&self, step: &str, detail: &str) {
        if detail.is_empty() {
            println!("PASS {step}");
        } else {
            println!("PASS {step}: {detail}");
        }
    }

    fn fail(&mut self, step: &str, why: &str) {
        self.failed += 1;
        println!("FAIL {step}: {why}");
    }

    /// Waits for `ok`, printing the step's result. True when it held.
    fn wait(&mut self, step: &str, mut ok: impl FnMut(&Run) -> bool) -> bool {
        let deadline = Instant::now() + self.wait;
        while Instant::now() < deadline {
            if ok(self) {
                self.pass(step, "");
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
            if !self.board.is_empty() {
                self.core.kick_board(&self.board);
            }
        }
        let secs = self.wait.as_secs();
        self.fail(step, &format!("not seen within {secs} s"));
        false
    }

    fn person_actor(&self) -> String {
        format!("{PERSON_ACTOR_PREFIX}{}", self.core.person().id)
    }

    fn tool(&self, name: &str, args: serde_json::Value) -> String {
        let caller = tools::Caller {
            sender: inbox::Sender::new(&self.agent, "agent", "claude"),
            path: String::new(),
        };
        let raw = serde_json::to_vec(&args).unwrap_or_default();
        match self.core.tools().call(&caller, name, &raw) {
            Ok(s) => s,
            Err(e) => format!("Error: {e}"),
        }
    }

    fn coord(&self) -> Option<Arc<coord::Coord>> {
        self.core.board_coord(&self.board).ok()
    }

    fn card(&self, title: &str) -> Option<coord::Task> {
        self.coord()?
            .list_tasks(&coord::TaskFilter::default())
            .into_iter()
            .find(|t| t.title == title)
    }

    /// The deliveries to this run's agent from another person on a board,
    /// held ones too.
    fn bridged(&self) -> Vec<inbox::Delivery> {
        self.core
            .inbox()
            .list(0)
            .into_iter()
            .filter(|d| d.to == self.agent && d.message.from.key.starts_with("board:"))
            .collect()
    }

    fn done(self) -> ExitCode {
        let n = self.failed;
        let _ = self.core.close();
        if n == 0 {
            println!("boards-live: every step passed");
        } else {
            println!("boards-live: {n} step(s) failed");
        }
        ExitCode::from(n.min(100) as u8)
    }
}

fn start(o: &Opts, name: &str) -> Result<Run, String> {
    empty_dir(&o.home)?;
    empty_dir(&o.claude)?;
    write_session(&o.claude, name)?;
    open(o, name)
}

/// Starts the Core on --home (fresh or from an earlier open) and waits for
/// the made-up agent.
fn open(o: &Opts, name: &str) -> Result<Run, String> {
    let core = Core::start(Options {
        data_dir: o.home.clone(),
        claude_dir: o.claude.clone(),
        version: "boards-live".into(),
        headless: true,
        keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
        ..Options::default()
    })
    .map_err(|e| format!("start: {e}"))?;
    let agent = format!("{}/{SESSION}", core.device_id());
    let mut run = Run {
        core,
        agent,
        wait: o.step_wait,
        failed: 0,
        board: String::new(),
    };
    let agent = run.agent.clone();
    if !run.wait("the made-up agent is live", |r| {
        r.core.local_agents().iter().any(|a| a.key == agent)
    }) {
        return Err("presence never listed the made-up session".into());
    }
    println!(
        "boards-live: {} is person {} on device {}",
        name,
        run.core.person().id,
        run.core.device_id()
    );
    Ok(run)
}

pub fn run(o: &Opts) -> ExitCode {
    let r = if o.host { host(o) } else { guest(o) };
    match r {
        Ok(code) => code,
        Err(e) => {
            println!("FAIL setup: {e}");
            ExitCode::from(100)
        }
    }
}

fn host(o: &Opts) -> Result<ExitCode, String> {
    let mut r = start(o, "alice-live")?;
    match r.core.set_home_server(true, false, "") {
        Ok(()) => r.pass("home server on", ""),
        Err(e) => r.fail("home server on", &e),
    }
    // As on the PC, the server is on when Irori starts: restart on the same
    // home, and the endpoint homes on its own server's relay (#63). The
    // link names the relay the endpoint is on when it is made.
    let failed = r.failed;
    let _ = r.core.close();
    let mut r = open(o, "alice-live")?;
    r.failed += failed;
    r.wait("A's endpoint is on its own home relay", |r| {
        r.core.net().home_relays().iter().any(|(_, up)| *up)
    });
    let rt = r.core.pairing_runtime().clone();
    r.board = r
        .core
        .create_board("Live run", TAG)
        .map_err(|e| format!("create board: {e}"))?;
    r.pass("create board", &format!("{} ({TAG})", r.board));
    let st = rt
        .block_on(r.core.invite(&r.board))
        .map_err(|e| format!("invite: {e}"))?;
    r.pass("invite", "");
    // B reaches A through A's home relay when the link names it (the PC and
    // the Mac are on different networks at home).
    match irori_invite::Ticket::parse(&st.link) {
        Ok(t) if !t.relay.is_empty() => r.pass(
            "the link names A's home relay",
            &format!("{} (direct: {})", t.relay, t.addrs.join(", ")),
        ),
        Ok(t) => r.fail(
            "the link names A's home relay",
            &format!("no relay; direct only: {}", t.addrs.join(", ")),
        ),
        Err(e) => r.fail("the link names A's home relay", &e.to_string()),
    }
    println!("\nINVITE LINK (send it to B):\n{}\n", st.link);
    if let Some(p) = &o.link_out {
        std::fs::write(p, format!("{}\n", st.link)).map_err(|e| e.to_string())?;
        println!("boards-live: the link is also in {}", p.display());
    }
    if !r.wait("B joins and both screens show the words", |r| {
        r.core.invite_status().status.state == "words"
    }) {
        return Ok(r.done());
    }
    let stdin = std::io::stdin();
    let mut tries = 0;
    loop {
        print!("Type the five words B reads out: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            r.fail("words checked", "no words typed");
            return Ok(r.done());
        }
        match r.core.confirm_invite_words(line.trim()) {
            Ok(_) => break,
            Err(e) => {
                tries += 1;
                println!("those words don't match: {e}");
                if tries >= 2 {
                    r.fail("words checked", "wrong twice; the invite is over");
                    return Ok(r.done());
                }
            }
        }
    }
    if !r.wait("words checked, invite done", |r| {
        r.core.invite_status().status.state == "done"
    }) {
        return Ok(r.done());
    }

    // Cards both ways. A's card is shared (several holders), so both
    // people's agents can work it.
    let co = r.coord().ok_or("the board has no coord")?;
    let card_a = coord::NewTask {
        shared: true,
        ..task(CARD_A)
    };
    match co.create_task(&r.person_actor(), card_a) {
        Ok(_) => r.pass("A writes a card", ""),
        Err(e) => r.fail("A writes a card", &e.to_string()),
    }
    r.core.kick_board(&r.board);
    r.wait("B's card reaches A", |r| r.card(CARD_B).is_some());

    // B's agent joins A's card; A's comment goes to it through the bridge.
    r.wait("B's agent joins A's card", |r| {
        r.card(CARD_A)
            .is_some_and(|t| t.holders.iter().any(|h| h.starts_with("agent:")))
    });
    if let Some(t) = r.card(CARD_A) {
        let c = co.comment_on(
            &r.person_actor(),
            &t.id,
            coord::CommentInput {
                text: HELLO_AGENT.into(),
                ..Default::default()
            },
        );
        match c {
            Ok(_) => r.pass("A comments to B's agent", ""),
            Err(e) => r.fail("A comments to B's agent", &e.to_string()),
        }
        r.core.kick_board(&r.board);
    }

    // Share then withdraw, once B has seen the shared card.
    let p = r
        .core
        .coord()
        .create_task(&r.core.user_sender().key, task(CARD_SHARED));
    let shared = match p {
        Ok(p) => match r
            .core
            .preview_share(&p.id, &r.board)
            .and_then(|pv| r.core.share_to_board(&p.id, &r.board, &pv.hash))
        {
            Ok(t) => {
                r.pass("A shares a private card", &t.short_ref());
                Some(t)
            }
            Err(e) => {
                r.fail("A shares a private card", &e.to_string());
                None
            }
        },
        Err(e) => {
            r.fail("A shares a private card", &e.to_string());
            None
        }
    };
    r.core.kick_board(&r.board);
    if let Some(t) = shared {
        let id = t.id.clone();
        r.wait("B says it sees the shared card", |r| {
            r.coord()
                .and_then(|c| c.comments(&id))
                .unwrap_or_default()
                .iter()
                .any(|c| c.text == SEEN_SHARE)
        });
        match r.core.withdraw_card(&id) {
            Ok(_) => r.pass("A withdraws it", ""),
            Err(e) => r.fail("A withdraws it", &e.to_string()),
        }
        r.core.kick_board(&r.board);
    }

    // A board ask: A asks the room on the shared board.
    let agents: Vec<coord::roster::AgentCard> = Vec::new();
    match co.post_ask(
        &r.person_actor(),
        coord::NewAsk {
            text: ASK_TEXT.into(),
            ..Default::default()
        },
        &agents,
    ) {
        Ok(_) => r.pass("A posts a board ask", ""),
        Err(e) => r.fail("A posts a board ask", &e.to_string()),
    }
    r.core.kick_board(&r.board);

    // A review request from A's agent to B's agent on A's card.
    if let Some(t) = r.card(CARD_A) {
        let out = r.tool(
            "board_join",
            serde_json::json!({"task_id": format!("task:{}", t.id)}),
        );
        let joined = !out.starts_with("Error");
        let linked = co.add_link(&r.person_actor(), &t.id, PR).is_ok();
        let mine = r.core.board_key(&r.board, &r.agent).unwrap_or_default();
        let theirs = r
            .card(CARD_A)
            .and_then(|t| {
                t.holders
                    .into_iter()
                    .find(|h| h.starts_with("agent:") && *h != mine)
            })
            .unwrap_or_default();
        if !joined || !linked || theirs.is_empty() {
            r.fail(
                "A's agent asks B's agent for a review",
                &format!("joined {joined}, linked {linked}, B's agent card {theirs:?}: {out}"),
            );
        } else {
            match co.request_review(&mine, &t.id, PR, &theirs) {
                Ok(_) => r.pass("A's agent asks B's agent for a review", ""),
                Err(e) => r.fail("A's agent asks B's agent for a review", &e.to_string()),
            }
        }
        r.core.kick_board(&r.board);
    }

    // B leaves last.
    let theirs = |r: &Run| {
        r.core.board_verifier(&r.board).is_some_and(|v| {
            let m = v.membership();
            let me = r.core.person().id;
            !m.members
                .iter()
                .any(|p| p.person_id != me && m.active(&p.person_id))
        })
    };
    r.wait("B has left the board", theirs);
    Ok(r.done())
}

fn guest(o: &Opts) -> Result<ExitCode, String> {
    let mut r = start(o, "bob-live")?;
    let rt = r.core.pairing_runtime().clone();
    rt.block_on(r.core.join_invite(o.link.trim()))
        .map_err(|e| format!("join: {e}"))?;
    r.pass("join from the link", "");
    if !r.wait("the words are on screen", |r| {
        r.core.invite_status().status.state == "words"
    }) {
        return Ok(r.done());
    }
    let st = r.core.invite_status().status;
    println!(
        "\nREAD THESE FIVE WORDS TO A:\n    {}\n(A's fingerprint: {}; yours: {})\n",
        st.words.join(" "),
        st.peer_fingerprint,
        st.my_fingerprint
    );
    if !r.wait("A typed the words, invite done", |r| {
        r.core.invite_status().status.state == "done"
    }) {
        return Ok(r.done());
    }
    r.board = st.board_id.clone();
    if r.board.is_empty() {
        r.board = r.core.boards().into_iter().next().unwrap_or_default();
    }

    // Cards both ways.
    r.wait("A's card reaches B", |r| r.card(CARD_A).is_some());
    if let Some(co) = r.coord() {
        match co.create_task(&r.person_actor(), task(CARD_B)) {
            Ok(_) => r.pass("B writes a card", ""),
            Err(e) => r.fail("B writes a card", &e.to_string()),
        }
        r.core.kick_board(&r.board);
    }

    // B's agent joins A's card and hears A's comment through the bridge.
    if let Some(t) = r.card(CARD_A) {
        let out = r.tool(
            "board_join",
            serde_json::json!({"task_id": format!("task:{}", t.id)}),
        );
        if out.starts_with("Error") {
            r.fail("B's agent joins A's card", &out);
        } else {
            r.pass("B's agent joins A's card", "");
        }
        r.core.kick_board(&r.board);
    }
    r.wait(
        "A's comment reaches B's agent, from a board: sender, untrusted",
        |r| {
            r.bridged().iter().any(|d| {
                d.message.text.contains(HELLO_AGENT) && d.message.trust == inbox::trust::UNTRUSTED
            })
        },
    );

    // Share then withdraw.
    if r.wait("the shared card reaches B", |r| {
        r.card(CARD_SHARED).is_some()
    }) {
        let co = r.coord();
        if let (Some(co), Some(t)) = (co, r.card(CARD_SHARED)) {
            let _ = co.comment_on(
                &r.person_actor(),
                &t.id,
                coord::CommentInput {
                    text: SEEN_SHARE.into(),
                    ..Default::default()
                },
            );
            r.core.kick_board(&r.board);
        }
        r.wait("A's withdraw reaches B", |r| {
            r.card(CARD_SHARED).is_some_and(|t| t.withdrawn)
        });
    }

    // The board ask and the review request reach B's agent.
    r.wait("A's board ask reaches B's agent", |r| {
        r.bridged()
            .iter()
            .any(|d| d.message.text.contains(ASK_TEXT))
    });
    r.wait("the review request reaches B's agent", |r| {
        r.bridged()
            .iter()
            .any(|d| d.message.text.contains("review") && d.message.text.contains(PR))
    });

    // B leaves.
    match r.core.leave_board(&r.board) {
        Ok(()) => r.pass("B leaves the board", ""),
        Err(e) => r.fail("B leaves the board", &e.to_string()),
    }
    // Give the leave a moment to reach A before the Core stops.
    std::thread::sleep(Duration::from_secs(5));
    Ok(r.done())
}

fn task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}
