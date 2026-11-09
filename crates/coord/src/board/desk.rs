//! The merge desk's rows (docs/design/merge-desk.md §3, §7; card #415).
//!
//! Every GitHub repo an agent works in has a desk by itself (card #521):
//! a `repo.seen` row for a `remote:github.com/...` key gives it default
//! settings (on, polled from the device whose `repo.seen` is the latest).
//! The person's `repo.desk` row, once written, replaces the default: it
//! turns the desk off or picks the poll device. Only the person's own
//! repos count (card #625): the Core tells the fold which those are
//! ([`Fold::desk_repos`]), and a desk on any other repo is stray: off,
//! whatever its settings say. The desk never merges (card #418): it lists
//! PRs for review, and each agent asks the person "Can I merge PR #N?" on
//! its own card. One device, `pollDevice`, asks GitHub for the repo's
//! open PRs (`gh pr list`, the Core's desk poller) and hands the list to
//! [`Fold::record_desk_prs`], which writes a `desk.prs` snapshot only when
//! the list changed. coord never shells out.
//!
//! v1 keeps every desk row in the person's private log (§10 question 5).
//! `repo.desk` and `desk.ok` are the person's alone: the fold ignores them
//! from anyone else (§7). `desk.ok` is legacy: nothing writes it since
//! card #625, and old rows still fold. `desk.prs` is Irori's.
//!
//! Older builds had a desk holder that merged under a desk lease, with
//! check lines, a merge method, holds and a pause. Their rows still
//! decode: `repo.desk`'s `holder`, `checks` and `mergeMethod` keys are
//! dropped, and `desk.hold`, `desk.pause` and `desk.run` are folded past.

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::error::{Result, invalid, not_allowed};
use super::fold::Fold;
use super::resource::check_repo_key;
use super::rows::row;
use super::types::{Change, DESK_CHANGED};
use super::util::{SWEEPER, caller};
use crate::gojson::omit;
use crate::names::trim_space;

/// The most PRs one poll asks for. A list of exactly this many is marked
/// truncated: GitHub may have more (§3).
pub const DESK_LIMIT: usize = 1000;

crate::go_struct! {
    /// `repo.desk`: the person's settings for one repo's desk.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct RepoDeskRow {
        pub repo_key: String = "repoKey",
        pub enabled: bool = "enabled",
        pub takes_prs: bool = "takesPrs",
        pub poll_device: String = "pollDevice",
        pub trusted: Vec<String> = "trusted" "omit",
        /// Kept so rows round-trip; nothing reads it (§10 question 12).
        pub per_pr_ok: bool = "perPrOk" "omit",
        /// Other people's agents don't review this repo's PRs (merge desk
        /// §12.2, card #1077): off, the repo behaves like a solo one, with
        /// no PR card and today's private "PR #n" card. On when unset
        /// (decision 52); older devices ignore it.
        pub reviews_off: bool = "reviewsOff" "omit",
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
        /// GitHub's `reviewDecision` (APPROVED, CHANGES_REQUESTED,
        /// REVIEW_REQUIRED), "" in a repo with no required review (merge
        /// desk §12.2, card #1077).
        pub review_decision: String = "reviewDecision" "omit",
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
        /// The GitHub login the polling device's gh is signed in as: the
        /// person's own login, which the desk trusts (card #521).
        pub viewer: String = "viewer" "omit",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

crate::go_struct! {
    /// `repo.seen`, as capture writes it: a repo an agent works in.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct RepoSeenRow {
        pub repo_key: String = "repoKey",
    }
}

crate::go_struct! {
    /// `desk.ok`: the person's OK on an Outside PR at one head. Legacy:
    /// card #625 removed Outside PR and its OK, so nothing writes it; old
    /// rows still decode and fold.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct DeskOkRow {
        pub repo_key: String = "repoKey",
        pub pr: i64 = "pr",
        pub head_sha: String = "headSha",
        pub by: String = "by",
        pub at: GoTime = "at",
    }
}

