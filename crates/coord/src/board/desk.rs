//! The merge desk's rows (docs/design/merge-desk.md §3, §7; card #415).
//!
//! A repo has a desk when the person writes an enabled `repo.desk` row
//! for it. One device, the row's `pollDevice`, asks GitHub for the repo's
//! open PRs (`gh pr list`, the Core's desk poller) and hands the list to
//! [`Fold::record_desk_prs`], which writes a `desk.prs` snapshot only when
//! the list changed. coord never shells out.
//!
//! v1 keeps every desk row in the person's private log (§10 question 5).
//! `repo.desk`, `desk.hold`, `desk.pause` and `desk.ok` are the person's
//! alone: the fold ignores them from anyone else, so an agent can never
//! change the check command, the holder or whom the desk trusts (§7).
//! `desk.prs` is Irori's; `desk.run` the desk holder's (slice 4).

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::error::{Result, invalid, not_allowed, not_found};
use super::fold::Fold;
use super::fold::REASON_DESK_HOLDER_CHANGED;
use super::resource::{check_repo_key, desk_resource};
use super::rows::{LeaseReleasedRow, row};
use super::util::{SWEEPER, caller};
use crate::gojson::omit;
use crate::names::trim_space;

/// The most PRs one poll asks for. A list of exactly this many is marked
/// truncated: GitHub may have more (§3).
pub const DESK_LIMIT: usize = 1000;

/// Merge methods the holder may pass to `gh pr merge`.
pub const MERGE_SQUASH: &str = "squash";
pub const MERGE_MERGE: &str = "merge";
pub const MERGE_REBASE: &str = "rebase";

crate::go_struct! {
    /// One check line (§7): `command` always runs, or only when the diff
    /// touches `paths`; `runner` (an agent) and `device` say where.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskCheck {
        pub command: String = "command",
        pub paths: Vec<String> = "paths" "omit",
        pub runner: String = "runner" "omit",
        pub device: String = "device" "omit",
    }
}

crate::go_struct! {
    /// `repo.desk`: the person's settings for one repo's desk.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct RepoDeskRow {
        pub repo_key: String = "repoKey",
        pub enabled: bool = "enabled",
        pub takes_prs: bool = "takesPrs",
        pub holder: String = "holder" "omit",
        pub poll_device: String = "pollDevice",
        pub trusted: Vec<String> = "trusted" "omit",
        pub checks: Vec<DeskCheck> = "checks" "omit",
        pub merge_method: String = "mergeMethod" "omit",
        pub per_pr_ok: bool = "perPrOk" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// One open PR as `gh pr list` gave it. `checks` sums up
    /// statusCheckRollup: "" (none), "pending", "failure" or "success".
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskPr {
        pub number: i64 = "number",
        pub url: String = "url",
        pub title: String = "title",
        pub author: String = "author" "omit",
        pub is_draft: bool = "isDraft" "omit",
        pub head_ref_name: String = "headRefName" "omit",
        pub head_ref_oid: String = "headRefOid" "omit",
        pub base_ref_name: String = "baseRefName" "omit",
        pub mergeable: String = "mergeable" "omit",
        pub merge_state_status: String = "mergeStateStatus" "omit",
        pub checks: String = "checks" "omit",
        pub created_at: GoTime = "createdAt",
        pub updated_at: GoTime = "updatedAt",
    }
}

