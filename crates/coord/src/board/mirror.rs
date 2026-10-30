//! Mirrored leases on a shared board (v0.5 §5.4): an agent working a
//! shared task mirrors its leases in that task's repo into the board log,
//! where another person's agents meet them. Nobody steals another
//! person's lease; a release request goes to its holder on the shared card
//! instead. The Core's board router calls these on the board's coord with
//! the agent's card for that board.

use std::time::Duration;

use gocompat::time::GoTime;

use super::MAX_COMMENT;
use super::comments::check_reason;
use super::error::{Result, held, invalid, not_allowed, not_found};
use super::fold::{Fold, LeaseRec, is_holder};
use super::resource::{normalize_resource, parse_resource};
use super::types::*;
use super::util::{caller, collapse_space};
use crate::gotime;

/// The opening of a release request's comment.
pub const RELEASE_REQUEST: &str = "Release request";

impl Fold {
    /// The person a `person:`/`agent:` key is on this board, `""` off one.
    pub(crate) fn person_of(&self, key: &str) -> String {
        match (&self.key_person, self.on_board) {
            (Some(kp), true) => kp(key),
            _ => String::new(),
        }
    }

    /// Another person's lease: on a board, its holder's person isn't by's.
    pub(crate) fn others_lease(&self, by: &str, l: &LeaseRec) -> bool {
        if !self.on_board {
            return false;
        }
        let p = self.person_of(&l.lease.holder);
        p.is_empty() || p != self.person_of(by)
    }

    /// The refusal for taking another person's lease.
    pub(crate) fn not_yours(&self, l: &Lease) -> super::error::Error {
        not_allowed(format!(
            "{} is leased by {}, another person's agent; nobody steals another person's lease, so ask its holder to release it (a release request)",
            l.resource, l.holder
        ))
    }

    /// The shared cards on this board that make card's lease on resource
    /// mirror here: cards in the resource's repo that card holds or has
    /// joined (or every card in that repo, when `all`). A card without a
    /// repo, a port or a named resource mirrors nothing.
    pub(crate) fn mirror_tasks(&self, card: &str, resource: &str, all: bool) -> Vec<String> {
        if !self.on_board {
            return Vec::new();
        }
        let Ok(res) = parse_resource(resource) else {
            return Vec::new();
        };
        if res.repo_key.is_empty() {
            return Vec::new();
        }
        self.tasks
            .values()
            .filter(|t| {
                !is_terminal(&t.state) && t.repo_key == res.repo_key && (all || is_holder(t, card))
            })
            .map(|t| t.id.clone())
            .collect()
    }

    /// Mirrors card's lease on resource into the board, when a shared card
    /// makes it mirror: `None` when nothing does. Another person's live
    /// lease refuses it (and the refusal shows as a lease card), so the
    /// caller refuses the private lease too.
    pub(crate) fn mirror_acquire(
        &mut self,
        card: &str,
        resource: &str,
        ttl: Duration,
        all: bool,
    ) -> Result<Option<Lease>> {
        if !self.on_board {
            return Err(invalid("leases mirror only into a shared board"));
        }
        if self.mirror_tasks(card, resource, all).is_empty() {
            return Ok(None);
        }
        self.acquire(card, resource, ttl, false).map(Some)
    }

    /// Mirrors the release of card's lease on resource: false when the
    /// board holds no lease of card's there.
    pub(crate) fn mirror_release(&mut self, card: &str, resource: &str) -> Result<bool> {
        let name = normalize_resource(resource)?;
        let now = self.stamp();
        if !self
            .held
            .get(&name)
            .is_some_and(|l| l.lease.holder == card && self.live(l, &now))
        {
            return Ok(false);
        }
        self.release(card, &name).map(|()| true)
    }

    /// Asks the holder of another person's lease on resource to release
    /// it: a comment from by on the shared card the lease mirrors through,
    /// which reaches the holder at its next turn (the bridge carries it to
    /// the other person's agent, §7.1).
    pub(crate) fn request_release(
        &mut self,
        by: &str,
        resource: &str,
        why: &str,
        now: &GoTime,
    ) -> Result<(Comment, Notice)> {
        let by = caller(by)?;
        let name = normalize_resource(resource)?;
        let why = collapse_space(why);
        check_reason(&why, false)?;
        let Some(l) = self.held.get(&name).filter(|l| self.live(l, now)).cloned() else {
            return Err(not_found(format!("no lease on {name}")));
        };
        if !self.others_lease(&by, &l) {
            return Err(held(format!(
                "{name} is leased by {}, one of your own; release or steal it instead",
                l.lease.holder
            )));
        }
        let mut ids = self.mirror_tasks(&l.lease.holder, &name, false);
        // The card touched last, so the request lands where the work is.
        ids.sort_by(|a, b| gotime::cmp(&self.tasks[b].updated_at, &self.tasks[a].updated_at));
        let Some(id) = ids.first().cloned() else {
            return Err(not_found(format!(
                "{} holds no shared card in {name}'s repo to ask on",
                l.lease.holder
            )));
        };
        let mut text = format!(
            "{RELEASE_REQUEST}: {by} asks {} to release {name}",
            l.lease.holder
        );
        if !why.is_empty() {
            text.push_str(&format!(" ({why})"));
        }
        text.push_str(". Release it with the release tool when you can.");
        if text.chars().count() > MAX_COMMENT {
            return Err(invalid("release request too long"));
        }
        self.add_comment(&by, &id, &text, false, Vec::new())
    }
}
