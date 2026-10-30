//! Colors, palettes and the app-level setup (go-toast config.go).

use std::fmt;

/// An sRGB color made with [`rgb`] or [`hex`]. The zero `Color` is "unset"
/// (a palette fills it in), so black is `rgb(0, 0, 0)`, not `Color(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Color(pub u32);

const COLOR_SET: u32 = 0xFF00_0000;

/// The color r, g, b.
pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color(COLOR_SET | (r as u32) << 16 | (g as u32) << 8 | b as u32)
}

/// Parses `#rrggbb` or `rrggbb`. Panics on anything else: it is meant for
/// palette literals.
pub fn hex(s: &str) -> Color {
    let t = s.strip_prefix('#').unwrap_or(s);
    match u32::from_str_radix(t, 16) {
        Ok(v) if t.len() == 6 && t.bytes().all(|b| b.is_ascii_hexdigit()) => Color(COLOR_SET | v),
        _ => panic!("toast: bad hex color {s:?}"),
    }
}

/// [`hex`] for constants.
const fn hx(v: u32) -> Color {
    Color(COLOR_SET | v)
}

impl Color {
    /// Whether c is unset.
    pub fn is_zero(self) -> bool {
        self.0 & COLOR_SET == 0
    }

    /// The channels.
    pub fn rgb(self) -> (u8, u8, u8) {
        ((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8)
    }

    /// The color as a Win32 COLORREF (0x00BBGGRR).
    pub fn colorref(self) -> u32 {
        let (r, g, b) = self.rgb();
        r as u32 | (g as u32) << 8 | (b as u32) << 16
    }

    /// 0xRRGGBB, the AppKit side's form.
    pub fn rgb24(self) -> u32 {
        self.0 & 0xFF_FFFF
    }

    /// WCAG relative luminance.
    fn luminance(self) -> f64 {
        let lin = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = self.rgb();
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
}

impl fmt::Display for Color {
    /// `#rrggbb`, or `unset`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return f.write_str("unset");
        }
        write!(f, "#{:06x}", self.0 & 0xFF_FFFF)
    }
}

/// Moves a fraction t of the way from a toward b (Go rounds half away from
/// zero, as `f64::round` does).
pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let (ar, ag, ab) = a.rgb();
    let (br, bg, bb) = b.rgb();
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    rgb(m(ar, br), m(ag, bg), m(ab, bb))
}

/// The WCAG contrast ratio between a and b (1..21).
pub fn contrast(a: Color, b: Color) -> f64 {
    let (mut la, mut lb) = (a.luminance(), b.luminance());
    if la < lb {
        std::mem::swap(&mut la, &mut lb);
    }
    (la + 0.05) / (lb + 0.05)
}

/// A card's kind: its accent, icon and sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Kind {
    #[default]
    Info,
    Success,
    Warn,
    Error,
}

impl Kind {
    /// The kind as go-toast's int (the AppKit view's `kind`).
    pub fn index(self) -> i32 {
        match self {
            Kind::Info => 0,
            Kind::Success => 1,
            Kind::Warn => 2,
            Kind::Error => 3,
        }
    }
}

/// The per-kind colors of a card's stripe and icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Accents {
    pub info: Color,
    pub success: Color,
    pub warn: Color,
    pub error: Color,
}

impl Accents {
    pub fn for_kind(&self, k: Kind) -> Color {
        match k {
            Kind::Success => self.success,
            Kind::Warn => self.warn,
            Kind::Error => self.error,
            Kind::Info => self.info,
        }
    }
}

/// The cards' colors. Background, foreground, muted, border and accent are
/// the ones to set; the rest are derived when left unset. Any of the five
/// left unset comes from [`DARK`], and a wholly zero palette is Dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Accents,
    /// The x on a card the cursor isn't over; default muted 40% toward background.
    pub faint: Color,
    /// Hover fill; default background 8% toward foreground.
    pub hover: Color,
    /// Secondary button fill; default background 5% toward foreground.
    pub button: Color,
    /// The first button; default accent.info.
    pub primary: Color,
    /// Default primary 15% toward foreground.
    pub primary_hover: Color,
    /// Text on the first button; default whichever of background and
    /// foreground contrasts more with primary.
    pub on_primary: Color,
}

/// Toru's palette: near-black cards with a blue accent. Info and success
/// share the blue; warn is amber, error red. Irori's cards use it.
pub const DARK: Palette = Palette {
    background: hx(0x111213),
    foreground: hx(0xfafafa),
    muted: hx(0x9ea1a7),
    border: hx(0x2c2e31),
    accent: Accents {
        info: hx(0x4cabfd),
        success: hx(0x4cabfd),
        warn: hx(0xf5ae39),
        error: hx(0xea3c3f),
    },
    faint: hx(0x5c5f66),
    hover: hx(0x26292e),
    button: hx(0x202225),
    primary: hx(0x4cabfd),
    primary_hover: hx(0x6fbcff),
    on_primary: hx(0x060a13),
};

