//! The merge desk's PR list (docs/design/merge-desk.md §3, card #415).
//!
//! For each enabled desk whose `pollDevice` is this device, the Core asks
//! GitHub for the repo's open PRs (`gh pr list`, the person's own gh) every
//! [`DESK_EVERY`] and hands the list to coord, which writes a `desk.prs`
//! snapshot only when it changed. Exactly one device polls a desk, so two
//! never write competing snapshots, and the queue never goes blind when the
//! holder changes.
//!
//! A failed poll keeps the previous snapshot: the queue is empty only when
//! GitHub's list was. Failures back off (3, 6, 12, then 15 minutes) and the
//! first success returns to [`DESK_EVERY`]. When the last poll ran and since
//! when GitHub has been unreachable stay on this device ([`DeskPoll`]), so
//! an unchanged list adds no replicated rows.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use coord::{DESK_LIMIT, DeskPr};
use gocompat::time::GoTime;
use serde::Serialize;

use crate::prwatch::{gh_path, run_gh, run_gh_raw};

/// How often the Core looks for desks that are due.
pub const DESK_TICK: Duration = Duration::from_secs(30);
/// How often a desk's PR list is asked for.
pub const DESK_EVERY: Duration = Duration::from_secs(180);
/// The longest wait after repeated failures.
pub const DESK_BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);

/// The `gh pr list --json` fields the desk reads (§3).
pub const DESK_FIELDS: &str = "number,url,title,author,isDraft,headRefName,headRefOid,baseRefName,mergeable,mergeStateStatus,statusCheckRollup,createdAt,updatedAt";

/// Asks GitHub for one repo's open PRs (`owner/name`), at most `limit`.
pub trait PrLister: Send + Sync {
    fn list(&self, repo: &str, limit: usize) -> Result<Vec<DeskPr>, String>;

    /// The GitHub login this gh is signed in as: the person's own, which
    /// the desk trusts beside their agents (card #521). "" when unknown.
    fn viewer(&self) -> Result<String, String> {
        Ok(String::new())
    }

    /// PR `number`'s failing GitHub checks, each Actions job's failed log
    /// fetched for the first [`crate::deskpost::MAX_LOGS`] (card #420).
    fn failed_checks(
        &self,
        _repo: &str,
        _number: i64,
    ) -> Result<Vec<crate::deskpost::FailedCheck>, String> {
        Ok(Vec::new())
    }
}

/// The person's own `gh`: `gh pr list --repo <owner/name> --state open
/// --limit <limit> --json <DESK_FIELDS>`.
pub struct GhLister;

impl PrLister for GhLister {
    fn list(&self, repo: &str, limit: usize) -> Result<Vec<DeskPr>, String> {
        let Some(gh) = gh_path() else {
            return Err("gh (the GitHub CLI) isn't installed here, so the merge desk can't list PRs: https://cli.github.com".into());
        };
        let limit = limit.to_string();
        let v = run_gh(
            &gh,
            &[
                "pr",
                "list",
                "--repo",
                repo,
                "--state",
                "open",
                "--limit",
                &limit,
                "--json",
                DESK_FIELDS,
            ],
            "gh couldn't list this repo's PRs",
        )?;
        parse_pr_list(&v)
    }

    /// `gh api user`'s login.
    fn viewer(&self) -> Result<String, String> {
        let Some(gh) = gh_path() else {
            return Err("gh (the GitHub CLI) isn't installed here".into());
        };
        let v = run_gh(
            &gh,
            &["api", "user"],
            "gh couldn't say who it is signed in as",
        )?;
        Ok(v["login"].as_str().unwrap_or_default().to_string())
    }

