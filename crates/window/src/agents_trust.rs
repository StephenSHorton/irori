//! Trust and start (card #449, docs/design/spawned-agents.md §7 "The
//! pre-trust flow", §14 part 5).
//!
//! When a card's agent can't start because its repo isn't trusted for
//! agents on this device, the spawner fails the request with
//! `not trusted: <folder> on <device>` (card #446). The window shows that
//! card in Needs you with what trusting does; the person's Trust and start
//! records trust for the repo (its main root, so a worktree is covered,
//! card #445) and starts the agent again.
//!
//! Settings › Agents also lists this device's repos with whether each is
//! trusted for agents, and its Trust… records trust for one after the
//! person confirms (card #456, sa-6).
//!
//! These two are the only paths that write trust, and both are window
//! commands: no tool, row or agent reaches them (`tests/it/trust_writers.rs`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Window;
use crate::claude_trust;
use crate::roster::TrustSet;

/// The reason prefix of a not-trusted spawn failure
/// (`SpawnRefusal::NotTrusted`).
pub const NOT_TRUSTED: &str = "not trusted: ";

/// What the person sees before they confirm.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustNeed {
    pub task_id: String,
    /// The folder trust is recorded for: the repo's main root.
    pub folder: String,
    /// The device the agent runs on.
    pub device: String,
    /// Whether that device is this one: otherwise it's trusted on that
    /// device's own Irori.
    pub here: bool,
    /// What trusting does, in the words the confirm shows.
    pub what: String,
    /// The device that asked this one to trust it (part 11), by name.
    pub asked_by: Option<String>,
}

/// After Trust and start.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustStarted {
    pub folder: String,
    /// It was already trusted (another window got there first).
    pub already: bool,
    /// What happens next, in the words the row shows (card #717): its
    /// agent starting, or why none does now (an agent already holds the
    /// card, the device is at its cap, paused or on a usage limit, or
    /// another device starts it). Never a "Starting…" that doesn't.
    pub said: String,
}

/// A board's agent permissions as Settings › Agents edits them (card #449,
/// decision 8): the choice, extra rules, and how each repo reaches main.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissions {
    /// `""` for the private board.
    pub board_id: String,
    /// `limited`, `edits` or `bypass`; `""` is bypass, still refused on a
    /// shared board and when the brief has another person's text.
    pub permission: String,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    /// Repo key → `pr` or `main`.
    pub merge: std::collections::BTreeMap<String, String>,
}

/// One repo of Settings › Agents' "Trusted repos for agents" (card #456,
/// sa-6): a repo with a checkout on this device and whether Claude Code
/// trusts it, so an agent Irori starts for its cards can run there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustedRepo {
    pub repo_key: String,
    pub label: String,
    /// The folder trust is recorded for: the repo's main root, the one the
    /// spawner starts its agents in.
    pub folder: String,
    pub trusted: bool,
    /// What trusting does, in the words the confirm shows.
    pub what: String,
}

/// The rows for `repos` (repo key → its sorted roots, as the capture
/// lists them): per repo the first root, as the spawner picks it, when
/// that is a git checkout on this device, and whether `trust` covers it.
pub(crate) fn trusted_rows(
    repos: &std::collections::BTreeMap<String, Vec<String>>,
    trust: &TrustSet,
) -> Vec<TrustedRepo> {
    let mut out: Vec<TrustedRepo> = Vec::new();
    for (key, roots) in repos {
        let Some(root) = roots.first().filter(|_| !key.is_empty()) else {
            continue;
        };
        let dir = Path::new(root);
        if !dir.is_dir() || crate::spawn::trust_root(dir).is_none() {
            continue;
        }
        let folder = trust_folder(root);
        if out.iter().any(|r| r.folder == folder) {
            continue;
        }
        out.push(TrustedRepo {
            repo_key: key.clone(),
            label: crate::board::repo_label(key),
            trusted: crate::spawn::trusted_in(trust, dir).is_ok(),
            what: what_trust_does(&folder, "this computer"),
            folder,
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.folder.cmp(&b.folder)));
    out
}

/// The words the confirm shows (§7 point 2).
pub fn what_trust_does(folder: &str, device: &str) -> String {
    format!(
        "Claude Code will run agents in {folder} on {device} without asking you first. \
         It's the trust Claude Code asks for the first time you open a folder."
    )
}

/// The folder to trust for a spawn folder: the main repo's root when it's
/// in git (a worktree follows its repo), else the folder itself.
pub fn trust_folder(folder: &str) -> String {
    let p = Path::new(folder);
    crate::spawn::trust_root(p)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.to_string())
}

