//! The seams to the parts other milestones build. Each trait is the
//! smallest thing the window needs; each default is what this build does
//! without that part. A merge replaces a default with the real one in
//! [`Seams`] (see the wiring notes in the crate docs).

use std::sync::Arc;

use blob::FileRef;

use crate::devices::{DeviceView, HomeServerView, PairStatus};
use crate::invites::InviteStatus;
use crate::settings::NotifyOptions;
use crate::update::UpdateStatus;

/// The mesh (M5): the device rows with their live state, the home server,
/// and files held by other devices.
pub trait Mesh: Send + Sync {
    /// The person's devices, this one first (Go `Mesh().Devices(false)`),
    /// without their colours (the window adds those).
    fn devices(&self) -> Vec<DeviceView>;
    fn home_server(&self) -> HomeServerView;
    fn set_home_server(&self, on: bool, outside: bool, public: &str) -> Result<(), String>;
    fn set_home_server_awake(&self, on: bool) -> Result<(), String>;
    fn remove_device(&self, device_id: &str) -> Result<(), String>;
    /// Makes sure the file is on this device, fetching it from a device
    /// that holds it first (Go `mesh.FetchFile`). `Err` is the window's
    /// text: offline holders, a timeout, anything else.
    fn fetch_file(&self, r: &FileRef) -> Result<(), FetchError>;
    /// Tells the mesh this device holds a new file (Go `mesh.Offer`).
    fn offer(&self, r: &FileRef) -> Result<(), String>;
}

/// Why a file could not be brought to this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// No device that holds it is connected; names them.
    Offline(String),
    TimedOut,
    Other(String),
}

/// Pairing (M7): Add a device, Join with a code, the words.
pub trait Pairing: Send + Sync {
    fn status(&self) -> PairStatus;
    fn add(&self, role: &str) -> Result<PairStatus, String>;
    fn join(&self, code: &str, server: &str) -> Result<PairStatus, String>;
    fn confirm_words(&self, words: &str) -> Result<(), String>;
    fn confirm_key(&self, key: &str) -> Result<(), String>;
    fn cancel(&self);
    /// Calls `f` with every pairing change (`devices:pairing`); `None`
    /// stops.
    fn watch(&self, _f: Option<Arc<dyn Fn(PairStatus) + Send + Sync>>) {}
}

/// Invites to a shared board (#44, design §4.1): one invite at a time on
/// this device, as inviter or invitee.
pub trait Invites: Send + Sync {
    /// Boards → a board → Invite (owners): opens the invite window; the
    /// status carries the link to send.
    fn invite(&self, board_id: &str) -> Result<InviteStatus, String>;
    /// Boards → Join with an invite: the pasted link.
    fn join(&self, link: &str) -> Result<InviteStatus, String>;
    /// The inviter types the five words the invitee reads out.
    fn confirm_words(&self, typed: &str) -> Result<InviteStatus, String>;
    fn status(&self) -> InviteStatus;
    fn cancel(&self);
    /// Calls `f` with every invite change (`boards:invite`); `None` stops.
    fn watch(&self, _f: Option<Arc<dyn Fn(InviteStatus) + Send + Sync>>) {}
}

/// The notifier (M6 toast, Go `notify.Notifier`): the settings and the
/// sample card.
pub trait Notifier: Send + Sync {
    fn options(&self) -> NotifyOptions;
    fn set_options(&self, o: NotifyOptions) -> Result<NotifyOptions, String>;
    /// Shows the sample card (Go `SettingsService.TestNotification`).
    fn test(&self) -> Result<(), String>;
}

/// Launch at login (M6 shell, D9: own winreg / SMAppService code).
pub trait LoginItem: Send + Sync {
    fn enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
}

/// The updater (M8).
pub trait Updater: Send + Sync {
    fn status(&self) -> UpdateStatus;
    fn supported(&self) -> bool;
    fn check_and_download(&self) -> UpdateStatus;
    fn restart_to_update(&self) -> Result<(), String>;
}

/// Every seam, with this build's defaults.
#[derive(Clone)]
pub struct Seams {
    pub mesh: Arc<dyn Mesh>,
    pub pairing: Arc<dyn Pairing>,
    pub invites: Arc<dyn Invites>,
    pub notifier: Arc<dyn Notifier>,
    pub login: Arc<dyn LoginItem>,
    pub updater: Arc<dyn Updater>,
}
