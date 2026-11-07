//! The Mac's Screen Recording grant (TCC `kTCCServiceScreenCapture`).
//!
//! ScreenCaptureKit only captures other apps' windows for a process the
//! person has allowed in System Settings > Privacy & Security > Screen &
//! System Audio Recording. The grant is keyed to the app's code signature:
//! a Developer ID build keeps it across updates, an ad-hoc build loses it on
//! every rebuild (docs/research/08-voice-calls/screen-share.md, Results).

/// Whether this process holds the grant now. Never prompts.
pub fn granted() -> bool {
    objc2_core_graphics::CGPreflightScreenCaptureAccess()
}

/// Ask for the grant. The first call for an app shows the system prompt and
/// adds the app, switched off, to the Settings list; it returns at once,
/// before the person answers, and the app must restart to see a yes.
pub fn request() -> bool {
    objc2_core_graphics::CGRequestScreenCaptureAccess()
}

/// The Settings pane where the person switches the grant on.
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
