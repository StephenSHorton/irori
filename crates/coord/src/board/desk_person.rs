//! The person's own writes on a merge desk (card #417, docs/design/
//! merge-desk.md §7, §9): a hold on one PR and pausing the desk. Like
//! `repo.desk` (`set_desk`, desk.rs) they are the person's alone: an agent
//! that could hold or unpause could steer what the holder merges. Each
//! refuses anyone else here, and the fold ignores such rows from anyone
//! else besides. The OK on an Outside PR (`desk.ok`) is gone with Outside
//! PRs (card #625): the desk lists only the person's and their people's
//! PRs. Old `desk.ok` rows still fold (desk.rs), nothing writes new ones.

use store::kinds;

use super::desk::{DeskHoldRow, DeskPauseRow};
use super::error::{Result, invalid, not_allowed};
use super::fold::Fold;
use super::util::caller;
use crate::names::trim_space;

/// The most a hold's note may say.
pub const DESK_NOTE_MAX: usize = 500;

impl Fold {
    /// The person, on this log, on a repo that has a desk.
    fn desk_person(&self, by: &str, repo_key: &str, what: &str) -> Result<(String, String)> {
        let by = caller(by)?;
        if self.on_board {
            return Err(invalid(
                "a merge desk lives in your own log, not a shared space's",
            ));
        }
        if !self.is_owner(&by) {
            return Err(not_allowed(format!(
                "only the person {what} on a merge desk, in the app"
            )));
        }
        let repo = ledger::canonical_repo_key(repo_key);
        if !self.desk_on(&repo) {
            return Err(invalid(format!(
                "{} has no merge desk",
                ledger::repo_name(&repo)
            )));
        }
        Ok((by, repo))
    }

    /// The person holds PR `pr` ("not this one yet"), or with `cleared`
    /// lifts the hold.
    pub(crate) fn hold_desk_pr(
        &mut self,
        by: &str,
        repo_key: &str,
        pr: i64,
        note: &str,
        cleared: bool,
    ) -> Result<()> {
        let (by, repo_key) = self.desk_person(by, repo_key, "holds a PR")?;
        if pr <= 0 {
            return Err(invalid("hold which PR?"));
        }
        let note = trim_space(note).to_string();
        if note.chars().count() > DESK_NOTE_MAX {
            return Err(invalid(format!(
                "a hold's note is at most {DESK_NOTE_MAX} characters"
            )));
        }
        let at = self.stamp();
        self.write(
            &by,
            kinds::DESK_HOLD,
            &DeskHoldRow {
                repo_key,
                pr,
                note: if cleared { String::new() } else { note },
                cleared,
                by: by.clone(),
                at,
            },
        )
    }

    /// The person pauses the desk (the holder takes no lease) or resumes it.
    pub(crate) fn pause_desk(&mut self, by: &str, repo_key: &str, paused: bool) -> Result<()> {
        let (by, repo_key) = self.desk_person(by, repo_key, "pauses")?;
        let at = self.stamp();
        self.write(
            &by,
            kinds::DESK_PAUSE,
            &DeskPauseRow {
                repo_key,
                paused,
                by: by.clone(),
                at,
            },
        )
    }
}
