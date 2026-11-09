//! A card's linked pull requests as GitHub says they stand (card #208).
//!
//! The device that holds a card asks GitHub (the person's own `gh`) how
//! each PR the card links stands and writes a `pr.state` row when that
//! changes. coord never shells out: the Core's poller does, and hands what
//! it heard to [`Fold::record_pr_state`], which writes the row and acts on
//! it:
//! - merged: the holders are woken to complete the card with proof, or,
//!   while another PR the card links isn't merged yet, told without a wake;
//! - closed without a merge: the holders hear it;
//! - changes requested on GitHub: a card in review goes back to working
//!   and its holders are woken, as a reviewer's changes verdict does;
//! - GitHub couldn't be asked (no access, a PR gone) about a PR nothing is
//!   known of yet: said once on the card, and an agent's completed falls
//!   back to the rules without it;
//! - reopened after it was closed: the holders hear it, without a wake.
//!
//! What GitHub said outlasts a device that couldn't ask it (card #542): an
//! error never replaces a known state, in the fold or at the writer. A
//! device with no gh doesn't ask at all, but older phones still write
//! these rows, in any order against the rows that know better. When gh
//! fails on a PR known to be open, the error shows beside OPEN (card
//! #548), and the row still says nothing about the PR: it can't undo a
//! closing it hadn't heard of.
//!
//! An agent's completed is refused while a PR its card links is known to
//! be open ([`Fold::open_pr_refusal`]), and the refusal says when GitHub
//! can't be asked.
//!
//! On a shared board the notices above reach only the writer's person's
//! agents: another person's agents' keys mean nothing on that device. So
//! each device asks [`Fold::pr_news`] what its own holders may not have
//! heard, and the Core tells them through the bridge, once (card #548).

use std::collections::BTreeMap;

use gocompat::time::GoTime;
use serde::Serialize;
use store::{Event, kinds};

use super::error::{Result, invalid, not_found};
use super::fold::Fold;
use super::links::pr_link;
use super::rows::{PrReview, PrStateRow, TaskCommentRow, TaskStateRow, row};
use super::types::*;
use super::util::{SWEEPER, deeper};
use gocompat::quote::quote;

/// A PR notice's kind.
pub const NOTICE_PR: &str = "pr";

/// GitHub's PR states, as `gh pr view --json state` spells them.
pub const PR_OPEN: &str = "OPEN";
pub const PR_MERGED: &str = "MERGED";
pub const PR_CLOSED: &str = "CLOSED";
/// GitHub's review decision that sends a card back to work.
pub const PR_CHANGES_REQUESTED: &str = "CHANGES_REQUESTED";
/// GitHub's review decision on a repo that wants an approval before a
/// merge: it comes from another member's agents (card #471).
pub const PR_REVIEW_REQUIRED: &str = "REVIEW_REQUIRED";

/// How one linked PR stands, as last heard from GitHub.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrState {
    /// The PR's URL, as the card links it.
    pub pr: String,
    /// OPEN, MERGED or CLOSED; empty when GitHub couldn't be asked.
    pub state: String,
    /// GitHub's reviewDecision: APPROVED, CHANGES_REQUESTED,
    /// REVIEW_REQUIRED or empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub review: String,
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    pub merged_at: GoTime,
    /// The commit the merge made on the base branch (GitHub's
    /// mergeCommit), once a device heard it (card #421).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub merge_sha: String,
    /// Why GitHub couldn't be asked (no gh, no access); empty when it was.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// When it was heard.
    pub at: GoTime,
    /// The newest error row and when it was written (card #548). `error`
    /// shows it while nothing newer was heard and the PR isn't settled.
    #[serde(skip)]
    pub last_error: String,
    /// Who commented on the PR, and GitHub's reviews of it (merge desk
    /// §12.4): from the newest row that carried any, whatever order rows
    /// come in. A row without them leaves what is known.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commenters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<PrReview>,
    /// The first changed paths, and whether there were more.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub paths_cut: bool,
    /// Which row those came from: its time and device, to pick the newest.
    #[serde(skip)]
    pub activity_from: (GoTime, String),
    /// The PR's head commit (merge desk §12.5), from the newest row that
    /// carried one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub head: String,
    #[serde(skip)]
    pub head_from: (GoTime, String),
    #[serde(skip)]
    pub error_at: GoTime,
    /// Everything GitHub was heard to say of it, oldest first, whatever
    /// order the rows came in (card #548): [`PrState::news`] reads it.
    #[serde(skip)]
    pub heard: Vec<PrHeard>,
}

/// One thing GitHub was heard to say about a PR, and on which device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrHeard {
    at: GoTime,
    device: String,
    state: String,
    review: String,
    merged_at: GoTime,
    merge_sha: String,
    over_closed: bool,
}

impl PrState {
    fn note_heard(&mut self, h: PrHeard) {
        if !self.heard.contains(&h) {
            self.heard.push(h);
            self.heard.sort_by(|a, b| {
                crate::gotime::cmp(&a.at, &b.at).then_with(|| a.device.cmp(&b.device))
            });
        }
    }

