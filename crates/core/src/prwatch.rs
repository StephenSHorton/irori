//! Asks GitHub how the PRs this device's agents' cards link stand (card
//! #208) and hands the answers to coord, which records them and says what
//! changed (coord's `prstate`).
//!
//! Only the device whose agent holds a card asks about its PRs, with the
//! person's own `gh`; the rows it writes reach the other devices like any
//! other. An open PR is asked about again every [`OPEN_EVERY`]; one that
//! couldn't be asked about every [`ERROR_EVERY`]; a closed one, which
//! GitHub can reopen, every [`CLOSED_EVERY`]; a merged one never again. A
//! device that can't ask (no gh, card #542) asks about nothing and writes
//! nothing.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use coord::{Coord, PR_CLOSED, PR_MERGED, PrCheck};

/// How often the poller looks for PRs that are due.
pub const TICK: Duration = Duration::from_secs(30);
/// How often an open PR is asked about.
pub const OPEN_EVERY: Duration = Duration::from_secs(180);
/// How often a PR GitHub couldn't be asked about is tried again.
pub const ERROR_EVERY: Duration = Duration::from_secs(1800);
/// How often a closed PR is asked about, in case it is reopened (card #548).
pub const CLOSED_EVERY: Duration = Duration::from_secs(1800);
/// How long one `gh` call may take.
const GH_TIMEOUT: Duration = Duration::from_secs(30);

/// Asks GitHub about one PR. The answer carries an error instead of a
/// state when it couldn't be asked.
pub trait PrChecker: Send + Sync {
    fn check(&self, url: &str) -> PrCheck;

    /// Whether this device can ask at all. While it can't, no PR is due
    /// and no row is written (card #542).
    fn ready(&self) -> bool {
        true
    }
}

/// The person's own `gh`: `gh pr view <url> --json state,mergedAt,reviewDecision`.
pub struct GhChecker;

impl PrChecker for GhChecker {
    /// Looked up each tick, so a gh installed later is picked up.
    fn ready(&self) -> bool {
        gh_path().is_some()
    }

    fn check(&self, url: &str) -> PrCheck {
        let mut c = PrCheck {
            pr: url.into(),
            ..PrCheck::default()
        };
        let Some(gh) = gh_path() else {
            c.error = "gh (the GitHub CLI) isn't installed here, so Irori can't see whether this PR is merged: https://cli.github.com".into();
            return c;
        };
        match run_gh(
            &gh,
            &["pr", "view", url, "--json", "state,mergedAt,reviewDecision"],
            "gh couldn't read this PR",
        ) {
            Ok(v) => {
                c.state = v["state"].as_str().unwrap_or_default().to_string();
                c.review = v["reviewDecision"].as_str().unwrap_or_default().to_string();
                if let Some(m) = v["mergedAt"].as_str().filter(|m| !m.is_empty()) {
                    c.merged_at = gocompat::time::GoTime::parse_rfc3339(m).unwrap_or_default();
                }
                if c.state.is_empty() {
                    c.error = "gh answered without the PR's state".into();
                }
            }
            Err(e) => c.error = e,
        }
        c
    }
}

pub(crate) fn gh_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::split_paths(&path)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Runs the person's `gh` with `args` (prompts off, no console window, a
/// timeout) and parses its JSON answer; a failure is `fail: <gh's first
/// error line>`.
pub(crate) fn run_gh(gh: &PathBuf, args: &[&str], fail: &str) -> Result<serde_json::Value, String> {
    let mut cmd = Command::new(gh);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: Irori is a GUI app, so without it every PR check
        // flashes a console (a Windows Terminal window) on the desktop.
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run gh: {e}"))?;
    // Read both pipes while gh runs: a long answer (desk lists, card #415)
    // would otherwise fill the pipe and stall gh until the timeout.
    let drain = |p: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = p {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > GH_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("gh took too long to answer".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(format!("gh: {e}")),
        }
    };
    let out_stdout = stdout.join().unwrap_or_default();
    let out_stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        let err = String::from_utf8_lossy(&out_stderr);
        let line = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("gh failed");
        return Err(format!("{fail}: {}", line.trim()));
    }
    serde_json::from_slice(&out_stdout).map_err(|e| format!("gh's answer isn't JSON: {e}"))
}

/// When each PR was last asked about, by `taskId\x00url`.
#[derive(Default)]
pub(crate) struct PrWatch {
    asked: Mutex<HashMap<String, Instant>>,
    /// The PRs gh last failed on here, by the same key: they keep to
    /// [`ERROR_EVERY`] even when another device's good row clears the
    /// error in the fold, as this gh is still broken (card #548).
    failed: Mutex<HashSet<String>>,
}

/// One card's PR that is due, and the log it lives in.
pub(crate) struct Due {
    pub co: Arc<Coord>,
    pub task_id: String,
    pub url: String,
}

fn pr_key(task_id: &str, url: &str) -> String {
    format!("{task_id}\x00{url}")
}

impl PrWatch {
    /// Notes how asking about `url` went on this device.
    pub(crate) fn answered(&self, task_id: &str, url: &str, ok: bool) {
        let mut failed = self.failed.lock().unwrap_or_else(|e| e.into_inner());
        if ok {
            failed.remove(&pr_key(task_id, url));
        } else {
            failed.insert(pr_key(task_id, url));
        }
    }

    /// The PRs to ask about now: on a card in `coords` that isn't done and
    /// that an agent in `local` holds, not yet merged, and not asked about
    /// too recently. Marks them asked.
    pub(crate) fn due(
        &self,
        coords: &[Arc<Coord>],
        local: &HashSet<String>,
        private_key: &dyn Fn(&str) -> String,
        now: Instant,
    ) -> Vec<Due> {
        let mut asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
        let failed = self.failed.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for co in coords {
            let tasks = co.list_tasks(&coord::TaskFilter {
                open: true,
                ..coord::TaskFilter::default()
            });
            for t in tasks {
                let ours = t
                    .holders
                    .iter()
                    .chain(&t.co_holders)
                    .any(|h| local.contains(&private_key(h)));
                // Card #509: a wait on a PR's merge is asked about whoever
                // holds the card (often nobody), so it leaves the person's
                // Needs you once the PR merges.
                let waits = co.polled_waits(&t.id);
                if !ours && waits.is_empty() {
                    continue;
                }
                let mut prs: Vec<String> = if ours {
                    t.links
                        .iter()
                        .filter(|l| coord::pr_link(l).is_some())
                        .map(|l| l.reference.clone())
                        .collect()
                } else {
                    Vec::new()
                };
                for w in waits {
                    if !prs.contains(&w) {
                        prs.push(w);
                    }
                }
                for url in prs {
                    let heard = co.pr_state(&t.id, &url);
                    let k = heard.as_ref();
                    if k.is_some_and(|p| p.state == PR_MERGED) {
                        continue;
                    }
                    let key = pr_key(&t.id, &url);
                    let every = if k.is_some_and(|p| p.state == PR_CLOSED) {
                        CLOSED_EVERY
                    } else if failed.contains(&key)
                        || k.is_some_and(|p| p.state.is_empty() && !p.error.is_empty())
                    {
                        ERROR_EVERY
                    } else {
                        OPEN_EVERY
                    };
                    if asked
                        .get(&key)
                        .is_some_and(|at| now.duration_since(*at) < every)
                    {
                        continue;
                    }
                    asked.insert(key, now);
                    out.push(Due {
                        co: co.clone(),
                        task_id: t.id.clone(),
                        url,
                    });
                }
            }
        }
        out
    }
}
