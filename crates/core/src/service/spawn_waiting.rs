//! "Waiting for your permission" (card #449, docs/design/spawned-agents.md
//! §7 "Seeing a permission prompt", §14 part 5).
//!
//! A spawned agent nobody is attached to can stop on a permission prompt
//! and would look busy forever. Claude Code's session file says so: status
//! `waiting`, `waitingFor: "permission prompt"` (card #445, on presence as
//! `AgentCard::waiting_for`). Each sweep raises one blocker on the card the
//! agent was started for, on the person, with the way out (Attach, or add
//! the command to the board's allow list), and resolves it once the agent
//! is past the prompt or gone.
//!
//! Every device sweeps, but presence only knows this device's agents. So
//! only a device that runs one of the card's agents raises or resolves its
//! blocker (card #603): before that, the PC resolved the Mac agent's
//! blocker every sweep and the Mac raised it again.

use coord::roster::Agents as _;
use coord::{BlockedBy, BlockerAnswer, NewBlocker, TaskFilter};

use super::Services;
use crate::logf;

/// The blocker's question; also how a sweep finds its own open one.
pub const WAITING_PERMISSION: &str = "Waiting for your permission";

/// What Claude Code writes while a tool call waits on a person.
pub const PERMISSION_PROMPT: &str = "permission prompt";

const SWEEPER: &str = "irori";

/// Whether presence says the agent is stopped on a permission prompt.
pub fn on_permission_prompt(a: &presence::AgentCard) -> bool {
    a.status == "waiting" && a.waiting_for == PERMISSION_PROMPT
}

impl Services {
    /// Raises or resolves each spawned card's "Waiting for your
    /// permission" blocker from its agent's presence.
    pub(crate) fn sweep_permission_waits(&self) {
        let me = self.device.id.as_str();
        let agents = self.presence.snapshot();
        let waiting: std::collections::HashMap<String, String> = agents
            .iter()
            .filter(|a| on_permission_prompt(a) && runs_here(&a.key, &a.device_id, me))
            .filter_map(|a| self.coord.spawned_card(&a.key).map(|t| (t, a.key.clone())))
            .collect();
        let open = self.coord.list_tasks(&TaskFilter {
            open: true,
            ..TaskFilter::default()
        });
        for t in open {
            let open = t.blockers.iter().find(|b| {
                b.open() && b.raised_by == SWEEPER && b.question.starts_with(WAITING_PERMISSION)
            });
            match (waiting.get(&t.id), open) {
                (Some(key), None) => {
                    let name = {
                        let n = self.roster.name_of(key);
                        if n.is_empty() {
                            "Its agent".to_string()
                        } else {
                            n
                        }
                    };
                    let nb = NewBlocker {
                        question: format!("{WAITING_PERMISSION}: {name} is stopped on a prompt"),
                        context: format!(
                            "{name}, the agent Irori started for {}, is waiting for a permission \
                             nobody is there to give. Attach to it to answer the prompt, or add \
                             the command to the board's allow list in Settings › Agents and \
                             start a fresh agent.",
                            t.short_ref()
                        ),
                        blocked_by: Some(BlockedBy {
                            kind: coord::BLOCKED_ON_PERSON.into(),
                            ..BlockedBy::default()
                        }),
                        ..NewBlocker::default()
                    };
                    if let Err(e) = self.coord.raise(SWEEPER, &t.id, nb) {
                        logf(format_args!("permission wait on {}: {e}", t.short_ref()));
                    }
                }
                (None, Some(b)) if self.decides_permission_wait(&t, me) => {
                    let a = BlockerAnswer {
                        text: "The agent is past the prompt.".into(),
                        resolve: true,
                        ..BlockerAnswer::default()
                    };
                    if let Err(e) = self.coord.answer(SWEEPER, &t.id, &b.id, a) {
                        logf(format_args!("permission wait on {}: {e}", t.short_ref()));
                    }
                }
                _ => {}
            }
        }
    }

    /// Whether this device may resolve card t's permission wait: it runs
    /// one of the card's agents (its own, a step's or a subtask's), and no
    /// agent of the card on another device is on a prompt by the roster.
    /// A device that runs none of them can't tell whether the agent is
    /// past the prompt, so it leaves the blocker to the one that can.
    fn decides_permission_wait(&self, t: &coord::Task, me: &str) -> bool {
        if !t.spawns.iter().any(|s| runs_here(&s.key, &s.device, me)) {
            return false;
        }
        let elsewhere: Vec<&str> = t
            .spawns
            .iter()
            .filter(|s| !s.key.is_empty() && !runs_here(&s.key, &s.device, me))
            .map(|s| s.key.as_str())
            .collect();
        elsewhere.is_empty()
            || !self
                .roster
                .agents()
                .iter()
                .any(|a| elsewhere.contains(&a.key.as_str()) && roster_on_prompt(a))
    }
}

/// Whether agent `key` (on `device`, when known) runs on device `me`.
/// Agent keys are `<device>/<session>`.
fn runs_here(key: &str, device: &str, me: &str) -> bool {
    if !device.is_empty() {
        return device == me;
    }
    key.split_once('/').is_some_and(|(d, _)| d == me)
}

/// [`on_permission_prompt`] for the mesh roster's card of an agent.
fn roster_on_prompt(a: &coord::roster::AgentCard) -> bool {
    a.status == "waiting" && a.waiting_for == PERMISSION_PROMPT
}
