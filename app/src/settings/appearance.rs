//! Settings > Appearance (the React app's appearance-pane.tsx and the
//! colour half of lib/appearance.ts, card #275): the theme, your own
//! colours over it, and light, dark or the system's. Saved on this device
//! in the window's prefs: `appearance.theme` (a theme id, "custom", or
//! none for Irori's own look in ui/theme.rs), `appearance.custom`
//! ({base, accent, tint}) and ui/theme.rs's `appearance.mode`.
//!
//! The themes are seeds (the neutrals' hue and chroma, the accent per
//! mode); lightness comes from a fixed ramp so text stays readable, and a
//! custom accent is nudged until its text passes WCAG AA. ui/theme.rs
//! builds the window's kit from [`seed`].
//!
//! The notification cards follow the look (lib/card-look.ts, card #527):
//! [`card_look`] flattens it into the plain colours the native card
//! windows draw with, sent to the host at start and on every change. A
//! first start with nothing picked here takes the look the React window
//! last sent (settings.json's "cards": its theme and mode), so nobody's
//! appearance is lost to the move.

use irori_window::settings::{CardColors, CardLook};
use rune::{El, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ColorPicker, ColorPickerState, ColorSwatch, PaletteSeed,
    SegmentedControl,
};
use serde::{Deserialize, Serialize};

use crate::ui::prefs::Prefs;
use crate::ui::theme::MODE;
use crate::ui::{Frame, State, icons};

use super::{hint, muted_line};

pub const THEME: &str = "appearance.theme";
pub const CUSTOM: &str = "appearance.custom";

// ---- colour --------------------------------------------------------------

/// An OKLCH colour, with an alpha over whatever is behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
    pub a: f32,
}

fn ok(l: f32, c: f32, h: f32) -> Oklch {
    Oklch {
        l,
        c,
        h: h.rem_euclid(360.0),
        a: 1.0,
    }
}

impl Oklch {
    fn alpha(self, a: f32) -> Oklch {
        Oklch { a, ..self }
    }
    pub fn color(self) -> rune::Color {
        rune::color::oklcha(self.l, self.c, self.h, self.a)
    }
}

type Rgb = [f32; 3];

