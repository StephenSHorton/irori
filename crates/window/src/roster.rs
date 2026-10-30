//! The window's view of who is running (I:roster_service.go): the agents,
//! this device, the newest log rows, recently gone sessions and Revive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gocompat::time::GoTime;
use presence::AgentCard;
use serde::Serialize;
use specta::Type;
use store::{Event, Filter, Timestamp};

use crate::Window;
use crate::ts;

/// The roster:changed payload: the change plus the full roster after it.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RosterChange {
    #[specta(type = ts::Change)]
    pub change: presence::Change,
    #[specta(type = Vec<ts::AgentCard>)]
    pub agents: Vec<coord::roster::AgentCard>,
}

/// A recently gone Claude Code session the person may revive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GoneSession {
    pub key: String,
    pub session_id: String,
    pub name: String,
    pub title: String,
    pub cwd: String,
    pub repo_key: String,
    #[specta(type = ts::Time)]
    pub gone_at: GoTime,
    pub reason: String,
    pub transcript: bool,
    pub trusted: bool,
}

/// How long a revived session stays claimed.
pub const REVIVE_WINDOW: Duration = Duration::from_secs(60);
/// How long Revive waits on `claude --bg` before answering anyway.
const REVIVE_WAIT: Duration = Duration::from_secs(30);
/// How long Revive keeps reading the child's pipes after it exits.
const WAIT_DELAY: Duration = Duration::from_secs(3);
/// The output Revive keeps.
const OUTPUT_CAP: usize = 64 << 10;

/// The newest agent.seen card per key on this device (Go `cardIndex`).
#[derive(Default)]
pub(crate) struct CardIndex {
    cursor: Timestamp,
    cards: HashMap<String, AgentCard>,
}

/// Sessions revived a moment ago (Go `reviveGuard`).
#[derive(Default)]
pub(crate) struct ReviveGuard {
    at: HashMap<String, Instant>,
    wall: HashMap<String, SystemTime>,
}

impl ReviveGuard {
    fn claim(&mut self, id: &str) -> Result<(), Duration> {
        self.at.retain(|_, t| t.elapsed() < REVIVE_WINDOW);
        let keep: Vec<String> = self.at.keys().cloned().collect();
        self.wall.retain(|k, _| keep.contains(k));
        if let Some(t) = self.at.get(id) {
            return Err(t.elapsed());
        }
        self.at.insert(id.into(), Instant::now());
        self.wall.insert(id.into(), SystemTime::now());
        Ok(())
    }

    fn release(&mut self, id: &str) {
        self.at.remove(id);
        self.wall.remove(id);
    }

    /// Whether `id` was revived inside the window after `after`.
    fn recent(&self, id: &str, after: SystemTime) -> bool {
        match (self.at.get(id), self.wall.get(id)) {
            (Some(t), Some(w)) => *w > after && t.elapsed() < REVIVE_WINDOW,
            _ => false,
        }
    }
}

#[derive(serde::Deserialize)]
struct GonePayload {
    #[serde(default)]
    key: String,
    #[serde(default)]
    reason: String,
}

fn err(e: store::Error) -> String {
    e.to_string()
}

impl Window {
    /// RosterService.Agents: the live agents on every one of the person's
    /// devices.
    pub fn agents(&self) -> Vec<coord::roster::AgentCard> {
        self.core.mesh_agents()
    }

    /// RosterService.Recent: the newest log rows, newest first.
    pub fn recent(&self, limit: i64) -> Result<Vec<Event>, String> {
        let limit = if limit <= 0 { 100 } else { limit };
        self.core
            .events(&Filter {
                limit,
                desc: true,
                ..Filter::default()
            })
            .map_err(err)
    }

