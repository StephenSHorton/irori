//! Notifications (P9 Notifications, Held and Message): the Rune port of the React app's
//! components/notifications-page.tsx, cards/held-dialog.tsx, cards/message-dialog.tsx (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {}

pub fn view(s: &State, f: &mut Frame) -> El {
    let _ = s;
    crate::ui::not_yet(f, "Notifications", "Not ported to Rune yet (P9 Notifications, Held and Message).")
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene inbox`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Notifications);
}

/// Its dialog (mounted in `f.layer` when `s.dialog` names it).
pub fn dialog(s: &State, f: &mut Frame) {
    let k = f.k;
    rune_kit::Dialog::new("dlg-inbox", "Notifications")
        .open(true)
        .description("Not ported to Rune yet (P9 Notifications, Held and Message).")
        .on_close(|s: &mut State| s.close_dialog())
        .build(k, f.layer);
    let _ = s;
}
