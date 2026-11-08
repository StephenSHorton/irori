//! The merge desk's PR list (docs/design/merge-desk.md §3, card #415).
//!
//! For each enabled desk whose `pollDevice` is this device, the Core asks
//! GitHub for the repo's open PRs (`gh pr list`, the person's own gh) every
//! [`DESK_EVERY`] and hands the list to coord, which writes a `desk.prs`
//! snapshot only when it changed. Exactly one device polls a desk, so two
//! never write competing snapshots.
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
pub const DESK_FIELDS: &str = "number,url,title,author,isDraft,headRefName,headRefOid,baseRefName,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision,createdAt,updatedAt";

/// Asks GitHub for one repo's open PRs (`owner/name`), at most `limit`.
pub trait PrLister: Send + Sync {
    fn list(&self, repo: &str, limit: usize) -> Result<Vec<DeskPr>, String>;

    /// The GitHub login this gh is signed in as: the person's own, which
    /// the desk trusts beside their agents (card #521). "" when unknown.
    fn viewer(&self) -> Result<String, String> {
        Ok(String::new())
    }

    /// The repos (`owner/name`) `login` has opened PRs in, open or not,
    /// newest first, at most [`AUTHORED_LIMIT`] PRs' worth: which repos are
    /// the person's (card #625). Empty when unknown.
    fn authored_repos(&self, login: &str) -> Result<Vec<String>, String> {
        let _ = login;
        Ok(Vec::new())
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

    /// PR `number`'s comments, reviews and changed files (merge desk
    /// §12.4, card #1077): `gh pr view --json
    /// comments,reviews,files,headRefOid,reviewDecision`.
    fn activity(&self, _repo: &str, _number: i64) -> Result<crate::prcard::Activity, String> {
        Ok(crate::prcard::Activity::default())
    }
}

/// How many of a login's PRs one search reads (card #625).
pub const AUTHORED_LIMIT: usize = 1000;
/// How often a login's PR repos are searched again (card #625): GitHub's
/// search API allows 30 calls a minute, and a repo rarely becomes one of
/// the person's.
pub const AUTHORED_EVERY: Duration = Duration::from_secs(30 * 60);

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
            &[
                "pr",
                "view",
                &n,
                "--repo",
                repo,
                "--json",
                "statusCheckRollup",
            ],
            "gh couldn't read this PR's checks",
        )?;
        let mut checks = crate::deskpost::failing_checks(&v["statusCheckRollup"]);
        for c in checks.iter_mut().take(crate::deskpost::MAX_LOGS) {
            let Some(job) = crate::deskpost::actions_job(&c.url).map(str::to_string) else {
                continue;
            };
            // The job's own log comes as soon as the job ends, while the
            // rest of its run may go on; `run view --log-failed` only once
            // the whole run is done, so it is the fallback (an older gh).
            let api = format!("repos/{repo}/actions/jobs/{job}/logs");
            let got = run_gh_raw(
                &gh,
                &["api", &api, "--allow-escape-sequences"],
                "gh couldn't fetch the job's log",
            )
            .or_else(|_| {
                run_gh_raw(
                    &gh,
                    &["run", "view", "--job", &job, "--repo", repo, "--log-failed"],
                    "gh couldn't fetch the job's log",
                )
            });
            match got {
                Ok(out) => c.log = String::from_utf8_lossy(&out).into_owned(),
                Err(e) => crate::logf(format_args!("desk: PR #{number} job {job} log: {e}")),
            }
        }
        Ok(checks)
    }

    /// `gh pr view <n> --repo <repo> --json
    /// comments,reviews,files,headRefOid,reviewDecision`.
    fn activity(&self, repo: &str, number: i64) -> Result<crate::prcard::Activity, String> {
        let Some(gh) = gh_path() else {
            return Err("gh (the GitHub CLI) isn't installed here".into());
        };
        let n = number.to_string();
        let v = run_gh(
            &gh,
            &[
                "pr",
                "view",
                &n,
                "--repo",
                repo,
                "--json",
                "comments,reviews,files,headRefOid,reviewDecision",
            ],
            "gh couldn't read this PR's comments and reviews",
        )?;
        Ok(crate::prcard::parse_activity(&v))
    }

    /// `gh search prs --author <login> --sort updated --limit
    /// AUTHORED_LIMIT --json repository`.
    fn authored_repos(&self, login: &str) -> Result<Vec<String>, String> {
        let Some(gh) = gh_path() else {
            return Err("gh (the GitHub CLI) isn't installed here".into());
        };
        let limit = AUTHORED_LIMIT.to_string();
        let v = run_gh(
            &gh,
            &[
                "search",
                "prs",
                "--author",
                login,
                "--sort",
                "updated",
                "--limit",
                &limit,
                "--json",
                "repository",
            ],
            "gh couldn't search this login's PRs",
        )?;
        Ok(parse_authored(&v))
    }
}

