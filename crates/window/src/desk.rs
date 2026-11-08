//! Merge desks (card #417; docs/design/merge-desk.md §3, §7, §9): the
//! desks on the person's repos for the desk page, each listing only their
//! and their people's PRs (card #625), and the person's own write: a
//! desk's settings. It goes in as the person; coord refuses the same row
//! from an agent. The desk never merges (card #418): agents ask the person
//! "Can I merge PR #N?" on their own cards.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::ts;

/// A repo's desk settings (`repo.desk`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskSettings {
    pub repo_key: String,
    pub enabled: bool,
    pub takes_prs: bool,
    #[serde(default)]
    pub poll_device: String,
    #[serde(default)]
    pub trusted: Vec<String>,
    #[serde(default)]
    pub per_pr_ok: bool,
    /// Other people's agents don't review this repo's PRs (card #1077):
    /// off, it behaves like a solo repo. On (false here) when unset.
    #[serde(default)]
    pub reviews_off: bool,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    #[specta(type = ts::Time)]
    pub at: gocompat::time::GoTime,
}

/// One open PR in the latest snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskPrView {
    pub number: i64,
    pub url: String,
    pub title: String,
    pub author: String,
    pub is_draft: bool,
    pub head_ref_name: String,
    pub head_ref_oid: String,
    pub base_ref_name: String,
    pub mergeable: String,
    pub merge_state_status: String,
    /// "" (none), `pending`, `failure` or `success`.
    pub checks: String,
    #[specta(type = ts::Time)]
    pub created_at: gocompat::time::GoTime,
    #[specta(type = ts::Time)]
    pub updated_at: gocompat::time::GoTime,
}

/// The poller's latest snapshot (`desk.prs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskSnapshotView {
    pub truncated: bool,
    pub prs: Vec<DeskPrView>,
    /// The person's own GitHub login, as the poll's gh heard it; "" not
    /// known yet. The desk trusts it (card #521).
    pub viewer: String,
    #[specta(type = ts::Time)]
    pub at: gocompat::time::GoTime,
}

/// How polling stands, on the device that polls.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskPollView {
    /// Whether this device is the desk's poll device.
    pub here: bool,
    #[specta(type = ts::Time)]
    pub last_poll_at: gocompat::time::GoTime,
    /// Zero while GitHub can be asked.
    #[specta(type = ts::Time)]
    pub unreachable_since: gocompat::time::GoTime,
    pub error: String,
}

/// One of our PRs' author card (slice 2's read-only verdict).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskAuthorView {
    pub pr: i64,
    /// The author card's id; "" none yet.
    pub card: String,
    /// Its short ref (`#12`, `wind#4`).
    pub card_ref: String,
    /// The agent behind it; "" none.
    pub agent: String,
}

/// One repo's desk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskView {
    pub settings: DeskSettings,
    /// The settings are the default a GitHub repo an agent works in gets
    /// (card #521), not ones the person saved.
    pub is_default: bool,
    /// The device that saw the repo last: the poll device unless the
    /// person picks another; "" never seen.
    pub seen_on: String,
    /// The poll device follows `seen_on` (the default, or a saved row that
    /// left it blank); a save keeps it blank so it keeps following.
    pub follows_seen: bool,
    #[specta(optional)]
    pub prs: Option<DeskSnapshotView>,
    pub poll: DeskPollView,
    /// Each PR in the snapshot: its author card. The snapshot holds only
    /// our PRs (the person's, their members' and their agents'; card
    /// #625): anyone else's is left out here.
    pub authors: Vec<DeskAuthorView>,
}

fn settings_of(r: coord::RepoDeskRow) -> DeskSettings {
    DeskSettings {
        repo_key: r.repo_key,
        enabled: r.enabled,
        takes_prs: r.takes_prs,
        poll_device: r.poll_device,
        trusted: r.trusted,
        per_pr_ok: r.per_pr_ok,
        reviews_off: r.reviews_off,
        by: r.by,
        at: r.at,
    }
}

fn row_of(s: DeskSettings) -> coord::RepoDeskRow {
    coord::RepoDeskRow {
        repo_key: s.repo_key,
        enabled: s.enabled,
        takes_prs: s.takes_prs,
        poll_device: s.poll_device,
        trusted: s
            .trusted
            .iter()
            .map(|t| crate::trim(t).to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        per_pr_ok: s.per_pr_ok,
        reviews_off: s.reviews_off,
        ..coord::RepoDeskRow::default()
    }
}

impl Window {
    /// DeskService.Desks: the merge desks on the person's repos (a stray
    /// desk on a repo nobody in their spaces opens PRs in is left out, card
    /// #625), with how polling stands when this device polls it.
    pub fn desks(&self) -> Vec<DeskView> {
        let here = self.core.device_id();
        let mut out: Vec<DeskView> = self
            .core
            .coord()
            .desks()
            .into_iter()
            .filter(|d| !d.stray)
            .map(|d| {
                let polled = d.settings.poll_device == here;
                let poll = if polled {
                    let p = self.core.desk_poll(&d.settings.repo_key);
                    DeskPollView {
                        here: true,
                        last_poll_at: p.last_poll_at,
                        unreachable_since: p.unreachable_since,
                        error: p.error,
                    }
                } else {
                    DeskPollView::default()
                };
                let authors: Vec<DeskAuthorView> = self
                    .core
                    .desk_verdicts(&d.settings.repo_key)
                    .into_iter()
                    .map(|a| DeskAuthorView {
                        pr: a.pr,
                        card: a.card,
                        card_ref: a.card_ref,
                        agent: a.agent,
                    })
                    .collect();
                let ours: std::collections::HashSet<i64> = authors.iter().map(|a| a.pr).collect();
                DeskView {
                    authors,
                    is_default: d.is_default(),
                    seen_on: d.seen_on.clone(),
                    follows_seen: d.is_default() || d.follow_poll,
                    settings: settings_of(d.settings),
                    prs: d.prs.map(|s| DeskSnapshotView {
                        truncated: s.truncated,
                        viewer: s.viewer,
                        at: s.at,
                        prs: s
                            .prs
                            .into_iter()
                            .filter(|p| ours.contains(&p.number))
                            .map(|p| DeskPrView {
                                number: p.number,
                                url: p.url,
                                title: p.title,
                                author: p.author,
                                is_draft: p.is_draft,
                                head_ref_name: p.head_ref_name,
                                head_ref_oid: p.head_ref_oid,
                                base_ref_name: p.base_ref_name,
                                mergeable: p.mergeable,
                                merge_state_status: p.merge_state_status,
                                checks: p.checks,
                                created_at: p.created_at,
                                updated_at: p.updated_at,
                            })
                            .collect(),
                    }),
                    poll,
                }
            })
            .collect();
        out.sort_by(|a, b| a.settings.repo_key.cmp(&b.settings.repo_key));
        out
    }

    fn person(&self) -> String {
        self.core.user_sender().key
    }

    /// DeskService.Set: the person sets up or changes a repo's desk.
    pub fn desk_set(&self, s: DeskSettings) -> Result<(), String> {
        self.core
            .coord()
            .set_desk(&self.person(), row_of(s))
            .map_err(|e| e.to_string())
    }
}
