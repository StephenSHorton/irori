//! Terminals for the Rune window (card #674, P10): the Tauri `term_*`
//! commands (main:crates/window/src/commands.rs) re-homed as `Window`
//! methods over `core.terms()`. Frames and messages reach the window
//! through a [`Viewer`] the window builds from two callbacks; everything
//! else is a quick call the UI thread may make directly: each only queues
//! work for the pane's actor or takes the engine's lock for a moment.
//!
//! The terminal types are re-exported here so the app needs no direct
//! dependency on irori-term.

pub use term::{Link, Mouse, MouseKind, Scope, SelectKind, TermId, TermInfo, TermMsg, Viewer};

use crate::Window;

impl Window {
    /// Sends a terminal's frames (raw bytes, rsz-term's frame format) and
    /// messages to the window, starting with its cached state and a full
    /// frame. Each callback runs on rsz-term's threads, must not block,
    /// and answers false once the view is gone. A second attach replaces
    /// the first view.
    pub fn term_attach(
        &self,
        id: TermId,
        frame: impl Fn(Vec<u8>) -> bool + Send + Sync + 'static,
        msg: impl Fn(TermMsg) -> bool + Send + Sync + 'static,
    ) -> Result<(), String> {
        self.core.terms().attach(id, Viewer::new(frame, msg))
    }

    /// The view went away; the shell keeps running.
    pub fn term_detach(&self, id: TermId) {
        self.core.terms().detach(id);
    }

    /// What the person typed, in the order typed.
    pub fn term_write(&self, id: TermId, data: &[u8]) -> Result<(), String> {
        self.core.terms().write(id, data.to_vec())
    }

    /// A paste, framed for bracketed paste when the program asked for it.
    pub fn term_paste(&self, id: TermId, text: &str) -> Result<(), String> {
        self.core.terms().paste(id, text)
    }

    pub fn term_resize(&self, id: TermId, cols: u16, rows: u16) -> Result<(), String> {
        self.core.terms().resize(id, cols, rows)
    }

    /// Scrolls the history: positive goes back, negative toward the end.
    pub fn term_scroll(&self, id: TermId, delta: i32) -> Result<(), String> {
        self.core.terms().scroll(id, delta)
    }

    /// The view's cell size in pixels (for images and pixel reports).
    pub fn term_cell_px(&self, id: TermId, w: u16, h: u16) -> Result<(), String> {
        self.core.terms().cell_px(id, w, h)
    }

    pub fn term_focus(&self, id: TermId, on: bool) -> Result<(), String> {
        self.core.terms().focus(id, on)
    }

    /// Asks for a full frame.
    pub fn term_redraw(&self, id: TermId) -> Result<(), String> {
        self.core.terms().redraw(id)
    }

    /// The view painted frames up to `seq` (flow control: one frame is in
    /// flight until it is acked).
    pub fn term_ack(&self, id: TermId, seq: u32) -> Result<(), String> {
        self.core.terms().ack(id, seq)
    }

    /// Kills the shell and forgets the terminal.
    pub fn term_close(&self, id: TermId) -> Result<(), String> {
        self.core.terms().close(id)
    }

    pub fn term_select_start(
        &self,
        id: TermId,
        col: u16,
        row: u16,
        right: bool,
        kind: SelectKind,
    ) -> Result<(), String> {
        self.core.terms().select_start(id, col, row, right, kind)
    }

    pub fn term_select_extend(&self, id: TermId, col: u16, row: u16, right: bool) -> Result<(), String> {
        self.core.terms().select_extend(id, col, row, right)
    }

    pub fn term_select_clear(&self, id: TermId) -> Result<(), String> {
        self.core.terms().select_clear(id)
    }

    /// The selected text (Copy), if any.
    pub fn term_selection(&self, id: TermId) -> Option<String> {
        self.core.terms().selection_text(id).ok().flatten()
    }

    pub fn term_link_at(&self, id: TermId, col: u16, row: u16) -> Option<Link> {
        self.core.terms().link_at(id, col, row).ok().flatten()
    }

    /// Reports the mouse to a program that asked for it; false when it
    /// didn't.
    pub fn term_mouse(&self, id: TermId, m: Mouse) -> bool {
        self.core.terms().mouse(id, m).unwrap_or(false)
    }
}