/// The folder a not-trusted failure names: the row's folder when it has
/// one, else the one in `not trusted: <folder> on <device>`
/// (`SpawnRefusal::NotTrusted`). `None` for any other failure.
pub fn refused_folder(reason: &str, device: &str, folder: &str) -> Option<String> {
    let rest = reason.strip_prefix(NOT_TRUSTED)?;
    if !folder.is_empty() {
        return Some(folder.to_string());
    }
    let named = rest.strip_suffix(&format!(" on {device}")).unwrap_or(rest);
    (!named.is_empty()).then(|| named.to_string())
}

impl Window {
    /// The card's trust need, when its agent couldn't start for lack of
    /// trust; `None` otherwise.
    /// A card an agent holds, or one waiting in the queue, no longer waits
    /// on trust (card #717): its failed request stays its newest, but
    /// nothing starts an agent for a held card.
    pub fn agents_trust_need(&self, task_id: &str) -> Result<Option<TrustNeed>, String> {
        let co = self.core.coord();
        let id = co.task_ref(task_id).unwrap_or_else(|_| task_id.to_string());
        let t = co
            .get_task(&id)
            .ok_or_else(|| format!("no card {task_id}"))?;
        if !t.holders.is_empty() || !t.assigned.is_empty() {
            return Ok(None);
        }
        if self.core.spawn_queue().places.contains_key(&t.id) {
            return Ok(None);
        }
        self.trust_wait(&t)
    }

    /// Card t's trust need from its newest request alone, held or not.
    fn trust_wait(&self, t: &coord::Task) -> Result<Option<TrustNeed>, String> {
        let Some(sp) = t.spawn_request() else {
            return Ok(None);
        };
        if sp.op != coord::SPAWN_FAILED {
            return Ok(None);
        }
        let Some(spawn_folder) = refused_folder(&sp.reason, &sp.device, &sp.folder) else {
            return Ok(None);
        };
        let folder = trust_folder(&spawn_folder);
        let here = sp.device == self.core.device_id();
        let device = if here {
            "this computer".to_string()
        } else {
            self.device_label(&sp.device)
        };
        let asked_by = if here {
            self.core
                .trust_asked_by(&t.id)
                .map(|d| self.device_label(&d))
        } else {
            None
        };
        Ok(Some(TrustNeed {
            task_id: t.id.clone(),
            what: what_trust_does(&folder, &device),
            folder,
            device,
            here,
            asked_by,
        }))
    }

    /// A device's name, or the start of its id when it has none.
    fn device_label(&self, id: &str) -> String {
        match self.core.device_label(id) {
            l if l.is_empty() => id.chars().take(8).collect(),
            l => l,
        }
    }

    /// The person, on this device, asks the device a card's agent must
    /// start on to trust its repo there (part 11): that device asks the
    /// person in its own window. Nothing here writes trust. The device's
    /// name.
    pub fn agents_trust_request(&self, task_id: &str) -> Result<String, String> {
        let co = self.core.coord();
        let id = co.task_ref(task_id).unwrap_or_else(|_| task_id.to_string());
        let dev = self.core.request_trust(&id)?;
        Ok(self.device_label(&dev))
    }

