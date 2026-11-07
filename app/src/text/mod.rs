//! Text (F0b Rich text, mentions and the composer): the Rune port of the React app's
//! common/rich-text.tsx, lib/rich.ts, common/mention-textarea.tsx, lib/mentions.ts, attachments/* (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

pub mod mentions;
pub mod rich;

use rune::El;

use crate::ui::{Event, Frame, State};


#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Text", "Not ported to Rune yet (F0b Rich text, mentions and the composer).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene text`).
pub fn scene(s: &mut State) {
    let _ = s;
}
