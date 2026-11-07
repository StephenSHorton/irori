//! Merge desks (P9 Merge desk): the Rune port of the React app's
//! components/desk/desk-page.tsx, lib/desk.ts (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Merge desks", "Not ported to Rune yet (P9 Merge desk).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene desk`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Desk(None));
}