    /// RosterService.Gone: sessions that left the roster recently and
    /// aren't live again (newest first, default 20). Claude sessions only.
    pub fn gone(&self, limit: i64) -> Result<Vec<GoneSession>, String> {
        let limit = if limit <= 0 { 20 } else { limit as usize };
        let gone = self
            .core
            .events(&Filter {
                kinds: vec![store::kinds::AGENT_GONE.into()],
                limit: 200,
                desc: true,
                ..Filter::default()
            })
            .map_err(err)?;
        let live: std::collections::HashSet<String> = self
            .core
            .local_agents()
            .into_iter()
            .map(|a| a.key)
            .collect();
        let mut cards: Option<HashMap<String, AgentCard>> = None;
        let mut trust: Option<TrustSet> = None;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for ev in gone {
            let Ok(p) = serde_json::from_slice::<GonePayload>(&ev.payload) else {
                continue;
            };
            if p.key.is_empty() || seen.contains(&p.key) || live.contains(&p.key) {
                continue;
            }
            seen.insert(p.key.clone());
            if cards.is_none() {
                cards = Some(self.seen_cards()?);
                trust = Some(TrustSet::load());
            }
            let Some(c) = cards.as_ref().and_then(|m| m.get(&p.key)) else {
                continue;
            };
            let wall = ev.hlc.wall();
            let recent = self
                .revives
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recent(&c.session_id, wall);
            if c.agent != "claude" || !valid_session_id(&c.session_id) || recent {
                continue;
            }
            out.push(GoneSession {
                key: c.key.clone(),
                session_id: c.session_id.clone(),
                name: c.name.clone(),
                title: c.title.clone(),
                cwd: c.cwd.clone(),
                repo_key: c.repo_key.clone(),
                gone_at: GoTime::from_system(wall),
                reason: p.reason,
                transcript: self.has_transcript(c),
                trusted: trust.as_ref().is_some_and(|t| t.covers(&c.cwd)),
            });
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }

    /// The newest agent.seen card per key, for every session ever seen on
    /// this device; each call reads only the rows since the last.
    fn seen_cards(&self) -> Result<HashMap<String, AgentCard>, String> {
        let mut x = self.cards.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let rows = self
                .core
                .events(&Filter {
                    kinds: vec![store::kinds::AGENT_SEEN.into()],
                    device_id: self.core.device_id().into(),
                    after_hlc: x.cursor,
                    limit: store::MAX_LIST_LIMIT,
                    ..Filter::default()
                })
                .map_err(err)?;
            let n = rows.len() as i64;
            for ev in rows {
                x.cursor = ev.hlc;
                if let Ok(c) = serde_json::from_slice::<AgentCard>(&ev.payload)
                    && !c.key.is_empty()
                {
                    x.cards.insert(c.key.clone(), c);
                }
            }
            if n < store::MAX_LIST_LIMIT {
                break;
            }
        }
        Ok(x.cards.clone())
    }

    /// Whether the session's transcript is on disk (the card's path, else
    /// Claude Code's own location).
    fn has_transcript(&self, c: &AgentCard) -> bool {
        if file_exists(Path::new(&c.transcript)) {
            return true;
        }
        if c.cwd.is_empty() {
            return false;
        }
        file_exists(
            &self
                .cfg
                .claude_dir
                .join("projects")
                .join(presence::slug(&c.cwd))
                .join(format!("{}.jsonl", c.session_id)),
        )
    }

