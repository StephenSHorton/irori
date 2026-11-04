//! Progress resets a chain's hop count (card #488). The hop limit is a
//! runaway-loop guard: two agents messaging each other with nothing to
//! show for it. Once either of them ticks a step, changes a card's state
//! or commits, the back-and-forth is getting somewhere, so the next
//! message in the chain starts again at hop 0.
//!
//! Nothing here is folded from the log: the hop is baked into each
//! `message.sent` row when it is written, so what progress was seen only
//! matters at send time. After a restart a chain just isn't reset until
//! the next progress row arrives.

use gocompat::time::GoTime;
use serde::Deserialize;
use store::Event;
use store::kinds::{AGENT_ACTOR_PREFIX, CAPTURE_COMMITCMD, COMMIT_LINKED, TASK_STATUS, TASK_STEP};

use crate::Inbox;
use crate::state::Inner;

/// Who made the progress a row records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressBy {
    /// The row's actor, `agent:<…>`: on this device's own log that is the
    /// agent's inbox key; on a shared board the caller maps it.
    Actor(String),
    /// An agent's inbox key the payload names (a commit).
    Key(String),
}

#[derive(Deserialize)]
struct StepRow {
    #[serde(default)]
    done: Option<bool>,
}

#[derive(Deserialize)]
struct CommitCmdRow {
    #[serde(default)]
    key: String,
}

#[derive(Deserialize)]
struct Committer {
    #[serde(default)]
    key: String,
}

#[derive(Deserialize)]
struct CommitLinkedRow {
    #[serde(default)]
    committer: Option<Committer>,
}

/// The progress `ev` records, if any, and when: an agent ticking a step
/// done, an agent setting a card's state, or a commit an agent made.
/// Unticking, giving a step an owner, and everything the person does are
/// not an agent's progress.
pub fn progress_of(ev: &Event) -> Option<(ProgressBy, GoTime)> {
    let at = GoTime::unix_milli(ev.hlc.wall_ms());
    let actor = || {
        ev.actor
            .starts_with(AGENT_ACTOR_PREFIX)
            .then(|| ProgressBy::Actor(ev.actor.clone()))
    };
    let by = match ev.kind.as_str() {
        TASK_STEP => {
            let r: StepRow = serde_json::from_slice(&ev.payload).ok()?;
            if r.done != Some(true) {
                return None;
            }
            actor()?
        }
        TASK_STATUS => actor()?,
        CAPTURE_COMMITCMD => {
            let r: CommitCmdRow = serde_json::from_slice(&ev.payload).ok()?;
            ProgressBy::Key(r.key)
        }
        COMMIT_LINKED => {
            let r: CommitLinkedRow = serde_json::from_slice(&ev.payload).ok()?;
            ProgressBy::Key(r.committer?.key)
        }
        _ => return None,
    };
    match &by {
        ProgressBy::Key(k) if k.is_empty() => None,
        _ => Some((by, at)),
    }
}

impl Inner {
    /// Whether agent `key` made progress after `t`.
    pub(crate) fn progressed_since(&self, key: &str, t: &GoTime) -> bool {
        !key.is_empty()
            && self
                .progress
                .get(key)
                .is_some_and(|at| at.unix_nano() > t.unix_nano())
    }

    /// Whether any of `keys` made progress after `t`.
    pub(crate) fn chain_progressed(&self, keys: &[&str], t: &GoTime) -> bool {
        keys.iter().any(|k| self.progressed_since(k, t))
    }
}

impl Inbox {
    /// Records that agent `key` (its inbox key) made progress at `at`.
    pub fn progressed(&self, key: &str, at: GoTime) {
        if key.is_empty() {
            return;
        }
        self.mutate(|b| {
            let e = b.progress.entry(key.to_string()).or_insert(GoTime::ZERO);
            if at.unix_nano() > e.unix_nano() {
                *e = at;
            }
        });
    }

    /// Records the progress `ev`, a row of this device's own log, shows.
    pub(crate) fn note_progress(&self, ev: &Event) {
        match progress_of(ev) {
            Some((ProgressBy::Actor(a), at)) => {
                if let Some(k) = a.strip_prefix(AGENT_ACTOR_PREFIX) {
                    self.progressed(k, at);
                }
            }
            Some((ProgressBy::Key(k), at)) => self.progressed(&k, at),
            None => {}
        }
    }
}