    /// The rows that stand, oldest first: an OPEN after a closing counts
    /// only from a device that knew of the closing (card #548). Another
    /// device's OPEN from before it heard is stale, not a reopen.
    fn standing(&self) -> Vec<&PrHeard> {
        let mut out: Vec<&PrHeard> = Vec::new();
        for h in &self.heard {
            let closed = out.last().is_some_and(|w| w.state == PR_CLOSED);
            if closed && h.state == PR_OPEN && !h.over_closed {
                continue;
            }
            out.push(h);
        }
        out
    }

    /// What the holders should have heard of this PR by now: the change,
    /// a name for it and the devices that heard it. Read from the sorted
    /// rows, so it comes out the same in any arrival order and after a
    /// replay. Rows in a run that say the same are one piece of news,
    /// named by how many times that change has happened: a late row in a
    /// run, or one that adds no such change before it, moves nothing.
    fn news(&self) -> Option<(PrChange, String, Vec<String>)> {
        let devices = |rows: &mut dyn Iterator<Item = &PrHeard>| {
            let mut v: Vec<String> = rows.map(|h| h.device.clone()).collect();
            v.sort();
            v.dedup();
            v
        };
        let merged = devices(&mut self.heard.iter().filter(|h| h.state == PR_MERGED));
        if !merged.is_empty() {
            return Some((PrChange::Merged, PR_MERGED.into(), merged));
        }
        let standing = self.standing();
        // A review means nothing once a PR is closed.
        let says = |h: &PrHeard| {
            let review = if h.state == PR_OPEN { &h.review } else { "" };
            (h.state.clone(), review.to_string())
        };
        let runs: Vec<&[&PrHeard]> = standing.chunk_by(|a, b| says(a) == says(b)).collect();
        let change_at = |i: usize| {
            let (state, review) = says(runs[i][0]);
            let was = i.checked_sub(1).and_then(|j| runs[j].last());
            if state == PR_CLOSED {
                Some(PrChange::Closed)
            } else if state != PR_OPEN {
                None
            } else if review == PR_CHANGES_REQUESTED
                && !was.is_some_and(|w| w.review == PR_CHANGES_REQUESTED)
            {
                Some(PrChange::ChangesRequested)
            } else if was.is_some_and(|w| w.state == PR_CLOSED) {
                Some(PrChange::Reopened)
            } else {
                None
            }
        };
        let last = runs.len().checked_sub(1)?;
        let at = (0..=last).rev().find(|&i| change_at(i).is_some())?;
        let change = change_at(at)?;
        // Only a reopen outlasts what came after it: a review on the PR
        // since doesn't close it again. Changes since answered are over.
        if at != last && change != PrChange::Reopened {
            return None;
        }
        let nth = (0..=at).filter(|&i| change_at(i) == Some(change)).count();
        let mut name = format!("{change:?}#{nth}");
        // Changes requested after a closing are news again, even when a
        // late closing row is what came between (card #548).
        if change == PrChange::ChangesRequested {
            let closings = (0..at)
                .filter(|&i| change_at(i) == Some(PrChange::Closed))
                .count();
            name = format!("{name}.{closings}");
        }
        // A device that wrote a reopen it saw told its person's agents by
        // its own notice, whichever run its row is in; one that only wrote
        // OPEN since told nobody but this news's own.
        let told: Vec<&PrHeard> = if change == PrChange::Reopened {
            runs[at..]
                .iter()
                .flat_map(|r| r.iter().copied())
                .filter(|h| h.over_closed)
                .collect()
        } else {
            runs[at].to_vec()
        };
        Some((change, name, devices(&mut told.into_iter())))
    }
}

/// What the poller heard about one PR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrCheck {
    pub pr: String,
    pub state: String,
    pub review: String,
    pub merged_at: GoTime,
    /// GitHub's mergeCommit oid, for a merged PR (card #421); "" unknown.
    pub merge_sha: String,
    pub error: String,
    /// What GitHub's comments and reviews say (merge desk §12.4). Empty
    /// is "not asked": it keeps what is known and never clears.
    pub commenters: Vec<String>,
    pub reviews: Vec<PrReview>,
    pub paths: Vec<String>,
    pub paths_cut: bool,
    /// The PR's head commit; "" not asked, which keeps what is known.
    pub head: String,
}

impl PrCheck {
    /// Whether this check carries what GitHub's comments, reviews or
    /// files said.
    fn has_activity(&self) -> bool {
        !self.commenters.is_empty()
            || !self.reviews.is_empty()
            || !self.paths.is_empty()
            || self.paths_cut
    }
}

/// The PR states by card, then by PR URL.
pub(crate) type PrStates = BTreeMap<String, BTreeMap<String, PrState>>;

