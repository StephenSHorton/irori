//! The Core side of merge desk slice 7 (card #421): see
//! [`crate::deskmerge`].

use std::collections::HashSet;
use std::sync::Arc;

use super::Services;
use crate::{logf, notice};

impl Services {
    /// After a good desk poll: asks GitHub about each PR an open card
    /// links on this desk's repo that left the open list, and records what
    /// it says on that card, held here, on another device or by nobody.
    /// A list cut at the limit says nothing about what left it.
    pub(super) fn settle_gone_prs(self: &Arc<Self>, repo_key: &str) {
        let Some(checker) = self.pr_checker.clone().filter(|c| c.ready()) else {
            return;
        };
        let Some(desk) = self.coord.desk(repo_key) else {
            return;
        };
        let (Some(snap), Some(repo)) = (desk.prs.as_ref(), coord::github_repo(repo_key)) else {
            return;
        };
        if snap.truncated {
            return;
        }
        let open: HashSet<i64> = snap.prs.iter().map(|p| p.number).collect();
        let (tasks, home) = self.desk_cards(true);
        let known = |id: &str, url: &str| home.get(id).unwrap_or(&self.coord).pr_state(id, url);
        let (gone, listed) = crate::deskmerge::gone_prs(&tasks, &repo, &open, &known);
        // Back in the list (reopened): asked again once it leaves.
        for (id, url) in listed {
            self.desk_merges.forget(&id, &url);
        }
        for (id, url) in gone {
            if self.desk_merges.settled(&id, &url) {
                continue;
            }
            let c = checker.check(&url);
            // What the desk can't ask, or what is still open, is the
            // holder's poller's to say: nothing is written for it here.
            if !c.error.is_empty() || c.state.is_empty() {
                self.desk_merges.failed(&id, &url);
                continue;
            }
            if c.state == coord::PR_OPEN {
                continue;
            }
            let co = home.get(&id).cloned().unwrap_or_else(|| self.coord.clone());
            match co.record_pr_state(&id, c) {
                Ok((_, ns)) => {
                    // Merged or closed: asked again only if it is listed
                    // again (a reopen) or Irori restarts.
                    self.desk_merges.settle(&id, &url);
                    for n in ns {
                        if let Some(e) =
                            notice::notice_err(&self.route_notice(&inbox::system_sender(), &n))
                        {
                            logf(format_args!("desk: pr notice {}: {e}", n.reference));
                        }
                    }
                }
                Err(e) => {
                    // Said once: a log that won't take the row (a board's
                    // writer rules) would refuse it at every poll.
                    self.desk_merges.settle(&id, &url);
                    logf(format_args!("desk: {url} on {id}: {e}"));
                }
            }
        }
        // A shared board's holders on this device hear it here too.
        self.tell_pr_news();
        self.tell_missed_reviews();
    }

    /// When an agent on any device ran `gh pr merge` since the last look:
    /// asks GitHub about the open PRs this device's agents' cards link now
    /// (card #970: an agent that merged its PR completes its card without
    /// waiting on the next poll), and has the desks thread poll every desk
    /// this device polls, whatever its cadence. Every device looks, merge
    /// desk or not.
    pub(super) fn merge_heard(self: &Arc<Self>) {
        let cmds = self.capture.commit_cmds();
        if !self
            .desk_merges
            .news(cmds.iter().map(|c| (c.command.as_str(), c.at)))
        {
            return;
        }
        if self.pr_checker.is_some() {
            self.pr_watch.rush();
            self.check_prs(std::time::Instant::now());
        }
        if self.desk_lister.is_some() {
            self.desks_due
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Polls every desk this device polls now, whatever its cadence, when
    /// [`Self::merge_heard`] asked for it. Only the desks thread calls it,
    /// so a desk is never polled twice at once.
    pub(super) fn desks_heard_merge(self: &Arc<Self>) {
        if !self
            .desks_due
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        for d in self.polled_desks() {
            let k = d.settings.repo_key;
            // Not a counted poll: the pick waits for the next one.
            if let Err(e) = self.poll_desk(&k, false) {
                logf(format_args!("desk {k}: {e}"));
            }
        }
    }
}
