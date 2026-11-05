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
        let agents = self.presence.snapshot();
        let waiting: std::collections::HashMap<String, String> = agents
            .iter()
            .filter(|a| on_permission_prompt(a))
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
                             the command to the space's allow list in Settings › Agents and \
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
                (None, Some(b)) => {
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
}