/// White cards with slate text. Pair it with a corner radius of about 10.
pub const LIGHT: Palette = Palette {
    background: hx(0xffffff),
    foreground: hx(0x0f172a),
    muted: hx(0x64748b),
    border: rgb(203, 213, 225),
    accent: Accents {
        info: hx(0x2563eb),
        success: hx(0x15803d),
        warn: hx(0xc2620a),
        error: hx(0xef4444),
    },
    faint: Color(0),
    hover: Color(0),
    button: Color(0),
    primary: Color(0),
    primary_hover: Color(0),
    on_primary: Color(0),
};

impl Palette {
    /// Every unset color filled.
    pub fn resolved(self) -> Palette {
        let mut p = self;
        if p == Palette::default() {
            p = DARK;
        }
        fn fill(c: &mut Color, def: Color) {
            if c.is_zero() {
                *c = def;
            }
        }
        fill(&mut p.background, DARK.background);
        fill(&mut p.foreground, DARK.foreground);
        fill(&mut p.muted, DARK.muted);
        fill(&mut p.border, DARK.border);
        fill(&mut p.accent.info, DARK.accent.info);
        fill(&mut p.accent.success, DARK.accent.success);
        fill(&mut p.accent.warn, DARK.accent.warn);
        fill(&mut p.accent.error, DARK.accent.error);
        fill(&mut p.faint, mix(p.muted, p.background, 0.4));
        fill(&mut p.hover, mix(p.background, p.foreground, 0.08));
        fill(&mut p.button, mix(p.background, p.foreground, 0.05));
        fill(&mut p.primary, p.accent.info);
        fill(&mut p.primary_hover, mix(p.primary, p.foreground, 0.15));
        let on = if contrast(p.foreground, p.primary) > contrast(p.background, p.primary) {
            p.foreground
        } else {
            p.background
        };
        fill(&mut p.on_primary, on);
        p
    }
}

/// The app-level setup, set once at startup with [`crate::configure`]
/// before the first post. Start from [`Config::default`]: its
/// `capture_exclude` is true.
///
/// Not ported: go-toast's `IgnoreInput` and `HoldClickThrough` (Toru's
/// auto clicker); Irori never set them (inbox.md §3.13).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The windows' title and the log prefix (`irori: toast: ...`).
    pub app_name: String,
    /// The window classes: the cards are `class_prefix`, the hidden host
    /// `class_prefix + "Host"`. Empty derives it from the app name.
    pub class_prefix: String,
    /// The zero palette is Dark.
    pub palette: Palette,
    /// In dip; 0 is square (Toru, and Irori).
    pub corner_radius: f64,
    /// Keeps the cards out of screenshots and recordings
    /// (WDA_EXCLUDEFROMCAPTURE, NSWindowSharingNone).
    pub capture_exclude: bool,
}

impl Default for Config {
    /// App name "Toast", Dark, square, excluded from capture.
    fn default() -> Self {
        Config {
            app_name: "Toast".into(),
            class_prefix: String::new(),
            palette: DARK,
            corner_radius: 0.0,
            capture_exclude: true,
        }
    }
}

pub const MAX_CORNER_RADIUS: f64 = 24.0;

impl Config {
    /// Defaults filled and out-of-range values clamped.
    pub fn normalized(mut self) -> Config {
        self.app_name = self.app_name.trim().to_string();
        if self.app_name.is_empty() {
            self.app_name = Config::default().app_name;
        }
        if self.class_prefix.is_empty() {
            self.class_prefix = class_prefix_for(&self.app_name);
        }
        self.palette = self.palette.resolved();
        if self.corner_radius.is_nan() || self.corner_radius < 0.0 {
            self.corner_radius = 0.0;
        } else if self.corner_radius > MAX_CORNER_RADIUS {
            self.corner_radius = MAX_CORNER_RADIUS;
        }
        self
    }

    pub fn host_class_name(&self) -> String {
        format!("{}Host", self.class_prefix)
    }

    pub fn card_class_name(&self) -> String {
        self.class_prefix.clone()
    }

    /// `<app>: toast: `, lowercased.
    pub fn log_prefix(&self) -> String {
        let a = self.app_name.to_lowercase();
        if a == "toast" {
            return "toast: ".into();
        }
        format!("{a}: toast: ")
    }
}

/// A window class prefix from an app name: its ASCII letters and digits
/// plus `Toast`.
pub fn class_prefix_for(app: &str) -> String {
    let mut s: String = app.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !s.ends_with("Toast") {
        s.push_str("Toast");
    }
    s
}
