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

use coord::{PR_MERGED, PrState, Task};
use gocompat::time::GoTime;

/// One (card id, PR URL) pair.
pub type CardPr = (String, String);

/// What a desk poll says of the PRs that open cards in `tasks` link on
/// `repo` (`owner/name`, any case), given `open` (the desk's latest list)
/// and `known` (what a card's log knows of a PR). `gone`: to ask GitHub
/// about, no longer in the list and not known merged with its sha (a
/// closed PR is asked too: it may have been reopened and merged since).
/// `listed`: in the list now, so whatever this device settled about them
/// is stale. Each pair once, in card order.
pub fn gone_prs(
    tasks: &[Task],
    repo: &str,
    open: &HashSet<i64>,
    known: &dyn Fn(&str, &str) -> Option<PrState>,
) -> (Vec<CardPr>, Vec<CardPr>) {
    let (mut gone, mut listed): (Vec<CardPr>, Vec<CardPr>) = (Vec::new(), Vec::new());
    for t in tasks {
        for l in &t.links {
            let Some((r, n)) = coord::pr_link(l) else {
                continue;
            };
            if !r.eq_ignore_ascii_case(repo) {
                continue;
            }
            let k = (t.id.clone(), l.reference.clone());
            if open.contains(&n) {
                if !listed.contains(&k) {
                    listed.push(k);
                }
                continue;
            }
            let settled = known(&t.id, &l.reference)
                .is_some_and(|p| p.state == PR_MERGED && !p.merge_sha.is_empty());
            if !settled && !gone.contains(&k) {
                gone.push(k);
            }
        }
    }
    (gone, listed)
}

/// How many failed answers about one PR the desk takes before it stops
/// asking about it until it shows in the list again or Irori restarts.
pub const MAX_ASK_FAILS: u32 = 3;

/// Whether a captured Bash command line runs `gh pr merge` (loosely: a
/// stray match only costs one extra desk poll).
pub fn is_pr_merge(command: &str) -> bool {
    if !capture::gitlink::is_commit_command(command) {
        return false;
    }
    capture::gitlink::split_commands(command).iter().any(|w| {
        let words: Vec<String> = w
            .iter()
            .map(|s| {
                s.trim_matches(|c| c == '"' || c == '\'')
                    .to_ascii_lowercase()
            })
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
    /// The `gh pr merge` commands heard, by (finished at, command); `None`
    /// before the first look, which only notes where things stand. A set,
    /// not the newest time: a row from a device with a slower clock, or
    /// one that synced late, is still news.
    heard: Mutex<Option<HashSet<(i64, String)>>>,
    /// (card, PR) pairs the desk stops asking about while this Irori runs:
    /// GitHub said merged or closed, the answer couldn't be written, or
    /// GitHub couldn't be asked [`MAX_ASK_FAILS`] times. A pair seen in
    /// the open list again is forgotten ([`Self::forget`]).
    settled: Mutex<HashSet<CardPr>>,
    fails: Mutex<std::collections::HashMap<CardPr, u32>>,
}

fn pair(task_id: &str, url: &str) -> CardPr {
    (task_id.to_string(), url.to_string())
}

impl MergeHeard {
    /// Whether `cmds` (command, finished at) hold a `gh pr merge` not seen
    /// at the last look. The first look answers false: the desks' own
    /// first poll covers what came before.
    pub(crate) fn news<'a>(&self, cmds: impl Iterator<Item = (&'a str, GoTime)>) -> bool {
        let now: HashSet<(i64, String)> = cmds
            .filter(|(c, _)| is_pr_merge(c))
            .map(|(c, at)| (at.unix_nano(), c.to_string()))
            .collect();
        let mut h = self.heard.lock().unwrap_or_else(|e| e.into_inner());
        let new = h
            .as_ref()
            .is_some_and(|was| now.iter().any(|k| !was.contains(k)));
        match h.as_mut() {
            Some(was) => was.extend(now),
            None => *h = Some(now),
        }
        new
    }

    pub(crate) fn settled(&self, task_id: &str, url: &str) -> bool {
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&pair(task_id, url))
    }

    pub(crate) fn settle(&self, task_id: &str, url: &str) {
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pair(task_id, url));
    }

    /// GitHub couldn't be asked about the pair: settled after
    /// [`MAX_ASK_FAILS`] in a row, so a link that is no PR isn't asked at
    /// every poll.
    pub(crate) fn failed(&self, task_id: &str, url: &str) {
        let k = pair(task_id, url);
        let mut f = self.fails.lock().unwrap_or_else(|e| e.into_inner());
        let n = f.entry(k.clone()).or_default();
        *n += 1;
        if *n >= MAX_ASK_FAILS {
            f.remove(&k);
            drop(f);
            self.settle(task_id, url);
        }
    }

    /// The pair is in the open list again (reopened): what was settled
    /// about it no longer holds.
    pub(crate) fn forget(&self, task_id: &str, url: &str) {
        let k = pair(task_id, url);
        self.settled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&k);
        self.fails
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&k);
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
        for c in [
            "gh pr view 12",
            "git commit -m merge",
            "gh pr list",
            "echo gh pr merge",
        ] {
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
        // Older than the newest seen (a slow clock, a late sync): still news.
        assert!(h.news([("gh pr merge 2", t(30)), ("gh pr merge 3", t(20))].into_iter()));
    }

    #[test]
    fn failures_settle_after_a_few_and_a_listed_pr_is_forgotten() {
        let h = MergeHeard::default();
        for _ in 1..MAX_ASK_FAILS {
            h.failed("c", "u");
        }
        assert!(!h.settled("c", "u"));
        h.failed("c", "u");
        assert!(h.settled("c", "u"));
        h.forget("c", "u");
        assert!(!h.settled("c", "u"));
    }
}