/// Whether a row from `from` (time, device) is newer than `was`, or
/// nothing was: every device ends on the same row.
fn newer_from(from: &(GoTime, String), was: &(GoTime, String)) -> bool {
    was.0.is_zero()
        || crate::gotime::cmp(&from.0, &was.0)
            .then_with(|| from.1.cmp(&was.1))
            .is_gt()
}

/// pr.state rows a board keeps for a card whose holder or wait hasn't
/// folded yet, at most (card #548).
const MAX_PR_AHEAD: usize = 256;

/// What a PR's holders are told when it comes to stand some way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrChange {
    Merged,
    Closed,
    ChangesRequested,
    Reopened,
}

/// What a device owes its own agents about one PR on a shared board
/// (card #548): see [`Fold::pr_news`].
#[derive(Debug, Clone, Default)]
pub struct PrNews {
    /// Names this news, so one agent hears it once.
    pub mark: String,
    /// The devices that wrote it.
    pub writers: Vec<String>,
    /// What to say, and to whom, in board keys.
    pub notice: Notice,
}

/// How far a row's state can be trusted over another's (card #542): an
/// error says nothing about the PR, and a merge can't be undone, while
/// GitHub can reopen a closed PR.
fn certainty(state: &str) -> u8 {
    match state {
        "" => 0,
        PR_MERGED => 2,
        _ => 1,
    }
}

/// Whether `t` is a "PR #n" card the merge desk made for a PR with no
/// author card (merge desk §3 rule 4): Irori made it, and its title and
/// objective say so. Only such a card completes itself on its merge.
pub fn desk_pr_card(t: &Task) -> bool {
    t.created_by == SWEEPER
        && t.title.starts_with("PR #")
        && t.objective.contains(DESK_PR_CARD_MARK)
}

/// What a PR card's cancel note starts with when its PR closed: only such
/// a cancel is undone when the PR reopens.
pub const PR_CARD_CLOSED: &str = "its PR was closed without merging";

/// The words a "PR #n" card's objective carries (the desk poller writes
/// them, `map_desk`), which [`desk_pr_card`] looks for.
pub const DESK_PR_CARD_MARK: &str = "has no author card";

impl Fold {
    pub(crate) fn fold_pr_state(&mut self, ev: &Event) {
        let Some(p) = row::<PrStateRow>(&ev.payload) else {
            return;
        };
        if p.task_id.is_empty() || p.pr.is_empty() {
            return;
        }
        let by_pr = self.pr_states.entry(p.task_id.clone()).or_default();
        let st = by_pr.entry(p.pr.clone()).or_insert_with(|| PrState {
            pr: p.pr.clone(),
            ..PrState::default()
        });
        let shown = (
            st.state.clone(),
            st.review.clone(),
            st.error.clone(),
            st.at,
            st.merge_sha.clone(),
            (
                st.commenters.clone(),
                st.reviews.clone(),
                st.paths.clone(),
                st.paths_cut,
            ),
            st.head.clone(),
        );
        // Merge desk §12.5: the head too, from the newest row carrying one.
        if !p.head.is_empty() {
            let from = (p.at, ev.device_id.clone());
            if newer_from(&from, &st.head_from) {
                st.head = p.head.clone();
                st.head_from = from;
            }
        }
        // Merge desk §12.4: the newest row that carries any wins, by time
        // then device, so every device ends on the same one.
        if !p.commenters.is_empty() || !p.reviews.is_empty() || !p.paths.is_empty() || p.paths_cut {
            let from = (p.at, ev.device_id.clone());
            if newer_from(&from, &st.activity_from) {
                st.commenters = p.commenters.clone();
                st.reviews = p.reviews.clone();
                st.paths = p.paths.clone();
                st.paths_cut = p.paths_cut;
                st.activity_from = from;
            }
        }
        if p.error.is_empty() && !p.state.is_empty() {
            st.note_heard(PrHeard {
                at: p.at,
                device: ev.device_id.clone(),
                state: p.state.clone(),
                review: p.review.clone(),
                merged_at: p.merged_at,
                merge_sha: p.merge_sha.clone(),
                over_closed: p.over_closed,
            });
            // Card #542: a merge wins whatever the times, else the newest
            // row that stands, read from the sorted rows so every device
            // ends on the same one in whatever order they arrive.
            let standing = st.standing();
            let won = standing
                .iter()
                .rev()
                .find(|h| h.state == PR_MERGED)
                .or(standing.last())
                .map(|h| (h.state.clone(), h.review.clone(), h.merged_at, h.at));
            if let Some((state, review, merged_at, at)) = won {
                (st.state, st.review, st.merged_at, st.at) = (state, review, merged_at, at);
            }
            // Card #421: the first merged row that carries the sha, by the
            // sorted rows, so any arrival order ends on the same one.
            st.merge_sha = st
                .heard
                .iter()
                .find(|h| h.state == PR_MERGED && !h.merge_sha.is_empty())
                .map(|h| h.merge_sha.clone())
                .unwrap_or_default();
        } else if !crate::gotime::before(&p.at, &st.error_at) {
            // An error says nothing of the PR, so it changes nothing known.
            st.last_error = p.error;
            st.error_at = p.at;
            if st.state.is_empty() {
                st.at = p.at;
            }
        }
        // Card #548: worked out from the two, so any arrival order agrees.
        st.error = if st.state.is_empty()
            || st.state == PR_OPEN && crate::gotime::before(&st.at, &st.error_at)
        {
            st.last_error.clone()
        } else {
            String::new()
        };
        if shown
            == (
                st.state.clone(),
                st.review.clone(),
                st.error.clone(),
                st.at,
                st.merge_sha.clone(),
                (
                    st.commenters.clone(),
                    st.reviews.clone(),
                    st.paths.clone(),
                    st.paths_cut,
                ),
                st.head.clone(),
            )
        {
            return;
        }
        // A phone hears a merge only through this row: Needs you must redraw.
        self.note(Change {
            kind: TASK_CHANGED.into(),
            task_id: p.task_id,
            ..Change::default()
        });
    }

