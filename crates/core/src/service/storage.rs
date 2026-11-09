//! The Storage page's Core side (card #728, docs/design/storage.md): the
//! worktrees Irori made on this device and their build output, which the
//! person may clear by hand, and which cards are settled, for the agents'
//! briefs. Nothing here runs by itself (board.md "No retention rules"):
//! the automatic sweep stays [`crate::buildclean`]'s.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// Every one of its cards is closed and at least one waits on the
    /// person's Done (card #1118): why the sweep keeps it.
    pub waiting_for_done: bool,
    /// Every one of its cards is canceled (card #1118): its work is the
    /// person's to drop, so Storage offers to delete it.
    pub canceled: bool,
    /// Processes that have their folder in it (card #1118), which the
    /// sweep can't remove it past and Storage can end.
    pub holders: Vec<treeclean::Holder>,
}

/// Whether card `t` is closed and waits on the person's Done (card #1118):
/// why the sweep keeps its tree.
pub fn waiting_for_done(t: &coord::Task) -> bool {
    coord::is_terminal(&t.state) && t.blockers.iter().any(|b| b.open() && b.done_check)
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
        let waiting: HashSet<&str> = tasks
            .iter()
            .filter(|t| waiting_for_done(t))
            .map(|t| t.id.as_str())
            .collect();
        let terminal: HashSet<&str> = tasks
            .iter()
            .filter(|t| coord::is_terminal(&t.state))
            .map(|t| t.id.as_str())
            .collect();
        let canceled: HashSet<&str> = tasks
            .iter()
            .filter(|t| t.state == coord::CANCELED)
            .map(|t| t.id.as_str())
            .collect();
        let live: BTreeSet<String> = self
            .s
            .session_dirs()
            .iter()
            .filter_map(|d| tree_of(d))
            .collect();
        let trees: Vec<(String, BTreeSet<String>)> = trees
            .into_iter()
            .filter(|(tree, _)| Path::new(tree).is_dir())
            .collect();
        // One look at the processes for every tree. When it can't be made
        // the trees list without holders: the sweep says why it can't look.
        let names: Vec<String> = trees.iter().map(|(t, _)| t.clone()).collect();
        let mut held = treeclean::holders_of(&names).unwrap_or_default();
        trees
            .into_iter()
            .map(|(tree, ids)| BuildTree {
                waiting_for_done: !ids.is_empty()
                    && ids.iter().all(|id| terminal.contains(id.as_str()))
                    && ids.iter().any(|id| waiting.contains(id.as_str())),
                canceled: !ids.is_empty() && ids.iter().all(|id| canceled.contains(id.as_str())),
                // Only a settled tree is the sweep's to remove, and so only
                // there is a process worth ending.
                holders: held
                    .remove(&tree)
                    .filter(|_| {
                        !ids.is_empty()
                            && ids
                                .iter()
                                .all(|id| settled.get(id.as_str()).copied().unwrap_or(false))
                    })
                    .unwrap_or_default(),
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

    /// Whether an agent is in `tree`, by sessions and then `claude agents`
    /// (presence lags for an agent just started). `Err` when the agents
    /// can't be listed: nobody can then say no agent is there.
    fn agent_in(&self, tree: &str) -> Result<bool, String> {
        if self
            .s
            .session_dirs()
            .iter()
            .any(|d| treeclean::under(d, tree))
        {
            return Ok(true);
        }
        let list = self
            .s
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.list.clone())
            .ok_or("Irori can't list the agents yet")?;
        let agents = list().map_err(|e| format!("claude agents: {e}"))?;
        if !super::trees::placed(&agents) {
            return Err("claude agents listed a live agent with no folder".into());
        }
        Ok(agents
            .iter()
            .any(|a| a.live && treeclean::under(&a.cwd, tree)))
    }

    /// Ends process `pid` that holds `tree` (card #1118, Storage's End it):
    /// only a settled tree [`Self::build_trees`] lists, with no agent in it
    /// (an agent's own shell would be a holder too), and only a process
    /// that is in it now ([`treeclean::end_holder`] looks again).
    pub fn end_tree_holder(&self, tree: &str, pid: u32) -> Result<treeclean::Holder, String> {
        let k = key(tree);
        let Some(t) = self.build_trees().into_iter().find(|t| t.tree == k) else {
            return Err(format!("{tree} isn't a worktree Irori made here"));
        };
        if t.live || self.agent_in(&k)? {
            return Err(LIVE_TREE.into());
        }
        if !t.settled {
            return Err("its card is still open".into());
        }
        treeclean::end_holder(&k, pid)
    }

    /// Deletes a canceled card's tree at the person's word (card #1118,
    /// gate 5 of the sweep, which keeps it for its unpushed work): only a
    /// tree every one of whose cards is canceled, with no agent in it.
    pub fn delete_canceled_tree(&self, tree: &str) -> Result<(), String> {
        let k = key(tree);
        let Some(t) = self.build_trees().into_iter().find(|t| t.tree == k) else {
            return Err(format!("{tree} isn't a worktree Irori made here"));
        };
        if !t.canceled {
            return Err("its card isn't canceled".into());
        }
        if t.live || self.agent_in(&k)? {
            return Err(LIVE_TREE.into());
        }
        let live = self.s.session_dirs();
        let check = treeclean::Check {
            live: &live,
            idle: Duration::ZERO,
            now: SystemTime::now(),
            recheck: None,
        };
        treeclean::remove_anyway(Path::new(&t.tree), &check)
    }
}
