//! Card terminals (card #369; docs/design/inline-terminals.md 6.1): where
//! a card's shell starts, and opening it.
//!
//! A card has no folder of its own (its `repo_key` is a remote key), so the
//! folder is found in this order:
//! 1. where a holder on this device works: its worktree or cwd on the
//!    card's Where line, then its session cwd;
//! 2. else the checkouts of the card's repo that capture has seen on this
//!    device, newest first;
//! 3. else none, and the page asks for a folder.
//!
//! A card with no repo and no local holder opens in the home folder.

use std::path::Path;
use std::time::SystemTime;

use serde::Serialize;
use specta::Type;

use crate::Window;

/// A folder a card's terminal could start in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct Folder {
    pub path: String,
    /// Why it is offered: "holder" (an agent on this device holds the card
    /// and works there), "repo" (a checkout of the card's repo) or "home".
    pub why: String,
}

impl Window {
    /// The folders a card's terminal can start in, best first. Empty: the
    /// card has a repo but this device has no checkout of it, so the page
    /// asks for one.
    pub fn term_folders(&self, card: &str) -> Result<Vec<Folder>, String> {
        let id = crate::trim(card);
        let co = self.read(id);
        let Some(mut t) = co.get_task(id) else {
            return Err(format!("no card {id}"));
        };
        if co.is_board() {
            t = self.priv_task(t);
        }
        let local = self.core.local_agents();
        let mut holders: Vec<String> = Vec::new();
        let mut held = |k: &str| {
            if !holders.iter().any(|h| h == k) {
                holders.push(k.to_string());
            }
        };
        if let Some(w) = &t.where_ {
            held(&w.who);
        }
        for k in t.holders.iter().chain(&t.co_holders) {
            held(k);
        }
        let mut hits = Vec::new();
        for k in &holders {
            let Some(a) = local.iter().find(|a| &a.key == k) else {
                continue;
            };
            if let Some(w) = t.where_.as_ref().filter(|w| &w.who == k) {
                hits.push(w.worktree.clone());
                hits.push(w.cwd.clone());
            }
            hits.push(a.cwd.clone());
        }
        let mut out = Vec::new();
        push_dirs(&mut out, hits, "holder");
        if !t.repo_key.is_empty() {
            let roots = self
                .core
                .capture()
                .repos()
                .remove(&t.repo_key)
                .unwrap_or_default();
            push_dirs(&mut out, newest_first(roots), "repo");
        }
        if out.is_empty() && t.repo_key.is_empty() {
            let home = capture::user_home_dir();
            if !home.is_empty() {
                out.push(Folder {
                    path: home,
                    why: "home".into(),
                });
            }
        }
        Ok(out)
    }

    /// Starts a new shell for a scope (a card can have several).
    pub fn term_open(
        &self,
        scope: term::Scope,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
    ) -> Result<term::TermInfo, String> {
        if let Some(d) = cwd.as_deref().filter(|d| !d.is_empty())
            && !Path::new(d).is_dir()
        {
            return Err(format!("no folder {d} on this device"));
        }
        self.core.terms().open(term::OpenSpec {
            scope,
            cols,
            rows,
            cwd,
            command: None,
        })
    }
}

impl Window {
    /// A card's terminals on this device, oldest first.
    pub fn term_list(&self, card: &str) -> Vec<term::TermInfo> {
        self.core
            .terms()
            .in_scope(&term::Scope::Card(crate::trim(card).to_string()))
    }
}

/// Adds the folders that exist on this device, once each.
fn push_dirs(out: &mut Vec<Folder>, dirs: Vec<String>, why: &str) {
    for d in dirs {
        if d.is_empty() || !Path::new(&d).is_dir() || out.iter().any(|f| same_dir(&f.path, &d)) {
            continue;
        }
        out.push(Folder {
            path: d,
            why: why.into(),
        });
    }
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches(['/', '\\']) == b.trim_end_matches(['/', '\\'])
}

/// Checkouts newest first, by when their git HEAD last moved.
fn newest_first(roots: Vec<String>) -> Vec<String> {
    let mut v: Vec<(SystemTime, String)> = roots.into_iter().map(|r| (head_time(&r), r)).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    v.into_iter().map(|(_, r)| r).collect()
}

/// The mtime of a checkout's HEAD (a worktree's `.git` is a file pointing
/// at its gitdir, so its own mtime stands in).
fn head_time(root: &str) -> SystemTime {
    let git = Path::new(root).join(".git");
    let head = if git.is_dir() { git.join("HEAD") } else { git };
    std::fs::metadata(head)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}
