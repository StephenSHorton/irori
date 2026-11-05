//! "Main is red" (card #436): a test that fails on clean main gets one
//! card, and every card that reports the same test waits on it instead of
//! raising its own blocker and spending its own hour on it.
//!
//! The card is an ordinary unheld card whose scope names the test
//! (`main-red/<test>`): that is how a second report finds it. A report is
//! a task wait on it, so the existing task-blocker path closes the wait
//! and wakes the reporter when the card completes.

use super::error::{Result, invalid};
use super::fold::Fold;
use super::tasks::CleanTask;
use super::types::*;
use super::util::{caller, runes};
use crate::names::trim_space;

/// The scope prefix that marks a "main is red" card.
pub const MAIN_RED_SCOPE: &str = "main-red/";

/// Longest test name a report may give.
const MAX_TEST: usize = 120;

/// A report's test and commit, from `<test> [commit]`.
pub fn parse_main_red(text: &str) -> Result<(String, String)> {
    let mut it = trim_space(text).split_whitespace();
    let test = it.next().unwrap_or_default().to_string();
    let commit = it.next().unwrap_or_default().to_string();
    if test.is_empty() {
        return Err(invalid(
            "main-red needs the failing test's name, e.g. main-red attach_exe 3573cf11",
        ));
    }
    if runes(&test) > MAX_TEST || runes(&commit) > 64 {
        return Err(invalid(format!(
            "main-red: test name longer than {MAX_TEST} characters"
        )));
    }
    Ok((test, commit))
}

/// The scope entry for `test`: a glob, so `::` and other ':' go to '/'.
pub fn main_red_scope(test: &str) -> String {
    let mut s = String::from(MAIN_RED_SCOPE);
    let mut last_slash = true;
    for c in test.chars() {
        let c = if c == ':' || c == '\\' || c.is_whitespace() || c.is_control() {
            '/'
        } else {
            c
        };
        if c == '/' {
            if last_slash {
                continue;
            }
            last_slash = true;
        } else {
            last_slash = false;
        }
        s.push(c);
    }
    while s.ends_with('/') && s.len() > MAIN_RED_SCOPE.len() {
        s.pop();
    }
    s
}

/// The test a "main is red" card is for, if it is one.
pub fn main_red_test(t: &Task) -> Option<&str> {
    t.scope.iter().find_map(|s| s.strip_prefix(MAIN_RED_SCOPE))
}

/// One "main is red" card on a Board view and how many open cards wait on
/// it (card #436).
#[derive(Debug, Clone)]
pub struct MainRed {
    pub task: Task,
    pub test: String,
    pub waiting: usize,
}

/// The open "main is red" cards in `v`, most waited on first.
pub fn main_reds(v: &BoardView) -> Vec<MainRed> {
    let open: Vec<&Task> = [&v.needs_you, &v.blocked, &v.working, &v.open]
        .into_iter()
        .flatten()
        .map(|c| &c.task)
        .collect();
    let mut out: Vec<MainRed> = open
        .iter()
        .filter_map(|t| {
            let test = main_red_test(t)?.to_string();
            let waiting = open
                .iter()
                .filter(|o| {
                    o.open_blockers().iter().any(|b| {
                        b.on.as_ref()
                            .is_some_and(|on| on.kind == BLOCKED_ON_TASK && on.reference == t.id)
                    })
                })
                .count();
            Some(MainRed {
                task: (*t).clone(),
                test,
                waiting,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.waiting
            .cmp(&a.waiting)
            .then_with(|| a.task.id.cmp(&b.task.id))
    });
    out
}

impl Fold {
    /// The open "main is red" card for scope entry `scope`, if any.
    pub(crate) fn main_red_card(&self, scope: &str) -> Option<&Task> {
        let mut found: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| !is_terminal(effective_state(t)) && t.scope.iter().any(|s| s == scope))
            .collect();
        // Two devices apart may each have made one: the oldest is the card.
        found.sort_by(|a, b| {
            crate::gotime::cmp(&a.created_at, &b.created_at).then_with(|| a.id.cmp(&b.id))
        });
        found.into_iter().next()
    }

    /// Finds or makes the one open card for main failing `test` (card
    /// #436), reported by `by` from card `from`. Returns it and whether it
    /// was made now (then the caller hands it out).
    pub(crate) fn main_red(
        &mut self,
        by: &str,
        from: &str,
        test: &str,
        commit: &str,
    ) -> Result<(Task, bool)> {
        let by = caller(by)?;
        let scope = main_red_scope(test);
        if let Some(t) = self.main_red_card(&scope) {
            if t.id == from {
                return Err(invalid(format!(
                    "{} is the card for main failing {test}: fix it there rather than wait on it",
                    t.short_ref()
                )));
            }
            return Ok((t.clone(), false));
        }
        let src = self.open_task(from)?.clone();
        let at = if commit.is_empty() {
            "on clean main".to_string()
        } else {
            format!("on clean main at {commit}")
        };
        let obj = format!(
            "{test} fails {at}, first reported from {} (card #436). Every card that hits the same failure waits on this one instead of fixing it on its own: find why it fails on clean main, fix it there, and land the fix. The waiting cards wake when this completes.",
            src.short_ref()
        );
        let ct = CleanTask {
            by: by.clone(),
            title: format!("{test} fails on clean main"),
            obj,
            done: format!(
                "{test} passes on clean main (a fresh worktree of origin/main), the fix is on main, and check.sh is green."
            ),
            repo_key: src.repo_key.clone(),
            // It runs on the reporting card's side (card #494).
            side: src.side.clone(),
            scope: vec![scope],
            ..CleanTask::default()
        };
        let t = self.create_locked(&by, ct)?;
        Ok((t, true))
    }
}
