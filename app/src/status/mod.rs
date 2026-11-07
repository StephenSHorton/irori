//! Status (P11 Meters, usage, updater, launch at login, tray): the Rune port of the React app's
//! components/meters/*, updater/* (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Status", "Not ported to Rune yet (P11 Meters, usage, updater, launch at login, tray).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene status`).
pub fn scene(s: &mut State) {
    let _ = s;
}

/// The strip above every page (update ready, meters); empty for now.
pub fn banner(s: &State, f: &mut Frame) -> El {
    let _ = (s, f);
    rune::el().key("status-banner").h(0.0)
}
