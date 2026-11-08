//! The Core side of the PR card (merge desk §12.2 and §12.4, card #1077):
//! see [`crate::prcard`]. After a good desk poll, the PR author's poll
//! device makes the PR card in a shared space that already works in the
//! repo, reads GitHub's comments, reviews and files into its `pr.state`,
//! and tells the author's holders what members said on the PR.
//!
//! A solo repo, a repo whose switch is off and a space that is Ask me never
//! get a card, and the PR stays on the person's own desk with its private
//! "PR #n" card. A space gets cards only while every other member device
//! has said `REVIEWS_PROTOCOL` (the gate), failing closed on a device not
//! heard from.

use std::sync::Arc;

use coord::{Coord, DeskPr, NewTask, PrCard, Task, TaskFilter};
use mesh::boardmesh::REVIEWS_PROTOCOL;

use super::Services;
use super::boards::BoardLog;
use super::spaces::ReadsLevel;
use crate::deskmap::Verdict;
use crate::prcard::{self, RepoCard, SpaceFacts};
use crate::{logf, notice};

/// A PR card found on a board.
struct Found {
    board: Arc<BoardLog>,
    task: Task,
}

impl Services {
    /// The PR cards of the PRs in `repo_key`'s desk snapshot that the
    /// person opened (§12.2). `verdicts` is what [`Services::map_desk`]
    /// just said: the author card a relayed comment goes to.
    pub(super) fn pr_cards(self: &Arc<Self>, repo_key: &str, verdicts: &[(i64, Verdict)]) {
        let Some(lister) = self.desk_lister.clone() else {
            return;
        };
        let Some(desk) = self.coord.desk(repo_key) else {
            return;
        };
        let (Some(snap), Some(repo)) = (desk.prs.as_ref(), coord::github_repo(repo_key)) else {
            return;
        };
        // The switch (§12.2): off, the repo behaves like a solo one.
        if desk.settings.reviews_off {
            return;
        }
        let own = self.own_github_login();
        if own.is_empty() {
            return;
        }
        let boards = self.visible_boards();
        if boards.is_empty() {
            return;
        }
        let me = self.person_actor();
        let found = self.pr_cards_found(&boards);
        let mut facts: Option<Vec<SpaceFacts>> = None;
        for p in snap.prs.iter().filter(|p| p.author.eq_ignore_ascii_case(&own)) {
            let cards: Vec<&Found> = found
                .iter()
                .filter(|f| f.task.pr_card.as_ref().is_some_and(|k| k.pr == p.url))
                .collect();
            // The lowest id stands on every device; the author's device
            // cancels the others it made (§12.2).
            let stands = cards.iter().min_by(|a, b| a.task.id.cmp(&b.task.id)).copied();
            if let Some(keep) = stands {
                self.cancel_duplicates(&me, keep, &cards);
                self.desk_watch.set_note(repo_key, p.number, None);
                self.follow_pr_card(&lister, &repo, p, keep, verdicts);
                continue;
            }
            if !prcard::needs_card(p, &[]) {
                self.desk_watch.set_note(repo_key, p.number, None);
                continue;
            }
            let facts = facts.get_or_insert_with(|| self.space_facts(&boards, repo_key));
            let Some((space, board)) = prcard::choose_space(facts) else {
                self.desk_watch.set_note(repo_key, p.number, None);
                continue;
            };
            let Some(b) = boards.iter().find(|b| b.id == space) else {
                continue;
            };
            match self.space_waiting_at(b, ReadsLevel::Reviews, REVIEWS_PROTOCOL) {
                Ok((old, unheard)) => {
                    if let Some(line) = prcard::waiting_line(&old, &unheard) {
                        if self
                            .desk_watch
                            .set_note(repo_key, p.number, Some(line.clone()))
                        {
                            logf(format_args!("desk: PR #{}: {line}", p.number));
                        }
                        continue;
                    }
                }
                Err(e) => {
                    logf(format_args!("desk: PR #{}: {e}", p.number));
                    continue;
                }
            }
            self.desk_watch.set_note(repo_key, p.number, None);
            let nt = NewTask {
                title: prcard::card_title(p),
                objective: prcard::card_objective(p),
                repo_key: repo_key.into(),
                links: vec![p.url.clone()],
                board,
                pr_card: Some(PrCard {
                    pr: p.url.clone(),
                    author: p.author.clone(),
                }),
                ..NewTask::default()
            };
            match b.coord().create_task(&me, nt) {
                Ok(task) => {
                    let f = Found {
                        board: b.clone(),
                        task,
                    };
                    self.follow_pr_card(&lister, &repo, p, &f, verdicts);
                }
                Err(e) => logf(format_args!("desk: PR #{} card: {e}", p.number)),
            }
        }
    }

    /// Every PR card on the boards this device shows, open or not.
    fn pr_cards_found(&self, boards: &[Arc<BoardLog>]) -> Vec<Found> {
        let mut out = Vec::new();
        for b in boards {
            for t in b.coord().list_tasks(&TaskFilter::default()) {
                if t.pr_card.is_some() {
                    out.push(Found {
                        board: b.clone(),
                        task: t,
                    });
                }
            }
        }
        out
    }

