//! Terminals (P10 Inline terminals): the Rune port of the React app's
//! components/term/*, suzuri ui/view (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Terminals", "Not ported to Rune yet (P10 Inline terminals).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene term`).
pub fn scene(s: &mut State) {
    let _ = s;
}
