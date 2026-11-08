//! `irori-term`: live terminals on cards (docs/design/inline-terminals.md,
//! slice S1, card #369).
//!
//! The Core owns every pane here, not the window, so a shell outlives the
//! card page that showed it (design section 2). A pane is one rsz-term
//! `Pane`; its frames go to the one local view attached to it as raw
//! bytes, everything else as [`TermMsg`] JSON.
//!
//! The public API is the same on every target. On Android and iOS the PTY
//! half is a stub that answers "not on this device" (section 2), so the
//! window's `term_*` commands and the bindings never change by target.
//!
//! S1 leaves out what later slices add: other devices' viewers and the
//! per-viewer queues (S6), agents' `term_run` (S4), takeover (S5) and the
//! quit-time deadline joins (S2).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod pty;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use pty::Terms;

#[cfg(any(target_os = "android", target_os = "ios"))]
mod stub;
#[cfg(any(target_os = "android", target_os = "ios"))]
pub use stub::Terms;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod replay;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use replay::replay_text;

/// The text a recorded terminal stream leaves on screen (card #741). A
/// phone has no VT engine and no `claude` to record from; it gives the
/// bytes back as text.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn replay_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// A terminal's id on this device.
pub type TermId = u32;

/// What a terminal belongs to (design section 6.1). A scope can have
/// several terminals on a device (Stephen, #369 b5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum Scope {
    /// A card, by its task id.
    Card(String),
    /// An agent session, by its key (S3).
    Agent(String),
    /// The person's own terminal on this device, tied to no card: the
    /// bottom panel's shell on pages other than a card's (card #734).
    Device,
}

/// What `term_open` answers.
#[derive(Clone, Debug, Serialize, Type)]
pub struct TermInfo {
    pub id: TermId,
    pub scope: Scope,
    /// The folder the shell started in.
    pub cwd: String,
    /// The shell's title, as the program last set it.
    pub title: String,
    /// The shell still runs (false: it exited, and the view offers Restart).
    pub running: bool,
}

/// How a terminal is opened.
#[derive(Clone, Debug)]
pub struct OpenSpec {
    pub scope: Scope,
    pub cols: u16,
    pub rows: u16,
    /// The folder; `None` is the home folder.
    pub cwd: Option<String>,
    /// A program to run instead of the person's shell, as argv (card
    /// #456: `claude attach <id>` in a card's tab). `None` runs the shell.
    pub command: Option<Vec<String>>,
}

/// Everything a view gets besides frames, as JSON. The shapes follow
/// suzuri's `Msg` (`suzuri:app/src/main.rs:50-76`), so suzuri's view code
/// reads them as they are.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
#[serde(tag = "t")]
pub enum TermMsg {
    #[serde(rename = "title")]
    Title { title: String },
    #[serde(rename = "bell")]
    Bell,
    /// OSC 52: a program asked to copy. The view offers it; Irori never
    /// writes the clipboard on its own (design section 5).
    #[serde(rename = "clip")]
    Clipboard { text: String },
    #[serde(rename = "exit")]
    Exit { code: Option<u32> },
    #[serde(rename = "shell")]
    Shell {
        integrated: bool,
        cwd: String,
        program: bool,
        prompts: u64,
    },
    #[serde(rename = "done")]
    CommandDone { exit: Option<i32>, command: String },
    /// An image's bytes (base64), once, before it is placed.
    #[serde(rename = "img")]
    Image {
        id: u32,
        name: String,
        path: Option<String>,
        format: u32,
        w: u32,
        h: u32,
        z: bool,
        data: String,
    },
    #[serde(rename = "imgdrop")]
    ImageDrop { ids: Vec<u32> },
    /// Images on screen: [image, row, col, cols, rows, z, inline, crop x, y, w, h].
    #[serde(rename = "imgpos")]
    Placements { list: Vec<[i64; 11]> },
    #[serde(rename = "progress")]
    Progress { state: u8, percent: u8 },
}

/// The view a pane's output goes to. Each callback answers false once its
/// view is gone (the window's channel closed), and the pane then forgets it.
/// Both are called from rsz-term's threads, so they must not block.
pub struct Viewer {
    pub frame: Box<dyn Fn(Vec<u8>) -> bool + Send + Sync>,
    pub msg: Box<dyn Fn(TermMsg) -> bool + Send + Sync>,
}

impl Viewer {
    pub fn new(
        frame: impl Fn(Vec<u8>) -> bool + Send + Sync + 'static,
        msg: impl Fn(TermMsg) -> bool + Send + Sync + 'static,
    ) -> Arc<Viewer> {
        Arc::new(Viewer {
            frame: Box::new(frame),
            msg: Box::new(msg),
        })
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) const NOT_HERE: &str = "terminals are not on this device";
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) const NO_TERM: &str = "no such terminal";

#[cfg(test)]
mod tests {
    use super::Scope;

    #[test]
    fn scopes_keep_their_wire_shape() {
        // The window's bindings send these as they are (card #734 adds
        // the device scope, which has no id).
        let j = |s: &Scope| serde_json::to_string(s).unwrap();
        assert_eq!(j(&Scope::Card("t1".into())), r#"{"kind":"card","id":"t1"}"#);
        assert_eq!(j(&Scope::Device), r#"{"kind":"device"}"#);
        let back: Scope = serde_json::from_str(r#"{"kind":"device"}"#).unwrap();
        assert_eq!(back, Scope::Device);
        let back: Scope = serde_json::from_str(r#"{"kind":"card","id":"t1"}"#).unwrap();
        assert_eq!(back, Scope::Card("t1".into()));
    }
}
