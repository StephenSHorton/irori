//! Notes (P7 Notes): the Rune port of the React app's
//! components/notes/* (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Notes", "Not ported to Rune yet (P7 Notes).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene notes`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Notes);
}
