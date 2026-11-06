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
        let known = |id: &str, url: &str| {
            home.get(id)
                .unwrap_or(&self.coord)
                .pr_state(id, url)
        };
        for (id, url) in crate::deskmerge::gone_prs(&tasks, &repo, &open, &known) {
            if self.desk_merges.settled(&id, &url) {
                continue;
            }
            let c = checker.check(&url);
            // What the desk can't ask, or what is still open, is the
            // holder's poller's to say: nothing is written for it here.
            if !c.error.is_empty() || c.state.is_empty() || c.state == coord::PR_OPEN {
                continue;
            }
            let merged = c.state == coord::PR_MERGED;
            let co = home.get(&id).cloned().unwrap_or_else(|| self.coord.clone());
            match co.record_pr_state(&id, c) {
                Ok((_, ns)) => {
                    if merged {
                        self.desk_merges.settle(&id, &url);
                    }
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
    }

    /// Polls every desk this device polls now, whatever its cadence, when
    /// an agent on any device ran `gh pr merge` since the last look.
    pub(super) fn desk_merge_heard(self: &Arc<Self>) {
        if self.desk_lister.is_none() {
            return;
        }
        let cmds = self.capture.commit_cmds();
        if !self
            .desk_merges
            .news(cmds.iter().map(|c| (c.command.as_str(), c.at)))
        {
            return;
        }
        for d in self.polled_desks() {
            let k = d.settings.repo_key;
            if let Err(e) = self.poll_desk(&k) {
                logf(format_args!("desk {k}: {e}"));
            }
        }
    }
}
