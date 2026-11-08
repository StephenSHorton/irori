//! What the Rune window's shared card and agent widgets (app/src/widgets,
//! card #674) read that had no `Window` method: the Tauri commands
//! `roster_check_slots`, `devices_colors` and `devices_agent_avatars` did
//! it inline.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::Window;

/// One of this device's check slots, held by a `scripts/check.sh` run
/// (card #437).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CheckSlot {
    /// `check/<host>/slot-<n>`.
    pub resource: String,
    /// The run holding it (`check-irori-437-19174`).
    pub run: String,
    /// When the lease runs out unless renewed (RFC 3339).
    pub until: String,
}

impl Window {
    /// This device's check slots held now, by `scripts/check.sh` runs
    /// (the `roster_check_slots` command).
    pub fn check_slots(&self) -> Vec<CheckSlot> {
        let prefix = format!("{}/{}", self.core.device_id(), coord::RUN_HOLDER);
        self.core
            .coord()
            .leases()
            .into_iter()
            .filter(|l| l.resource.starts_with("check/"))
            .filter_map(|l| {
                let run = l.holder.strip_prefix(&prefix)?.to_string();
                Some(CheckSlot {
                    resource: l.resource.clone(),
                    run,
                    until: l.until.format_rfc3339(),
                })
            })
            .collect()
    }

    /// The device colours a person can pick (`devices_colors`).
    pub fn device_palette(&self) -> Vec<String> {
        coord::devcolor::PALETTE
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Agents' own faces by name key, open-bot-avatar JSON (card #407,
    /// `devices_agent_avatars`).
    pub fn agent_avatars(&self) -> BTreeMap<String, String> {
        self.core.agent_avatars().into_iter().collect()
    }
}
