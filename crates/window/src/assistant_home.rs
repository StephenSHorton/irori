//! The one assistant's computer (one-assistant.md §3, card 1): which of the
//! person's computers runs the assistant, and whether it can be reached.
//! The Core folds the `assistant.home` rows; this is what the chat and
//! Settings read, and the `assistant:home` payload.

use serde::Serialize;
use specta::Type;

use crate::Window;

pub const EVENT_ASSISTANT_HOME: &str = "assistant:home";

/// `assistant_home_status`, and the `assistant:home` payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistantHomeStatus {
    /// The assistant's computer, "" when none is recorded yet.
    pub device: String,
    /// What it is called, "" with none.
    pub name: String,
    /// This device is it.
    pub here: bool,
    /// `connected`, `recent` or `offline`; "" with none.
    pub state: String,
    /// When it was last heard from, Unix ms; 0 for never.
    pub last_seen_ms: i64,
    /// Its newest `spawn.host` says it can find Claude Code.
    pub claude: bool,
    /// Its newest `spawn.host` says it answers the chat for other devices.
    pub chat: bool,
    /// With none recorded: `setting_up`, `needs_update` or `no_claude`.
    /// "" otherwise.
    pub none: String,
    /// The computers that can run the assistant (they say `assistantTurn`
    /// and `assistantChat`), this one first.
    pub candidates: Vec<HomeCandidate>,
}

/// One computer the assistant can run on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeCandidate {
    pub device: String,
    pub name: String,
    pub here: bool,
    /// `connected`, `recent` or `offline`.
    pub state: String,
    /// Unix ms; 0 for never.
    pub last_seen_ms: i64,
}

fn ms(t: gocompat::time::GoTime) -> i64 {
    if t.is_zero() {
        0
    } else {
        t.unix_nano() / 1_000_000
    }
}

impl From<irori_core::assistant_home::HomeStatus> for AssistantHomeStatus {
    fn from(s: irori_core::assistant_home::HomeStatus) -> Self {
        AssistantHomeStatus {
            device: s.device,
            name: s.name,
            here: s.here,
            state: s.state,
            last_seen_ms: ms(s.last_seen),
            claude: s.claude,
            chat: s.chat,
            none: s.none,
            candidates: s
                .candidates
                .into_iter()
                .map(|c| HomeCandidate {
                    device: c.device,
                    name: c.name,
                    here: c.here,
                    state: c.state,
                    last_seen_ms: ms(c.last_seen),
                })
                .collect(),
        }
    }
}

impl Window {
    /// Where the one assistant runs, and whether it can be reached.
    pub fn assistant_home_status(&self) -> AssistantHomeStatus {
        self.core.assistant_home_status().into()
    }

    /// The person picks the assistant's computer.
    pub fn assistant_home_set(&self, device: &str) -> Result<(), String> {
        self.core.set_assistant_home(device)
    }
}
