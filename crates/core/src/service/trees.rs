//! Card #565's side of the Services: which card each tree belongs to, the
//! live folders a removal must stay out of, and what is said on a card
//! whose tree stays. The rules are [`crate::treeclean`]'s.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use coord::Coord;

use super::spawn::{KEPT_WORKTREE, SWEEPER};
use super::*;
use crate::buildclean::{self, Cleared, Looked, SettledTree};
use crate::treeclean::{self, Book, Check, KEPT_EVERY, Verdict, tree_of};

/// The start of the line Irori leaves on a settled card whose worktree
/// stays; the tree and why follow.
pub const KEPT_TREE: &str = "Irori kept this card's worktree so no work is lost: ";
/// How many looks in a row a tree is found in use before its card says so.
pub const BUSY_SAY: u32 = 3;

/// Says on card `id` once, restarts included, why `tree` stays. The line
/// `claude rm` left about the same folder counts as saying it.
pub fn say_kept(co: &Coord, id: &str, tree: &str, why: &str) {
    let start = format!("{KEPT_TREE}{tree}:");
    let told = co.comments(id).unwrap_or_default().iter().any(|c| {
        c.by == SWEEPER
            && (c.text.starts_with(&start)
                || (c.text.starts_with(KEPT_WORKTREE) && treeclean::names_tree(&c.text, tree)))
    });
    if told {
        return;
    }
    if let Err(e) = co.add_comment(SWEEPER, id, &format!("{start} {why}."), false) {
        logf(format_args!("kept {tree}: {e}"));
    }
}

/// What the `n`th look in a row that finds a tree in use does: whether its
/// card is told (from the [`BUSY_SAY`]th, once: [`say_kept`] says a tree
/// once), and how long the tree then rests, doubling from
/// [`buildclean::EVERY`] up to [`KEPT_EVERY`].
pub fn busy_plan(n: u32) -> (bool, Option<Duration>) {
    if n < BUSY_SAY {
        return (false, None);
    }
    let wait = buildclean::EVERY
        .saturating_mul(1 << (n - BUSY_SAY).min(8))
        .min(KEPT_EVERY);
    (true, Some(wait))
}

/// One pass over `trees`, on the sweep's own thread. `live` is where
/// attached sessions are; `list`, when there is one, adds `claude agents`'.
/// Without that list nobody can say no agent is in a tree, so none is
/// removed: only `target/` is cleared, as card #427 did.
pub(super) struct Pass {
    pub trees: Vec<SettledTree>,
    pub homes: BTreeMap<String, Arc<Coord>>,
    pub live: Vec<String>,
    pub list: Option<super::spawn::ListFn>,
    pub idle: Duration,
    pub looked: Arc<Mutex<Looked>>,
    /// A fresh look at one tree just before it goes (finding 7), so a card
    /// reopened while this pass ran keeps its tree. Without one, nothing
    /// is removed.
    pub recheck: Option<Recheck>,
}

/// See [`Pass::recheck`] and [`Check::recheck`].
pub(super) type Recheck = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Whether `agents` can say where every live one is: one listed live with
/// no folder could be in any tree (finding 11).
pub(super) fn placed(agents: &[super::spawn::ClaudeAgent]) -> bool {
    agents.iter().all(|a| !a.live || !a.cwd.trim().is_empty())
}

impl Pass {
    pub(super) fn run(mut self) {
        let listed = match self.list.as_ref().map(|f| f()) {
            Some(Ok(agents)) if placed(&agents) => {
                let live = agents.into_iter().filter(|a| a.live).map(|a| a.cwd);
                self.live.extend(live);
                self.recheck.is_some()
            }
            Some(Ok(_)) => {
                logf(format_args!(
                    "clear trees: claude agents listed a live agent with no folder"
                ));
                false
            }
            Some(Err(e)) => {
                logf(format_args!("clear trees: claude agents: {e}"));
                false
            }
            None => false,
        };
        for t in &self.trees {
            let w = t.worktree.as_str();
            if self.live.iter().any(|d| treeclean::under(d, w)) {
                continue;
            }
            if listed {
                self.judge(t);
            } else {
                self.clear_target(w);
            }
        }
    }