    /// A board's agent permissions.
    pub fn agents_permissions(&self, board_id: &str) -> AgentPermissions {
        let s = self.core.spawn_setting(board_id);
        AgentPermissions {
            board_id: board_id.to_string(),
            permission: s.permission,
            allow: s.allow,
            deny: s.deny,
            merge: s.merge,
        }
    }

    /// The person sets a board's agent permissions (the app only, never a
    /// tool); the board's switch and other spawn settings stay.
    pub fn agents_permissions_set(&self, p: AgentPermissions) -> Result<AgentPermissions, String> {
        self.core
            .set_spawn_permissions(&p.board_id, &p.permission, p.allow, p.deny, p.merge)?;
        Ok(self.agents_permissions(&p.board_id))
    }

    /// The person's Trust and start: records trust for the card's repo on
    /// this device, then starts its agent again.
    pub fn agents_trust_and_start(&self, task_id: &str) -> Result<TrustStarted, String> {
        let file = claude_trust::config_path()
            .ok_or("can't find Claude Code's settings (.claude.json): no home folder")?;
        self.trust_and_start_in(&file, task_id)
    }

    /// [`Window::agents_trust_and_start`] with `.claude.json` at `file`
    /// (tests point it at a scratch file).
    #[doc(hidden)]
    ///
    /// The row may be a beat behind the card: an agent may have taken it
    /// since, or another click got there first. Trust is still recorded
    /// (the person asked for it), and the answer says what happens now
    /// (card #717).
    pub fn trust_and_start_in(&self, file: &Path, task_id: &str) -> Result<TrustStarted, String> {
        let co = self.core.coord();
        let id = co.task_ref(task_id).unwrap_or_else(|_| task_id.to_string());
        let t = co
            .get_task(&id)
            .ok_or_else(|| format!("no card {task_id}"))?;
        let need = self
            .trust_wait(&t)?
            .ok_or("this card's agent isn't waiting on trust")?;
        if !need.here {
            return Err(format!(
                "its agent runs on {}: trust it in Irori on that computer",
                need.device
            ));
        }
        let r = claude_trust::record_trust(file, &need.folder).map_err(|e| e.to_string())?;
        if !claude_trust::is_trusted(file, &need.folder) {
            return Err(format!("{} still reads as not trusted", need.folder));
        }
        let said = match self.held_by(&need.task_id) {
            Some(who) => format!("Trusted. {who} already has it, so no new agent starts."),
            None => {
                self.core.start_agent(&need.task_id, "")?;
                self.what_starts(&need.task_id)
            }
        };
        Ok(TrustStarted {
            folder: need.folder,
            already: r == claude_trust::Recorded::Already,
            said,
        })
    }

    /// Who card `id` is with, when an agent holds it or it was given to
    /// one: no agent starts for it then.
    fn held_by(&self, id: &str) -> Option<String> {
        let t = self.core.coord().get_task(id)?;
        let key = t.holders.first().or(t.assigned.first())?;
        let name = self.core.agent_name(key);
        Some(if name.is_empty() {
            "An agent".to_string()
        } else {
            name
        })
    }

    /// What happens to card `id` after the person's ask to start its agent
    /// (card #717), in the row's words.
    pub(crate) fn what_starts(&self, id: &str) -> String {
        let co = self.core.coord();
        if !co.hands_out_here(id) {
            return "Trusted. Another of your computers starts its agent.".into();
        }
        if self.core.agents_pause().on {
            return "Trusted. Its agent starts when you turn off Pause agents.".into();
        }
        let q = self.core.spawn_queue();
        if q.held_until.is_some() {
            return "Trusted. Its agent starts when the usage limit resets.".into();
        }
        if q.places.contains_key(id) {
            return format!(
                "Trusted. It starts when a slot frees ({} of {} in use).",
                q.running, q.cap
            );
        }
        "Trusted. Its agent is starting.".into()
    }
}