/// Reads `gh search prs --json repository`'s answer: each repo once, in
/// the order first seen.
pub fn parse_authored(v: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in v.as_array().map(Vec::as_slice).unwrap_or_default() {
        let r = p["repository"]["nameWithOwner"]
            .as_str()
            .unwrap_or_default()
            .trim();
        if !r.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(r)) {
            out.push(r.to_string());
        }
    }
    out
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
                review_decision: s(p, "reviewDecision"),
                created_at: t(p, "createdAt"),
                updated_at: t(p, "updatedAt"),
            })
        })
        .collect())
}

/// Sums up GitHub's statusCheckRollup: "failure" when any check failed,
/// else "pending" when any is still running, else "success"; "" when the
/// repo has no checks on GitHub.
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
    /// The login this device's gh is signed in as, once learned (card
    /// #521); asked again after a failure, never after a success.
    viewer: Mutex<Option<String>>,
    /// Each login's PR repos as the last search found them, and when to
    /// search again (card #625).
    authored: Mutex<HashMap<String, (Instant, Vec<String>)>>,
    /// Polls that couldn't fetch a PR's failing checks in full, by
    /// `repo\0pr\0head` (card #420).
    tries: Mutex<HashMap<String, u32>>,
    /// Posts that went out as inbox notices, by `task\0tag` (card #420).
    fallbacks: Mutex<std::collections::HashSet<String>>,
    /// The `updatedAt` each PR card's activity was last read at, by
    /// `card\0pr` (card #1077): GitHub moves it on every comment and review,
    /// so an unchanged one needs no `gh pr view`.
    activity_at: Mutex<HashMap<String, (i64, crate::prcard::Activity)>>,
    /// Why a PR has no review card yet, by `repo\0number` (card #1077).
    notes: Mutex<HashMap<String, String>>,
}

/// Polls a "checks failed" post waits for its checks and logs before it
/// goes up with what it has.
pub const FETCH_TRIES: u32 = 3;

impl DeskWatch {
    /// The person's GitHub login if this run has learned it, without
    /// asking gh.
    pub(crate) fn known_viewer(&self) -> String {
        let v = self.viewer.lock().unwrap_or_else(|e| e.into_inner());
        v.clone().unwrap_or_default()
    }

    /// The repos `login` opened PRs in, searched again every
    /// [`AUTHORED_EVERY`] (sooner after a failure: the next tick). A failed
    /// search keeps what the last one found.
    pub(crate) fn authored(&self, lister: &dyn PrLister, login: &str, now: Instant) -> Vec<String> {
        let key = login.to_ascii_lowercase();
        {
            let a = self.authored.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((next, repos)) = a.get(&key)
                && now < *next
            {
                return repos.clone();
            }
        }
        let heard = lister.authored_repos(login);
        let mut a = self.authored.lock().unwrap_or_else(|e| e.into_inner());
        match heard {
            Ok(repos) => {
                a.insert(key, (now + AUTHORED_EVERY, repos.clone()));
                repos
            }
            Err(e) => {
                crate::log(&format!("desk: search {login}'s PRs: {e}"));
                let old = a.get(&key).map(|x| x.1.clone()).unwrap_or_default();
                // Try again in a few minutes, not every tick.
                a.insert(key, (now + DESK_EVERY, old.clone()));
                old
            }
        }
    }