    /// The states of the PRs `t` links, in link order; a linked PR not yet
    /// heard of is left out.
    pub(crate) fn pr_states_of(&self, t: &Task) -> Vec<PrState> {
        let Some(by_pr) = self.pr_states.get(&t.id) else {
            return Vec::new();
        };
        t.links
            .iter()
            .filter(|l| pr_link(l).is_some())
            .filter_map(|l| by_pr.get(&l.reference).cloned())
            .collect()
    }

    /// The PRs t's open, unanswered external waits are on (card #509) that
    /// the poller still asks about, whoever holds the card: not known
    /// merged (a closed one too, which GitHub can reopen, card #548). Its
    /// holders hear the merge; the person is never asked to merge (card
    /// #970). On a shared board only this person's own agents' waits.
    pub(crate) fn polled_waits(&self, t: &Task) -> Vec<String> {
        if t.state != BLOCKED {
            return Vec::new();
        }
        let by_pr = self.pr_states.get(&t.id);
        let mut out: Vec<String> = Vec::new();
        for b in &t.blockers {
            if b.status == BLOCKER_ANSWERED || self.on_board && !self.own_on_board(b) {
                continue;
            }
            let Some(pr) = b.pr_wait() else {
                continue;
            };
            let merged = by_pr
                .and_then(|m| m.get(&pr))
                .is_some_and(|p| p.state == PR_MERGED);
            if !merged && !out.contains(&pr) {
                out.push(pr);
            }
        }
        out
    }

