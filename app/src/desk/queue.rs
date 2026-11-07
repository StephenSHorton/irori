//! Merge desk logic (card #417, docs/design/merge-desk.md §3, §7): the
//! React app's lib/desk.ts without its sample and its React store. The
//! list of open PRs each desk shows for review, worked out from the
//! poller's snapshot. The desk never merges (card #418): each agent asks
//! the person "Can I merge PR #N?" on its own card. The window's
//! snapshot holds only our PRs (card #625).

use irori_window::desk::{DeskAuthorView, DeskView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskState {
    Draft,
    Rebase,
    Running,
    Failed,
    Ready,
    Merged,
}

pub fn state_label(s: DeskState) -> &'static str {
    match s {
        DeskState::Draft => "Draft",
        DeskState::Rebase => "Needs rebase",
        DeskState::Running => "Checks running",
        DeskState::Failed => "Checks failed",
        DeskState::Ready => "Ready for review",
        DeskState::Merged => "Merged",
    }
}

/// Where a PR's thread is (slice 2): the author card, and the agent behind
/// it when the author is one of the person's own agents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrAuthor {
    pub card: String,
    pub card_ref: String,
    pub agent: String,
}

/// A merged PR (the window doesn't send these yet; lib/desk.ts's fromHost
/// left them out too).
#[derive(Clone, Debug, Default)]
pub struct MergedPr {
    pub pr: i64,
    pub title: String,
    pub sha: String,
    pub card: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueLine {
    pub pr: i64,
    pub url: String,
    pub title: String,
    pub login: String,
    pub author: PrAuthor,
    pub state: DeskState,
    /// One short line under the title: why it is in this state.
    pub note: String,
    pub head_sha: String,
    /// Milliseconds, for the order.
    pub created: i64,
}

/// What [`desk_queue`] needs from one desk.
#[derive(Clone, Debug, Default)]
pub struct DeskInputs<'a> {
    pub desk: Option<&'a DeskView>,
    pub merged: Vec<MergedPr>,
}

pub fn inputs_of(d: &DeskView) -> DeskInputs<'_> {
    DeskInputs {
        desk: Some(d),
        merged: Vec::new(),
    }
}

fn author_of(authors: &[DeskAuthorView], pr: i64) -> PrAuthor {
    authors
        .iter()
        .find(|a| a.pr == pr)
        .map(|a| PrAuthor {
            card: a.card.clone(),
            card_ref: a.card_ref.clone(),
            agent: a.agent.clone(),
        })
        .unwrap_or_default()
}

pub fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// The list's lines in order: Ready for review oldest PR first, then the
/// rest (also oldest first), then any just-merged PRs (§3).
pub fn desk_queue(inp: &DeskInputs) -> Vec<QueueLine> {
    let mut lines = Vec::new();
    let Some(d) = inp.desk else { return lines };
    let prs = d.prs.as_ref().map(|s| s.prs.as_slice()).unwrap_or(&[]);
    for p in prs {
        let author = author_of(&d.authors, p.number);
        let (state, note) = if p.is_draft {
            (DeskState::Draft, "draft · listed, never queued".to_string())
        } else if p.merge_state_status == "BEHIND" || p.merge_state_status == "DIRTY" || p.mergeable == "CONFLICTING" {
            let why = if p.mergeable == "CONFLICTING" { "CONFLICTING" } else { p.merge_state_status.as_str() };
            (DeskState::Rebase, format!("GitHub: {why}"))
        } else if p.checks == "pending" {
            (DeskState::Running, "GitHub checks running".into())
        } else if p.checks == "failure" {
            (DeskState::Failed, "GitHub checks failed".into())
        } else if !author.card.is_empty() {
            (DeskState::Ready, "ready for review".into())
        } else {
            (DeskState::Ready, "no author card yet".into())
        };
        lines.push(QueueLine {
            pr: p.number,
            url: p.url.clone(),
            title: p.title.clone(),
            login: p.author.clone(),
            author,
            state,
            note,
            head_sha: p.head_ref_oid.clone(),
            created: p.created_at.unix_milli_value(),
        });
    }
    let open: Vec<i64> = lines.iter().map(|l| l.pr).collect();
    let rank = |s: DeskState| if s == DeskState::Ready { 0 } else { 1 };
    lines.sort_by(|a, b| {
        rank(a.state)
            .cmp(&rank(b.state))
            .then(a.created.cmp(&b.created))
            .then(a.pr.cmp(&b.pr))
    });
    for m in &inp.merged {
        if open.contains(&m.pr) {
            continue;
        }
        lines.push(QueueLine {
            pr: m.pr,
            url: String::new(),
            title: m.title.clone(),
            login: String::new(),
            author: PrAuthor {
                card: m.card.clone(),
                ..PrAuthor::default()
            },
            state: DeskState::Merged,
            note: format!("merged as {}", short(&m.sha)),
            head_sha: m.sha.clone(),
            created: 0,
        });
    }
    lines
}

