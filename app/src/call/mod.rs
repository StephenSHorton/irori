//! Calls (P6 Calls, voice and screen share): the Rune port of the React app's
//! components/talk/huddle.tsx, share.tsx, transcript-pane.tsx, common/mic-button.tsx (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Calls", "Not ported to Rune yet (P6 Calls, voice and screen share).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene call`).
pub fn scene(s: &mut State) {
    let _ = s;
}

/// Over every page (the call bar and ringing); nothing while no call.
pub fn overlay(s: &State, f: &mut Frame) {
    let _ = (s, f);
}