    /// Refuses an agent's completed while a PR its card links is known to
    /// be open (card #208). A PR GitHub was never heard on doesn't count:
    /// the rules without it apply. One GitHub can't be asked about now
    /// still counts, open being the last it said (card #548): a gh that
    /// failed once must not let open work through. The refusal says so.
    pub(crate) fn open_pr_refusal(&self, t: &Task) -> Result<()> {
        let open: Vec<PrState> = self
            .pr_states_of(t)
            .into_iter()
            .filter(|p| p.state == PR_OPEN)
            .collect();
        if open.is_empty() {
            return Ok(());
        }
        let prs = open
            .iter()
            .map(|p| p.pr.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        if let Some(p) = open.iter().find(|p| !p.error.is_empty()) {
            return Err(invalid(format!(
                "{} links a pull request last heard open ({prs}), and GitHub can't be asked now: {}. If it has merged, fix gh on this device, or ask the person with board_block to complete the card",
                t.short_ref(),
                p.error
            )));
        }
        Err(invalid(format!(
            "{} links a pull request that GitHub still shows open ({prs}). If you merged it, check it with `gh pr view` and set completed again once it shows merged. If not, ask the person “Can I merge…?” with board_block and merge it yourself on their yes. Never raise a wait on it: Irori checks GitHub and tells you when it merges",
            t.short_ref()
        )))
    }

    /// Whether every PR `t` links is known to be merged (and it links one).
    pub(crate) fn all_prs_merged(&self, t: &Task) -> bool {
        let linked = t.links.iter().filter(|l| pr_link(l).is_some()).count();
        let states = self.pr_states_of(t);
        linked > 0 && states.len() == linked && states.iter().all(|p| p.state == PR_MERGED)
    }

    /// Puts "Merged as <sha>" on `t` as Irori (merge desk §6, card #421),
    /// once per sha: a card that already says it, from another device
    /// that heard the merge too, gets nothing more. A log that refuses the
    /// comment (a board's writer rules) loses only the comment: the
    /// pr.state row carrying the sha is written already. Two devices that
    /// hear the merge at once both write it, under one id made from the
    /// card, PR, sha and GitHub's merge time, so the card shows it once.
    fn say_merge_sha(&mut self, t: &Task, pr: &str, sha: &str, merged_at: GoTime) {
        if t.withdrawn {
            return;
        }
        let said = self
            .comments
            .values()
            .any(|c| c.task_id == t.id && c.by == SWEEPER && c.text.contains(sha));
        if said {
            return;
        }
        let short: String = sha.chars().take(8).collect();
        let at = self.stamp();
        let id_at = if merged_at.is_zero() { at } else { merged_at };
        let _ = self.write_comment(
            SWEEPER,
            TaskCommentRow {
                task_id: t.id.clone(),
                comment_id: super::util::fixed_id(&id_at, &format!("{} {pr} {sha}", t.id)),
                by: SWEEPER.into(),
                text: format!("Merged as {short} ({sha}): {pr}"),
                at,
                ..TaskCommentRow::default()
            },
        );
    }

    /// Records what the poller heard about PR `c.pr` on card `id` and acts
    /// on what changed (module doc). Nothing is written when nothing
    /// changed, when GitHub still can't be asked (that is said once), or
    /// when it can't be asked about a PR known merged or closed. Failing
    /// on a PR known open is said once, beside OPEN.
    /// Returns the card and the notices to route.
    pub(crate) fn record_pr_state(&mut self, id: &str, c: PrCheck) -> Result<(Task, Vec<Notice>)> {
        let Some(t) = self.tasks.get(id).cloned() else {
            return Err(not_found(format!("task {}", quote(id))));
        };
        // Card #509: or a PR an open external wait is on.
        if !t
            .links
            .iter()
            .any(|l| l.reference == c.pr && pr_link(l).is_some())
            && !t
                .blockers
                .iter()
                .any(|b| b.pr_wait().is_some_and(|w| w == c.pr))
        {
            return Err(invalid(format!(
                "{} links no pull request {}",
                t.short_ref(),
                quote(&c.pr)
            )));
        }
        let old = self
            .pr_states
            .get(id)
            .and_then(|m| m.get(&c.pr))
            .cloned()
            .unwrap_or_default();
        // What the fold wouldn't take isn't written: it would only repeat.
        let beside_open = !c.error.is_empty() && old.state == PR_OPEN;
        // Card #421: a merge heard before without its sha (an older device,
        // or one that didn't ask for it) is heard again with it.
        let new_sha = c.error.is_empty()
            && c.state == PR_MERGED
            && !c.merge_sha.is_empty()
            && old.merge_sha.is_empty();
        // Merge desk §12.4: a comment or review alone is a change; a check
        // without any keeps what is known and says nothing of it.
        let same_activity = !c.has_activity()
            || (old.commenters == c.commenters
                && old.reviews == c.reviews
                && old.paths == c.paths
                && old.paths_cut == c.paths_cut);
        // A push alone is a change (merge desk §12.5).
        let same_head = c.head.is_empty() || old.head == c.head;
        let unchanged = !new_sha
            && (!beside_open && certainty(&c.state) < certainty(&old.state)
                || if c.error.is_empty() {
                    old.error.is_empty()
                        && old.state == c.state
                        && old.review == c.review
                        && same_activity
                        && same_head
                        && !old.at.is_zero()
                } else {
                    // Said once: a later failure stays quiet until it is heard again.
                    !old.error.is_empty()
                });
        if unchanged {
            // A PR card follows its PR on every poll, so a step that
            // didn't land is made up (merge desk §12.2).
            if c.error.is_empty() && t.pr_card.as_ref().is_some_and(|k| k.pr == c.pr) {
                self.follow_pr_card(id, &c.pr)?;
                return Ok((self.tasks[id].clone(), Vec::new()));
            }
            return Ok((t, Vec::new()));
        }
        // Card #548: heard after what this fold knows, so stamped after
        // it, as a clock behind another device's would sort it before the
        // closing or the error it answers.
        let mut at = self.stamp();
        for known in [old.at, old.error_at] {
            if !crate::gotime::before(&known, &at) {
                at = crate::gotime::add(&known, std::time::Duration::from_nanos(1000));
            }
        }
        self.write_as(
            SWEEPER,
            kinds::PR_STATE,
            &PrStateRow {
                task_id: id.into(),
                pr: c.pr.clone(),
                state: c.state.clone(),
                review: c.review.clone(),
                merged_at: c.merged_at,
                merge_sha: if c.state == PR_MERGED {
                    c.merge_sha.clone()
                } else {
                    String::new()
                },
                error: c.error.clone(),
                by: SWEEPER.into(),
                at,
                over_closed: c.state == PR_OPEN && old.state == PR_CLOSED,
                commenters: c.commenters.clone(),
                reviews: c.reviews.clone(),
                paths: c.paths.clone(),
                paths_cut: c.paths_cut,
                head: c.head.clone(),
            },
        )?;
        // A row this log didn't take (a board's writer rules) changes
        // nothing here: telling the holders would repeat at every poll.
        let took = self
            .pr_states
            .get(id)
            .and_then(|m| m.get(&c.pr))
            .is_some_and(|p| p.at == at || p.error_at == at);
        if !took {
            return Err(invalid(format!(
                "{}: the pr.state row for {} wasn't taken here",
                t.short_ref(),
                quote(&c.pr)
            )));
        }
        if new_sha {
            self.say_merge_sha(&t, &c.pr, &c.merge_sha, c.merged_at);
        }
        // Merge desk §12.2: a PR card ends with its PR, and comes back if
        // it reopens. Before the terminal return below, which a cancelled
        // card would take.
        if c.error.is_empty() && t.pr_card.as_ref().is_some_and(|k| k.pr == c.pr) {
            self.follow_pr_card(id, &c.pr)?;
            return Ok((self.tasks[id].clone(), Vec::new()));
        }
        if !c.error.is_empty() || is_terminal(&t.state) {
            return Ok((self.tasks[id].clone(), Vec::new()));
        }
        let newly = |s: &str| c.state == s && old.state != s;
        // Card #421: a "PR #n" card Irori made for a PR with no author card
        // completes itself once its PRs are merged, held or not. A merge an
        // older device heard first (no sha, no such rule) completes it when
        // the sha comes.
        if (newly(PR_MERGED) || new_sha) && desk_pr_card(&t) && self.all_prs_merged(&self.tasks[id])
        {
            let sha = self
                .pr_states
                .get(id)
                .and_then(|m| m.get(&c.pr))
                .map(|p| p.merge_sha.clone())
                .unwrap_or_default();
            let note = if sha.is_empty() {
                format!("{} was merged on GitHub", c.pr)
            } else {
                format!("{} was merged on GitHub as {sha}", c.pr)
            };
            let at = self.stamp();
            self.write_as(
                SWEEPER,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: COMPLETED.into(),
                    note,
                    by: SWEEPER.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
            return Ok((self.tasks[id].clone(), Vec::new()));
        }
        let change = if newly(PR_MERGED) {
            Some(PrChange::Merged)
        } else if newly(PR_CLOSED) {
            Some(PrChange::Closed)
        } else if c.state == PR_OPEN
            && c.review == PR_CHANGES_REQUESTED
            && old.review != PR_CHANGES_REQUESTED
        {
            Some(PrChange::ChangesRequested)
        } else if c.state == PR_OPEN && old.state == PR_CLOSED {
            Some(PrChange::Reopened)
        } else {
            None
        };
        if change == Some(PrChange::ChangesRequested) && t.state == IN_REVIEW {
            let at = self.stamp();
            self.write_as(
                SWEEPER,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: WORKING.into(),
                    note: "changes requested on GitHub".into(),
                    by: SWEEPER.into(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        let ns = change
            .map(|ch| self.change_notice(&t, &c.pr, ch))
            .filter(|n| !n.to.is_empty());
        Ok((self.tasks[id].clone(), ns.into_iter().collect()))
    }

    /// Makes PR card `id` stand as its PR `pr` does (merge desk §12.2),
    /// as the card's creator's person: completed on a merge, cancelled on
    /// a close, and put back when the PR is open again after that close.
    /// Only the creator's person's own devices write it (they are the ones
    /// that poll and may cancel). Safe to repeat.
    fn follow_pr_card(&mut self, id: &str, pr: &str) -> Result<()> {
        let Some(t) = self.tasks.get(id).cloned() else {
            return Ok(());
        };
        let by = t.created_by.clone();
        if t.pr_card.is_none() || !self.is_owner(&by) {
            return Ok(());
        }
        let Some(p) = self.pr_states.get(id).and_then(|m| m.get(pr)).cloned() else {
            return Ok(());
        };
        let to = if p.state == PR_MERGED && !is_terminal(&t.state) {
            let note = if p.merge_sha.is_empty() {
                format!("{pr} was merged on GitHub")
            } else {
                format!("{pr} was merged on GitHub as {}", p.merge_sha)
            };
            Some((COMPLETED, note))
        } else if p.state == PR_CLOSED && !is_terminal(&t.state) {
            Some((CANCELED, format!("{PR_CARD_CLOSED}: {pr}")))
        } else if p.state == PR_OPEN && t.state == CANCELED && t.note.starts_with(PR_CARD_CLOSED) {
            // Only a cancel this made: a duplicate's stays cancelled.
            return self.reopen_locked(&by, id, &format!("its PR was reopened on GitHub: {pr}"));
        } else {
            None
        };
        if let Some((state, note)) = to {
            let at = self.stamp();
            self.write(
                &by,
                kinds::TASK_STATUS,
                &TaskStateRow {
                    task_id: id.into(),
                    state: state.into(),
                    note,
                    by: by.clone(),
                    at,
                    ..TaskStateRow::default()
                },
            )?;
        }
        Ok(())
    }

    /// The notice for `change` to PR `pr` on `t`.
    fn change_notice(&self, t: &Task, pr: &str, change: PrChange) -> Notice {
        match change {
            PrChange::Merged => {
                // Another linked PR still open (the card's own, with the
                // merged one linked as context, #230): say it merged, but
                // don't ask for completed and don't wake anyone over it.
                let waiting = self.unfinished_prs_besides(t, pr);
                if waiting.is_empty() {
                    return self.pr_notice(
                        t,
                        pr,
                        "was merged",
                        format!(
                            "Check the work landed, then set it completed with proof: board_status task_id={} state=completed proof=\"…\".",
                            t.id
                        ),
                    );
                }
                let mut n = self.pr_notice(
                    t,
                    pr,
                    "was merged",
                    format!(
                        "The card also links {}, not merged yet: keep it as it is; Irori tells you when that merges.",
                        waiting.join(", ")
                    ),
                );
                n.wake = false;
                n
            }
            PrChange::Closed => self.pr_notice(
                t,
                pr,
                "was closed without merging",
                // Card #614: a merge card or a release batch often lands the
                // work in another PR and closes this one with a comment.
                format!(
                    "Read the closing comment first (`gh pr view {pr} --comments`): a merge card or a release batch may have landed the work in another PR. Then say on the card what happens to the work now: reopen it, link the PR that replaces it, or ask the person."
                ),
            ),
            PrChange::ChangesRequested => {
                // Card #548: only a device of the card's author or the
                // board's owner moves it back; anyone else's agent does.
                let hint = if self.tasks.get(&t.id).is_some_and(|c| c.state == IN_REVIEW) {
                    format!(
                        "The card is still in review: set board_status task_id={} state=working now. Read the review with `gh pr view --comments`, make the changes, push, then set board_status in_review again.",
                        t.id
                    )
                } else {
                    "Read the review with `gh pr view --comments`, make the changes, push, then set board_status in_review again.".into()
                };
                self.pr_notice(t, pr, "has changes requested on GitHub", hint)
            }
            PrChange::Reopened => {
                let mut n = self.pr_notice(
                    t,
                    pr,
                    "was reopened on GitHub",
                    "It counts as open again: the card can't be completed until it merges or closes.".into(),
                );
                n.wake = false;
                n
            }
        }
    }

    /// What this board's PR rows say now, for each device to pass to the
    /// agents it hosts (card #548): the writer's device could only tell
    /// its own person's. Read from the fold, not from rows as they come,
    /// so a row that came ahead of the card's holder still counts;
    /// [`PrNews::mark`] keeps it to once.
    pub(crate) fn pr_news(&self) -> Vec<PrNews> {
        let mut out = Vec::new();
        for (id, by_pr) in &self.pr_states {
            // A card waiting on the person's Done is past hearing it.
            let Some(t) = self.tasks.get(id).filter(|t| {
                !is_terminal(&t.state) && !t.blockers.iter().any(|b| b.open() && b.done_check)
            }) else {
                continue;
            };
            for (pr, p) in by_pr {
                // A PR the card let go of is nobody's news.
                let on_card = t
                    .links
                    .iter()
                    .any(|l| l.reference == *pr && pr_link(l).is_some())
                    || t.blockers
                        .iter()
                        .any(|b| b.pr_wait().is_some_and(|w| w == *pr));
                let Some((change, name, writers)) = p.news().filter(|_| on_card) else {
                    continue;
                };
                let notice = self.change_notice(t, pr, change);
                if !notice.to.is_empty() {
                    out.push(PrNews {
                        mark: format!("pr:{id}/{pr}/{name}"),
                        writers,
                        notice,
                    });
                }
            }
        }
        out
    }

    /// Keeps a pr.state row a board refused (card #548): its holder's or
    /// wait's row may be on its way from another device, and nothing
    /// writes a settled PR's row again. Every row that says something of
    /// the PR, as news needs each; of the errors only the newest per PR and
    /// device. Returns whether it was kept.
    pub(crate) fn hold_pr_row(&mut self, ev: &Event) -> bool {
        // Only a row refused for want of who vouches for it, and not this
        // device's own: it wrote that knowing who held the card.
        if !self.on_board
            || ev.kind != kinds::PR_STATE
            || ev.actor != SWEEPER
            || ev.device_id == self.dev
            || self.sweeper_ok(ev)
        {
            return false;
        }
        let Some(p) = row::<PrStateRow>(&ev.payload) else {
            return false;
        };
        if self
            .pr_ahead
            .iter()
            .any(|o| o.device_id == ev.device_id && o.seq == ev.seq)
        {
            return true;
        }
        if p.state.is_empty() {
            self.pr_ahead.retain(|o| {
                o.device_id != ev.device_id
                    || !row::<PrStateRow>(&o.payload).is_some_and(|r| {
                        r.task_id == p.task_id && r.pr == p.pr && r.state.is_empty()
                    })
            });
        }
        if self.pr_ahead.len() >= MAX_PR_AHEAD {
            // The oldest in log order goes. Which rows are kept depends on
            // what came first here, so past the cap devices can disagree
            // on that PR until a restart replays the log in order.
            let at = (0..self.pr_ahead.len())
                .min_by_key(|&i| {
                    let e = &self.pr_ahead[i];
                    (e.hlc, e.device_id.clone(), e.seq)
                })
                .unwrap_or(0);
            let gone = self.pr_ahead.remove(at);
            self.ignore(&gone.device_id);
        }
        self.pr_ahead.push(ev.clone());
        true
    }

    /// Notes who card `id`'s pr.state rows count from now, for good
    /// (`Fold::pr_vouchers`). Kept rows get another look when it grows.
    pub(crate) fn note_pr_vouchers(&mut self, id: &str) {
        let Some(t) = self.tasks.get(id) else {
            return;
        };
        // A PR card's PR state is vouched for by its creator alone (merge
        // desk §12.2): nobody holds it, and anyone else who joined, held or
        // raised a wait on it could feed rows that end claims or fake
        // verdicts.
        let now: Vec<(String, String)> = if t.pr_card.is_some() {
            vec![(String::new(), t.created_by.clone())]
        } else {
            t.holders
                .iter()
                .chain(&t.co_holders)
                .map(|h| (String::new(), h.clone()))
                .chain(
                    t.blockers
                        .iter()
                        .filter_map(|b| b.pr_wait().map(|pr| (pr, b.raised_by.clone()))),
                )
                .collect()
        };
        let vs = self.pr_vouchers.entry(id.to_string()).or_default();
        for v in now {
            if !vs.contains(&v) {
                vs.push(v);
                self.pr_vouched = true;
            }
        }
    }

    /// Folds the kept pr.state rows that count now, when someone new
    /// vouches for a card's rows.
    pub(crate) fn retry_pr_ahead(&mut self) {
        if !std::mem::take(&mut self.pr_vouched) || self.pr_ahead.is_empty() {
            return;
        }
        for ev in std::mem::take(&mut self.pr_ahead) {
            if self.sweeper_ok(&ev) {
                self.fold(&ev);
            } else {
                self.pr_ahead.push(ev);
            }
        }
    }

    /// The PRs `t` links, other than `pr`, that aren't known to be merged
    /// or closed (open, or not heard of yet), in link order.
    fn unfinished_prs_besides(&self, t: &Task, pr: &str) -> Vec<String> {
        let heard = self.pr_states.get(&t.id);
        t.links
            .iter()
            .filter(|l| pr_link(l).is_some() && l.reference != pr)
            .filter(|l| {
                !heard
                    .and_then(|m| m.get(&l.reference))
                    .is_some_and(|p| p.state == PR_MERGED || p.state == PR_CLOSED)
            })
            .map(|l| l.reference.clone())
            .collect()
    }

    /// Puts back on the Board each in-review card nobody holds while a PR
    /// it links is open or not heard of yet (card #471): its agent went,
    /// so nobody merges it and nobody hears it merge. Not a card waiting
    /// on someone else: one whose PR GitHub says needs an approval (that
    /// comes from another member's agents) or that another member's agent
    /// was asked to review on a shared board.
    pub(crate) fn return_unheld_prs(&mut self) -> Result<()> {
        let ids: Vec<(String, String)> = self
            .tasks
            .values()
            .filter(|t| {
                t.state == IN_REVIEW
                    && t.holders.is_empty()
                    && t.co_holders.is_empty()
                    && t.assigned.is_empty()
                    && t.subtasks.is_empty()
                    && t.open_ask().is_none()
                    && self.hands_out(t)
                    && !(self.on_board && self.review_asks(&t.id).next().is_some())
                    && !self
                        .pr_states_of(t)
                        .iter()
                        .any(|p| p.state == PR_OPEN && p.review == PR_REVIEW_REQUIRED)
            })
            .filter_map(|t| {
                let pr = self.unfinished_prs_besides(t, "").into_iter().next()?;
                let n = t
                    .links
                    .iter()
                    .find(|l| l.reference == pr)
                    .and_then(pr_link)
                    .map(|(_, n)| format!("PR #{n}"))
                    .unwrap_or(pr);
                Some((t.id.clone(), n))
            })
            .collect();
        for (id, n) in ids {
            self.back_on_board(
                SWEEPER,
                &id,
                &format!(
                    "its {n} is still open and nobody is on it: ask the person “Can I merge…?” and merge it on their yes, or complete the card if it merged"
                ),
            )?;
        }
        Ok(())
    }

    fn pr_notice(&self, t: &Task, pr: &str, what: &str, next: String) -> Notice {
        let nt = self.tasks.get(&t.id).unwrap_or(t);
        Notice {
            kind: NOTICE_PR.into(),
            task_id: t.id.clone(),
            reference: format!("task:{}", t.id),
            title: t.title.clone(),
            by: SWEEPER.into(),
            to: self.recipients(nt, SWEEPER),
            wake: true,
            text: format!(
                "The pull request on {} ({}) {what}: {pr}\n{next}{}",
                quote(&t.title),
                t.refs(),
                deeper(&t.id)
            ),
            ..Notice::default()
        }
    }
}