/// Linear sRGB, clipped.
fn to_linear(x: Oklch) -> Rgb {
    let hr = x.h.to_radians();
    let (a, b) = (x.c * hr.cos(), x.c * hr.sin());
    let l = (x.l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (x.l - 0.105_561_35 * a - 0.063_854_17 * b).powi(3);
    let s = (x.l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let clip = |v: f32| v.clamp(0.0, 1.0);
    [
        clip(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        clip(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s),
        clip(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}

fn over(x: Oklch, bg: Rgb) -> Rgb {
    let fg = to_linear(x);
    [0, 1, 2].map(|i| fg[i] * x.a + bg[i] * (1.0 - x.a))
}

fn luminance([r, g, b]: Rgb) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// The WCAG ratio of fg on bg, each composited on what's under it (white
/// when nothing is).
pub fn contrast(fg: Oklch, bg: Oklch, under: Option<Oklch>) -> f32 {
    let base = under.map(to_linear).unwrap_or([1.0; 3]);
    let b = over(bg, base);
    let f = over(fg, b);
    let (x, y) = (luminance(f), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Reads "#rrggbb" (a bad one is a mid blue, as the React app's).
pub fn hex_to_oklch(hex: &str) -> Oklch {
    let t = hex.trim().trim_start_matches('#');
    let Some(v) = (t.len() == 6)
        .then(|| u32::from_str_radix(t, 16).ok())
        .flatten()
    else {
        return ok(0.6, 0.15, 250.0);
    };
    let lin = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|c| {
        let s = c as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    });
    let [r, g, b] = lin;
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let ll = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let aa = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let c = aa.hypot(bb);
    let h = if c < 1e-4 {
        0.0
    } else {
        bb.atan2(aa).to_degrees()
    };
    ok(ll, c, h)
}

/// The nearest sRGB "#rrggbb" (for the colour fields).
pub fn oklch_to_hex(x: Oklch) -> String {
    let enc = |v: f32| {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    let [r, g, b] = to_linear(x).map(|v| (enc(v).clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn valid_hex(s: &str) -> bool {
    let t = s.trim();
    t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit())
}

// ---- themes ----------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Accent {
    pub h: f32,
    pub c: f32,
    pub l: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Seed {
    pub nh: f32,
    pub nc: f32,
    pub light: Accent,
    pub dark: Accent,
}

/// The colours a theme sets that the window uses.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub background: Oklch,
    pub foreground: Oklch,
    pub primary: Oklch,
    pub primary_fg: Oklch,
    pub accent: Oklch,
    pub accent_fg: Oklch,
    pub muted_fg: Oklch,
    pub secondary: Oklch,
    pub border: Oklch,
    pub glass_tint: Oklch,
    pub glass_ember_tint: Oklch,
}

/// Info and person hues, kept apart from the accent (appearance.ts).
fn tokens_from(seed: &Seed, dark: bool) -> Tokens {
    let (nh, nc) = (seed.nh, seed.nc);
    if !dark {
        let a = seed.light;
        let p = ok(a.l.unwrap_or(0.56), a.c, a.h);
        let fg = ok(0.22, nc, nh);
        return Tokens {
            background: ok(0.955, nc * 0.67, nh + 15.0),
            foreground: fg,
            primary: p,
            primary_fg: ok(0.99, 0.005, nh + 15.0),
            accent: p.alpha(0.1),
            accent_fg: ok(p.l.min(0.42), a.c * 0.82, a.h),
            muted_fg: ok(0.46, nc * 1.33, nh + 5.0),
            secondary: fg.alpha(0.06),
            border: fg.alpha(0.12),
            glass_tint: ok(0.995, 0.004, nh + 15.0),
            glass_ember_tint: ok(0.97, (a.c * 0.2).min(0.035), a.h + 22.0),
        };
    }
    let a = seed.dark;
    let p = ok(a.l.unwrap_or(0.75), a.c, a.h);
    Tokens {
        background: ok(0.17, nc, nh - 5.0),
        foreground: ok(0.96, nc.min(0.006), nh + 20.0),
        primary: p,
        primary_fg: ok(0.2, (a.c * 0.2).min(0.03), a.h - 12.0),
        accent: p.alpha(0.14),
        accent_fg: ok(p.l.max(0.84), a.c * 0.8, a.h + 8.0),
        muted_fg: ok(0.76, nc * 1.17, nh + 10.0),
        secondary: ok(1.0, 0.0, 0.0).alpha(0.07),
        border: ok(1.0, 0.0, 0.0).alpha(0.11),
        glass_tint: ok(0.23, nc * 1.17, nh - 2.0),
        glass_ember_tint: ok(0.27, (a.c * 0.4).min(0.06), a.h - 10.0),
    }
}

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    pub seed: Seed,
    /// Exact background, foreground and accent that win over the seed's,
    /// light then dark.
    exact: Option<[(Oklch, Oklch, Oklch); 2]>,
}

const fn acc(h: f32, c: f32, l: Option<f32>) -> Accent {
    Accent { h, c, l }
}

const fn k(l: f32, c: f32, h: f32) -> Oklch {
    Oklch { l, c, h, a: 1.0 }
}

/// The window's own look (ui/theme.rs): near-black or white, Irori's
/// violet. Shown first; picking it clears `appearance.theme`.
pub const IRORI: &str = "irori";

pub static THEMES: &[Theme] = &[
    Theme {
        id: IRORI,
        name: "Irori",
        note: "The window's own: plain neutrals, a violet accent",
        seed: Seed {
            nh: 270.0,
            nc: 0.006,
            light: acc(292.0, 0.17, Some(0.6)),
            dark: acc(292.0, 0.17, Some(0.6)),
        },
        exact: None,
    },
    Theme {
        id: "blue",
        name: "Blue",
        note: "Cool slate, a clear blue accent",
        seed: Seed {
            nh: 255.0,
            nc: 0.014,
            light: acc(255.0, 0.17, Some(0.53)),
            dark: acc(250.0, 0.14, Some(0.74)),
        },
        exact: None,
    },
    Theme {
        id: "hearth",
        name: "Hearth",
        note: "Warm ash, an ember accent",
        seed: Seed {
            nh: 55.0,
            nc: 0.012,
            light: acc(38.0, 0.17, None),
            dark: acc(52.0, 0.15, None),
        },
        // Hearth keeps #266's numbers.
        exact: Some([
            (
                k(0.955, 0.008, 70.0),
                k(0.22, 0.012, 50.0),
                k(0.56, 0.17, 38.0),
            ),
            (
                k(0.17, 0.012, 45.0),
                k(0.96, 0.006, 70.0),
                k(0.75, 0.15, 52.0),
            ),
        ]),
    },
    Theme {
        id: "midnight",
        name: "Midnight",
        note: "Deep indigo, a violet glow",
        seed: Seed {
            nh: 280.0,
            nc: 0.02,
            light: acc(285.0, 0.17, Some(0.5)),
            dark: acc(290.0, 0.13, Some(0.74)),
        },
        exact: None,
    },
    Theme {
        id: "forest",
        name: "Forest",
        note: "Moss and bark, a green accent",
        seed: Seed {
            nh: 145.0,
            nc: 0.014,
            light: acc(150.0, 0.13, Some(0.5)),
            dark: acc(150.0, 0.13, Some(0.76)),
        },
        exact: None,
    },
    Theme {
        id: "sakura",
        name: "Sakura",
        note: "Petal pink on soft stone",
        seed: Seed {
            nh: 350.0,
            nc: 0.012,
            light: acc(355.0, 0.16, Some(0.54)),
            dark: acc(355.0, 0.12, Some(0.78)),
        },
        exact: None,
    },
    Theme {
        id: "mono",
        name: "Mono",
        note: "Greys only; the status colours keep theirs",
        seed: Seed {
            nh: 0.0,
            nc: 0.0,
            light: acc(0.0, 0.0, Some(0.3)),
            dark: acc(0.0, 0.0, Some(0.88)),
        },
        exact: None,
    },
];

pub fn theme(id: &str) -> &'static Theme {
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[1])
}

pub fn theme_tokens(t: &Theme, dark: bool) -> Tokens {
    let mut out = tokens_from(&t.seed, dark);
    if t.id == IRORI {
        // ui/theme.rs's own colours.
        let (bg, fg) = if dark {
            ("#0b0c0f", "#e8e9ec")
        } else {
            ("#ffffff", "#17181c")
        };
        out.background = hex_to_oklch(bg);
        out.foreground = hex_to_oklch(fg);
        out.primary = hex_to_oklch("#8b6cf0");
    }
    if let Some(ex) = t.exact {
        let (bg, fg, p) = ex[usize::from(dark)];
        out.background = bg;
        out.foreground = fg;
        out.primary = p;
    }
    if t.id == "hearth" {
        // The rest of #266's numbers that the cards use.
        if dark {
            out.primary_fg = k(0.2, 0.03, 40.0);
            out.muted_fg = k(0.76, 0.014, 60.0);
            out.accent = k(0.75, 0.15, 52.0).alpha(0.14);
            out.accent_fg = k(0.84, 0.12, 60.0);
            out.glass_tint = k(0.23, 0.014, 48.0);
            out.glass_ember_tint = k(0.27, 0.06, 42.0);
        } else {
            out.primary_fg = k(0.99, 0.005, 70.0);
            out.secondary = k(0.22, 0.012, 50.0).alpha(0.06);
            out.muted_fg = k(0.46, 0.016, 55.0);
            out.accent = k(0.56, 0.17, 38.0).alpha(0.1);
            out.accent_fg = k(0.42, 0.14, 38.0);
            out.border = k(0.22, 0.012, 50.0).alpha(0.12);
            out.glass_tint = k(0.995, 0.004, 70.0);
            out.glass_ember_tint = k(0.97, 0.035, 60.0);
        }
    }
    out
}

/// Your colours over a theme.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Custom {
    pub base: String,
    pub accent: String,
    pub tint: String,
}

/// Keeps the hues and chroma of the picks; the ramp sets the lightness, so
/// a near-white or near-black pick still reads.
pub fn custom_seed(c: &Custom) -> Seed {
    let base = theme(&c.base);
    let a = hex_to_oklch(&c.accent);
    let n = hex_to_oklch(&c.tint);
    let ac = a.c.min(0.2);
    Seed {
        nh: n.h,
        nc: (n.c * 0.3).min(0.03),
        light: acc(a.h, ac, base.seed.light.l),
        dark: acc(a.h, ac.min(0.16), base.seed.dark.l),
    }
}

const AA: f32 = 4.5;

/// Nudges the accent until primary text and accent text pass AA.
pub fn guard(mut t: Tokens, dark: bool) -> Tokens {
    let bg = t.background;
    let step = if dark { 0.02 } else { -0.02 };
    let fgs = [ok(0.99, 0.005, t.primary.h), ok(0.2, 0.03, t.primary.h)];
    for _ in 0..30 {
        let best = if contrast(fgs[0], t.primary, Some(bg)) >= contrast(fgs[1], t.primary, Some(bg))
        {
            fgs[0]
        } else {
            fgs[1]
        };
        t.primary_fg = best;
        if contrast(best, t.primary, Some(bg)) >= AA {
            break;
        }
        let away = if best.l > 0.5 { -0.02 } else { 0.02 };
        t.primary.l += away;
    }
    t.accent = t.primary.alpha(t.accent.a);
    let mut af = t.accent_fg;
    for _ in 0..40 {
        if contrast(af, t.accent, Some(bg)) >= AA {
            break;
        }
        af.l = (af.l + step).clamp(0.0, 1.0);
    }
    t.accent_fg = af;
    t
}

pub fn custom_tokens(c: &Custom, dark: bool) -> Tokens {
    guard(tokens_from(&custom_seed(c), dark), dark)
}

/// The saved custom colours, when both are "#rrggbb".
pub fn saved_custom(p: &Prefs) -> Option<Custom> {
    let mut c = p.get::<Custom>(CUSTOM)?;
    if !valid_hex(&c.accent) || !valid_hex(&c.tint) {
        return None;
    }
    if !THEMES.iter().any(|t| t.id == c.base) {
        c.base = "blue".into();
    }
    Some(c)
}

/// The theme shown: a known id, "custom" with its colours, else Irori's.
pub fn current(p: &Prefs) -> String {
    match p.get::<String>(THEME) {
        Some(t) if t == "custom" && saved_custom(p).is_some() => t,
        Some(t) if t != "custom" && THEMES.iter().any(|x| x.id == t) => t,
        _ => IRORI.into(),
    }
}

fn tokens_for(id: &str, p: &Prefs, dark: bool) -> Tokens {
    match (id, saved_custom(p)) {
        ("custom", Some(c)) => custom_tokens(&c, dark),
        _ => theme_tokens(theme(id), dark),
    }
}

/// The window's palette for the picked theme; None for Irori's own, which
/// ui/theme.rs draws (with `appearance.accent`).
pub fn seed(p: &Prefs, dark: bool) -> Option<PaletteSeed> {
    let id = current(p);
    if id == IRORI {
        return None;
    }
    let t = tokens_for(&id, p, dark);
    Some(PaletteSeed {
        bg: t.background.color(),
        fg: t.foreground.color(),
        accent: t.primary.color(),
    })
}

// ---- the notification cards' look (lib/card-look.ts, card #527) ----------

/// Encoded sRGB, 0..1.
fn encoded(x: Oklch) -> Rgb {
    let enc = |v: f32| {
        if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    };
    to_linear(x).map(enc)
}

fn encoded_hex(rgb: Rgb) -> String {
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Flattens layers, bottom first, into one opaque "#rrggbb" (mixHex): each
/// over those below by its alpha, the first opaque. It blends in encoded
/// sRGB, as the browser paints a translucent layer, so a card matches the
/// window.
pub fn mix_hex(layers: &[Oklch]) -> String {
    let Some((first, rest)) = layers.split_first() else {
        return "#000000".into();
    };
    let mut rgb = encoded(*first);
    for x in rest {
        let fg = encoded(*x);
        rgb = [0, 1, 2].map(|i| fg[i] * x.a + rgb[i] * (1.0 - x.a));
    }
    encoded_hex(rgb)
}

/// The glass style the cards draw with: this window has none to pick, so
/// the React app's default, Frost (design/irori-ds.css).
pub const GLASS: &str = "frost";

/// A glass style's tint opacity (--glass-alpha) and white rim
/// (--glass-border), per mode.
fn glass_knobs(glass: &str, dark: bool) -> (f32, f32) {
    let (light, dk) = match glass {
        "clear" => ((0.3, 0.7), (0.3, 0.12)),
        "milk" => ((0.84, 0.7), (0.84, 0.12)),
        "rim" => ((0.56, 0.95), (0.56, 0.24)),
        "ember" => ((0.64, 0.7), (0.6, 0.12)),
        "liquid" => ((0.22, 0.9), (0.3, 0.3)),
        _ => ((0.62, 0.7), (0.62, 0.12)),
    };
    if dark { dk } else { light }
}

/// The status hues (they don't follow the theme): success, warn, error.
fn status_hues(dark: bool) -> [Oklch; 3] {
    if dark {
        [
            ok(0.78, 0.14, 150.0),
            ok(0.84, 0.13, 80.0),
            ok(0.68, 0.19, 27.0),
        ]
    } else {
        [
            ok(0.5, 0.13, 150.0),
            ok(0.52, 0.12, 70.0),
            ok(0.55, 0.2, 27.0),
        ]
    }
}

/// Below this the rim can't be told from the card.
const MIN_EDGE: f32 = 1.15;

/// One mode's card colours (cardColors), for theme `id` ("custom" with
/// `custom`'s colours).
pub fn card_colors(id: &str, custom: Option<&Custom>, glass: &str, dark: bool) -> CardColors {
    let t = match (id, custom) {
        ("custom", Some(c)) => custom_tokens(c, dark),
        _ => theme_tokens(theme(id), dark),
    };
    let (alpha, rim_a) = glass_knobs(glass, dark);
    let tint = if glass == "ember" {
        t.glass_ember_tint
    } else {
        t.glass_tint
    };
    let background = mix_hex(&[t.background, tint.alpha(alpha)]);
    let bg = hex_to_oklch(&background);
    // The glass rim; where it all but vanishes into a pale card, the
    // theme's hairline keeps an edge.
    let rim = mix_hex(&[bg, ok(1.0, 0.0, 0.0).alpha(rim_a)]);
    let border = if contrast(hex_to_oklch(&rim), bg, None) >= MIN_EDGE {
        rim
    } else {
        mix_hex(&[bg, t.border])
    };
    let hover = if dark {
        ok(1.0, 0.0, 0.0).alpha(0.1)
    } else {
        t.foreground.alpha(0.08)
    };
    let [success, warn, error] = status_hues(dark);
    CardColors {
        background,
        tint: mix_hex(&[tint]),
        foreground: mix_hex(&[t.foreground]),
        muted: mix_hex(&[bg, t.muted_fg]),
        border,
        hover: mix_hex(&[bg, hover]),
        button: mix_hex(&[bg, t.secondary]),
        primary: mix_hex(&[t.primary]),
        on_primary: mix_hex(&[t.primary, t.primary_fg]),
        // Info takes the theme's accent.
        info: mix_hex(&[t.primary]),
        success: mix_hex(&[success]),
        warn: mix_hex(&[warn]),
        error: mix_hex(&[error]),
    }
}

/// The whole look, both modes (cardLook): the Rust side picks light or
/// dark itself and follows the OS under "system".
pub fn card_look(p: &Prefs) -> CardLook {
    let id = current(p);
    let custom = saved_custom(p);
    let mode = match p.get::<String>(MODE).as_deref() {
        Some("light") => "light",
        Some("dark") => "dark",
        _ => "system",
    };
    // One alpha for both: the mode that will show, dark's under "system".
    let alpha = glass_knobs(GLASS, mode != "light").0;
    CardLook {
        theme: id.clone(),
        mode: mode.into(),
        glass: GLASS.into(),
        alpha: (f64::from(alpha) * 100.0).round() / 100.0,
        radius: 14.0,
        light: card_colors(&id, custom.as_ref(), GLASS, false),
        dark: card_colors(&id, custom.as_ref(), GLASS, true),
    }
}

/// Sends the cards the look when it changed (sendCardLook); a card in the
/// old look is no reason to complain, so a failure only logs.
pub fn send_card_look(s: &mut State) {
    if s.w.is_none() {
        return;
    }
    let look = card_look(&s.prefs);
    if s.settings.appearance.sent.as_ref() == Some(&look) {
        return;
    }
    s.settings.appearance.sent = Some(look.clone());
    s.spawn(
        move |w| w.set_card_look(look),
        |s, r| {
            if let Err(e) = r {
                s.settings.appearance.sent = None;
                eprintln!("irori: couldn't send the notification cards' look: {e}");
            }
        },
    );
}

/// Marks that the look was carried over (or there was none to carry), so
/// picking Irori's or System later (which clears the keys) isn't undone.
pub const CARRIED: &str = "appearance.carried";

/// On a first start, the look the React window last sent the cards
/// (settings.json "cards": theme and mode), when nothing is picked here.
pub fn carry_over(p: &mut Prefs, data_dir: &std::path::Path) {
    if p.get::<bool>(CARRIED).unwrap_or(false) {
        return;
    }
    let picked = p.get::<serde_json::Value>(THEME).is_some()
        || p.get::<serde_json::Value>(MODE).is_some()
        || p.get::<serde_json::Value>(CUSTOM).is_some();
    if !picked {
        let cards = std::fs::read(data_dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("cards").cloned());
        if let Some(c) = cards {
            let theme = c.get("theme").and_then(|v| v.as_str()).unwrap_or_default();
            if theme != IRORI && THEMES.iter().any(|t| t.id == theme) {
                p.set(THEME, theme);
            }
            if let Some(m @ ("light" | "dark")) = c.get("mode").and_then(|v| v.as_str()) {
                p.set(MODE, m);
            }
        }
    }
    p.set(CARRIED, true);
}

/// Every change in the pane: the cards follow.
fn changed(s: &mut State) {
    send_card_look(s);
}

// ---- the pane ----------------------------------------------------------------

#[derive(Default)]
pub struct Model {
    /// The colour pickers' hex fields.
    pub accent: ColorPickerState,
    pub tint: ColorPickerState,
    /// Which colour's picker is open: "accent" or "tint".
    pub picking: Option<&'static str>,
    pub error: String,
    /// The look the cards were last sent.
    pub sent: Option<CardLook>,
}

/// The pickers start from the theme's own accent and its neutrals' hue.
fn custom_or_base(p: &Prefs) -> (String, Custom) {
    let cur = current(p);
    let saved = saved_custom(p);
    let base = match (&*cur, &saved) {
        ("custom", Some(c)) => c.base.clone(),
        _ => cur.clone(),
    };
    let bt = theme(&base);
    let c = saved.unwrap_or_else(|| Custom {
        base: base.clone(),
        accent: oklch_to_hex(theme_tokens(bt, false).primary),
        tint: oklch_to_hex(ok(0.6, bt.seed.nc / 0.3, bt.seed.nh)),
    });
    (base, c)
}

/// Fills the colour fields from what's saved (when the pane opens).
pub fn load(s: &mut State) {
    let (_, c) = custom_or_base(&s.prefs);
    s.settings.appearance.accent = ColorPickerState::new(super::hex(&c.accent));
    s.settings.appearance.tint = ColorPickerState::new(super::hex(&c.tint));
    s.settings.appearance.error.clear();
}

fn pick_theme(s: &mut State, id: &str) {
    if id == IRORI {
        s.prefs.remove(THEME);
    } else {
        s.prefs.set(THEME, id);
    }
    if id != "custom" {
        s.prefs.remove(CUSTOM);
    }
    load(s);
    changed(s);
}

/// Sets one of your colours ("accent" or "tint") and switches to Custom.
/// Each change in the picker sets it, as main's colour input does.
/// Sets a custom colour. Each step of a drag or keystroke shows at once
/// (`save` false); it is written and sent to the cards when it settles.
fn pick_colour(s: &mut State, which: &str, hex: &str, save: bool) {
    let hex = hex.trim().to_ascii_lowercase();
    let hex = if hex.starts_with('#') {
        hex
    } else {
        format!("#{hex}")
    };
    if !valid_hex(&hex) {
        s.settings.appearance.error = "A colour is # and six hex digits, like #3b82f6.".into();
        return;
    }
    let (base, mut c) = custom_or_base(&s.prefs);
    c.base = if base == "custom" {
        "blue".into()
    } else {
        base
    };
    if which == "accent" {
        c.accent = hex;
    } else {
        c.tint = hex;
    }
    s.prefs.set_live(CUSTOM, &c);
    s.prefs.set_live(THEME, "custom");
    // The picker stays open (as main's native one does while you pick) and
    // keeps its field, so a hue drag turns the colour it started from.
    s.settings.appearance.error.clear();
    if save {
        s.prefs.flush();
        changed(s);
    }
}

/// The picker's colour as `#rrggbb`, without its alpha.
fn hex6(c: rune::Color) -> String {
    let mut h = c.hex();
    h.truncate(7);
    h
}

/// Saves a colour typed but not yet committed, for when a picker goes
/// away with its field still focused (no blur is left to commit it).
pub fn hidden(s: &mut State) {
    let a = &mut s.settings.appearance;
    let typed = [
        ("accent", a.accent.take_pending()),
        ("tint", a.tint.take_pending()),
    ];
    for (which, c) in typed {
        if let Some(c) = c {
            pick_colour(s, which, &hex6(c), true);
        }
    }
}

/// The swatches a colour field offers besides typing one.
const PRESETS: &[&str] = &[
    "#e5534b", "#e8762d", "#e0a526", "#8fbf3f", "#3fb37f", "#2bb5a8", "#3fa7d6", "#3b82f6",
    "#6b6cf0", "#8b6cf0", "#c25fd6", "#e5609a", "#8b8f98", "#1f2937",
];

pub fn view(s: &State, f: &mut Frame) -> El {
    let k = f.k;
    let dark = f.dark;
    let cur = current(&s.prefs);
    let saved = saved_custom(&s.prefs);
    let (base, _) = custom_or_base(&s.prefs);
    let base_theme = theme(if base == "custom" { "blue" } else { &base });

    let mut grid = col().key("settings-themes").w_full().gap(8.0);
    let mut cards: Vec<El> = THEMES
        .iter()
        .map(|t| {
            let id = t.id;
            swatch(
                f,
                id,
                t.name,
                t.note,
                theme_tokens(t, dark),
                cur == id,
                move |s| pick_theme(s, id),
            )
        })
        .collect();
    if let Some(c) = &saved {
        cards.push(swatch(
            f,
            "custom",
            "Custom",
            &format!("Your colours over {}", theme(&c.base).name),
            custom_tokens(c, dark),
            cur == "custom",
            |s| {
                s.prefs.set(THEME, "custom");
                load(s);
                changed(s);
            },
        ));
    }
    let mut it = cards.into_iter().peekable();
    let mut i = 0;
    while it.peek().is_some() {
        let mut r = row()
            .key(format!("settings-theme-row-{i}"))
            .w_full()
            .gap(8.0);
        for _ in 0..3 {
            match it.next() {
                Some(c) => r = r.child(c),
                None => {
                    r = r.child(
                        el().w(0.0)
                            .grow(1.0)
                            .min_w(0.0)
                            .padding(8.0)
                            .border(1.0, rune::Color::TRANSPARENT),
                    )
                }
            }
        }
        grid = grid.child(r);
        i += 1;
    }

    let m = &s.settings.appearance;
    let (_, custom) = custom_or_base(&s.prefs);
    // main's `<input type="color">`: the colour, which opens a picker (kit
    // ColorPicker) under the row, and its name.
    let colour = |f: &mut Frame, which: &'static str, label: &str, value: &str| -> El {
        let k = f.k;
        let open = m.picking == Some(which);
        let shown = if valid_hex(value) {
            super::hex(value)
        } else {
            k.c.layer_02
        };
        row()
            .key(format!("settings-colour-{which}"))
            .gap(8.0)
            .center_y()
            .child(
                ColorSwatch::new(shown)
                    .size(32.0)
                    .label(format!("{label}: pick a colour"))
                    .opener(open)
                    .on_press(move |s: &mut State| {
                        hidden(s);
                        let p = &mut s.settings.appearance.picking;
                        *p = if *p == Some(which) { None } else { Some(which) };
                    })
                    .build(k)
                    .key(format!("settings-colour-{which}-swatch")),
            )
            .child(text(label.to_string()).font(k.ty.ui).color(k.c.text_base))
    };
    let colours = row()
        .key("settings-colours")
        .w_full()
        .gap(16.0)
        .center_y()
        .flex_wrap()
        .child(colour(f, "accent", "Accent", &custom.accent))
        .child(colour(f, "tint", "Background tint", &custom.tint))
        .child(
            Button::new(format!("Reset to {}", base_theme.name))
                .variant(ButtonVariant::Outline)
                .disabled(saved.is_none() && cur != "custom")
                .on_press(move |s: &mut State| {
                    let b = custom_or_base(&s.prefs).1.base;
                    pick_theme(s, &b);
                })
                .build(k),
        );
    let mut your = col()
        .key("settings-your-colours")
        .w_full()
        .gap(8.0)
        .child(text("Your colours").font(k.ty.ui).color(k.c.text_base))
        .child(hint(
            f,
            "Pick an accent and a background tint. Irori keeps the lightness, so text stays readable in light and dark.",
        ))
        .child(colours);
    if let Some(which) = m.picking {
        let presets: Vec<rune::Color> = PRESETS.iter().map(|p| super::hex(p)).collect();
        let (state, value) = if which == "accent" {
            (&m.accent, &custom.accent)
        } else {
            (&m.tint, &custom.tint)
        };
        let picker = ColorPicker::new(
            format!("settings-picker-{which}"),
            state,
            move |s: &mut State| {
                if which == "accent" {
                    &mut s.settings.appearance.accent
                } else {
                    &mut s.settings.appearance.tint
                }
            },
        )
        .value(super::hex(value))
        .presets(&presets)
        .columns(7)
        .hue(true)
        .on_change(move |s: &mut State, c: rune::Color, _: &mut Tx| {
            pick_colour(s, which, &hex6(c), false);
        })
        .on_commit(move |s: &mut State, c: rune::Color, _: &mut Tx| {
            pick_colour(s, which, &hex6(c), true);
        })
        .build(k);
        your = your.child(picker);
    }
    if !m.error.is_empty() {
        your = your.child(super::error_line(f, &m.error));
    }

    let mode = s.prefs.get::<String>(MODE);
    let sel = match mode.as_deref() {
        Some("light") => 0,
        Some("dark") => 1,
        _ => 2,
    };
    col()
        .key("settings-appearance")
        .w_full()
        .gap(16.0)
        .child(
            col()
                .w_full()
                .gap(8.0)
                .child(text("Theme").font(k.ty.ui).color(k.c.text_base))
                .child(grid),
        )
        .child(your)
        .child(
            row()
                .key("settings-mode")
                .w_full()
                .center_y()
                .gap(12.0)
                .child(text("Light or dark").font(k.ty.ui).color(k.c.text_base))
                .child(el().grow(1.0))
                .child(
                    SegmentedControl::new(&["Light", "Dark", "System"], Some(sel))
                        .label("Light or dark")
                        .fit()
                        .on_pick(|s: &mut State, i: usize, _: &mut Tx| {
                            match i {
                                0 => s.prefs.set(MODE, "light"),
                                1 => s.prefs.set(MODE, "dark"),
                                _ => s.prefs.remove(MODE),
                            }
                            changed(s);
                        })
                        .build(k),
                ),
        )
        .child(muted_line(f, "Saved on this device."))
}

fn swatch(
    f: &Frame,
    id: &str,
    name: &str,
    note: &str,
    t: Tokens,
    checked: bool,
    pick: impl Fn(&mut State) + 'static,
) -> El {
    let k = f.k;
    let mut label = row()
        .w_full()
        .center_y()
        .child(text(name.to_string()).font(k.ty.ui).color(k.c.text_base))
        .child(el().grow(1.0));
    if checked {
        label = label.child(rune::icon(&icons::CHECK, 14.0).color(k.c.text_accent));
    }
    col()
        .key(format!("settings-swatch-{id}"))
        // Width 0 and grow: three equal columns, a short last row too.
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .padding(8.0)
        .radius(8.0)
        .border(
            if checked { 2.0 } else { 1.0 },
            if checked {
                t.primary.color()
            } else {
                k.c.border_base
            },
        )
        .role("radio")
        .checked(checked)
        .label(name.to_string())
        .help(note.to_string())
        .focusable(true)
        .on_press(pick)
        .child(
            row()
                .w_full()
                .h(40.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(6.0)
                .border(1.0, k.c.border_base)
                .fill(t.background.color())
                .child(el().size(16.0, 16.0).radius(8.0).fill(t.primary.color()))
                .child(
                    el().grow(1.0)
                        .h(6.0)
                        .radius(3.0)
                        .fill(t.foreground.color())
                        .opacity(0.7),
                ),
        )
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks(t: &Tokens) -> Vec<(&'static str, f32)> {
        let bg = t.background;
        vec![
            ("foreground on background", contrast(t.foreground, bg, None)),
            ("muted text on background", contrast(t.muted_fg, bg, None)),
            (
                "primary text on primary",
                contrast(t.primary_fg, t.primary, Some(bg)),
            ),
            (
                "accent text on accent",
                contrast(t.accent_fg, t.accent, Some(bg)),
            ),
        ]
    }

    #[test]
    fn themes_pass_wcag_aa() {
        for t in THEMES.iter().filter(|t| t.id != IRORI) {
            for dark in [false, true] {
                for (what, r) in checks(&theme_tokens(t, dark)) {
                    assert!(r >= AA, "{} dark={dark}: {what} is {r}", t.id);
                }
            }
        }
    }

    #[test]
    fn custom_colours_are_guarded() {
        for accent in [
            "#ff0000", "#ffff00", "#00ff00", "#0000ff", "#ffffff", "#000000", "#808080",
        ] {
            for tint in ["#000000", "#ffffff", "#ff00ff"] {
                for dark in [false, true] {
                    let c = Custom {
                        base: "blue".into(),
                        accent: accent.into(),
                        tint: tint.into(),
                    };
                    for (what, r) in checks(&custom_tokens(&c, dark)) {
                        if what.starts_with("primary") || what.starts_with("accent") {
                            assert!(r >= AA, "{accent} on {tint} dark={dark}: {what} is {r}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hex_round_trips_through_oklch() {
        for h in ["#3b82f6", "#d9480f", "#000000", "#ffffff", "#7a7a7a"] {
            assert_eq!(oklch_to_hex(hex_to_oklch(h)), h);
        }
    }

    /// What the React window sent for Forest (settings.json "cards" in
    /// the person's data dir, card #527), colour by colour.
    #[test]
    fn forest_cards_match_the_react_windows() {
        let want_dark = [
            ("background", "#141a13"),
            ("tint", "#181f18"),
            ("foreground", "#eef3f1"),
            ("muted", "#a9b4ad"),
            ("border", "#30352f"),
            ("hover", "#2c312b"),
            ("button", "#242a24"),
            ("primary", "#6fc884"),
            ("onPrimary", "#10190d"),
            ("info", "#6fc884"),
            ("success", "#6fd087"),
            ("warn", "#f7c15f"),
            ("error", "#f75e54"),
        ];
        let want_light = [
            ("background", "#f5faf7"),
            ("tint", "#fbfefc"),
            ("foreground", "#171c17"),
            ("muted", "#515b53"),
            ("border", "#dadfdc"),
            ("hover", "#e3e8e5"),
            ("button", "#e8edea"),
            ("primary", "#137738"),
            ("onPrimary", "#f9fdfb"),
            ("info", "#137738"),
            ("success", "#137738"),
            ("warn", "#945a00"),
            ("error", "#cc2827"),
        ];
        let near = |a: &str, b: &str| {
            let ch = |h: &str, i: usize| i32::from_str_radix(&h[1 + 2 * i..3 + 2 * i], 16).unwrap();
            (0..3).all(|i| (ch(a, i) - ch(b, i)).abs() <= 1)
        };
        for (dark, want) in [(true, &want_dark), (false, &want_light)] {
            let got = serde_json::to_value(card_colors("forest", None, GLASS, dark)).unwrap();
            for (name, hex) in want.iter() {
                let g = got[name].as_str().unwrap();
                assert!(
                    near(g, hex),
                    "dark={dark} {name}: {g}, the React window's {hex}"
                );
            }
        }
    }

    #[test]
    fn the_card_look_follows_the_choice() {
        let d = tempfile::tempdir().unwrap();
        let mut p = Prefs::load(d.path());
        let l = card_look(&p);
        assert_eq!(
            (l.theme.as_str(), l.mode.as_str(), l.glass.as_str()),
            (IRORI, "system", "frost")
        );
        assert_eq!((l.alpha, l.radius), (0.62, 14.0));
        p.set(THEME, "sakura");
        p.set(MODE, "light");
        let l2 = card_look(&p);
        assert_eq!((l2.theme.as_str(), l2.mode.as_str()), ("sakura", "light"));
        assert_ne!(l2.light, l.light);
        p.set(
            CUSTOM,
            Custom {
                base: "forest".into(),
                accent: "#ff8800".into(),
                tint: "#112233".into(),
            },
        );
        p.set(THEME, "custom");
        let l3 = card_look(&p);
        assert_eq!(l3.theme, "custom");
        assert!(valid_hex(&l3.dark.primary));
    }

    #[test]
    fn mix_hex_blends_in_encoded_srgb() {
        let black = hex_to_oklch("#000000");
        let white = ok(1.0, 0.0, 0.0);
        // Half white over black is 50% grey in encoded sRGB (#7f or #80,
        // 127.5 rounded), not linear light's #bcbcbc.
        let grey = mix_hex(&[black, white.alpha(0.5)]);
        assert!(grey == "#7f7f7f" || grey == "#808080", "{grey}");
        assert_eq!(mix_hex(&[white]), "#ffffff");
    }

    fn settings_json(dir: &std::path::Path, theme: &str, mode: &str) {
        let v = serde_json::json!({"cards": {"theme": theme, "mode": mode, "glass": "frost"}});
        std::fs::write(dir.join("settings.json"), v.to_string()).unwrap();
    }

    #[test]
    fn the_react_windows_look_is_carried_over_once() {
        // Forest, dark: both carried.
        let d = tempfile::tempdir().unwrap();
        settings_json(d.path(), "forest", "dark");
        let mut p = Prefs::load(d.path());
        carry_over(&mut p, d.path());
        assert_eq!(current(&p), "forest");
        assert_eq!(p.get::<String>(MODE).as_deref(), Some("dark"));
        // Picking Irori's own and System later isn't undone at the next start.
        p.remove(THEME);
        p.remove(MODE);
        let mut p = Prefs::load(d.path());
        carry_over(&mut p, d.path());
        assert_eq!(current(&p), IRORI);
        assert_eq!(p.get::<String>(MODE), None);

        // System, and a theme this build doesn't know: nothing set.
        let d = tempfile::tempdir().unwrap();
        settings_json(d.path(), "custom", "system");
        let mut p = Prefs::load(d.path());
        carry_over(&mut p, d.path());
        assert_eq!(current(&p), IRORI);
        assert_eq!(p.get::<String>(MODE), None);

        // Something already picked here wins.
        let d = tempfile::tempdir().unwrap();
        settings_json(d.path(), "forest", "light");
        let mut p = Prefs::load(d.path());
        p.set(MODE, "dark");
        carry_over(&mut p, d.path());
        assert_eq!(current(&p), IRORI);
        assert_eq!(p.get::<String>(MODE).as_deref(), Some("dark"));

        // No settings.json: nothing, and it isn't looked for again.
        let d = tempfile::tempdir().unwrap();
        let mut p = Prefs::load(d.path());
        carry_over(&mut p, d.path());
        assert_eq!(p.get::<bool>(CARRIED), Some(true));
    }

    #[test]
    fn the_saved_choice_keeps_what_it_knows() {
        let d = tempfile::tempdir().unwrap();
        let mut p = Prefs::load(d.path());
        assert_eq!(current(&p), IRORI);
        assert!(seed(&p, true).is_none());
        p.set(THEME, "nope");
        assert_eq!(current(&p), IRORI);
        p.set(THEME, "forest");
        assert_eq!(current(&p), "forest");
        assert!(seed(&p, false).is_some());
        // A custom theme without colours falls back.
        p.set(THEME, "custom");
        assert_eq!(current(&p), IRORI);
        p.set(
            CUSTOM,
            Custom {
                base: "forest".into(),
                accent: "#ff8800".into(),
                tint: "#112233".into(),
            },
        );
        assert_eq!(current(&p), "custom");
    }
}
