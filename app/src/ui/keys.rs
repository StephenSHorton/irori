//! The window's keyboard shortcuts (the React app's keydown handlers:
//! lib/commands.ts and App.tsx), as `AppSpec::on_key`. Cmd is Ctrl off
//! the Mac (`Key::Command`).
//!
//! | Keys | What |
//! |---|---|
//! | Cmd+K, Cmd+P | the command palette |
//! | Esc | back from a card's page |
//! | Cmd+[, Cmd+] | back, forward |
//! | Cmd+N | New card (on the board shown, if any) |
//! | Cmd+, | Settings |
//! | / | focus the board's search (Home and boards: `board::on_key`, ahead of this) |
//!
//! A text field takes its keys first, so `/` types there and only reaches
//! this when no field has focus; an open dialog takes Esc (its modal closes
//! it) before this sees it.

use rune::{Key, KeyEvent, Tx};

use super::{Dialog, Route, State};

pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    match &ev.key {
        Key::Command(c) if !m.shift && !m.alt => match c {
            'k' | 'p' => {
                s.open_dialog(Dialog::Palette);
                true
            }
            '[' => {
                s.back();
                true
            }
            ']' => {
                s.nav.forward();
                true
            }
            'n' => {
                let board = match &s.nav.route {
                    Route::Board { space, .. } => Some(space.clone()),
                    _ => None,
                };
                s.open_dialog(Dialog::NewCard { board });
                true
            }
            ',' => {
                s.open_dialog(Dialog::Settings(String::new()));
                true
            }
            _ => false,
        },
        Key::Escape if s.dialog.is_some() => {
            s.close_dialog();
            true
        }
        Key::Escape if s.status.meters_open => {
            s.status.meters_open = false;
            true
        }
        // A card page's text field passes Esc on: it only lets go of the
        // field, so a draft isn't lost to a stray key (App.tsx). A focused
        // control on the page took Esc already (card/page.rs).
        Key::Escape if s.nav.card.is_some() && tx.focused().is_some() => {
            tx.blur();
            true
        }
        // Esc is Back on a card's page only (App.tsx); elsewhere it does
        // nothing.
        Key::Escape if s.nav.card.is_some() => {
            s.back();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use rune::Mods;

    use super::*;

    fn key(k: Key, cmd: bool) -> KeyEvent {
        KeyEvent {
            key: k,
            mods: Mods {
                cmd,
                ..Mods::default()
            },
        }
    }

    fn run(s: &mut State, ev: KeyEvent) -> bool {
        let mut tx = Tx::default();
        on_key(s, &ev, &mut tx)
    }

    #[test]
    fn palette_settings_and_new_card() {
        let mut s = State::default();
        assert!(run(&mut s, key(Key::Command('k'), true)));
        assert_eq!(s.dialog, Some(Dialog::Palette));
        assert!(run(&mut s, key(Key::Escape, false)));
        assert_eq!(s.dialog, None);
        assert!(run(&mut s, key(Key::Command(','), true)));
        assert_eq!(s.dialog, Some(Dialog::Settings(String::new())));
        s.close_dialog();
        s.nav.go(Route::Board {
            space: "sp".into(),
            board: None,
        });
        assert!(run(&mut s, key(Key::Command('n'), true)));
        assert_eq!(
            s.dialog,
            Some(Dialog::NewCard {
                board: Some("sp".into())
            })
        );
    }

    #[test]
    fn escape_and_brackets_walk_the_history() {
        let mut s = State::default();
        assert!(
            !run(&mut s, key(Key::Escape, false)),
            "nothing to go back to"
        );
        s.nav.go(Route::Talk(None));
        assert!(
            !run(&mut s, key(Key::Escape, false)),
            "Esc on a page that isn't a card's does nothing"
        );
        assert_eq!(s.nav.route, Route::Talk(None));
        s.nav.open_card("c1".into());
        assert!(run(&mut s, key(Key::Escape, false)));
        assert_eq!(s.nav.card, None);
        assert!(run(&mut s, key(Key::Command('['), true)));
        assert_eq!(s.nav.route, Route::Home);
        assert!(run(&mut s, key(Key::Command(']'), true)));
        assert_eq!(s.nav.route, Route::Talk(None));
    }
}
