//! Settings (P8 Settings and devices): the Rune port of the React app's
//! components/settings/* (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Settings", "Not ported to Rune yet (P8 Settings and devices).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene settings`).
pub fn scene(s: &mut State) {
    s.open_dialog(crate::ui::Dialog::Settings(String::new()));
}

/// Its dialog (mounted in `f.layer` when `s.dialog` names it).
pub fn dialog(s: &State, f: &mut Frame) {
    let k = f.k;
    rune_kit::Dialog::new("dlg-settings", "Settings")
        .open(true)
        .description("Not ported to Rune yet (P8 Settings and devices).")
        .on_close(|s: &mut State| s.close_dialog())
        .build(k, f.layer);
    let _ = s;
}
