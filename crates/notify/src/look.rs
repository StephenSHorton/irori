//! The cards' look (card #527): the person's appearance (Settings >
//! Appearance: theme, light/dark/system, glass style), resolved by the
//! window into card colours and kept in settings.json under `cards`, so
//! the cards are right at startup before the window has said anything.
//!
//! The window works the colours out from its own theme code
//! (frontend/src/lib/card-look.ts) and sends both the light and the dark
//! set: with "system" the toast crate picks one from the OS's mode and
//! flips with it, even while the window is closed.

use serde::{Deserialize, Serialize};

/// One mode's colours, each `#rrggbb` and already opaque (the window
/// composites its translucent tokens). A missing or malformed one falls
/// back to the toast crate's default for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CardColors {
    /// The solid card (and the fallback where there is no glass).
    pub background: String,
    /// The glass's colour, drawn at [`CardLook::alpha`] over the blur.
    pub tint: String,
    pub foreground: String,
    pub muted: String,
    pub border: String,
    pub hover: String,
    /// The secondary button.
    pub button: String,
    pub primary: String,
    pub on_primary: String,
    pub info: String,
    pub success: String,
    pub warn: String,
    pub error: String,
}

/// The cards' look.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CardLook {
    /// The theme's id ("blue", ..., "custom"); kept for reference only.
    pub theme: String,
    /// "light", "dark" or "system".
    pub mode: String,
    /// The glass style's id ("frost", "clear", ...); for reference only.
    pub glass: String,
    /// The glass tint's opacity, 0..1; 0 or 1 is solid cards.
    pub alpha: f64,
    /// Corner radius in dip.
    pub radius: f64,
    pub light: CardColors,
    pub dark: CardColors,
}

/// `#rrggbb` (or `rrggbb`), or unset.
fn color(s: &str) -> toast::Color {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if t.len() == 6 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        toast::hex(t)
    } else {
        toast::Color(0)
    }
}

impl CardColors {
    /// The toast palette: what this leaves unset keeps base's for the
    /// main colours and is derived for the rest.
    fn palette(&self, base: toast::Palette) -> toast::Palette {
        let main = |s: &str, d: toast::Color| {
            let v = color(s);
            if v.is_zero() { d } else { v }
        };
        toast::Palette {
            background: main(&self.background, base.background),
            foreground: main(&self.foreground, base.foreground),
            muted: main(&self.muted, base.muted),
            border: main(&self.border, base.border),
            accent: toast::Accents {
                info: main(&self.info, base.accent.info),
                success: main(&self.success, base.accent.success),
                warn: main(&self.warn, base.accent.warn),
                error: main(&self.error, base.accent.error),
            },
            faint: toast::Color(0),
            hover: color(&self.hover),
            button: color(&self.button),
            primary: color(&self.primary),
            primary_hover: toast::Color(0),
            on_primary: color(&self.on_primary),
            tint: color(&self.tint),
        }
    }
}

/// The corner radius of the app's surfaces (`--radius`, 0.875rem), the
/// cards' with no saved look.
pub const DEFAULT_RADIUS: f64 = 14.0;

impl CardLook {
    /// Applies the look to c (its app name and capture stay).
    pub fn apply(&self, c: toast::Config) -> toast::Config {
        let scheme = match self.mode.as_str() {
            "light" => toast::Scheme::Light,
            "system" => toast::Scheme::System,
            _ => toast::Scheme::Dark,
        };
        let radius = if self.radius.is_finite() && self.radius >= 0.0 {
            self.radius
        } else {
            DEFAULT_RADIUS
        };
        toast::Config {
            light: self.light.palette(toast::LIGHT),
            dark: self.dark.palette(toast::DARK),
            scheme,
            corner_radius: radius,
            glass: if self.alpha.is_finite() {
                self.alpha
            } else {
                1.0
            },
            ..c
        }
    }
}

/// The cards' setup with no saved look: Irori's name, rounded like the
/// window, dark and solid, and capture as asked.
pub fn base_config(capture_exclude: bool) -> toast::Config {
    toast::Config {
        app_name: "Irori".into(),
        corner_radius: DEFAULT_RADIUS,
        capture_exclude,
        ..toast::Config::default()
    }
}
