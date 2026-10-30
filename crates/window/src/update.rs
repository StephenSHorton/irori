//! The updater's window face (I:internal/update/service.go). The updater
//! itself is M8; this build is `dev` or unpublished, so it never offers an
//! update (surface.md 7.20) and the status stays idle.

use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::seams::Updater;

/// The update:status payload (Go `update.Status`).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct UpdateStatus {
    /// idle, checking, downloading, ready, installing or error.
    pub phase: String,
    pub version: String,
    pub notes: String,
    pub error: String,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        UpdateStatus {
            phase: "idle".into(),
            version: String::new(),
            notes: String::new(),
            error: String::new(),
        }
    }
}

/// The update:status event.
pub const EVENT_UPDATE_STATUS: &str = "update:status";

/// The updater this build has: none.
pub struct NoUpdater;

impl Updater for NoUpdater {
    fn status(&self) -> UpdateStatus {
        UpdateStatus::default()
    }
    fn supported(&self) -> bool {
        false
    }
    fn check_and_download(&self) -> UpdateStatus {
        UpdateStatus::default()
    }
    fn restart_to_update(&self) -> Result<(), String> {
        Err("no update ready to install (phase \"idle\")".into())
    }
}

impl Window {
    pub fn update_status(&self) -> UpdateStatus {
        self.seams.updater.status()
    }
    pub fn update_supported(&self) -> bool {
        self.seams.updater.supported()
    }
    pub fn update_check_and_download(&self) -> UpdateStatus {
        self.seams.updater.check_and_download()
    }
    pub fn update_restart(&self) -> Result<(), String> {
        self.seams.updater.restart_to_update()
    }
}
