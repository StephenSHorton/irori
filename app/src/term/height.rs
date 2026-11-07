//! The card-page terminal's height (term-height.ts, card #730): dragging
//! the handle on its bottom edge (or its arrow keys) sets it between
//! [`MIN`] and the window's height less some room for the page, and this
//! device remembers it, so every terminal opens at the height last chosen.

use crate::ui::prefs::Prefs;

pub const DEFAULT: f32 = 360.0;
pub const MIN: f32 = 200.0;
/// What the window keeps above and below the tallest terminal: the card's
/// top bar and the tabs row, so the handle stays on screen.
pub const PAGE_ROOM: f32 = 96.0;

pub const KEY: &str = "irori.term.height";

/// The tallest the terminal goes in a window `inner` tall, never under
/// [`MIN`].
pub fn max_height(inner: f32) -> f32 {
    if !inner.is_finite() {
        return DEFAULT;
    }
    MIN.max((inner - PAGE_ROOM).round())
}

/// `h` kept between [`MIN`] and [`max_height`]; a bad value is the
/// default.
pub fn clamp_height(h: f32, inner: f32) -> f32 {
    if !h.is_finite() {
        return DEFAULT.min(max_height(inner));
    }
    max_height(inner).min(MIN.max(h)).round()
}

/// The remembered height, or the default when there is none. It is not
/// clamped to the window: the view does that as it draws, so a window
/// that is only opened smaller still gets the full height back when grown.
pub fn read(p: &Prefs) -> f32 {
    match p.get::<f64>(KEY) {
        Some(n) if n.is_finite() && n >= MIN as f64 => (n as f32).round(),
        _ => DEFAULT,
    }
}

/// Remembers `h` (rounded); None forgets it.
pub fn save(p: &mut Prefs, h: Option<f32>) {
    match h {
        Some(h) => p.set(KEY, h.round() as i64),
        None => p.remove(KEY),
    }
}

/// The arrow keys move the edge too, a step of 16 points (Shift: 64);
/// Home and End go to the ends. None for any other key.
pub fn key_step(key: &rune::Key, shift: bool, now: f32, inner: f32) -> Option<f32> {
    let step = if shift { 64.0 } else { 16.0 };
    let now = clamp_height(now, inner);
    let next = match key {
        rune::Key::Up => now - step,
        rune::Key::Down => now + step,
        rune::Key::Home => MIN,
        rune::Key::End => max_height(inner),
        _ => return None,
    };
    Some(clamp_height(next, inner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_height_is_the_window_less_the_page_room() {
        assert_eq!(max_height(900.0), 900.0 - PAGE_ROOM);
    }

    #[test]
    fn max_height_is_never_under_the_minimum() {
        assert_eq!(max_height(150.0), MIN);
    }

    #[test]
    fn clamp_keeps_it_between_the_ends() {
        assert_eq!(clamp_height(50.0, 900.0), MIN);
        assert_eq!(clamp_height(5000.0, 900.0), 900.0 - PAGE_ROOM);
        assert_eq!(clamp_height(412.6, 900.0), 413.0);
    }

    #[test]
    fn a_bad_value_is_the_default() {
        assert_eq!(clamp_height(f32::NAN, 900.0), DEFAULT);
    }

    #[test]
    fn nothing_saved_is_the_default() {
        assert_eq!(read(&Prefs::default()), DEFAULT);
    }

    #[test]
    fn a_saved_height_comes_back() {
        let mut p = Prefs::default();
        save(&mut p, Some(520.4));
        assert_eq!(p.get::<i64>(KEY), Some(520));
        assert_eq!(read(&p), 520.0);
    }

    #[test]
    fn none_forgets_it() {
        let mut p = Prefs::default();
        save(&mut p, Some(520.0));
        save(&mut p, None);
        assert_eq!(read(&p), DEFAULT);
    }

    #[test]
    fn junk_or_a_height_under_the_minimum_is_the_default() {
        let mut p = Prefs::default();
        p.set(KEY, "tall");
        assert_eq!(read(&p), DEFAULT);
        p.set(KEY, 40);
        assert_eq!(read(&p), DEFAULT);
    }

    #[test]
    fn keys_step_and_jump_to_the_ends() {
        use rune::Key;
        assert_eq!(key_step(&Key::Up, false, 360.0, 900.0), Some(344.0));
        assert_eq!(key_step(&Key::Down, true, 360.0, 900.0), Some(424.0));
        assert_eq!(key_step(&Key::Home, false, 360.0, 900.0), Some(MIN));
        assert_eq!(key_step(&Key::End, false, 360.0, 900.0), Some(804.0));
        assert_eq!(key_step(&Key::Down, false, 800.0, 900.0), Some(804.0));
        assert_eq!(key_step(&Key::Enter, false, 360.0, 900.0), None);
    }
}
