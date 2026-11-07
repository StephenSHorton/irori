//! Faces (the React app's cards/faces.tsx and profile/face.tsx): an
//! agent's 5x5 identicon from its key, in its device's colour once there
//! are two devices (card #22), with the live dot; the person's user icon;
//! a row of faces with "+N"; the talk bubble; and a person's face (their
//! avatar, else their initial on their colour, card #393).

use irori_window::cards::AgentRef;
use rune::{Color, El, Shape, canvas, el, icon, row, text};

use super::{agent_hover, avatar};
use crate::logic::agent_status::{Who, agent_status, now_ms};
use crate::logic::names::display_name;
use crate::ui::theme::status;
use crate::ui::{Frame, State, icons};

/// 32-bit FNV-1a over UTF-16 code units (minidenticons' own hash leaves
/// near keys too sparse).
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for u in s.encode_utf16() {
        h ^= u32::from(u);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// minidenticon's lit squares for one hash: the 15 low bits fill three
/// columns, mirrored.
fn lit(hash: u32) -> Vec<(u8, u8)> {
    (0..25u32)
        .filter(|i| hash & (1 << (i % 15)) != 0)
        .map(|i| {
            let x = if i > 14 { 7 - i / 5 } else { i / 5 };
            (x as u8, (i % 5) as u8)
        })
        .collect()
}

/// The lit squares of a seed's identicon; a pattern under 8 squares is
/// drawn again from a salted seed (`seed#1` ..).
pub fn cells(seed: &str) -> Vec<(u8, u8)> {
    let mut c = Vec::new();
    for salt in 0..8 {
        let s = if salt == 0 { seed.to_string() } else { format!("{seed}#{salt}") };
        c = lit(fnv1a(&s));
        if c.len() >= 8 {
            break;
        }
    }
    c
}

/// A seed's identicon, `size` points square, in `color`.
pub fn identicon(seed: &str, size: f32, color: Color) -> El {
    let mut sh = Shape::new().view_box(5.0, 5.0);
    for (x, y) in cells(seed) {
        let (x, y) = (f32::from(x), f32::from(y));
        sh = sh.fill(vec![[x, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]], color);
    }
    canvas(sh).size(size, size).a11y_hidden()
}

/// The colours a device (or a person) can wear (coord::devcolor::PALETTE),
/// with the window's shade of each (the React app's index.css
/// `--device-*`), as oklch.
pub const DEVICE_COLORS: &[(&str, (f32, f32, f32))] = &[
    ("teal", (0.76, 0.12, 185.0)),
    ("orange", (0.74, 0.14, 50.0)),
    ("pink", (0.74, 0.14, 350.0)),
    ("lime", (0.8, 0.15, 125.0)),
    ("cyan", (0.78, 0.11, 215.0)),
    ("indigo", (0.7, 0.13, 275.0)),
    ("sand", (0.78, 0.06, 80.0)),
];

/// A device colour's shade by its palette name; None for "" or one
/// unknown.
pub fn device_color(name: &str) -> Option<Color> {
    DEVICE_COLORS.iter().find(|(n, _)| *n == name).map(|(_, (l, c, h))| rune::color::oklch(*l, *c, *h))
}

/// An agent face's colour outside a face: amber while it isn't
/// answering, else its device's, else the accent.
pub fn face_color(f: &Frame, status_: &str, device: &str) -> Color {
    if status_ == "unresponsive" {
        return status::decide();
    }
    device_color(device).unwrap_or(f.k.c.icon_accent)
}

/// How a face is drawn.
#[derive(Clone, Copy, Debug, Default)]
pub struct FaceOpts {
    /// A watcher: grey, no dot.
    pub watching: bool,
    pub no_dot: bool,
    /// No hover card (the face already sits in one).
    pub no_hover: bool,
    /// The small face beside a name in a thread row.
    pub inline: bool,
}

/// One agent (or the person) on a card (cards #79, #104, #108, #179,
/// #489): `key` must be unique in the view.
pub fn face(s: &State, f: &mut Frame, key: &str, who: &AgentRef, o: FaceOpts) -> El {
    let k = f.k;
    let person = who.status == "person";
    let gone = who.status == "gone";
    let tint = if o.watching { None } else { device_color(&who.color) };
    let quiet = !person && !o.watching && who.status == "unresponsive";
    let name = display_name(who);
    let said = agent_status(&Who::from(who), None, now_ms()).text;
    let title = [
        name.clone(),
        who.where_.clone(),
        if who.device.is_empty() { String::new() } else { format!("on {}", who.device) },
        if said == "idle" { String::new() } else { said },
        if who.conflict.is_empty() { String::new() } else { format!("lease conflict on {}", who.conflict) },
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let faded = gone || who.offline;
    let ink = if quiet {
        status::decide()
    } else if o.watching {
        k.c.text_muted.alpha(0.7)
    } else {
        tint.unwrap_or(k.c.icon_accent)
    };
    let face = if o.inline {
        let ink = if person || gone { k.c.text_muted } else { ink };
        let inner = if person {
            icon(&icons::USER, 14.0).color(ink)
        } else {
            identicon(&who.key, 14.0, ink)
        };
        row().key(key.to_string()).size(14.0, 14.0).center().child(inner)
    } else {
        let inner = if person {
            icon(&icons::USER, 20.0).color(ink)
        } else {
            identicon(&who.key, 20.0, ink)
        };
        let mut b = el()
            .key(key.to_string())
            .size(26.0, 26.0)
            
            .border(1.0, k.c.text_muted.alpha(0.35))
            .child(el().absolute(2.0, 2.0).size(22.0, 22.0).center().child(inner));
        if !who.conflict.is_empty() {
            b = b.border(2.0, status::decide());
        }
        if !o.watching && !o.no_dot && who.live && !who.offline && !person {
            b = b.child(
                el().absolute(19.0, 19.0)
                    .size(8.0, 8.0)
                    .radius(4.0)
                    .fill(status::working())
                    .border(1.5, k.c.bg_base)
                    .a11y_hidden(),
            );
        }
        b
    };
    let face = face.role("img").label(title.clone()).opacity(if faded { 0.55 } else { 1.0 });
    if o.no_hover || person {
        return face.help(title);
    }
    agent_hover::trigger(s, f, key, who, face)
}

/// How many faces a tile's working or watching row shows before "+N".
pub const FACES_MAX: usize = 5;

/// A row of faces; with `max`, the rest as "+N" whose tooltip names them.
pub fn faces(s: &State, f: &mut Frame, key: &str, who: &[AgentRef], watching: bool, max: Option<usize>) -> El {
    let over = max.is_some_and(|m| who.len() > m);
    let shown = if over { max.unwrap_or(1).saturating_sub(1).max(1) } else { who.len() };
    let mut r = row().key(key.to_string()).gap(4.0).center_y().min_w(0.0);
    for (i, w) in who.iter().take(shown).enumerate() {
        r = r.child(face(
            s,
            f,
            &format!("{key}/{i}"),
            w,
            FaceOpts {
                watching,
                ..FaceOpts::default()
            },
        ));
    }
    let rest = &who[shown.min(who.len())..];
    if !rest.is_empty() {
        let names = rest.iter().map(display_name).collect::<Vec<_>>().join(", ");
        r = r.child(
            text(format!("+{}", rest.len()))
                .font(f.k.ty.text_xsmall)
                .color(f.k.c.text_muted)
                .help(names.clone())
                .label(format!("and {} more: {names}", rest.len())),
        );
    }
    r
}

/// Agents talking to each other on a card (card #22): their faces and a
/// speech bubble whose dots pulse in turn.
pub fn talk_bubble(s: &State, f: &mut Frame, key: &str, who: &[AgentRef]) -> Option<El> {
    if who.len() < 2 {
        return None;
    }
    let k = f.k;
    let names = who.iter().map(display_name).collect::<Vec<_>>().join(", ");
    let mut fs = row().key(format!("{key}/faces")).gap(2.0).center_y();
    for (i, w) in who.iter().enumerate() {
        fs = fs.child(face(
            s,
            f,
            &format!("{key}/{i}"),
            w,
            FaceOpts {
                no_dot: true,
                ..FaceOpts::default()
            },
        ));
    }
    let mut bubble = row()
        .key(format!("{key}/bubble"))
        .h(16.0)
        .px(6.0)
        .gap(3.0)
        .center_y()
        .radius(8.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_muted)
        .a11y_hidden();
    for i in 0..3 {
        bubble = bubble.child(
            el().key(format!("{key}/dot{i}"))
                .size(3.0, 3.0)
                .radius(1.5)
                .fill(k.c.text_base)
                .looping(rune::LoopSpec::opacity(0.9, f64::from(i) * 0.15, 0.25, 1.0)),
        );
    }
    Some(
        row()
            .key(key.to_string())
            .gap(4.0)
            .center_y()
            .help(format!("Talking on this card: {names}"))
            .label(format!("{names} are talking on this card"))
            .child(fs)
            .child(bubble),
    )
}

/// The green "live" dot in Who's on it.
pub fn live_dot() -> El {
    el().size(7.0, 7.0).radius(3.5).fill(status::working()).a11y_hidden()
}

/// A person's face's size (profile/face.tsx sm, md, lg, xl).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PersonSize {
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl PersonSize {
    fn px(self) -> (f32, f32) {
        match self {
            PersonSize::Sm => (20.0, 10.0),
            PersonSize::Md => (28.0, 12.0),
            PersonSize::Lg => (48.0, 18.0),
            PersonSize::Xl => (64.0, 24.0),
        }
    }
}

/// A person on the window (card #393): their avatar's pixels, else their
/// initial on their colour (a device colour name), else on grey.
pub fn person_face(f: &Frame, key: &str, name: &str, color: &str, pic: Option<&rune::Pixels>, size: PersonSize) -> El {
    let k = f.k;
    let (side, font) = size.px();
    let base = el()
        .key(key.to_string())
        .size(side, side)
        
        .radius(side / 2.0)
        .clip_rounded(side / 2.0)
        .role("img")
        .label(name.to_string())
        .help(name.to_string());
    if let Some(p) = pic {
        return base.pixels_fit(p, rune::SurfaceFit::Fill);
    }
    let (bg, fg) = match device_color(color) {
        Some(c) => (c, k.c.bg_base),
        None => (k.c.layer_03, k.c.text_muted),
    };
    base.fill(bg).center().child(
        text(avatar::initials(name))
            .font(rune::tok::TextStyle::new(font, 600.0))
            .color(fg)
            .a11y_hidden(),
    )
}

/// The person's own face: their avatar from the window (decoded in the
/// widgets model), else their initial on their colour.
pub fn my_face(s: &State, f: &Frame, key: &str, size: PersonSize) -> El {
    let name = if s.data.your_name.is_empty() { "You" } else { s.data.your_name.as_str() };
    person_face(f, key, name, &s.widgets.person_color, s.widgets.avatar.pixels.as_ref(), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identicons_are_mirrored_and_never_sparse() {
        for seed in ["sample-wren", "dev/heron-key", "a", ""] {
            let c = cells(seed);
            if !seed.is_empty() {
                assert!(c.len() >= 8 || c.len() == lit(fnv1a(&format!("{seed}#7"))).len(), "{seed}");
            }
            for &(x, y) in &c {
                assert!(x < 5 && y < 5);
                // Columns 0 and 4, 1 and 3 mirror each other.
                if x == 0 || x == 1 {
                    assert!(c.contains(&(4 - x, y)), "{seed}: {x},{y}");
                }
            }
        }
    }

    #[test]
    fn fnv1a_matches_the_js_hash() {
        // Math.imul-based FNV-1a of "a" (>>> 0).
        assert_eq!(fnv1a("a"), 0xe40c_292c);
        assert_eq!(fnv1a(""), 0x811c_9dc5);
    }

    #[test]
    fn device_colours_are_the_palette() {
        for c in ["teal", "orange", "pink", "lime", "cyan", "indigo", "sand"] {
            assert!(device_color(c).is_some());
        }
        assert!(device_color("").is_none());
    }
}
