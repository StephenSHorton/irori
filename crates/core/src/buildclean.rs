//! Card #427: a settled card's build folder doesn't stay behind. Each
//! agent worktree builds into its own `target/` (the isolation card #90
//! needed), 20-90 GB each, and some 45 of them filled the disk on
//! 2026-11-03. So once a card is settled (the person said Done, or it was
//! canceled or failed), this device clears the `target/` of the worktree
//! its holder worked in, when nothing still uses it:
//!
//! - the worktree is this device's (the card's `where` is a local session's)
//!   and one Irori made ([`crate::treeclean::ours`], card #565): never a main
//!   checkout, and never a linked worktree the person made;
//! - `target/` is cargo's (it holds `CACHEDIR.TAG`);
//! - no card that isn't settled says it works there (a completed card
//!   still waiting on the person's Done keeps its build: it may come back);
//! - nothing in `target/` changed for [`IDLE`], and no cargo holds its lock.
//!
//! `target/` is renamed aside first (one step, so a build never sees it
//! half gone), then deleted; a leftover from a stop midway is deleted on
//! the next pass. Most trees go whole now ([`crate::treeclean`]); this is
//! what is left for a tree kept for its work.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use coord::{Task, is_terminal};

use crate::treeclean::{key, tree_of};

/// How often the sweep looks for a card that has just settled.
pub const LOOK: Duration = Duration::from_secs(30);
/// How often it looks again at worktrees it kept.
pub const EVERY: Duration = Duration::from_secs(10 * 60);
/// How long a `target/` must be untouched before it goes.
pub const IDLE: Duration = Duration::from_secs(60 * 60);
/// The name a `target/` is moved to while it is deleted.
pub const CLEARING: &str = "target.irori-clearing";
/// Why [`clear`] left a worktree with no cargo `target/`.
pub const NO_TARGET: &str = "no cargo target";
/// Why [`clear`] left a `target` that is a link.
pub const TARGET_LINK: &str = "target is a link";

/// Why a `target/` holding a repo, a worktree or a link out of the tree is
/// kept: that is work, not build output.
pub const HOLDS_REPO: &str = "target holds a repo, a worktree or a link";

/// When the sweep last listed the cards, when it last cleared, the
/// worktrees it cleared then, and when it may look again at each tree it
/// kept (card #565; `None` is never, for one that isn't a worktree now).
#[derive(Debug, Default)]
pub struct Looked {
    pub listed: Option<std::time::Instant>,
    pub cleared: Option<std::time::Instant>,
    pub seen: Vec<String>,
    pub rest: HashMap<String, Option<std::time::Instant>>,
    /// How many looks in a row found each tree in use (card #565).
    pub busy: HashMap<String, u32>,
}

/// Whether a card is settled: closed, and not waiting on the person's Done.
pub fn settled(t: &Task) -> bool {
    is_terminal(&t.state) && !t.blockers.iter().any(|b| b.open() && b.done_check)
}

/// A tree whose every card is settled, and those cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledTree {
    pub worktree: String,
    pub cards: Vec<String>,
}

/// The worktrees on `device` that may go: those settled cards' holders
/// worked in, and those `book` (tree to card ids) ties to cards that are all
/// settled, less any a card that isn't settled works in. A card in the book
/// that isn't in `tasks` can't be judged, so its tree stays.
pub fn settled_worktrees<'a>(
    tasks: impl IntoIterator<Item = &'a Task>,
    device: &str,
    book: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<SettledTree> {
    let mine = format!("{device}/");
    let mut done: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut busy = BTreeSet::new();
    let mut shut = BTreeSet::new();
    for t in tasks {
        let over = settled(t);
        if over {
            shut.insert(t.id.as_str());
        }
        let Some(w) = t.where_.as_ref() else {
            continue;
        };
        if over {
            if !w.worktree.is_empty() && w.who.starts_with(&mine) {
                done.entry(key(&w.worktree))
                    .or_default()
                    .insert(t.id.clone());
            }
        } else {
            if !w.worktree.is_empty() {
                busy.insert(key(&w.worktree));
            }
            busy.extend(tree_of(&w.cwd));
        }
    }
    for (tree, ids) in book {
        if ids.iter().all(|id| shut.contains(id.as_str())) {
            done.entry(tree.clone())
                .or_default()
                .extend(ids.iter().cloned());
        } else {
            busy.insert(tree.clone());
        }
    }
    done.into_iter()
        .filter(|(tree, _)| !busy.contains(tree))
        .map(|(worktree, cards)| SettledTree {
            worktree,
            cards: cards.into_iter().collect(),
        })
        .collect()
}

