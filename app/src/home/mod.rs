//! Home (P1 Home and Needs you): the Rune port of the React app's
//! frontend/src/App.tsx (home branch), components/cards/needs-you.tsx, ask-box.tsx, home/*, wind-down-dialog.tsx, done-rounds.tsx (on main, under frontend/src/).
//!
//! Not ported yet: this is the package's stub. See ui/mod.rs for the
//! package contract.

use rune::El;

use crate::ui::{Event, Frame, Route, State};

#[derive(Default)]
pub struct Model {}

/// Until P1 lands: every card in the Board's groups (Needs you first).
pub fn view(s: &State, f: &mut Frame) -> El {
    let cards: Vec<_> = s.data.cards.iter().collect();
    rune::col()
        .key("home-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(crate::ui::page_header(f, "Home", vec![]))
        .child(crate::board::cards_list(s, f, &cards).grow(1.0).min_h(0.0))
}

pub fn on_event(s: &mut State, ev: &Event) {
    let _ = (s, ev);
}

/// The headless scene that shows it (`irori --snap x.png --scene home`).
pub fn scene(s: &mut State) {
    s.close_card();
    s.go(Route::Home);
}

/// Its dialog (mounted in `f.layer` when `s.dialog` names it).
pub fn wind_down_dialog(s: &State, f: &mut Frame) {
    let k = f.k;
    rune_kit::Dialog::new("dlg-home", "Home")
        .open(true)
        .description("Not ported to Rune yet (P1 Home and Needs you).")
        .on_close(|s: &mut State| s.close_dialog())
        .build(k, f.layer);
    let _ = s;
}
