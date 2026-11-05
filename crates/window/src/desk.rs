//! Merge desks (card #417; docs/design/merge-desk.md §3, §7, §9): every
//! desk in the person's log for the interim desk page, and the person's
//! own writes: a desk's settings, a hold on one PR, pausing the desk and
//! the OK on an Outside PR. Each write goes in as the person; coord
//! refuses the same rows from an agent.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Window;
use crate::ts;

/// One check line (§7).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskCheckLine {
    pub command: String,
    #[serde(default)]
    pub paths: Vec<String>,
    /// An agent key; "" the holder.
    #[serde(default)]
    pub runner: String,
    /// `mac`, `windows`, `linux`; "" any.
    #[serde(default)]
    pub device: String,
}

/// A repo's desk settings (`repo.desk`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskSettings {
    pub repo_key: String,
    pub enabled: bool,
    pub takes_prs: bool,
    /// The holder's agent key; "" unheld.
    #[serde(default)]
    pub holder: String,
    #[serde(default)]
    pub poll_device: String,
    #[serde(default)]
    pub trusted: Vec<String>,
    #[serde(default)]
    pub checks: Vec<DeskCheckLine>,
    #[serde(default)]
    pub merge_method: String,
    #[serde(default)]
    pub per_pr_ok: bool,
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

/// A hold, a pause, an OK or the holder's run: one PR (0 for a pause).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeskMark {
    pub pr: i64,
    /// A hold's note; an OK's or a run's head sha.
    pub note: String,
    pub head_sha: String,
    pub by: String,
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

/// A PR's author card and trust (slice 2's read-only verdict).
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
    /// An Outside PR: never checked or merged until the person's OK.
    pub outside: bool,
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
    #[specta(optional)]
    pub prs: Option<DeskSnapshotView>,
    pub holds: Vec<DeskMark>,
    #[specta(optional)]
    pub paused: Option<DeskMark>,
    pub oks: Vec<DeskMark>,
    #[specta(optional)]
    pub run: Option<DeskMark>,
    pub poll: DeskPollView,
    /// Each PR in the snapshot: its author card and whether it is Outside.
    pub authors: Vec<DeskAuthorView>,
}

fn settings_of(r: coord::RepoDeskRow) -> DeskSettings {
    DeskSettings {
        repo_key: r.repo_key,
        enabled: r.enabled,
        takes_prs: r.takes_prs,
        holder: r.holder,
        poll_device: r.poll_device,
        trusted: r.trusted,
        checks: r
            .checks
            .into_iter()
            .map(|c| DeskCheckLine {
                command: c.command,
                paths: c.paths,
                runner: c.runner,
                device: c.device,
            })
            .collect(),
        merge_method: r.merge_method,
        per_pr_ok: r.per_pr_ok,
        by: r.by,
        at: r.at,
    }
}

fn row_of(s: DeskSettings) -> coord::RepoDeskRow {
    coord::RepoDeskRow {
        repo_key: s.repo_key,
        enabled: s.enabled,
        takes_prs: s.takes_prs,
        holder: crate::trim(&s.holder).to_string(),
        poll_device: s.poll_device,
        trusted: s
            .trusted
            .iter()
            .map(|t| crate::trim(t).to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        checks: s
            .checks
            .into_iter()
            .map(|c| coord::DeskCheck {
                command: crate::trim(&c.command).to_string(),
                paths: c
                    .paths
                    .iter()
                    .map(|p| crate::trim(p).to_string())
                    .filter(|p| !p.is_empty())
                    .collect(),
                runner: c.runner,
                device: c.device,
            })
            .collect(),
        merge_method: s.merge_method,
        per_pr_ok: s.per_pr_ok,
        ..coord::RepoDeskRow::default()
    }
}

impl Window {
    /// DeskService.Desks: every merge desk in the person's log, with how
    /// polling stands when this device polls it.
    pub fn desks(&self) -> Vec<DeskView> {
        let here = self.core.device_id();
        let mut out: Vec<DeskView> = self
            .core
            .coord()
            .desks()
            .into_iter()
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
                let authors = self
                    .core
                    .desk_verdicts(&d.settings.repo_key)
                    .into_iter()
                    .map(|a| DeskAuthorView {
                        pr: a.pr,
                        card: a.card,
                        card_ref: a.card_ref,
                        agent: a.agent,
                        outside: a.outside,
                    })
                    .collect();
                DeskView {
                    authors,
                    is_default: d.is_default(),
                    seen_on: d.seen_on.clone(),
                    settings: settings_of(d.settings),
                    prs: d.prs.map(|s| DeskSnapshotView {
                        truncated: s.truncated,
                        viewer: s.viewer,
                        at: s.at,
                        prs: s
                            .prs
                            .into_iter()
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
                    holds: d
                        .holds
                        .into_values()
                        .map(|h| DeskMark {
                            pr: h.pr,
                            note: h.note,
                            by: h.by,
                            at: h.at,
                            ..DeskMark::default()
                        })
                        .collect(),
                    paused: d.pause.filter(|p| p.paused).map(|p| DeskMark {
                        by: p.by,
                        at: p.at,
                        ..DeskMark::default()
                    }),
                    oks: d
                        .oks
                        .into_values()
                        .map(|o| DeskMark {
                            pr: o.pr,
                            head_sha: o.head_sha,
                            by: o.by,
                            at: o.at,
                            ..DeskMark::default()
                        })
                        .collect(),
                    run: d.run.map(|r| DeskMark {
                        pr: r.pr,
                        head_sha: r.head_sha,
                        note: r.base_sha,
                        by: r.by,
                        at: r.at,
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

    /// DeskService.Hold: the person holds PR `pr`, or lifts the hold.
    pub fn desk_hold(
        &self,
        repo_key: &str,
        pr: i64,
        note: &str,
        cleared: bool,
    ) -> Result<(), String> {
        self.core
            .coord()
            .hold_desk_pr(&self.person(), repo_key, pr, note, cleared)
            .map_err(|e| e.to_string())
    }

    /// DeskService.Pause: the person pauses or resumes the desk.
    pub fn desk_pause(&self, repo_key: &str, paused: bool) -> Result<(), String> {
        self.core
            .coord()
            .pause_desk(&self.person(), repo_key, paused)
            .map_err(|e| e.to_string())
    }

    /// DeskService.Ok: the person okays an Outside PR at one head.
    pub fn desk_ok(&self, repo_key: &str, pr: i64, head_sha: &str) -> Result<(), String> {
        self.core
            .coord()
            .ok_desk_pr(&self.person(), repo_key, pr, head_sha)
            .map_err(|e| e.to_string())
    }
}
