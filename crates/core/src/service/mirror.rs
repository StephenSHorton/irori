//! Leases across people (v0.5 §5.4): an agent's lease in a shared card's
//! repo is mirrored into that board before the private lease is taken, and
//! a release request goes to another person's holder in place of a steal.
//! Also the person's dedicated reviewers (§10), set from the app.

use std::sync::Arc;
use std::time::Duration;

use coord::{Comment, Notice};
use inbox::autonomy;
use store::kinds::AGENT_ACTOR_PREFIX;

use super::Services;
use super::boards::{BoardLog, lock};

impl Services {
    /// `agent`'s card key on `b` when it should mirror a lease on
    /// `resource` there: the card it already has (it holds or joined a
    /// shared card only through one), or, under the Autonomous preset
    /// (§7.3: every lease in a shared card's repo mirrors), a card minted
    /// now when a shared card is in that repo.
    fn mirror_card(&self, b: &BoardLog, agent: &str, resource: &str) -> Option<(String, bool)> {
        let agent = agent.strip_prefix("api:").unwrap_or(agent);
        let all = self.inbox.autonomy(&b.id) == autonomy::AUTONOMOUS;
        let have = lock(&b.maps).keys.get(agent).cloned();
        if let Some(id) = have {
            return Some((format!("{AGENT_ACTOR_PREFIX}{id}"), all));
        }
        if !all || b.coord().mirror_tasks("", resource, true).is_empty() {
            return None;
        }
        self.board_key(&b.id, agent).ok().map(|k| (k, all))
    }

    /// Mirrors `agent`'s lease on `resource` into every board where a
    /// shared card makes it mirror, and returns their tags. Another
    /// person's lease on any of them refuses it, and the mirrors already
    /// taken are released.
    pub(crate) fn mirror_acquire(
        &self,
        agent: &str,
        resource: &str,
        ttl: Duration,
        steal: bool,
    ) -> Result<Vec<String>, coord::Error> {
        let mut done: Vec<(Arc<BoardLog>, String)> = Vec::new();
        let mut tags = Vec::new();
        for b in self.boards.list() {
            let Some((card, all)) = self.mirror_card(&b, agent, resource) else {
                continue;
            };
            match b.coord().mirror_acquire(&card, resource, ttl, all, steal) {
                Ok(Some(_)) => {
                    tags.push(b.verifier.membership().tag);
                    done.push((b, card));
                }
                Ok(None) => {}
                Err(e) => {
                    for (b, card) in done {
                        let _ = b.coord().mirror_release(&card, resource);
                    }
                    return Err(e);
                }
            }
        }
        Ok(tags)
    }

    /// Releases `agent`'s mirrored lease on `resource` wherever it is.
    pub(crate) fn mirror_release(&self, agent: &str, resource: &str) {
        let agent = agent.strip_prefix("api:").unwrap_or(agent);
        for b in self.boards.list() {
            let Some(id) = lock(&b.maps).keys.get(agent).cloned() else {
                continue;
            };
            let _ = b
                .coord()
                .mirror_release(&format!("{AGENT_ACTOR_PREFIX}{id}"), resource);
        }
    }

    /// Asks the holder of another person's lease on `resource` to release
    /// it (§5.4: nobody steals another person's lease): a comment by `by`
    /// on the shared card the lease mirrors through, which the bridge
    /// carries to the holder. `by` is an agent's key or the person's; the
    /// first board holding such a lease takes it.
    pub(crate) fn request_release(
        &self,
        by: &str,
        resource: &str,
        why: &str,
    ) -> Result<(String, Comment, Notice), coord::Error> {
        let resource = coord::normalize_resource(resource)?;
        let resource = resource.as_str();
        let mut last = None;
        for b in self.boards.list() {
            let co = b.coord();
            if !co.leases().iter().any(|l| l.resource == resource) {
                continue;
            }
            let key = self
                .board_key(&b.id, by)
                .map_err(super::boards::Error::into_coord)?;
            match co.request_release(&key, resource, why) {
                Ok((c, n)) => return Ok((b.id.clone(), c, n)),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| {
            coord::Error::new(
                coord::ErrorKind::NotFound,
                format_args!(
                    "no shared board holds a lease on {}",
                    gocompat::quote::quote(resource)
                ),
            )
        }))
    }
}

/// One of the person's dedicated reviewers on a board (§10).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardReviewer {
    /// The repoKey it reviews, or "" for the whole board.
    pub repo: String,
    /// The agent's session key on this person's devices.
    pub agent: String,
    /// Its board card's name.
    pub name: String,
}

impl super::Core {
    /// Dedicates `agent` (one of the person's sessions) as their reviewer
    /// on `board_id`, for `repo` or the whole board; an empty agent clears
    /// it. Only the person sets it, in the app (§10).
    pub fn set_board_reviewer(
        &self,
        board_id: &str,
        agent: &str,
        repo: &str,
    ) -> super::boards::Result<()> {
        let b = self.s.board(board_id)?;
        let card = if agent.trim().is_empty() {
            String::new()
        } else {
            self.s.board_key(board_id, agent)?
        };
        b.coord()
            .set_reviewer(&self.s.person_actor(), &card, repo)
            .map_err(Into::into)
    }

    /// The person's dedicated reviewers on `board_id`, by repo.
    pub fn board_reviewers(&self, board_id: &str) -> Vec<BoardReviewer> {
        let Ok(b) = self.s.board(board_id) else {
            return Vec::new();
        };
        let me = self.s.person_id();
        b.coord()
            .reviewers_of(&me)
            .into_iter()
            .map(|(repo, card)| {
                let id = card.strip_prefix(AGENT_ACTOR_PREFIX).unwrap_or(&card);
                let name = lock(&b.maps)
                    .cards
                    .get(id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                BoardReviewer {
                    repo,
                    agent: self.s.private_key(&card),
                    name,
                }
            })
            .collect()
    }
}