/// "Owner/Name" from a repo key, or the key itself.
pub fn owner_name(repo_key: &str) -> String {
    let Some(i) = repo_key.find("github.com") else {
        return repo_key.to_string();
    };
    let rest = &repo_key[i + "github.com".len()..];
    let Some(rest) = rest.strip_prefix('/').or_else(|| rest.strip_prefix(':')) else {
        return repo_key.to_string();
    };
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty()) {
        rest.to_string()
    } else {
        repo_key.to_string()
    }
}

/// The panel head's count: "n of ours open", or what the panel shows when
/// the list is cut short or can't be read.
pub fn queue_count(d: &DeskView) -> String {
    let Some(snap) = &d.prs else {
        return "not polled yet".into();
    };
    let n = snap.prs.len();
    if snap.truncated {
        format!("showing {n} of ours; GitHub has more")
    } else {
        format!("{n} of ours open")
    }
}

/// Enabled, and the repo takes PRs.
pub fn desk_on(d: &DeskView) -> bool {
    d.settings.enabled && d.settings.takes_prs
}

/// On, with an open PR of ours (card #625). The page lists only these; a
/// desk comes back once one of ours opens.
pub fn desk_shown(d: &DeskView) -> bool {
    desk_on(d) && d.prs.as_ref().is_some_and(|s| !s.prs.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use irori_window::desk::{DeskPrView, DeskSettings, DeskSnapshotView};
    use irori_window::rune_inbox::GoTime;

    const REPO: &str = "remote:github.com/StephenSHorton/Game-Repo";

    fn settings() -> DeskSettings {
        DeskSettings {
            repo_key: REPO.into(),
            enabled: true,
            takes_prs: true,
            poll_device: "pc".into(),
            trusted: vec!["StephenSHorton".into()],
            ..DeskSettings::default()
        }
    }

    fn at(s: &str) -> GoTime {
        GoTime::parse_rfc3339(s).unwrap()
    }

    fn pr(n: i64, f: impl FnOnce(&mut DeskPrView)) -> DeskPrView {
        let mut p = DeskPrView {
            number: n,
            url: format!("https://github.com/x/y/pull/{n}"),
            title: format!("PR {n}"),
            author: "some-bot".into(),
            head_ref_oid: format!("sha{n}aaaaaaa"),
            base_ref_name: "main".into(),
            mergeable: "MERGEABLE".into(),
            merge_state_status: "CLEAN".into(),
            created_at: at(&format!("2026-11-03T10:{:02}:00Z", n % 60)),
            updated_at: at("2026-11-03T12:00:00Z"),
            ..DeskPrView::default()
        };
        f(&mut p);
        p
    }

    fn desk(prs: Vec<DeskPrView>, mine: bool) -> DeskView {
        let authors = if mine {
            prs.iter()
                .map(|p| DeskAuthorView {
                    pr: p.number,
                    card: "c".into(),
                    card_ref: String::new(),
                    agent: "agent:a".into(),
                })
                .collect()
        } else {
            Vec::new()
        };
        DeskView {
            settings: settings(),
            prs: Some(DeskSnapshotView {
                prs,
                ..DeskSnapshotView::default()
            }),
            authors,
            ..DeskView::default()
        }
    }

    fn one(p: DeskPrView) -> QueueLine {
        let d = desk(vec![p], true);
        desk_queue(&inputs_of(&d)).remove(0)
    }

    #[test]
    fn one_with_no_author_entry_is_queued_as_any_other() {
        let d = desk(vec![pr(101, |p| p.author = "stranger".into())], false);
        let l = desk_queue(&inputs_of(&d)).remove(0);
        assert_eq!(l.state, DeskState::Ready);
        assert_eq!(l.note, "no author card yet");
        assert_eq!(l.author, PrAuthor::default());
    }

    #[test]
    fn draft_is_listed_never_queued() {
        assert_eq!(one(pr(102, |p| p.is_draft = true)).state, DeskState::Draft);
    }

    #[test]
    fn needs_rebase_behind_dirty_or_conflicting() {
        assert_eq!(one(pr(103, |p| p.merge_state_status = "BEHIND".into())).state, DeskState::Rebase);
        assert_eq!(one(pr(104, |p| p.merge_state_status = "DIRTY".into())).state, DeskState::Rebase);
        let c = one(pr(105, |p| p.mergeable = "CONFLICTING".into()));
        assert_eq!(c.state, DeskState::Rebase);
        assert_eq!(c.note, "GitHub: CONFLICTING");
    }

    #[test]
    fn checks_running_and_failed_come_from_githubs_rollup() {
        assert_eq!(one(pr(106, |p| p.checks = "pending".into())).state, DeskState::Running);
        assert_eq!(one(pr(107, |p| p.checks = "failure".into())).state, DeskState::Failed);
    }

    #[test]
    fn ready_for_review_when_none_of_the_above() {
        let l = one(pr(108, |p| p.checks = "success".into()));
        assert_eq!(l.state, DeskState::Ready);
        assert_eq!(l.note, "ready for review");
    }

    #[test]
    fn merged_a_pr_no_longer_in_the_open_list() {
        let p = pr(109, |_| {});
        let d = desk(vec![p], true);
        let mut inp = inputs_of(&d);
        inp.merged = vec![
            MergedPr {
                pr: 1,
                title: "old".into(),
                sha: "3f2a9c1deadbeef".into(),
                ..MergedPr::default()
            },
            MergedPr {
                pr: 109,
                title: "still open".into(),
                sha: "x".into(),
                ..MergedPr::default()
            },
        ];
        let lines = desk_queue(&inp);
        assert_eq!(lines.iter().map(|l| l.state).collect::<Vec<_>>(), [DeskState::Ready, DeskState::Merged]);
        assert_eq!(lines[1].note, "merged as 3f2a9c1");
    }

    #[test]
    fn ready_for_review_oldest_first_then_the_rest() {
        let newer = pr(110, |p| p.created_at = at("2026-11-03T12:00:00Z"));
        let older = pr(111, |p| p.created_at = at("2026-11-03T09:00:00Z"));
        let failed = pr(112, |p| {
            p.created_at = at("2026-11-03T08:00:00Z");
            p.checks = "failure".into();
        });
        let d = desk(vec![newer, failed, older], true);
        let order: Vec<i64> = desk_queue(&inputs_of(&d)).iter().map(|l| l.pr).collect();
        assert_eq!(order, [111, 110, 112]);
    }

    #[test]
    fn the_count_says_when_github_has_more_and_never_reads_empty_before_a_poll() {
        let mut d = desk(vec![], true);
        assert_eq!(queue_count(&d), "0 of ours open");
        d.prs = None;
        assert_eq!(queue_count(&d), "not polled yet");
        let mut d = desk(vec![pr(113, |_| {})], true);
        d.prs.as_mut().unwrap().truncated = true;
        assert_eq!(queue_count(&d), "showing 1 of ours; GitHub has more");
    }

    #[test]
    fn a_desk_with_no_prs_queues_nothing() {
        let d = desk(vec![], false);
        assert!(desk_queue(&inputs_of(&d)).is_empty());
    }

    #[test]
    fn which_desks_are_listed() {
        assert!(desk_shown(&desk(vec![pr(114, |_| {})], false)));
        assert!(!desk_shown(&desk(vec![], false)));
        let mut none = desk(vec![], false);
        none.prs = None;
        assert!(!desk_shown(&none));
        let mut off = desk(vec![pr(115, |_| {})], false);
        off.settings.enabled = false;
        assert!(!desk_shown(&off));
        let mut no_prs = desk(vec![pr(116, |_| {})], false);
        no_prs.settings.takes_prs = false;
        assert!(!desk_shown(&no_prs));
    }

    #[test]
    fn owner_names() {
        assert_eq!(owner_name(REPO), "StephenSHorton/Game-Repo");
        assert_eq!(owner_name("remote:github.com/o/r.git"), "o/r");
        assert_eq!(owner_name("git@github.com:o/r.git"), "o/r");
        assert_eq!(owner_name("root:abc"), "root:abc");
    }
}