    /// `gh pr view <n> --json statusCheckRollup`, then `gh run view --job
    /// <id> --log-failed` for each failing Actions job. A log gh can't give
    /// is left out; the check is still named and linked.
    fn failed_checks(
        &self,
        repo: &str,
        number: i64,
    ) -> Result<Vec<crate::deskpost::FailedCheck>, String> {
        let Some(gh) = gh_path() else {
            return Err("gh (the GitHub CLI) isn't installed here".into());
        };
        let n = number.to_string();
        let v = run_gh(
            &gh,
            &["pr", "view", &n, "--repo", repo, "--json", "statusCheckRollup"],
            "gh couldn't read this PR's checks",
        )?;
        let mut checks = crate::deskpost::failing_checks(&v["statusCheckRollup"]);
        for c in checks.iter_mut().take(crate::deskpost::MAX_LOGS) {
            let Some(job) = crate::deskpost::actions_job(&c.url).map(str::to_string) else {
                continue;
            };
            match run_gh_raw(
                &gh,
                &["run", "view", "--job", &job, "--repo", repo, "--log-failed"],
                "gh couldn't fetch the job's log",
            ) {
                Ok(out) => c.log = String::from_utf8_lossy(&out).into_owned(),
                Err(e) => crate::logf(format_args!("desk: PR #{number} job {job} log: {e}")),
            }
        }
        Ok(checks)
    }
}

/// Reads `gh pr list --json DESK_FIELDS`'s answer. A missing field reads
/// as empty; an answer that isn't a list is an error.
pub fn parse_pr_list(v: &serde_json::Value) -> Result<Vec<DeskPr>, String> {
    let Some(items) = v.as_array() else {
        return Err("gh's PR list isn't a list".into());
    };
    let s = |p: &serde_json::Value, k: &str| p[k].as_str().unwrap_or_default().to_string();
    let t = |p: &serde_json::Value, k: &str| {
        p[k].as_str()
            .and_then(GoTime::parse_rfc3339)
            .unwrap_or_default()
    };
    Ok(items
        .iter()
        .filter_map(|p| {
            Some(DeskPr {
                number: p["number"].as_i64()?,
                url: s(p, "url"),
                title: s(p, "title"),
                author: p["author"]["login"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                is_draft: p["isDraft"].as_bool().unwrap_or(false),
                head_ref_name: s(p, "headRefName"),
                head_ref_oid: s(p, "headRefOid"),
                base_ref_name: s(p, "baseRefName"),
                mergeable: s(p, "mergeable"),
                merge_state_status: s(p, "mergeStateStatus"),
                checks: checks_of(&p["statusCheckRollup"]),
                created_at: t(p, "createdAt"),
                updated_at: t(p, "updatedAt"),
            })
        })
        .collect())
}

/// Sums up GitHub's statusCheckRollup: "failure" when any check failed,
/// else "pending" when any is still running, else "success"; "" when the
/// repo has no checks on GitHub (the desk's own check line is what counts
/// there, slice 4).
pub fn checks_of(rollup: &serde_json::Value) -> String {
    let Some(items) = rollup.as_array().filter(|a| !a.is_empty()) else {
        return String::new();
    };
    let mut pending = false;
    for c in items {
        if check_failed(c) {
            return "failure".into();
        }
        let up = |k: &str| c[k].as_str().unwrap_or_default().to_ascii_uppercase();
        let (status, state) = (up("status"), up("state"));
        let running = (!status.is_empty() && status != "COMPLETED")
            || matches!(state.as_str(), "PENDING" | "EXPECTED");
        pending |= running;
    }
    if pending { "pending" } else { "success" }.into()
}

/// Whether one statusCheckRollup item failed. A CheckRun has status and
/// conclusion; a StatusContext has state.
pub fn check_failed(c: &serde_json::Value) -> bool {
    let up = |k: &str| c[k].as_str().unwrap_or_default().to_ascii_uppercase();
    matches!(
        up("conclusion").as_str(),
        "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
    ) || matches!(up("state").as_str(), "FAILURE" | "ERROR")
}

/// The wait before the next poll after `failures` failures in a row.
pub fn backoff(failures: u32) -> Duration {
    if failures == 0 {
        return DESK_EVERY;
    }
    let d = DESK_EVERY.saturating_mul(1 << failures.min(8));
    d.min(DESK_BACKOFF_MAX)
}

/// How one desk's polling stands on this device: shown on the desk's
/// panel ("last poll …", "GitHub unreachable since …").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeskPoll {
    /// When the last poll that reached GitHub ran.
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    pub last_poll_at: GoTime,
    /// Since when GitHub couldn't be asked; zero while it can.
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    pub unreachable_since: GoTime,
    /// Why the last poll failed.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// Failed polls in a row.
    pub failures: u32,
}

