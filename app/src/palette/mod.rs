//! Command palette (P9 Command palette): the Rune port of the React app's
//! components/command-palette.tsx, lib/commands.ts, lib/fuzzy.ts (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Command palette", "Not ported to Rune yet (P9 Command palette).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene palette`).
pub fn scene(s: &mut State) {
    s.open_dialog(crate::ui::Dialog::Palette);
}

/// The palette over the window (Dialog::Palette).
pub fn dialog(s: &State, f: &mut Frame) {
    let _ = s;
    rune_kit::Dialog::new("dlg-palette", "Search and commands")
        .open(true)
        .description("Not ported to Rune yet (P9 Command palette).")
        .on_close(|s: &mut State| s.close_dialog())
        .build(f.k, f.layer);
}
