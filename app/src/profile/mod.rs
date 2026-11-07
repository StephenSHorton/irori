//! Profile (P8 Profile): the Rune port of the React app's
//! components/profile/* (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Profile", "Not ported to Rune yet (P8 Profile).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene profile`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Profile);
}
