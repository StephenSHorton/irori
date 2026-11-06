//! wait / waits / wait_cancel (card #87): wake me once when a card
//! reaches a state, or when a branch moves.

use capture::gitlink::{branch_tips, worktree_root};
use coord::{BoardCard, Wait, WaitSpec, until_text, wait_line};

use crate::args::Ty;
use crate::format::{one_line, q, trim};
use crate::{Caller, Inner, decode, err_text, map_err};

const WAIT_ARGS: &[(&str, Ty)] = &[
    ("task_id", Ty::Str),
    ("until", Ty::Strs),
    ("branch", Ty::Str),
    ("repo", Ty::Str),
    ("note", Ty::Str),
];

impl Inner {
    /// The worktree root and repoKey a branch wait reads: `repo` as a path
    /// or a repoKey Irori has seen, else the caller's own working folder.
    fn wait_repo(&self, c: &Caller, repo: &str) -> Result<(String, String), String> {
        let names = self.roster();
        let card = names.cards.get(&c.sender.key);
        let (dir, key) = if repo.is_empty() {
            let Some(card) = card.filter(|c| !c.cwd.is_empty()) else {
                return Err(err_text(
                    "Irori doesn't know your working folder: give repo (a path or a repoKey).",
                ));
            };
            (card.cwd.clone(), card.repo_key.clone())
        } else if std::path::Path::new(repo).is_absolute() {
            let key = card
                .filter(|c| !c.cwd.is_empty() && repo.starts_with(&c.cwd))
                .map(|c| c.repo_key.clone())
                .unwrap_or_default();
            (repo.to_string(), key)
        } else {
            let roots = self
                .b
                .capture()
                .map(|cap| cap.repos())
                .and_then(|m| m.get(repo).cloned())
                .unwrap_or_default();
            let Some(root) = roots.into_iter().next() else {
                return Err(err_text(&format!(
                    "Irori has no checkout of {} on this device: give repo as a path.",
                    q(repo)
                )));
            };
            (root, repo.to_string())
        };
        let Some(root) = worktree_root(&dir) else {
            return Err(err_text(&format!("{} is not in a git repo.", q(&dir))));
        };
        Ok((root, key))
    }
}

/// "#12 until done (note)" or "branch main of org/app moves".
fn describe(s: &Inner, w: &Wait) -> String {
    let r = if w.task_id.is_empty() {
        String::new()
    } else {
        s.short_of(&w.task_id)
    };
    one_line(&wait_line(w, &r), 300)
}

pub(crate) fn wait(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, WAIT_ARGS) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let task = trim(a.str("task_id"));
    let branch = trim(a.str("branch"))
        .trim_start_matches("refs/heads/")
        .to_string();
    let note = trim(a.str("note")).to_string();
    if task.is_empty() == branch.is_empty() {
        return err_text(
            "give task_id (wait on a card) or branch (wait on a branch moving), not both.",
        );
    }
    if !task.is_empty() {
        let id = match s.task_id(task) {
            Ok(id) => id,
            Err(m) => return m,
        };
        // No keys: mapping the caller to a board key would put it on the
        // shared board, and this wait stays off it (card #574).
        let (co, _) = match s.on(&id, &[]) {
            Ok(v) => v,
            Err(e) => return map_err(e),
        };
        // A wait stays in the waiter's own log (card #574): on a shared
        // board's card it holds what that log can't read.
        let on_board = if std::sync::Arc::ptr_eq(&co, s.b.coord()) {
            None
        } else {
            let (Some(t), Some(b)) = (
                co.get_task(&id),
                s.b.shared_boards().and_then(|sb| sb.board_of(&id)),
            ) else {
                return err_text(&format!("can't read {} here.", s.short_of(&id)));
            };
            Some(BoardCard {
                board: b.board_id,
                card: t.refs(),
                title: t.title.clone(),
                state: t.state.clone(),
                own: t.holders.len() == 1
                    && s.b
                        .router()
                        .is_some_and(|r| r.private_key(&t.holders[0]) == c.sender.key),
            })
        };
        let shared = on_board.is_some();
        let w = match s.b.coord().create_wait(
            &c.sender.key,
            WaitSpec {
                task_id: id.clone(),
                until: a.strs("until"),
                note,
                on_board,
                ..WaitSpec::default()
            },
        ) {
            Ok(w) => w,
            Err(e) => return map_err(e),
        };
        return format!(
            "Waiting ({}): you'll be woken once when {} is {} (it is {} now).{} It ends when it fires, after a day, or when your session ends; wait_cancel id={} drops it.",
            w.id,
            s.short_of(&id),
            until_text(&w.until),
            w.from_state,
            if shared {
                " Only you see this wait: it stays off the shared board."
            } else {
                ""
            },
            w.id
        );
    }
    if branch.contains("..") || branch.contains(' ') {
        return err_text(&format!("{} is not a branch name.", q(&branch)));
    }
    let (root, key) = match s.wait_repo(c, trim(a.str("repo"))) {
        Ok(v) => v,
        Err(m) => return m,
    };
    let Some(refs) = branch_tips(&root, &branch) else {
        return err_text(&format!("can't read the refs of {}.", q(&root)));
    };
    let w = match s.b.coord().create_wait(
        &c.sender.key,
        WaitSpec {
            repo: key,
            root: root.clone(),
            branch: branch.clone(),
            refs: refs.clone(),
            note,
            ..WaitSpec::default()
        },
    ) {
        Ok(w) => w,
        Err(e) => return map_err(e),
    };
    let now: Vec<String> = refs
        .iter()
        .map(|r| {
            let (n, sha) = r.split_once('=').unwrap_or((r, ""));
            let n = n
                .trim_start_matches("refs/")
                .replace("heads/", "")
                .replace("remotes/", "");
            if sha.is_empty() {
                format!("{n} doesn't exist yet")
            } else {
                format!("{n} at {}", &sha[..sha.len().min(7)])
            }
        })
        .collect();
    format!(
        "Waiting ({}): you'll be woken once when {branch} moves in {} ({}). Irori reads this device's refs: a push elsewhere shows once it is fetched here. It ends when it fires, after a day, or when your session ends; wait_cancel id={} drops it.",
        w.id,
        one_line(&root, 200),
        now.join(", "),
        w.id
    )
}

pub(crate) fn waits(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    if let Err(e) = decode(raw, &[]) {
        return e;
    }
    let ws = s.b.coord().waits(&c.sender.key);
    if ws.is_empty() {
        return "You have no open waits.".into();
    }
    let mut out = format!("Your open waits ({}):", ws.len());
    for w in &ws {
        out.push_str(&format!("\n- {}: {}", w.id, describe(s, w)));
    }
    out
}

pub(crate) fn wait_cancel(s: &Inner, c: &Caller, raw: &[u8]) -> String {
    let a = match decode(raw, &[("id", Ty::Str)]) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let id = trim(a.str("id"));
    if id.is_empty() {
        return err_text("give the wait's id (the waits tool lists them).");
    }
    match s.b.coord().cancel_wait(&c.sender.key, id) {
        Ok(w) => format!("Canceled your wait on {}.", describe(s, &w)),
        Err(e) => map_err(e),
    }
}
