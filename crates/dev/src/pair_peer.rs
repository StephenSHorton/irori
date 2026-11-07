//! `irori-dev pair-peer`: a scratch Irori on this computer for a phone to
//! pair with (card #680, the Rune phone prototype). It starts a real Core
//! in-process on a fresh scratch data dir, as a made-up person with one
//! made-up Claude session, puts a few cards on the Board as that agent,
//! turns its home server on at a fixed port and opens an Add window (the
//! Add that only the desktop app's Settings → Devices has; the API keeps no
//! Add route on purpose, crates/api/src/pair.rs). It prints the code, this
//! device's key and the server address for the phone's join screen, then
//! reads commands on stdin while it serves:
//!
//!   words <w1> <w2> ...   the words the phone shows (confirms the pairing)
//!   card <title>          a new card, as the agent (the phone sees it live)
//!   step <#ref> <sN>      ticks a card's step, as the agent
//!   show <#ref>           the card as the tools show it, comments included
//!   devices               this person's devices
//!   status                the pairing window's state
//!   quit
//!
//! It never touches a live data dir: --home must be empty, and the keys are
//! sealed with the file keystore, so no Keychain item is made. An emulator
//! reaches the home server at 10.0.2.2:<port> (the host's loopback) or at
//! the Mac's LAN address.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use irori_core::{Core, Options};

const SESSION: &str = "68068068-0680-4680-8680-680680680680";
const AGENT_NAME: &str = "pair-peer";

pub struct Opts {
    home: PathBuf,
    claude: PathBuf,
    port: u16,
}

impl Opts {
    pub fn parse(args: &[String]) -> Result<Opts, String> {
        let mut o = Opts {
            home: PathBuf::new(),
            claude: PathBuf::new(),
            port: 24817,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
            match a.as_str() {
                "--home" => o.home = val()?.into(),
                "--claude" => o.claude = val()?.into(),
                "--port" => o.port = val()?.parse().map_err(|_| "--port is a number")?,
                other => return Err(format!("unknown flag {other}")),
            }
        }
        if o.home.as_os_str().is_empty() || o.claude.as_os_str().is_empty() {
            return Err("--home and --claude are required".into());
        }
        Ok(o)
    }
}

fn empty_dir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut it = std::fs::read_dir(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if it.next().is_some() {
        return Err(format!(
            "{} is not empty: pair-peer only runs on a fresh scratch dir",
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
/// agent the cards can come from.
fn write_session(claude: &Path) -> Result<(), String> {
    let sessions = claude.join("sessions");
    let cwd = claude.join("work");
    std::fs::create_dir_all(&sessions).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
    let m = serde_json::json!({
        "pid": std::process::id(), "sessionId": SESSION, "cwd": cwd.to_string_lossy(),
        "name": AGENT_NAME, "status": "idle", "kind": "bg", "entrypoint": "cli",
        "version": "2.1.311", "startedAt": unix_ms(), "updatedAt": unix_ms(),
    });
    std::fs::write(
        sessions.join("900680.json"),
        serde_json::to_vec(&m).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

struct Peer {
    core: Core,
    agent: String,
}

impl Peer {
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

    fn card(&self, title: &str, objective: &str, steps: &[&str]) {
        let steps: Vec<_> = steps
            .iter()
            .map(|t| serde_json::json!({ "text": t }))
            .collect();
        let out = self.tool(
            "board_create",
            serde_json::json!({ "title": title, "objective": objective, "steps": steps }),
        );
        println!("card: {}", out.lines().next().unwrap_or(""));
    }
}

pub fn run(o: &Opts) -> ExitCode {
    match serve(o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pair-peer: {e}");
            ExitCode::FAILURE
        }
    }
}

fn serve(o: &Opts) -> Result<(), String> {
    empty_dir(&o.home)?;
    empty_dir(&o.claude)?;
    write_session(&o.claude)?;
    // The home server on at a fixed port, never mapped on the router.
    std::fs::write(
        o.home.join("home-server.json"),
        format!("{{\"on\":true,\"port\":{},\"outside\":false}}", o.port),
    )
    .map_err(|e| e.to_string())?;
    let core = Core::start(Options {
        data_dir: o.home.clone(),
        claude_dir: o.claude.clone(),
        version: "pair-peer".into(),
        headless: true,
        keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
        ..Options::default()
    })
    .map_err(|e| format!("start: {e}"))?;
    let agent = format!("{}/{SESSION}", core.device_id());
    let deadline = Instant::now() + Duration::from_secs(120);
    while !core.local_agents().iter().any(|a| a.key == agent) {
        if Instant::now() > deadline {
            return Err("presence never listed the made-up session".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let peer = Peer { core, agent };
    println!(
        "pair-peer: person {} on device {}, data {}",
        peer.core.person().id,
        peer.core.device_id(),
        o.home.display()
    );
    peer.card(
        "Try Irori's Board on a Rune phone",
        "Card #680's prototype: this card comes from the Mac and shows on the phone.",
        &[
            "Pair the phone",
            "See this card on the phone",
            "Comment from the phone",
        ],
    );
    peer.card(
        "Check the Mac sees the phone's comment",
        "The phone's comment arrives here over the mesh.",
        &["Read the thread on the Mac"],
    );
    peer.card("A third card, no steps", "", &[]);

    let rt = peer.core.pairing_runtime().clone();
    let st = rt
        .block_on(irori_core::devices::add_device(&peer.core, "desktop"))
        .map_err(|e| format!("add: {e}"))?;
    println!("code: {}", st.code);
    println!("key: {}", st.fingerprint);
    println!(
        "server: {} (from an emulator: 10.0.2.2:{})",
        st.server_addr, o.port
    );
    println!("pair-peer: ready; commands: words, card, step, show, devices, status, quit");
    let _ = std::io::stdout().flush();

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match cmd {
            "" => continue,
            "words" => match rt.block_on(irori_core::devices::confirm_words(&peer.core, rest)) {
                Ok(()) => println!("words: confirmed"),
                Err(e) => println!("words: {e}"),
            },
            "card" => peer.card(rest, "Made on the Mac while the phone watched.", &[]),
            "step" => {
                let (r, s) = rest.split_once(' ').unwrap_or((rest, ""));
                let out = peer.tool(
                    "board_step",
                    serde_json::json!({ "task_id": r, "step_id": s.trim(), "done": true }),
                );
                println!("step: {}", out.lines().next().unwrap_or(""));
            }
            "show" => println!("{}", peer.tool("show", serde_json::json!({ "ref": rest }))),
            "devices" => {
                for d in peer.core.certified_devices() {
                    println!("device: {d:?}");
                }
            }
            "status" => println!("status: {:?}", peer.core.pairing().status()),
            "quit" => break,
            other => println!("unknown command {other}"),
        }
        let _ = std::io::stdout().flush();
    }
    peer.core.close().map_err(|e| e.to_string())
}