    /// RosterService.Revive: `claude --bg --resume <id>` in the session's
    /// last working directory; returns what it printed.
    pub fn revive(&self, session_id: &str) -> Result<String, String> {
        let id = crate::trim(session_id);
        if !valid_session_id(id) {
            return Err("not a session id".into());
        }
        for a in self.core.local_agents() {
            if a.session_id == id {
                let name = if a.name.is_empty() {
                    "that session".to_string()
                } else {
                    a.name
                };
                return Err(format!("{name} is still running"));
            }
        }
        if let Err(ago) = self
            .revives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .claim(id)
        {
            return Err(format!(
                "that session was revived {}s ago; it should show up on the roster shortly",
                (ago.as_millis() + 500) / 1000
            ));
        }
        let release = || {
            self.revives
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .release(id)
        };
        let cards = match self.seen_cards() {
            Ok(c) => c,
            Err(e) => {
                release();
                return Err(e);
            }
        };
        let key = format!("{}/{id}", self.core.device_id());
        let c = match cards.get(&key) {
            Some(c) if c.agent == "claude" => c.clone(),
            _ => {
                release();
                return Err(
                    "no Claude Code session with that id has been seen on this device".into(),
                );
            }
        };
        if !self.has_transcript(&c) {
            release();
            return Err("that session's transcript is gone, so there is nothing to resume".into());
        }
        if !Path::new(&c.cwd).is_dir() {
            release();
            return Err(format!("its folder {} no longer exists", c.cwd));
        }
        let bin = match find_claude() {
            Ok(b) => b,
            Err(e) => {
                release();
                return Err(e);
            }
        };
        let mut cmd = std::process::Command::new(bin);
        cmd.args(["--bg", "--resume", id])
            .current_dir(&c.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        hide_console(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                release();
                return Err(format!("start claude: {e}"));
            }
        };
        // From here a session may be starting, so the claim stands.
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
        let status = match rx.recv_timeout(REVIVE_WAIT) {
            Ok(s) => s,
            // Never kill it: that could be the revived session itself.
            Err(_) => {
                let text = strip_ansi(&lossy(&out));
                return Ok(
                    format!("{text}\n(claude is still running; check the roster)")
                        .trim()
                        .to_string(),
                );
            }
        };
        let deadline = Instant::now() + WAIT_DELAY;
        for r in readers {
            while !r.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let text = strip_ansi(&lossy(&out)).trim().to_string();
        match status {
            Ok(s) if s.success() => Ok(text),
            Ok(s) => {
                let t = if text.is_empty() {
                    format!("exit status {}", s.code().unwrap_or(-1))
                } else {
                    text
                };
                Err(format!("claude --bg --resume failed: {t}"))
            }
            Err(e) => {
                let t = if text.is_empty() { e.to_string() } else { text };
                Err(format!("claude --bg --resume failed: {t}"))
            }
        }
    }
}

fn lossy(out: &Mutex<Vec<u8>>) -> String {
    String::from_utf8_lossy(&out.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
}

fn copy_capped(mut r: Box<dyn std::io::Read + Send>, out: &Mutex<Vec<u8>>) {
    let mut buf = [0u8; 4096];
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            return;
        }
        let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
        let room = OUTPUT_CAP.saturating_sub(o.len());
        o.extend_from_slice(&buf[..n.min(room)]);
    }
}

/// Claude Code's UUID-style ids: hex and dashes, 8 to 64, never a leading
/// dash.
pub fn valid_session_id(id: &str) -> bool {
    if id.len() < 8 || id.len() > 64 || id.starts_with('-') {
        return false;
    }
    id.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
}

fn file_exists(p: &Path) -> bool {
    !p.as_os_str().is_empty() && p.metadata().is_ok_and(|m| m.is_file())
}

/// The Claude Code CLI without a shell: PATH first, then ~/.local/bin.
fn find_claude() -> Result<PathBuf, String> {
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    };
    if let Some(path) = std::env::var_os("PATH") {
        for n in names {
            for dir in std::env::split_paths(&path) {
                let p = dir.join(n);
                if file_exists(&p) {
                    return Ok(p);
                }
            }
        }
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    if let Some(h) = home {
        for n in names {
            let p = PathBuf::from(&h).join(".local").join("bin").join(n);
            if file_exists(&p) {
                return Ok(p);
            }
        }
    }
    Err("can't find the claude command on PATH".into())
}

