//! Talk (P5 Talk): the Rune port of the React app's
//! components/talk/talk-page.tsx, conversation.tsx, threads.tsx (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

pub mod logic;

use rune::El;

use crate::ui::{Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Talk", "Not ported to Rune yet (P5 Talk).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene talk`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Talk(None));
}
