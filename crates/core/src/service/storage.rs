//! The Storage page's Core side (card #728, docs/design/storage.md): the
//! worktrees Irori made on this device and their build output, which the
//! person may clear by hand, and which cards are settled, for the agents'
//! briefs. Nothing here runs by itself (board.md "No retention rules"):
//! the automatic sweep stays [`crate::buildclean`]'s.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::*;
use crate::buildclean::{self, Cleared};
use crate::treeclean::{self, key, tree_of};

/// A worktree Irori made here for cards, as Storage lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildTree {
    /// The tree's folder, as [`key`] writes it.
    pub tree: String,
    /// The cards that worked in it (ids).
    pub cards: Vec<String>,
    /// Every one of its cards is closed and not waiting on the person's
    /// Done: the automatic sweep may clear it after an idle hour.
    pub settled: bool,
    /// An agent on this device works in it now.
    pub live: bool,
}

/// Why a tree's build output stays when the person clears it.
pub const LIVE_TREE: &str = "an agent is working in it";

impl Core {
    /// Each card on this device and whether it is settled
    /// ([`buildclean::settled`]).
    pub fn settled_cards(&self) -> HashMap<String, bool> {
        let (tasks, _) = self.s.all_tasks();
        tasks
            .iter()
            .map(|t| (t.id.clone(), buildclean::settled(t)))
            .collect()
    }

    /// The worktrees Irori made on this device that are still on disk:
    /// the ones it booked (card #565) and the ones a card here names in
    /// its Where.
    pub fn build_trees(&self) -> Vec<BuildTree> {
        let (tasks, _) = self.s.all_tasks();
        let mine = format!("{}/", self.s.device.id);
        let mut trees: BTreeMap<String, BTreeSet<String>> = self.s.booked_trees();
        for t in &tasks {
            if let Some(w) = t.where_.as_ref()
                && !w.worktree.is_empty()
                && w.who.starts_with(&mine)
            {
                trees
                    .entry(key(&w.worktree))
                    .or_default()
                    .insert(t.id.clone());
            }
        }
        let settled: HashMap<&str, bool> = tasks
            .iter()
            .map(|t| (t.id.as_str(), buildclean::settled(t)))
            .collect();
        let live: BTreeSet<String> = self
            .s
            .session_dirs()
            .iter()
            .filter_map(|d| tree_of(d))
            .collect();
        trees
            .into_iter()
            .filter(|(tree, _)| Path::new(tree).is_dir())
            .map(|(tree, ids)| BuildTree {
                settled: !ids.is_empty()
                    && ids
                        .iter()
                        .all(|id| settled.get(id.as_str()).copied().unwrap_or(false)),
                live: live.contains(&tree),
                cards: ids.into_iter().collect(),
                tree,
            })
            .collect()
    }

    /// Clears one tree's `target/` now, at the person's word (card #728):
    /// only a tree [`Self::build_trees`] lists, never one an agent works
    /// in, and through [`buildclean::clear`]'s checks (a tree Irori made, a
    /// real cargo target, no build holding its lock). Who is in a tree is
    /// asked as the sweep's last look asks it (`tree_recheck`): sessions,
    /// then `claude agents`, so an agent presence doesn't list (just
    /// started, waiting on a limit) still counts. When the agents can't be
    /// listed, only a build idle for [`buildclean::IDLE`] goes.
    pub fn clear_build(&self, tree: &str) -> Result<Cleared, String> {
        let k = key(tree);
        let Some(t) = self.build_trees().into_iter().find(|t| t.tree == k) else {
            return Err(format!("{tree} isn't a worktree Irori made here"));
        };
        if t.live
            || self
                .s
                .session_dirs()
                .iter()
                .any(|d| treeclean::under(d, &t.tree))
        {
            return Ok(Cleared::Kept(LIVE_TREE));
        }
        let list = self
            .s
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.list.clone());
        let idle = match list.map(|f| f()) {
            Some(Ok(agents)) if super::trees::placed(&agents) => {
                if agents
                    .iter()
                    .any(|a| a.live && treeclean::under(&a.cwd, &t.tree))
                {
                    return Ok(Cleared::Kept(LIVE_TREE));
                }
                Duration::ZERO
            }
            _ => buildclean::IDLE,
        };
        buildclean::clear(Path::new(&t.tree), idle, SystemTime::now())
    }
}
