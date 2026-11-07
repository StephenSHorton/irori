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

use rune::{El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, PaletteSeed, SegmentedControl, TextInput};
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
    let Some(v) = (t.len() == 6).then(|| u32::from_str_radix(t, 16).ok()).flatten() else {
        return ok(0.6, 0.15, 250.0);
    };
    let lin = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|c| {
        let s = c as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    });
    let [r, g, b] = lin;
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let ll = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let aa = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let c = aa.hypot(bb);
    let h = if c < 1e-4 { 0.0 } else { bb.atan2(aa).to_degrees() };
    ok(ll, c, h)
}

/// The nearest sRGB "#rrggbb" (for the colour fields).
pub fn oklch_to_hex(x: Oklch) -> String {
    let enc = |v: f32| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
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
    #[allow(dead_code)] // Checked for contrast in the tests.
    pub muted_fg: Oklch,
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
        seed: Seed { nh: 270.0, nc: 0.006, light: acc(292.0, 0.17, Some(0.6)), dark: acc(292.0, 0.17, Some(0.6)) },
        exact: None,
    },
    Theme {
        id: "blue",
        name: "Blue",
        note: "Cool slate, a clear blue accent",
        seed: Seed { nh: 255.0, nc: 0.014, light: acc(255.0, 0.17, Some(0.53)), dark: acc(250.0, 0.14, Some(0.74)) },
        exact: None,
    },
    Theme {
        id: "hearth",
        name: "Hearth",
        note: "Warm ash, an ember accent",
        seed: Seed { nh: 55.0, nc: 0.012, light: acc(38.0, 0.17, None), dark: acc(52.0, 0.15, None) },
        // Hearth keeps #266's numbers.
        exact: Some([
            (k(0.955, 0.008, 70.0), k(0.22, 0.012, 50.0), k(0.56, 0.17, 38.0)),
            (k(0.17, 0.012, 45.0), k(0.96, 0.006, 70.0), k(0.75, 0.15, 52.0)),
        ]),
    },
    Theme {
        id: "midnight",
        name: "Midnight",
        note: "Deep indigo, a violet glow",
        seed: Seed { nh: 280.0, nc: 0.02, light: acc(285.0, 0.17, Some(0.5)), dark: acc(290.0, 0.13, Some(0.74)) },
        exact: None,
    },
    Theme {
        id: "forest",
        name: "Forest",
        note: "Moss and bark, a green accent",
        seed: Seed { nh: 145.0, nc: 0.014, light: acc(150.0, 0.13, Some(0.5)), dark: acc(150.0, 0.13, Some(0.76)) },
        exact: None,
    },
    Theme {
        id: "sakura",
        name: "Sakura",
        note: "Petal pink on soft stone",
        seed: Seed { nh: 350.0, nc: 0.012, light: acc(355.0, 0.16, Some(0.54)), dark: acc(355.0, 0.12, Some(0.78)) },
        exact: None,
    },
    Theme {
        id: "mono",
        name: "Mono",
        note: "Greys only; the status colours keep theirs",
        seed: Seed { nh: 0.0, nc: 0.0, light: acc(0.0, 0.0, Some(0.3)), dark: acc(0.0, 0.0, Some(0.88)) },
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
        let (bg, fg) = if dark { ("#0b0c0f", "#e8e9ec") } else { ("#ffffff", "#17181c") };
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
        let best = if contrast(fgs[0], t.primary, Some(bg)) >= contrast(fgs[1], t.primary, Some(bg)) {
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

// ---- the pane ----------------------------------------------------------------

#[derive(Default)]
pub struct Model {
    pub accent: rune::TextEdit,
    pub tint: rune::TextEdit,
    /// Which colour's presets are open: "accent" or "tint".
    pub picking: Option<&'static str>,
    pub error: String,
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
    s.settings.appearance.accent = rune::TextEdit::new(c.accent);
    s.settings.appearance.tint = rune::TextEdit::new(c.tint);
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
}

/// Sets one of your colours ("accent" or "tint") and switches to Custom.
fn pick_colour(s: &mut State, which: &str, hex: &str) {
    let hex = hex.trim().to_ascii_lowercase();
    let hex = if hex.starts_with('#') { hex } else { format!("#{hex}") };
    if !valid_hex(&hex) {
        s.settings.appearance.error = "A colour is # and six hex digits, like #3b82f6.".into();
        return;
    }
    let (base, mut c) = custom_or_base(&s.prefs);
    c.base = if base == "custom" { "blue".into() } else { base };
    if which == "accent" {
        c.accent = hex;
    } else {
        c.tint = hex;
    }
    s.prefs.set(CUSTOM, &c);
    s.prefs.set(THEME, "custom");
    s.settings.appearance.picking = None;
    load(s);
}

/// The swatches a colour field offers besides typing one.
const PRESETS: &[&str] = &[
    "#e5534b", "#e8762d", "#e0a526", "#8fbf3f", "#3fb37f", "#2bb5a8", "#3fa7d6", "#3b82f6", "#6b6cf0",
    "#8b6cf0", "#c25fd6", "#e5609a", "#8b8f98", "#1f2937",
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
            swatch(f, id, t.name, t.note, theme_tokens(t, dark), cur == id, move |s| pick_theme(s, id))
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
            },
        ));
    }
    let mut it = cards.into_iter().peekable();
    let mut i = 0;
    while it.peek().is_some() {
        let mut r = row().key(format!("settings-theme-row-{i}")).w_full().gap(8.0);
        for _ in 0..3 {
            match it.next() {
                Some(c) => r = r.child(c),
                None => r = r.child(el().w(0.0).grow(1.0).min_w(0.0).padding(8.0).border(1.0, rune::Color::TRANSPARENT)),
            }
        }
        grid = grid.child(r);
        i += 1;
    }

    let m = &s.settings.appearance;
    let colour = |f: &mut Frame, which: &'static str, label: &str, edit: &rune::TextEdit| -> El {
        let k = f.k;
        let value = edit.text.clone();
        let shown = if valid_hex(&value) { super::hex(&value) } else { k.c.layer_02 };
        col()
            .key(format!("settings-colour-{which}"))
            .gap(6.0)
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        el().key(format!("settings-colour-{which}-swatch"))
                            .size(28.0, 28.0)
                            .radius(6.0)
                            .fill(shown)
                            .border(1.0, k.c.border_base)
                            .role("button")
                            .label(format!("{label}: pick a colour"))
                            .focusable(true)
                            .on_press(move |s: &mut State| {
                                let p = &mut s.settings.appearance.picking;
                                *p = if *p == Some(which) { None } else { Some(which) };
                            }),
                    )
                    .child(
                        TextInput::new(edit)
                            .label(label.to_string())
                            .placeholder("#rrggbb")
                            .width(Some(96.0))
                            .on_edit(move |s: &mut State| {
                                if which == "accent" {
                                    &mut s.settings.appearance.accent
                                } else {
                                    &mut s.settings.appearance.tint
                                }
                            })
                            .on_submit(move |s: &mut State, _: &mut Tx| {
                                let v = if which == "accent" {
                                    s.settings.appearance.accent.text.clone()
                                } else {
                                    s.settings.appearance.tint.text.clone()
                                };
                                pick_colour(s, which, &v);
                            })
                            .build(k)
                            .key(format!("settings-colour-{which}-field")),
                    )
                    .child(text(label.to_string()).font(k.ty.ui).color(k.c.text_base)),
            )
    };
    let mut colours = row()
        .key("settings-colours")
        .w_full()
        .gap(16.0)
        .center_y()
        .flex_wrap()
        .child(colour(f, "accent", "Accent", &m.accent))
        .child(colour(f, "tint", "Background tint", &m.tint))
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
    if m.picking.is_none() {
        colours = colours.child(el());
    }
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
        let mut presets = row().key("settings-presets").gap(6.0).flex_wrap();
        for (i, p) in PRESETS.iter().enumerate() {
            let p = *p;
            presets = presets.child(
                el().key(format!("settings-preset-{i}"))
                    .size(20.0, 20.0)
                    .radius(10.0)
                    .fill(super::hex(p))
                    .role("button")
                    .label(p)
                    .focusable(true)
                    .on_press(move |s: &mut State| pick_colour(s, which, p)),
            );
        }
        your = your.child(presets);
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
                        .on_pick(|s: &mut State, i: usize, _: &mut Tx| match i {
                            0 => s.prefs.set(MODE, "light"),
                            1 => s.prefs.set(MODE, "dark"),
                            _ => s.prefs.remove(MODE),
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
        .border(if checked { 2.0 } else { 1.0 }, if checked { t.primary.color() } else { k.c.border_base })
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
                .child(el().grow(1.0).h(6.0).radius(3.0).fill(t.foreground.color()).opacity(0.7)),
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
            ("primary text on primary", contrast(t.primary_fg, t.primary, Some(bg))),
            ("accent text on accent", contrast(t.accent_fg, t.accent, Some(bg))),
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
        for accent in ["#ff0000", "#ffff00", "#00ff00", "#0000ff", "#ffffff", "#000000", "#808080"] {
            for tint in ["#000000", "#ffffff", "#ff00ff"] {
                for dark in [false, true] {
                    let c = Custom { base: "blue".into(), accent: accent.into(), tint: tint.into() };
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
        p.set(CUSTOM, Custom { base: "forest".into(), accent: "#ff8800".into(), tint: "#112233".into() });
        assert_eq!(current(&p), "custom");
    }
}