crate::go_struct! {
    /// `desk.prs`: Irori's snapshot of the repo's open PRs.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskPrsRow {
        pub repo_key: String = "repoKey",
        pub truncated: bool = "truncated" "omit",
        pub prs: Vec<DeskPr> = "prs",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// `desk.hold`: the person holds PR `pr` ("not this one yet");
    /// `cleared` lifts it.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskHoldRow {
        pub repo_key: String = "repoKey",
        pub pr: i64 = "pr",
        pub note: String = "note" "omit",
        pub cleared: bool = "cleared" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// `desk.pause`: the holder takes no lease while paused.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskPauseRow {
        pub repo_key: String = "repoKey",
        pub paused: bool = "paused",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// `desk.ok`: the person's OK on an Outside PR at one head (§5).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskOkRow {
        pub repo_key: String = "repoKey",
        pub pr: i64 = "pr",
        pub head_sha: String = "headSha",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// `desk.run`: the holder's merge run under the desk lease (slice 4).
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskRunRow {
        pub repo_key: String = "repoKey",
        pub pr: i64 = "pr",
        pub head_sha: String = "headSha",
        pub base_sha: String = "baseSha" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

/// One repo's desk as folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Desk {
    /// The person's latest `repo.desk` row.
    pub settings: RepoDeskRow,
    /// The latest snapshot; `None` before the first poll.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prs: Option<DeskPrsRow>,
    /// Holds by PR number (cleared ones dropped).
    pub holds: BTreeMap<i64, DeskHoldRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pause: Option<DeskPauseRow>,
    /// The person's OKs on Outside PRs, by PR number (latest wins).
    pub oks: BTreeMap<i64, DeskOkRow>,
    /// The latest run row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<DeskRunRow>,
}

impl Desk {
    /// Whether the desk is on: enabled and the repo takes PRs.
    pub fn on(&self) -> bool {
        self.settings.enabled && self.settings.takes_prs
    }
}

/// GitHub's `owner/name` for a `remote:github.com/owner/name` repo key,
/// or `None` for any other repo (the desk only knows GitHub).
pub fn github_repo(repo_key: &str) -> Option<String> {
    let rest = repo_key.strip_prefix("remote:")?;
    let rest = rest
        .strip_prefix("github.com/")
        .or_else(|| rest.strip_prefix("www.github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// The settings a new desk starts from; the person changes them in the
/// app. The holder's device polls unless another is named.
pub fn new_desk(repo_key: &str, poll_device: &str) -> RepoDeskRow {
    RepoDeskRow {
        repo_key: repo_key.into(),
        enabled: true,
        takes_prs: true,
        poll_device: poll_device.into(),
        merge_method: MERGE_SQUASH.into(),
        ..RepoDeskRow::default()
    }
}

impl Fold {
    pub(crate) fn fold_desk(&mut self, ev: &Event) {
        // The person's rows count only from the person (module doc).
        let person = |f: &Fold, by: &str| !f.on_board && f.is_owner(by);
        match ev.kind.as_str() {
            kinds::REPO_DESK => {
                let Some(p) = row::<RepoDeskRow>(&ev.payload) else {
                    return;
                };
                if !person(self, &p.by) {
                    return;
                }
                let k = p.repo_key.clone();
                self.desks.entry(k).or_default().settings = p;
            }
            kinds::DESK_PRS => {
                let Some(p) = row::<DeskPrsRow>(&ev.payload) else {
                    return;
                };
                if p.by != SWEEPER {
                    return;
                }
                let k = p.repo_key.clone();
                self.desks.entry(k).or_default().prs = Some(p);
            }
            kinds::DESK_HOLD => {
                let Some(p) = row::<DeskHoldRow>(&ev.payload) else {
                    return;
                };
                if !person(self, &p.by) {
                    return;
                }
                let d = self.desks.entry(p.repo_key.clone()).or_default();
                if p.cleared {
                    d.holds.remove(&p.pr);
                } else {
                    d.holds.insert(p.pr, p);
                }
            }
            kinds::DESK_PAUSE => {
                let Some(p) = row::<DeskPauseRow>(&ev.payload) else {
                    return;
                };
                if !person(self, &p.by) {
                    return;
                }
                let k = p.repo_key.clone();
                self.desks.entry(k).or_default().pause = Some(p);
            }
            kinds::DESK_OK => {
                let Some(p) = row::<DeskOkRow>(&ev.payload) else {
                    return;
                };
                if !person(self, &p.by) {
                    return;
                }
                self.desks
                    .entry(p.repo_key.clone())
                    .or_default()
                    .oks
                    .insert(p.pr, p);
            }
            kinds::DESK_RUN => {
                let Some(p) = row::<DeskRunRow>(&ev.payload) else {
                    return;
                };
                // Only the desk holder's run counts (slice 4); its lease is
                // checked when the run is read ([`Fold::desk_view`]).
                let k = p.repo_key.clone();
                let d = self.desks.entry(k).or_default();
                if !d.settings.holder.is_empty() && p.by == d.settings.holder {
                    d.run = Some(p);
                }
            }
            _ => {}
        }
    }

    /// The person sets up (or changes) a repo's desk.
    pub(crate) fn set_desk(&mut self, by: &str, mut d: RepoDeskRow) -> Result<()> {
        let by = caller(by)?;
        if self.on_board {
            return Err(invalid(
                "a merge desk lives in your own log, not a shared board's",
            ));
        }
        if !self.is_owner(&by) {
            return Err(not_allowed(
                "only the person sets up a merge desk, in the app",
            ));
        }
        d.repo_key = trim_space(&d.repo_key).to_string();
        check_repo_key(&d.repo_key)?;
        if github_repo(&d.repo_key).is_none() {
            return Err(invalid(format!(
                "{} isn't a GitHub repo: the merge desk reads PRs from GitHub",
                d.repo_key
            )));
        }
        d.poll_device = trim_space(&d.poll_device).to_string();
        if d.enabled && d.poll_device.is_empty() {
            return Err(invalid("a desk needs a device that polls GitHub"));
        }
        if d.merge_method.is_empty() {
            d.merge_method = MERGE_SQUASH.into();
        }
        if ![MERGE_SQUASH, MERGE_MERGE, MERGE_REBASE].contains(&d.merge_method.as_str()) {
            return Err(invalid(format!(
                "merge method {} isn't squash, merge or rebase",
                d.merge_method
            )));
        }
        for c in &d.checks {
            if trim_space(&c.command).is_empty() {
                return Err(invalid("a check line needs a command"));
            }
        }
        d.holder = trim_space(&d.holder).to_string();
        let old = self
            .desks
            .get(&d.repo_key)
            .map(|x| x.settings.holder.clone())
            .unwrap_or_default();
        d.by = by.clone();
        d.at = self.stamp();
        let (repo_key, holder) = (d.repo_key.clone(), d.holder.clone());
        self.write(&by, kinds::REPO_DESK, &d)?;
        if old != holder {
            self.release_desk_lease(&repo_key, &holder)?;
        }
        Ok(())
    }

    /// The desk lease name for a desk's repo, `None` off GitHub.
    pub(crate) fn desk_lease_of(repo_key: &str) -> Option<String> {
        github_repo(repo_key).map(|r| desk_resource(&r))
    }

    /// The desk whose lease is `name`.
    fn desk_by_lease(&self, name: &str) -> Option<&Desk> {
        self.desks
            .values()
            .find(|d| Fold::desk_lease_of(&d.settings.repo_key).as_deref() == Some(name))
    }

    /// A merge desk lease is the desk holder's alone, taken while the desk
    /// is on and not paused, and never stolen (merge-desk.md §5).
    pub(crate) fn desk_may_take(&self, holder: &str, name: &str, steal: bool) -> Result<()> {
        if steal {
            return Err(not_allowed(format!(
                "{name} is a merge desk's lease and can't be stolen: only the desk holder takes it, one PR at a time"
            )));
        }
        let Some(d) = self.desk_by_lease(name) else {
            return Err(not_allowed(format!(
                "{name} is a merge desk lease, and no merge desk is set up for that repo: the person sets one up in the app"
            )));
        };
        if !d.on() {
            return Err(not_allowed(format!(
                "{name}: the merge desk is off (the person turns it on in Merge desk settings)"
            )));
        }
        if d.pause.as_ref().is_some_and(|p| p.paused) {
            return Err(not_allowed(format!(
                "{name}: the merge desk is paused; the holder takes no lease until the person resumes it"
            )));
        }
        match d.settings.holder.as_str() {
            "" => Err(not_allowed(format!(
                "{name}: the merge desk is unheld; the person names a holder in Merge desk settings"
            ))),
            h if h == holder => Ok(()),
            h => Err(not_allowed(format!(
                "{name} is the merge desk's lease and {} is its holder: only the holder takes it",
                self.named(h)
            ))),
        }
    }

    /// Releases a desk's live lease held by anyone but `keep` (the holder
    /// changed: the old holder's run ends).
    fn release_desk_lease(&mut self, repo_key: &str, keep: &str) -> Result<()> {
        let Some(name) = Fold::desk_lease_of(repo_key) else {
            return Ok(());
        };
        let now = self.stamp();
        let Some(l) = self
            .held
            .get(&name)
            .filter(|l| self.live(l, &now) && l.lease.holder != keep)
            .cloned()
        else {
            return Ok(());
        };
        if !self.lease_local(&l) {
            return self.steal(SWEEPER, &l.lease);
        }
        self.write(
            SWEEPER,
            kinds::LEASE_RELEASED,
            &LeaseReleasedRow {
                resource: name,
                holder: l.lease.holder,
                reason: REASON_DESK_HOLDER_CHANGED.into(),
                by: SWEEPER.into(),
                at: now,
            },
        )
    }

    /// The holder records the PR it runs under the desk lease (`desk.run`):
    /// it must hold the desk's lease now.
    pub(crate) fn record_desk_run(
        &mut self,
        by: &str,
        repo_key: &str,
        pr: i64,
        head_sha: &str,
        base_sha: &str,
    ) -> Result<()> {
        let by = caller(by)?;
        let Some(name) = Fold::desk_lease_of(repo_key) else {
            return Err(invalid(format!("{repo_key} isn't a GitHub repo")));
        };
        let now = self.stamp();
        if !self
            .held
            .get(&name)
            .is_some_and(|l| self.live(l, &now) && l.lease.holder == by)
        {
            return Err(not_found(format!(
                "take the lease {name} first: a merge run is recorded under it"
            )));
        }
        if pr <= 0 {
            return Err(invalid("pr is the PR's number"));
        }
        let head_sha = trim_space(head_sha);
        if head_sha.is_empty() {
            return Err(invalid("give head_sha: the PR head the run checks"));
        }
        self.write(
            &by,
            kinds::DESK_RUN,
            &DeskRunRow {
                repo_key: repo_key.into(),
                pr,
                head_sha: head_sha.into(),
                base_sha: trim_space(base_sha).into(),
                by: by.clone(),
                at: now,
            },
        )
    }

    /// A desk as the app and tools read it: its run only while the run's
    /// agent still holds the desk lease (Merging ends with the lease).
    pub(crate) fn desk_view(&self, d: &Desk) -> Desk {
        let mut d = d.clone();
        let now = self.stamp();
        let live = |run: &DeskRunRow| {
            Fold::desk_lease_of(&d.settings.repo_key).is_some_and(|name| {
                self.held
                    .get(&name)
                    .is_some_and(|l| self.live(l, &now) && l.lease.holder == run.by)
            })
        };
        if d.run.as_ref().is_some_and(|r| !live(r)) {
            d.run = None;
        }
        d
    }

    /// Records the open PRs the poller heard for `repo_key`: writes a
    /// `desk.prs` snapshot only when the list or a field in it changed.
    /// Returns whether a row was written.
    pub(crate) fn record_desk_prs(
        &mut self,
        repo_key: &str,
        mut prs: Vec<DeskPr>,
        truncated: bool,
    ) -> Result<bool> {
        if !self.desks.get(repo_key).is_some_and(|d| d.on()) {
            return Err(invalid(format!("{repo_key} has no merge desk")));
        }
        prs.sort_by_key(|p| p.number);
        let unchanged = self
            .desks
            .get(repo_key)
            .and_then(|d| d.prs.as_ref())
            .is_some_and(|old| old.prs == prs && old.truncated == truncated);
        if unchanged {
            return Ok(false);
        }
        let at = self.stamp();
        self.write_as(
            SWEEPER,
            kinds::DESK_PRS,
            &DeskPrsRow {
                repo_key: repo_key.into(),
                truncated,
                prs,
                by: SWEEPER.into(),
                at,
            },
        )?;
        Ok(true)
    }
}