#[derive(Default)]
struct Slot {
    next: Option<Instant>,
    poll: DeskPoll,
}

/// Each desk's next poll and how polling stands, by repo key.
#[derive(Default)]
pub(crate) struct DeskWatch {
    slots: Mutex<HashMap<String, Slot>>,
    /// The (PR, head) each desk's holder was last woken for (card #418),
    /// so a poll that finds the same PR still waiting doesn't wake it again.
    woke: Mutex<HashMap<String, (i64, String)>>,
    /// The login this device's gh is signed in as, once learned (card
    /// #521); asked again after a failure, never after a success.
    viewer: Mutex<Option<String>>,
    /// Polls that couldn't fetch a PR's failing checks in full, by
    /// `repo\0pr\0head` (card #420).
    tries: Mutex<HashMap<String, u32>>,
}

/// Polls a "checks failed" post waits for its checks and logs before it
/// goes up with what it has.
pub const FETCH_TRIES: u32 = 3;

impl DeskWatch {
    /// The person's GitHub login as `lister` says, asked once per run.
    /// "" while it can't be learned.
    pub(crate) fn viewer(&self, lister: &dyn PrLister) -> String {
        let mut v = self.viewer.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(login) = v.as_ref() {
            return login.clone();
        }
        match lister.viewer() {
            Ok(login) if !login.trim().is_empty() => {
                let login = login.trim().to_string();
                *v = Some(login.clone());
                login
            }
            _ => String::new(),
        }
    }

    /// Whether the holder of `repo_key`'s desk should be woken for `pr` at
    /// `head`: true once per (PR, head), recording it.
    pub(crate) fn first_wake(&self, repo_key: &str, pr: i64, head: &str) -> bool {
        let mut w = self.woke.lock().unwrap_or_else(|e| e.into_inner());
        let k = (pr, head.to_string());
        if w.get(&coord::canonical_repo_key(repo_key)) == Some(&k) {
            return false;
        }
        w.insert(coord::canonical_repo_key(repo_key), k);
        true
    }

    /// Notes one more poll that couldn't fetch PR `pr`'s failing checks at
    /// `head` in full; whether to wait for the next poll rather than post
    /// what it has (true for the first [`FETCH_TRIES`] - 1).
    pub(crate) fn fetch_again(&self, repo_key: &str, pr: i64, head: &str) -> bool {
        let mut t = self.tries.lock().unwrap_or_else(|e| e.into_inner());
        let k = format!("{}\0{pr}\0{head}", coord::canonical_repo_key(repo_key));
        let n = t.entry(k.clone()).or_insert(0);
        *n += 1;
        if *n < FETCH_TRIES {
            return true;
        }
        t.remove(&k);
        false
    }

    /// Whether `repo_key` is due at `now` (always, before its first poll).
    pub(crate) fn due(&self, repo_key: &str, now: Instant) -> bool {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots
            .get(&coord::canonical_repo_key(repo_key))
            .and_then(|s| s.next)
            .is_none_or(|n| now >= n)
    }

    /// Records a poll's outcome and schedules the next one.
    pub(crate) fn heard(&self, repo_key: &str, ok: Result<(), String>, now: Instant, at: GoTime) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let s = slots
            .entry(coord::canonical_repo_key(repo_key))
            .or_default();
        match ok {
            Ok(()) => {
                s.poll.last_poll_at = at;
                s.poll.unreachable_since = GoTime::default();
                s.poll.error.clear();
                s.poll.failures = 0;
            }
            Err(e) => {
                if s.poll.unreachable_since.is_zero() {
                    s.poll.unreachable_since = at;
                }
                s.poll.error = e;
                s.poll.failures = s.poll.failures.saturating_add(1);
            }
        }
        s.next = Some(now + backoff(s.poll.failures));
    }

    /// How polling `repo_key` stands on this device.
    pub(crate) fn poll(&self, repo_key: &str) -> DeskPoll {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots
            .get(&coord::canonical_repo_key(repo_key))
            .map(|s| s.poll.clone())
            .unwrap_or_default()
    }
}

/// [`DESK_LIMIT`], re-exported for callers that only see the Core.
pub const LIMIT: usize = DESK_LIMIT;