/// One repo's desk as folded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Desk {
    /// The person's latest `repo.desk` row; before they write one, the
    /// default a GitHub repo an agent works in gets (card #521): on,
    /// polled from the device that saw the repo last. `by` is
    /// empty while it is the default.
    pub settings: RepoDeskRow,
    /// The device whose `repo.seen` for this repo is the latest, among
    /// devices not revoked; "" none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub seen_on: String,
    /// Each device's latest `repo.seen` for this repo. The latest wins by
    /// the log's own order, (hlc, device), so every device and a replay
    /// pick the same one whatever order the rows arrive in.
    #[serde(skip)]
    pub(crate) seen: BTreeMap<String, hlc::Timestamp>,
    /// The person's row left the poll device blank: it follows `seen_on`.
    #[serde(skip)]
    pub follow_poll: bool,
    /// The latest snapshot; `None` before the first poll.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prs: Option<DeskPrsRow>,
    /// Old `desk.ok` rows by PR number (latest wins). Nothing reads them
    /// since card #625 removed Outside PR; folded so old logs replay.
    pub oks: BTreeMap<i64, DeskOkRow>,
    /// Not one of the person's repos (card #625): nobody in their spaces
    /// opens PRs there (a cloned upstream repo). A stray desk is off
    /// whatever its settings say: never polled or listed. Set on
    /// read from what the Core told [`Fold::desk_repos`]; no row says it.
    #[serde(skip)]
    pub stray: bool,
}

impl Desk {
    /// Whether the desk is on: enabled, the repo takes PRs, and it is one
    /// of the person's repos (not [`Desk::stray`]).
    pub fn on(&self) -> bool {
        self.settings.enabled && self.settings.takes_prs && !self.stray
    }

    /// Whether the settings are the default, not a row the person wrote.
    pub fn is_default(&self) -> bool {
        self.settings.by.is_empty()
    }

    /// Fills the settings from where the repo was seen: the whole default
    /// until the person writes a row, then only a poll device the row left
    /// blank. A poll device the person picked is never moved.
    /// `revoked` says whether a device was revoked: its rows never name
    /// the poll device, so a desk moves off a device that is gone.
    fn settle(&mut self, repo_key: &str, revoked: &dyn Fn(&str) -> bool) {
        self.seen_on = self
            .seen
            .iter()
            .filter(|(dev, _)| !revoked(dev))
            .max_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)))
            .map(|(dev, _)| dev.clone())
            .unwrap_or_default();
        if self.seen.is_empty() {
            return;
        }
        if self.is_default() {
            self.settings = new_desk(repo_key, &self.seen_on);
        } else if self.follow_poll {
            self.settings.poll_device = self.seen_on.clone();
        }
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
/// app (card #521).
pub fn new_desk(repo_key: &str, poll_device: &str) -> RepoDeskRow {
    RepoDeskRow {
        repo_key: repo_key.into(),
        enabled: true,
        takes_prs: true,
        poll_device: poll_device.into(),
        ..RepoDeskRow::default()
    }
}