    fn judge(&self, t: &SettledTree) {
        let w = t.worktree.as_str();
        let recheck = self.recheck.as_deref().map(|f| f as &treeclean::RecheckFn);
        let check = Check {
            live: &self.live,
            idle: self.idle,
            now: SystemTime::now(),
            recheck,
        };
        match treeclean::remove(Path::new(w), &check) {
            Verdict::Removed => {
                self.unbusy(w);
                logf(format_args!("removed {w}: its card is settled"));
            }
            Verdict::Work(why) => {
                self.unbusy(w);
                // The card is told once; the log says it at every look, so
                // a new reason after the first shows somewhere.
                logf(format_args!("kept {w}: it has {why}"));
                self.say(t, &format!("it has {why}"));
                self.clear_target(w);
                self.rest(w, Some(Instant::now() + KEPT_EVERY));
            }
            Verdict::Broken(why) => {
                self.unbusy(w);
                self.say(t, &why);
                self.rest(w, None);
            }
            // Waiting for an idle build is the normal way out.
            Verdict::Busy(why) if why == treeclean::BUILT_RECENTLY => self.unbusy(w),
            Verdict::Busy(why) => self.busy(t, &why),
            Verdict::NotOurs => {}
        }
    }

    /// A tree found in use again: looked at less often each time, and said
    /// on the card once it has been in use [`BUSY_SAY`] looks in a row (a
    /// shell left open in it, a file held), so the person learns why it is
    /// still there.
    fn busy(&self, t: &SettledTree, why: &str) {
        let w = t.worktree.as_str();
        let n = {
            let mut l = self.looked.lock().unwrap();
            let n = l.busy.entry(w.to_string()).or_insert(0);
            *n += 1;
            *n
        };
        logf(format_args!("kept {w} for now ({n} in a row): {why}"));
        let (say, wait) = busy_plan(n);
        if say {
            self.say(
                t,
                &format!("nothing could remove it yet ({why}); Irori looks again later"),
            );
        }
        if let Some(wait) = wait {
            self.rest(w, Some(Instant::now() + wait));
        }
    }

    fn unbusy(&self, w: &str) {
        self.looked.lock().unwrap().busy.remove(w);
    }

    fn say(&self, t: &SettledTree, why: &str) {
        for id in &t.cards {
            if let Some(co) = self.homes.get(id) {
                say_kept(co, id, &t.worktree, why);
            }
        }
    }

    fn rest(&self, tree: &str, until: Option<Instant>) {
        let mut l = self.looked.lock().unwrap();
        l.rest.insert(tree.to_string(), until);
    }

    fn clear_target(&self, w: &str) {
        match buildclean::clear(Path::new(w), self.idle, SystemTime::now()) {
            Ok(Cleared::Gone) => {
                logf(format_args!("cleared {w}/target: its card is settled"));
            }
            Ok(Cleared::Kept(_)) => {}
            Err(e) => logf(format_args!("clear build {w}: {e}")),
        }
    }
}

impl Services {
    /// Runs `f` on this device's [`Book`], saving it when `f` says it
    /// changed.
    fn book<R>(&self, f: impl FnOnce(&mut Book) -> (R, bool)) -> R {
        let mut b = self.spawn.book.lock().unwrap_or_else(|e| e.into_inner());
        let book = b.get_or_insert_with(|| Book::load(&self.data_dir));
        let (out, changed) = f(book);
        if changed && let Err(e) = book.save(&self.data_dir) {
            logf(format_args!("trees: {e}"));
        }
        out
    }

    /// Writes down that card `id` works in `tree`.
    pub(crate) fn note_tree(&self, tree: &str, id: &str) {
        self.book(|b| ((), b.note(tree, id)));
    }

    /// Whether `claude rm` has already kept agent `key`'s job for good.
    pub(crate) fn job_kept(&self, key: &str) -> bool {
        self.book(|b| (b.kept_jobs.contains(key), false))
    }

    /// Stops asking `claude rm` about agent `key`'s job.
    pub(crate) fn note_job_kept(&self, key: &str) {
        self.book(|b| ((), b.kept_jobs.insert(key.to_string())));
    }

    /// The trees and the cards the book ties them to.
    pub(super) fn booked_trees(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.book(|b| (b.trees.clone(), false))
    }

    /// Forgets the settled `trees` that are gone from disk. One stranded
    /// at its aside name (finding 5) isn't gone: the next look moves it
    /// back.
    pub(super) fn forget_gone_trees(&self, trees: &[SettledTree]) {
        self.book(|b| {
            let before = b.trees.len();
            let gone = |t: &&SettledTree| {
                let p = Path::new(&t.worktree);
                !p.is_dir() && !treeclean::aside_path(p).exists()
            };
            for t in trees.iter().filter(gone) {
                b.trees.remove(&t.worktree);
            }
            ((), b.trees.len() != before)
        });
    }