    /// The person's GitHub login as `lister` says, asked once per run.
    /// "" while it can't be learned.
    pub(crate) fn viewer(&self, lister: &dyn PrLister) -> String {
        if let Some(login) = self
            .viewer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            return login.clone();
        }
        // gh runs unlocked: it may take its whole timeout, and the desk page
        // and the profile writer read [`Self::known_viewer`] meanwhile.
        match lister.viewer() {
            Ok(login) if !login.trim().is_empty() => {
                let login = login.trim().to_string();
                *self.viewer.lock().unwrap_or_else(|e| e.into_inner()) = Some(login.clone());
                login
            }
            _ => String::new(),
        }
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

    /// PR `pr`'s failing checks at `head` were fetched in full: no more
    /// tries are kept for it.
    pub(crate) fn fetched(&self, repo_key: &str, pr: i64, head: &str) {
        let mut t = self.tries.lock().unwrap_or_else(|e| e.into_inner());
        t.remove(&format!(
            "{}\0{pr}\0{head}",
            coord::canonical_repo_key(repo_key)
        ));
    }

    /// Whether PR card `card`'s activity needs reading: `updated_ms` is not
    /// the `updatedAt` it was last read at (card #1077).
    pub(crate) fn activity_due(&self, card: &str, pr: &str, updated_ms: i64) -> bool {
        let m = self.activity_at.lock().unwrap_or_else(|e| e.into_inner());
        m.get(&format!("{card}\0{pr}")).map(|k| k.0) != Some(updated_ms)
    }

    /// PR card `card`'s activity was read at `updated_ms`.
    pub(crate) fn activity_read(
        &self,
        card: &str,
        pr: &str,
        updated_ms: i64,
        a: crate::prcard::Activity,
    ) {
        let mut m = self.activity_at.lock().unwrap_or_else(|e| e.into_inner());
        m.insert(format!("{card}\0{pr}"), (updated_ms, a));
    }

    /// What PR card `card`'s activity read last said. The relay goes by it
    /// at every poll, so a member whose login was learned after the read
    /// still has their words told.
    pub(crate) fn activity_last(&self, card: &str, pr: &str) -> Option<crate::prcard::Activity> {
        let m = self.activity_at.lock().unwrap_or_else(|e| e.into_inner());
        m.get(&format!("{card}\0{pr}")).map(|k| k.1.clone())
    }

    /// Why PR `number` of `repo_key` has no review card yet, or clears it
    /// (card #1077). Whether it changed.
    pub(crate) fn set_note(&self, repo_key: &str, number: i64, note: Option<String>) -> bool {
        let k = format!("{}\0{number}", coord::canonical_repo_key(repo_key));
        let mut m = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        match note {
            Some(n) => m.insert(k, n.clone()).as_ref() != Some(&n),
            None => m.remove(&k).is_some(),
        }
    }

    /// [`Self::set_note`]'s line for PR `number`, "" when none.
    pub(crate) fn note(&self, repo_key: &str, number: i64) -> String {
        let k = format!("{}\0{number}", coord::canonical_repo_key(repo_key));
        let m = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        m.get(&k).cloned().unwrap_or_default()
    }

    /// Whether post `tag` on card `task_id` already went out as an inbox
    /// notice because its board refused the comment. Comments are the
    /// record of what was said; a refused one leaves none, so this keeps
    /// the notice from going out again each poll (once more after a
    /// restart).
    pub(crate) fn fell_back(&self, task_id: &str, tag: &str) -> bool {
        let f = self.fallbacks.lock().unwrap_or_else(|e| e.into_inner());
        f.contains(&format!("{task_id}\0{tag}"))
    }

    /// Notes post `tag` on card `task_id` went out as a notice.
    pub(crate) fn fall_back(&self, task_id: &str, tag: &str) {
        let mut f = self.fallbacks.lock().unwrap_or_else(|e| e.into_inner());
        f.insert(format!("{task_id}\0{tag}"));
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