impl Fold {
    /// Desk `repo_key` changed what the desk list shows (list-signals.md
    /// §3.2); empty: every desk.
    pub(crate) fn note_desk(&mut self, repo_key: &str) {
        self.note(Change {
            kind: DESK_CHANGED.into(),
            resource: repo_key.into(),
            ..Change::default()
        });
    }

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
                let k = ledger::canonical_repo_key(&p.repo_key);
                let mut p = p;
                p.repo_key = k.clone();
                let revoked = self.revoked_fn();
                let d = self.desks.entry(k.clone()).or_default();
                d.follow_poll = trim_space(&p.poll_device).is_empty();
                d.settings = p;
                d.settle(&k, &revoked);
                self.note_desk(&k);
            }
            kinds::REPO_SEEN => {
                // Every GitHub repo an agent works in has a desk, on by
                // default, polled from the device that saw it last (card
                // #521); it stays off unless it is one of the person's
                // repos (#625, desk_ours). Desks live in the person's own
                // log, never a board's.
                if self.on_board {
                    return;
                }
                let Some(p) = row::<RepoSeenRow>(&ev.payload) else {
                    return;
                };
                let k = ledger::canonical_repo_key(&p.repo_key);
                if github_repo(&k).is_none() {
                    return;
                }
                let revoked = self.revoked_fn();
                let shown = |f: &Fold| {
                    f.desks
                        .get(&k)
                        .map(|d| (d.settings.clone(), d.seen_on.clone()))
                };
                let before = shown(self);
                let d = self.desks.entry(k.clone()).or_default();
                let at = d.seen.entry(ev.device_id.clone()).or_default();
                // Folding a row twice (a resync re-reads them) changes nothing.
                *at = (*at).max(ev.hlc);
                d.settle(&k, &revoked);
                if shown(self) != before {
                    self.note_desk(&k);
                }
            }
            kinds::DESK_PRS => {
                let Some(p) = row::<DeskPrsRow>(&ev.payload) else {
                    return;
                };
                if p.by != SWEEPER {
                    return;
                }
                let k = ledger::canonical_repo_key(&p.repo_key);
                self.desks.entry(k.clone()).or_default().prs = Some(p);
                self.note_desk(&k);
            }
            kinds::DESK_OK => {
                let Some(p) = row::<DeskOkRow>(&ev.payload) else {
                    return;
                };
                if !person(self, &p.by) {
                    return;
                }
                self.desks
                    .entry(ledger::canonical_repo_key(&p.repo_key))
                    .or_default()
                    .oks
                    .insert(p.pr, p);
            }
            _ => {}
        }
    }

    /// The person sets up (or changes) a repo's desk.
    pub(crate) fn set_desk(&mut self, by: &str, mut d: RepoDeskRow) -> Result<()> {
        let by = caller(by)?;
        if self.on_board {
            return Err(invalid(
                "a merge desk lives in your own log, not a shared space's",
            ));
        }
        if !self.is_owner(&by) {
            return Err(not_allowed(
                "only the person sets up a merge desk, in the app",
            ));
        }
        d.repo_key = ledger::canonical_repo_key(&d.repo_key);
        check_repo_key(&d.repo_key)?;
        if github_repo(&d.repo_key).is_none() {
            return Err(invalid(format!(
                "{} isn't a GitHub repo: the merge desk reads PRs from GitHub",
                ledger::repo_name(&d.repo_key)
            )));
        }
        d.poll_device = trim_space(&d.poll_device).to_string();
        // A blank poll device follows the device that saw the repo last
        // (card #521); a repo no device has seen needs one named.
        let seen = self
            .desks
            .get(&d.repo_key)
            .is_some_and(|x| !x.seen_on.is_empty());
        if d.enabled && d.poll_device.is_empty() && !seen {
            return Err(invalid("a desk needs a device that polls GitHub"));
        }
        d.by = by.clone();
        d.at = self.stamp();
        self.write(&by, kinds::REPO_DESK, &d)
    }

    /// Whether `repo_key` is one of the person's repos, as the Core last
    /// said (card #625); every repo until it has said.
    pub(crate) fn desk_ours(&self, repo_key: &str) -> bool {
        let k = ledger::canonical_repo_key(repo_key);
        self.desk_repos
            .as_ref()
            .is_none_or(|r| r.contains(&k) || !self.desk_judged.contains(&k))
    }

    /// Whether `repo_key` has a desk that is on and one of the person's.
    pub(crate) fn desk_on(&self, repo_key: &str) -> bool {
        self.desks.get(repo_key).is_some_and(|d| d.on()) && self.desk_ours(repo_key)
    }

    /// A desk as the app and tools read it: stray when it isn't one of the
    /// person's repos.
    pub(crate) fn desk_view(&self, d: &Desk) -> Desk {
        let mut d = d.clone();
        d.stray = !self.desk_ours(&d.settings.repo_key);
        // A device revoked since the repo was seen there no longer polls.
        if !d.seen.is_empty() {
            let k = d.settings.repo_key.clone();
            d.settle(&k, &self.revoked_fn());
        }
        d
    }

    /// Whether a device was revoked, as the fold was told.
    pub(crate) fn revoked_fn(&self) -> impl Fn(&str) -> bool + use<> {
        let r = self.revoked.clone();
        move |dev: &str| r.as_ref().is_some_and(|f| f(dev))
    }

    /// Records the open PRs the poller heard for `repo_key`: writes a
    /// `desk.prs` snapshot only when the list or a field in it changed.
    /// Returns whether a row was written.
    pub(crate) fn record_desk_prs(
        &mut self,
        repo_key: &str,
        mut prs: Vec<DeskPr>,
        truncated: bool,
        viewer: &str,
    ) -> Result<bool> {
        let repo_key = ledger::canonical_repo_key(repo_key);
        let repo_key = repo_key.as_str();
        if !self.desk_on(repo_key) {
            return Err(invalid(format!(
                "{} has no merge desk",
                ledger::repo_name(repo_key)
            )));
        }
        prs.sort_by_key(|p| p.number);
        let old = self.desks.get(repo_key).and_then(|d| d.prs.as_ref());
        // A poll that couldn't learn the login keeps the one heard before.
        let viewer = match trim_space(viewer) {
            "" => old.map(|o| o.viewer.clone()).unwrap_or_default(),
            v => v.to_string(),
        };
        let unchanged = old.is_some_and(|old| {
            old.prs == prs && old.truncated == truncated && old.viewer == viewer
        });
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
                viewer,
                by: SWEEPER.into(),
                at,
            },
        )?;
        Ok(true)
    }
}