/// Keeps a console child of the GUI from flashing a window on Windows.
fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Terminal colour and cursor escapes removed (Go `ansiEscape`).
pub fn strip_ansi(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != 0x1b || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        if c == b'[' {
            // CSI: params [0-9;?]*, intermediates [ -/]*, final [@-~].
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';' || b[j] == b'?') {
                j += 1;
            }
            while j < b.len() && (0x20..=0x2f).contains(&b[j]) {
                j += 1;
            }
            if j < b.len() && (0x40..=0x7e).contains(&b[j]) {
                i = j + 1;
                continue;
            }
        } else if c == b']' {
            // OSC: up to BEL or ESC \.
            let mut j = i + 2;
            while j < b.len() && b[j] != 0x07 && b[j] != 0x1b {
                j += 1;
            }
            if j < b.len() && b[j] == 0x07 {
                i = j + 1;
                continue;
            }
            if j + 1 < b.len() && b[j] == 0x1b && b[j + 1] == b'\\' {
                i = j + 2;
                continue;
            }
        } else if (b'@'..=b'Z').contains(&c) || (b'\\'..=b'_').contains(&c) {
            i += 2;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The folders Claude Code's workspace trust dialog was accepted for.
struct TrustSet(std::collections::HashSet<String>);

impl TrustSet {
    fn load() -> TrustSet {
        let file = match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
            Some(d) => PathBuf::from(d).join(".claude.json"),
            None => match std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
                Some(h) => PathBuf::from(h).join(".claude.json"),
                None => return TrustSet(Default::default()),
            },
        };
        let Ok(raw) = std::fs::read(&file) else {
            return TrustSet(Default::default());
        };
        #[derive(serde::Deserialize)]
        struct Proj {
            #[serde(rename = "hasTrustDialogAccepted", default)]
            accepted: bool,
        }
        #[derive(serde::Deserialize)]
        struct Cfg {
            #[serde(default)]
            projects: HashMap<String, Proj>,
        }
        let Ok(cfg) = serde_json::from_slice::<Cfg>(&raw) else {
            return TrustSet(Default::default());
        };
        TrustSet(
            cfg.projects
                .into_iter()
                .filter(|(_, v)| v.accepted)
                .map(|(p, _)| trust_key(&p))
                .collect(),
        )
    }

    fn covers(&self, dir: &str) -> bool {
        if dir.is_empty() {
            return false;
        }
        let mut p = gocompat::filepath::clean(dir);
        loop {
            if self.0.contains(&trust_key(&p)) {
                return true;
            }
            let parent = parent_dir(&p);
            if parent == p {
                return false;
            }
            p = parent;
        }
    }
}

fn trust_key(p: &str) -> String {
    let c = gocompat::filepath::clean(p).replace('\\', "/");
    let c = c.trim_end_matches('/').to_string();
    if cfg!(windows) { c.to_lowercase() } else { c }
}

/// Go `filepath.Dir` of a cleaned path.
fn parent_dir(p: &str) -> String {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let vol = gocompat::filepath::volume_name(p);
    let rest = &p[vol.len()..];
    match rest.rfind(seps) {
        Some(i) => gocompat::filepath::clean(&format!("{vol}{}", &rest[..=i])),
        None => gocompat::filepath::clean(&format!("{vol}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids() {
        assert!(valid_session_id("aaaaaaaa-1111-4111-8111-000000000001"));
        assert!(!valid_session_id("-aaaaaaaa"));
        assert!(!valid_session_id("short"));
        assert!(!valid_session_id("zzzzzzzzzz"));
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m done"), "ok done");
        assert_eq!(strip_ansi("\x1b]0;title\x07x"), "x");
        assert_eq!(strip_ansi("a\x1bMb"), "ab");
    }

    #[test]
    fn revive_guard_holds_a_minute() {
        let mut g = ReviveGuard::default();
        assert!(g.claim("s").is_ok());
        assert!(g.claim("s").is_err());
        assert!(g.recent("s", SystemTime::UNIX_EPOCH));
        g.release("s");
        assert!(g.claim("s").is_ok());
    }
}