    /// Ties each tree a session here is in to the cards it holds, so a card
    /// that never gets a Where (a thread, a shared board's card, an agent
    /// that made its own tree) can still take its tree with it.
    pub(super) fn note_held_trees(&self) {
        for c in self.presence.snapshot() {
            if c.status == "gone" || irori_identity::owner::device_of(&c.key) != self.device.id {
                continue;
            }
            let dir = self
                .capture
                .session_dir(&c.key)
                .unwrap_or_else(|| c.cwd.clone());
            let Some(tree) = tree_of(&dir) else {
                continue;
            };
            let held = self.coord.list_tasks(&coord::TaskFilter {
                holder: c.key.clone(),
                open: true,
                ..coord::TaskFilter::default()
            });
            for t in held {
                self.note_tree(&tree, &t.id);
            }
            if let Some((_, t, _)) = self.spawn_card_of(&c.key) {
                self.note_tree(&tree, &t.id);
            }
        }
    }

    /// The folders sessions here are in: a removal stays out of them.
    pub(super) fn session_dirs(&self) -> Vec<String> {
        let mut out = Vec::new();
        for c in self.presence.snapshot() {
            if c.status == "gone" {
                continue;
            }
            out.extend(self.capture.session_dir(&c.key));
            if !c.cwd.is_empty() {
                out.push(c.cwd);
            }
        }
        out
    }

    /// Every card here and the log each is on: this device's board and
    /// each shown shared board.
    pub(super) fn all_tasks(&self) -> (Vec<coord::Task>, BTreeMap<String, Arc<Coord>>) {
        let mut tasks = self.coord.list_tasks(&coord::TaskFilter::default());
        let mut homes: BTreeMap<String, Arc<Coord>> = tasks
            .iter()
            .map(|t| (t.id.clone(), self.coord.clone()))
            .collect();
        for b in self.shown_boards() {
            let on_board = b.coord().list_tasks(&coord::TaskFilter::default());
            homes.extend(on_board.iter().map(|t| (t.id.clone(), b.coord().clone())));
            tasks.extend(on_board);
        }
        (tasks, homes)
    }

    /// The last look at `tree` before it goes (finding 7): nobody, by a
    /// session or `claude agents`, is in it now, and its cards are all
    /// still settled by the book and Where. The cards are read last, after
    /// `claude agents` (up to 30 s), so a card reopened while that ran is
    /// seen. `Err` says why it stays.
    pub(super) fn tree_recheck(&self, tree: &str) -> Result<(), String> {
        if self
            .session_dirs()
            .iter()
            .any(|d| treeclean::under(d, tree))
        {
            return Err("a session is in it".into());
        }
        let list = self
            .spawn
            .spawner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|s| s.list.clone())
            .ok_or("no agent list")?;
        let agents = list().map_err(|e| format!("claude agents: {e}"))?;
        if !placed(&agents) {
            return Err("claude agents listed a live agent with no folder".into());
        }
        if agents
            .iter()
            .any(|a| a.live && treeclean::under(&a.cwd, tree))
        {
            return Err("an agent is in it".into());
        }
        self.note_held_trees();
        let (tasks, _) = self.all_tasks();
        let book = self.booked_trees();
        let k = treeclean::key(tree);
        let settled = buildclean::settled_worktrees(&tasks, &self.device.id, &book);
        if !settled.iter().any(|t| t.worktree == k) {
            return Err("a card that works in it isn't settled any more".into());
        }
        Ok(())
    }

    /// Moves back every tree a removal left aside in each repo this device
    /// knows a tree of, whatever its card's state (finding 9). Run only
    /// while no pass is running, so never under a removal in progress.
    pub(super) fn restore_probed(&self, tasks: &[coord::Task]) {
        let mut homes = BTreeSet::new();
        let book = self.booked_trees();
        let wheres = tasks.iter().filter_map(|t| t.where_.as_ref());
        let named = wheres.flat_map(|w| [w.worktree.as_str(), w.cwd.as_str()]);
        for p in named.chain(book.keys().map(String::as_str)) {
            if let Some(home) = tree_of(p).and_then(|t| {
                Path::new(&t)
                    .parent()
                    .map(|h| h.to_string_lossy().into_owned())
            }) {
                homes.insert(home);
            }
        }
        for home in homes {
            for why in treeclean::restore_all(Path::new(&home)) {
                logf(format_args!("trees: {why}"));
            }
        }
    }

    /// Says once per repo that the only checkout left is a job's temp clone.
    pub(crate) fn say_temp_root(&self, repo_key: &str, root: &str) {
        let mut said = self
            .spawn
            .temp_said
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if said.insert(repo_key.to_string()) {
            logf(format_args!(
                "spawn: {repo_key}: only a job's temp clone is checked out here, so nothing starts from it: {root}"
            ));
        }
    }
}