    /// What [`prcard::choose_space`] needs of each space the person is in
    /// (§12.2).
    fn space_facts(&self, boards: &[Arc<BoardLog>], repo_key: &str) -> Vec<SpaceFacts> {
        let me = self.person_id();
        boards
            .iter()
            .map(|b| {
                let ms = b.verifier.membership();
                let active = ms
                    .members
                    .iter()
                    .filter(|p| ms.active(&p.person_id))
                    .count();
                let author_active = ms.active(&me);
                let repo_cards = b
                    .coord()
                    .list_tasks(&TaskFilter::default())
                    .into_iter()
                    .filter(|t| {
                        t.ask.is_none()
                            && t.pr_card.is_none()
                            && !t.withdrawn
                            && t.state != coord::CANCELED
                            && t.state != coord::FAILED
                            && !t.repo_key.is_empty()
                            && coord::same_repo_key(&t.repo_key, repo_key)
                    })
                    .map(|t| RepoCard {
                        id: t.id,
                        board: t.board,
                    })
                    .collect();
                SpaceFacts {
                    id: b.id.clone(),
                    author_active,
                    others: active.saturating_sub(usize::from(author_active)),
                    autonomy: self.inbox.autonomy(&b.id),
                    repo_cards,
                }
            })
            .collect()
    }

    /// Cancels the PR cards for one PR other than `keep`, the lowest id,
    /// that this person made (§12.2): two of their devices made one each
    /// before their rows met.
    fn cancel_duplicates(&self, me: &str, keep: &Found, cards: &[&Found]) {
        for f in cards {
            if f.task.id == keep.task.id
                || coord::is_terminal(&f.task.state)
                || f.task.created_by != me
            {
                continue;
            }
            let note = format!("duplicate of {}", keep.task.short_ref());
            if let Err(e) = f
                .board
                .coord()
                .set_task_state(me, &f.task.id, coord::CANCELED, &note)
            {
                logf(format_args!("desk: cancel duplicate {}: {e}", f.task.id));
            }
        }
    }

    /// Records on PR card `f` that its PR is open (so it follows a close
    /// and a reopen at every poll, §12.2), with GitHub's comments, reviews
    /// and files when the PR's `updatedAt` moved (§12.4), and relays what
    /// members said to the author's holders.
    fn follow_pr_card(
        self: &Arc<Self>,
        lister: &Arc<dyn crate::deskwatch::PrLister>,
        repo: &str,
        p: &DeskPr,
        f: &Found,
        verdicts: &[(i64, Verdict)],
    ) {
        let co = f.board.coord();
        let id = &f.task.id;
        let updated = p.updated_at.unix_milli_value();
        let mut check = prcard::open_check(&p.url, &p.review_decision);
        let mut act = None;
        // Only a live card has anything to read: a closed card waits for
        // its PR to reopen.
        if self.desk_watch.activity_due(id, &p.url, updated) {
            match lister.activity(repo, p.number) {
                Ok(a) => {
                    check = a.check(&p.url, &p.review_decision);
                    act = Some(a);
                }
                Err(e) => logf(format_args!("desk: PR #{} comments: {e}", p.number)),
            }
        }
        match co.record_pr_state(id, check) {
            Ok((_, ns)) => {
                for n in ns {
                    if let Some(e) =
                        notice::notice_err(&self.route_notice(&inbox::system_sender(), &n))
                    {
                        logf(format_args!("desk: pr notice {}: {e}", n.reference));
                    }
                }
                if let Some(a) = act {
                    self.desk_watch.activity_read(id, &p.url, updated);
                    self.relay_comments(f, p, &a, verdicts);
                }
            }
            Err(e) => logf(format_args!("desk: PR #{} on {id}: {e}", p.number)),
        }
    }

    /// Tells the holders of PR `p`'s author card what members said on the
    /// PR (§12.4): their words, quoted as GitHub's; a stranger's only
    /// counted. The author card is the one the desk mapped, never the PR
    /// card, which nobody holds.
    fn relay_comments(
        &self,
        f: &Found,
        p: &DeskPr,
        a: &prcard::Activity,
        verdicts: &[(i64, Verdict)],
    ) {
        let Some(Verdict::Card(m)) = verdicts.iter().find(|(n, _)| *n == p.number).map(|v| &v.1)
        else {
            return;
        };
        if m.task_id == f.task.id {
            return;
        }
        let (_, home) = self.desk_cards(false);
        let co: Arc<Coord> = home
            .get(&m.task_id)
            .cloned()
            .unwrap_or_else(|| self.coord.clone());
        if co.get_task(&m.task_id).is_some_and(|t| t.pr_card.is_some()) {
            return;
        }
        let members = self.board_logins(&f.board);
        let said: Vec<String> = co
            .comments(&m.task_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.by == crate::deskpost::IRORI)
            .map(|c| c.text)
            .collect();
        let said: Vec<&str> = said.iter().map(String::as_str).collect();
        let own = self.own_github_login();
        for post in prcard::relay(
            p.number,
            &p.url,
            a,
            &members,
            &own,
            f.task.created_at,
            &said,
        ) {
            self.desk_post(&co, &m.task_id, p.number, &post.text, true, Vec::new());
        }
    }

    /// The GitHub logins of the active members of the space `b` is on, as
    /// their profiles say (lower case).
    fn board_logins(&self, b: &BoardLog) -> Vec<String> {
        let ms = b.verifier.membership();
        ms.members
            .iter()
            .filter(|p| ms.active(&p.person_id))
            .map(|p| b.verifier.profile_github_login(&p.person_id))
            .map(|l| l.trim().to_ascii_lowercase())
            .filter(|l| !l.is_empty())
            .collect()
    }
}
