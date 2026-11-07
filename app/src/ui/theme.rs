//! The window's look: a rune-kit [`Kit`] from Irori's colours (the React
//! app's design/themes.css), light, dark or the system's. Settings >
//! Appearance (the settings package) writes the prefs this reads:
//! `appearance.mode` ("system", "light", "dark") and `appearance.accent`
//! (a "#rrggbb" hex, empty for Irori's own).

use rune::{Color, Cx};
use rune_kit::{Kit, PaletteSeed};

use super::State;
use super::prefs::Prefs;

pub const MODE: &str = "appearance.mode";
pub const ACCENT: &str = "appearance.accent";

/// Irori's dark background (the Tauri window's #0B0C0F).
const DARK_BG: &str = "#0b0c0f";
const DARK_FG: &str = "#e8e9ec";
const LIGHT_BG: &str = "#ffffff";
const LIGHT_FG: &str = "#17181c";
/// Irori's violet accent (the person's colour, hue 300 in OKLCH).
const ACCENT_DEFAULT: &str = "#8b6cf0";

pub fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::BLACK)
}

pub fn is_dark(s: &State, cx: &Cx<State>) -> bool {
    match s.prefs.get::<String>(MODE).as_deref() {
        Some("light") => false,
        Some("dark") => true,
        _ => cx.system_dark(),
    }
}

pub fn kit(dark: bool, p: &Prefs) -> Kit {
    // A theme picked in Settings > Appearance (`appearance.theme`).
    let seed = crate::settings::appearance::seed(p, dark).unwrap_or_else(|| {
        let accent = p
            .get::<String>(ACCENT)
            .filter(|a| rune::color::hex(a).is_some())
            .unwrap_or_else(|| ACCENT_DEFAULT.to_string());
        let (bg, fg) = if dark {
            (DARK_BG, DARK_FG)
        } else {
            (LIGHT_BG, LIGHT_FG)
        };
        PaletteSeed {
            bg: hex(bg),
            fg: hex(fg),
            accent: hex(&accent),
        }
    });
    let mut k = Kit::from_palette(seed);
    if !dark {
        // shadcn's switch and slider thumbs are the page colour (white) in
        // light mode; the kit's default, the text colour, reads as a dark
        // knob there.
        k.c.knob = Color::WHITE;
        k.c.knob_on = Color::WHITE;
    }
    k
}

/// Status colours the cards use (the React app's status hues): a decision
/// amber, work green, review blue, waiting violet, idle grey, failed red.
pub mod status {
    use rune::Color;

    pub fn decide() -> Color {
        super::hex("#e0a526")
    }
    pub fn working() -> Color {
        super::hex("#3fb37f")
    }
    pub fn review() -> Color {
        super::hex("#5b9cf0")
    }
    pub fn waiting() -> Color {
        super::hex("#9a8cf0")
    }
    pub fn idle() -> Color {
        super::hex("#8b8f98")
    }
    pub fn failed() -> Color {
        super::hex("#e5534b")
    }
}