impl Window {
    /// Settings › Agents' trusted repos on this device (card #456).
    pub fn agents_trusted(&self) -> Result<Vec<TrustedRepo>, String> {
        let file = claude_trust::config_path()
            .ok_or("can't find Claude Code's settings (.claude.json): no home folder")?;
        Ok(self.trusted_repos_in(&file))
    }

    #[doc(hidden)]
    pub fn trusted_repos_in(&self, file: &Path) -> Vec<TrustedRepo> {
        trusted_rows(&self.core.capture().repos(), &TrustSet::load_file(file))
    }

    /// The person's confirmed Trust… on one of this device's repos in
    /// Settings › Agents (card #456): records trust for its folder, the
    /// one listed, and returns the list again.
    pub fn agents_trust_repo(&self, repo_key: &str) -> Result<Vec<TrustedRepo>, String> {
        let file = claude_trust::config_path()
            .ok_or("can't find Claude Code's settings (.claude.json): no home folder")?;
        self.trust_repo_in(&file, repo_key)
    }

    /// [`Window::agents_trust_repo`] with `.claude.json` at `file`.
    #[doc(hidden)]
    pub fn trust_repo_in(&self, file: &Path, repo_key: &str) -> Result<Vec<TrustedRepo>, String> {
        let row = self
            .trusted_repos_in(file)
            .into_iter()
            .find(|r| coord::same_repo_key(&r.repo_key, repo_key))
            .ok_or_else(|| format!("no copy of {} on this computer", coord::repo_name(repo_key)))?;
        if !row.trusted {
            claude_trust::record_trust(file, &row.folder).map_err(|e| e.to_string())?;
            if !claude_trust::is_trusted(file, &row.folder) {
                return Err(format!("{} still reads as not trusted", row.folder));
            }
        }
        Ok(self.trusted_repos_in(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_rows_take_the_repos_git_root_once_and_read_its_own_entry() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().canonicalize().map(crate::spawn::plain).unwrap();
        let a = base.join("a");
        let b = base.join("b");
        let plain = base.join("plain");
        for p in [&a, &b] {
            std::fs::create_dir_all(p.join(".git")).unwrap();
        }
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let file = base.join(".claude.json");
        // The parent is trusted, a isn't: a repo needs its own entry (§7).
        let cfg = serde_json::json!({ "projects": {
            s(&base): { "hasTrustDialogAccepted": true },
            s(&b): { "hasTrustDialogAccepted": true },
        }});
        std::fs::write(&file, cfg.to_string()).unwrap();
        let repos: std::collections::BTreeMap<String, Vec<String>> = [
            ("remote:x/a", vec![s(&a.join("sub")), s(&a)]),
            ("remote:x/b", vec![s(&b)]),
            ("path:plain", vec![s(&plain)]),
            ("remote:x/gone", vec![s(&base.join("gone"))]),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let rows = trusted_rows(&repos, &TrustSet::load_file(&file));
        let got: Vec<(&str, &str, bool)> = rows
            .iter()
            .map(|r| (r.label.as_str(), r.folder.as_str(), r.trusted))
            .collect();
        assert_eq!(
            got,
            vec![("a", s(&a).as_str(), false), ("b", s(&b).as_str(), true)]
        );
        assert!(rows[0].what.contains(&s(&a)), "{}", rows[0].what);
    }

    #[test]
    fn the_refused_folder_comes_from_the_row_or_its_reason() {
        assert_eq!(
            refused_folder("not trusted: /r/a on b/x on dev1", "dev1", ""),
            Some("/r/a on b/x".into()),
            "a folder with ' on ' in it"
        );
        assert_eq!(
            refused_folder("not trusted: /r on dev1", "dev1", "/kept"),
            Some("/kept".into())
        );
        assert_eq!(
            refused_folder("no checkout of r on dev1", "dev1", "/r"),
            None
        );
        assert_eq!(refused_folder("not trusted:  on dev1", "dev1", ""), None);
    }
}