/// What [`clear`] did with one worktree.
#[derive(Debug, PartialEq, Eq)]
pub enum Cleared {
    /// Its `target/` is gone.
    Gone,
    /// Nothing to clear, or not ours to: why.
    Kept(&'static str),
}

/// Clears `worktree`'s `target/` if every rule in the module doc holds.
pub fn clear(worktree: &Path, idle: Duration, now: SystemTime) -> Result<Cleared, String> {
    if crate::treeclean::ours(worktree).is_none() {
        return Ok(Cleared::Kept("not a tree Irori made"));
    }
    let aside = worktree.join(CLEARING);
    if aside.exists() {
        if crate::treeclean::scan_build(worktree, &aside).is_some() {
            return Ok(Cleared::Kept(HOLDS_REPO));
        }
        fs::remove_dir_all(&aside).map_err(|e| format!("{}: {e}", aside.display()))?;
    }
    let target = worktree.join("target");
    // A `target` that is a link (a junction to a shared build folder) is
    // never deleted through (card #565).
    if fs::symlink_metadata(&target).is_ok_and(|m| crate::treeclean::is_link(&m)) {
        return Ok(Cleared::Kept(TARGET_LINK));
    }
    if !target.join("CACHEDIR.TAG").is_file() {
        return Ok(Cleared::Kept(NO_TARGET));
    }
    // Never delete a clone or worktree an agent made inside the build
    // output, or follow a link out of the tree (card #565).
    if crate::treeclean::scan_build(worktree, &target).is_some() {
        return Ok(Cleared::Kept(HOLDS_REPO));
    }
    if recent(&target, idle, now) {
        return Ok(Cleared::Kept("built recently"));
    }
    // Hold every cargo lock in it while it moves, so no build starts midway.
    let mut locks = Vec::new();
    for p in cargo_locks(&target, 3) {
        let Ok(f) = File::options().read(true).write(true).open(&p) else {
            continue;
        };
        if f.try_lock().is_err() {
            return Ok(Cleared::Kept("a build holds its lock"));
        }
        locks.push(f);
    }
    // Windows refuses to rename a folder while any file in it is open, our
    // lock handles included, so there the locks are only a check: release
    // them and move at once. A build that starts in between holds files open
    // and the rename is refused; that is a kept build, not an error.
    #[cfg(windows)]
    drop(std::mem::take(&mut locks));
    match fs::rename(&target, &aside) {
        Ok(()) => {}
        Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Ok(Cleared::Kept("in use"));
        }
        Err(e) => return Err(format!("{}: {e}", target.display())),
    }
    drop(locks);
    fs::remove_dir_all(&aside).map_err(|e| format!("{}: {e}", aside.display()))?;
    Ok(Cleared::Gone)
}

/// Whether anything in `target` changed within `idle` of `now`.
pub(crate) fn recent(target: &Path, idle: Duration, now: SystemTime) -> bool {
    newest(target, 3).is_some_and(|m| now.duration_since(m).unwrap_or_default() < idle)
}

/// The latest change under `dir` (itself and its directories, `depth`
/// levels down): a build adds and removes entries in these.
fn newest(dir: &Path, depth: u32) -> Option<SystemTime> {
    let mut m = fs::metadata(dir).and_then(|m| m.modified()).ok();
    if depth == 0 {
        return m;
    }
    for e in fs::read_dir(dir).ok()?.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            m = m.max(newest(&e.path(), depth - 1));
        }
    }
    m
}

/// Cargo's `.cargo-lock` files under `dir`, `depth` levels down
/// (`target/debug`, `target/<triple>/debug`, ...).
fn cargo_locks(dir: &Path, depth: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_name() == ".cargo-lock" {
            out.push(p);
        } else if depth > 1 && e.file_type().is_ok_and(|t| t.is_dir()) {
            out.extend(cargo_locks(&p, depth - 1));
        }
    }
    out
}
