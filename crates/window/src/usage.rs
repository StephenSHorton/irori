//! Claude usage left in the top bar (card #599): the plan's rate-limit
//! windows (5-hour, weekly, a gateway's spend limit) as this device's
//! Claude Code sessions last measured them. The Irori mod posts them to
//! the Core (`POST /v1/usage/limits`, [`irori_core::usage`]); the Core
//! keeps the newest across restarts, and every reading is `usage:changed`.
//!
//! A window past its reset time is left out here; one that passes it while
//! the page shows it is the page's to show as reset.

use serde::Serialize;
use specta::Type;

use crate::Window;

pub const EVENT_USAGE_CHANGED: &str = "usage:changed";

/// Claude usage: `usage_limits`, and the `usage:changed` payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageLimits {
    /// When the newest reading came in, Unix ms; 0 for none yet.
    pub observed_at_ms: i64,
    pub windows: Vec<UsageWindow>,
}

/// One rate-limit window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// `five_hour`, `seven_day`, or another (a gateway's `spend_limit`).
    pub kind: String,
    /// How much is used, 0 to 100 (past 100 on an exceeded spend limit).
    /// Always finite (the Core refuses anything else), so `number`, never
    /// the `null` specta gives an f64 for NaN.
    #[specta(type = i64)]
    pub percent_used: f64,
    /// When it resets, Unix ms; null when the session didn't say.
    pub resets_at_ms: Option<i64>,
}

impl From<irori_core::usage::UsageLimits> for UsageLimits {
    fn from(u: irori_core::usage::UsageLimits) -> UsageLimits {
        UsageLimits {
            observed_at_ms: u.observed_at_ms,
            windows: u
                .windows
                .into_iter()
                .map(|w| UsageWindow {
                    kind: w.kind,
                    percent_used: w.percent_used,
                    resets_at_ms: w.resets_at_ms,
                })
                .collect(),
        }
    }
}

impl Window {
    /// Claude usage left, as the Core last heard it.
    pub fn usage_limits(&self) -> UsageLimits {
        self.core.usage_limits().into()
    }
}
