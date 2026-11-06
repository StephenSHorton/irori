//! SyncService (card #580, no-server.md §4.3): how fresh the private log
//! and each shared space are, for the window's "Last synced with your PC
//! 3h ago" lines.

use std::time::{Duration, UNIX_EPOCH};

use gocompat::time::GoTime;
use serde::Serialize;
use specta::Type;

use crate::Window;

/// One log's freshness.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogFreshness {
    /// "personal", or a shared space's id.
    pub log: String,
    /// When this device was last in step with another device on it; left
    /// out when never, as far as it knows.
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    #[specta(optional, type = crate::ts::Time)]
    pub at: GoTime,
    /// That device as the person says it: "your PC", or "Blake".
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub with: String,
    /// Another device holds this log at all.
    pub shared: bool,
}

impl From<irori_core::LogFreshness> for LogFreshness {
    fn from(f: irori_core::LogFreshness) -> LogFreshness {
        LogFreshness {
            log: f.log,
            at: if f.at_ms > 0 {
                GoTime::from_system(UNIX_EPOCH + Duration::from_millis(f.at_ms as u64))
            } else {
                GoTime::default()
            },
            with: f.with,
            shared: f.shared,
        }
    }
}

impl Window {
    /// SyncService.Freshness: every log's freshness, Personal first.
    pub fn freshness(&self) -> Vec<LogFreshness> {
        self.core.freshness().into_iter().map(Into::into).collect()
    }
}
