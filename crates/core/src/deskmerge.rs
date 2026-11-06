//! Merge desk slice 7 (merge-desk.md §6, card #421): the desk's polling
//! device puts a merged PR's sha on its author card, whoever merged it and
//! wherever that card is held.
//!
//! The PR poller (`prwatch`) asks only about PRs on cards an agent on its
//! own device holds. So a "PR #n" card nobody holds, or an author card
//! held on another device that is off, would never hear its PR merged.
//! After each desk poll, the polling device looks at every open card that
//! links one of the desk repo's PRs, and asks GitHub about each PR that is
//! no longer in the open list. A merge it hears goes through the same
//! `record_pr_state` as the poller's, carrying GitHub's mergeCommit, so the
//! card's holders are told to complete it, the sha is said on the card as
//! Irori, and a "PR #n" card completes itself.
//!
//! The desk never merges (#418): agents merge on the person's yes, or the
//! person merges on GitHub. A `gh pr merge` captured on any device makes
//! this device poll its desks at its next tick instead of waiting for the
//! next 3-minute poll ([`MergeHeard`]).

use std::collections::HashSet;
use std::sync::Mutex;

use coord::{PR_CLOSED, PR_MERGED, PrState, Task};
use gocompat::time::GoTime;

/// The PRs to ask GitHub about after a desk poll: on an open card in
/// `tasks`, a PR of `repo` (`owner/name`, any case) whose number isn't in
/// `open` (the desk's latest list), and not already known closed, or
/// merged with its sha. `known` gives what a card's log knows of a PR.
/// Returned as (card id, PR URL), each once, in card order.
pub fn gone_prs(
    tasks: &[Task],
    repo: &str,
    open: &HashSet<i64>,
    known: &dyn Fn(&str, &str) -> Option<PrState>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for t in tasks {
        for l in &t.links {
            let Some((r, n)) = coord::pr_link(l) else {
                continue;
            };
            if !r.eq_ignore_ascii_case(repo) || open.contains(&n) {
                continue;
            }
            let settled = known(&t.id, &l.reference).is_some_and(|p| {
                p.state == PR_CLOSED || p.state == PR_MERGED && !p.merge_sha.is_empty()
            });
            let k = (t.id.clone(), l.reference.clone());
            if !settled && !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

/// Whether a captured Bash command line runs `gh pr merge` (loosely: a
/// stray match only costs one extra desk poll).
pub fn is_pr_merge(command: &str) -> bool {
    if !capture::gitlink::is_commit_command(command) {
        return false;
    }
    capture::gitlink::split_commands(command).iter().any(|w| {
        let words: Vec<String> = w
            .iter()
            .map(|s| s.trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase())
            .collect();
        let Some(gh) = words.iter().position(|s| {
            let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
            base == "gh" || base == "gh.exe"
        }) else {
            return false;
        };
        let rest = &words[gh + 1..];
        rest.iter()
            .position(|s| s == "pr")
            .is_some_and(|i| rest[i + 1..].iter().any(|s| s == "merge"))
    }) || {
        // `bash -c "gh pr merge 12"` and the like: is_commit_command saw it.
        let l = command.to_ascii_lowercase();
        l.contains("gh ") && l.contains(" pr ") && l.contains(" merge")
    }
}

/// This device's memory of what slice 7 already did.
#[derive(Default)]
pub(crate) struct MergeHeard {
    /// The newest `gh pr merge` command heard; `None` before the first
    /// look, which only notes where things stand.
    newest: Mutex<Option<i64>>,
    /// (card, PR) pairs GitHub said were merged, or whose answer couldn't
    /// be written: not asked again by the desk while this Irori runs.
    settled: Mutex<HashSet<(String, String)>>,
}

impl MergeHeard {
    /// Whether a `gh pr merge` newer than the last look is among `at`s of
    /// the commands in `cmds` (command, finished at). The first look
    /// answers false: the desks' own first poll covers what came before.
    pub(crate) fn news<'a>(&self, cmds: impl Iterator<Item = (&'a str, GoTime)>) -> bool {
        let newest = cmds
            .filter(|(c, _)| is_pr_merge(c))
            .map(|(_, at)| at)
            .map(|at| at.unix_nano())
            .max();
        let mut n = self.newest.lock().unwrap_or_else(|e| e.into_inner());
        let Some(was) = *n else {
            *n = Some(newest.unwrap_or(i64::MIN));
            return false;
        };
        match newest {
            Some(at) if at > was => {
                *n = Some(at);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn settled(&self, task_id: &str, url: &str) -> bool {
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(task_id.to_string(), url.to_string()))
    }

    pub(crate) fn settle(&self, task_id: &str, url: &str) {
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((task_id.to_string(), url.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_gh_pr_merge() {
        for c in [
            "gh pr merge 12 --squash",
            "gh pr merge --squash --match-head-commit abc",
            "cd x && gh pr merge 3",
            "gh -R o/n pr merge 3",
            "C:/bin/gh.exe pr merge 3",
        ] {
            assert!(is_pr_merge(c), "{c}");
        }
        for c in ["gh pr view 12", "git commit -m merge", "gh pr list", "echo gh pr merge"] {
            assert!(!is_pr_merge(c), "{c}");
        }
    }

    #[test]
    fn news_only_after_first_look() {
        let h = MergeHeard::default();
        let t = |s: i64| GoTime::unix(s, 0);
        assert!(!h.news([("gh pr merge 1", t(10))].into_iter()));
        assert!(!h.news([("gh pr merge 1", t(10))].into_iter()));
        assert!(!h.news([("git commit -m x", t(20))].into_iter()));
        assert!(h.news([("gh pr merge 1", t(10)), ("gh pr merge 2", t(30))].into_iter()));
        assert!(!h.news([("gh pr merge 2", t(30))].into_iter()));
    }
}
