//! Box drawing (U+2500-257F), block elements (U+2580-259F) and the
//! powerline wedges, drawn edge to edge in the cell as Go suzuri does
//! (suzuri-view's `boxdraw.js`): font ink has padding, so stacked │ and
//! joined ─ would show gaps. [`glyph`] answers the cell's rects; the view
//! fills them in the glyph's colour.

/// A rect in the cell's own points, `[x, y, w, h]`, and its ink's alpha
/// (the shades ░▒▓ are a wash, not a dither).
pub type Rect = ([f32; 4], f32);

const N: u8 = 1;
const S: u8 = 2;
const E: u8 = 4;
const W: u8 = 8;

fn div(a: f32, b: f32) -> f32 {
    (a / b).trunc()
}

struct C {
    w: f32,
    h: f32,
    cx: f32,
    cy: f32,
}

fn thick(c: &C, heavy: bool) -> f32 {
    let m = c.w.min(c.h);
    let t = if heavy { div(m, 6.0) } else { div(m, 10.0) };
    if heavy { t.max(2.0) } else { t.max(1.0) }
}

fn h_stroke(out: &mut Vec<Rect>, x0: f32, x1: f32, y: f32, t: f32) {
    let (x0, x1) = if x1 < x0 { (x1, x0) } else { (x0, x1) };
    if x1 > x0 {
        out.push(([x0, y - div(t, 2.0), x1 - x0, t], 1.0));
    }
}

fn v_stroke(out: &mut Vec<Rect>, x: f32, y0: f32, y1: f32, t: f32) {
    let (y0, y1) = if y1 < y0 { (y1, y0) } else { (y0, y1) };
    if y1 > y0 {
        out.push(([x - div(t, 2.0), y0, t, y1 - y0], 1.0));
    }
}

fn arms(out: &mut Vec<Rect>, c: &C, a: u8, heavy: bool) {
    let t = thick(c, heavy);
    let half = div(t, 2.0);
    if a & N != 0 {
        v_stroke(out, c.cx, 0.0, c.cy + half, t);
    }
    if a & S != 0 {
        v_stroke(out, c.cx, c.cy - half, c.h, t);
    }
    if a & E != 0 {
        h_stroke(out, c.cx - half, c.w, c.cy, t);
    }
    if a & W != 0 {
        h_stroke(out, 0.0, c.cx + half, c.cy, t);
    }
}

/// Light and heavy junctions: arms and whether heavy. Double corners are
/// heavy single arms, as in Go.
fn join(ch: char) -> Option<(u8, bool)> {
    Some(match ch {
        '┌' => (S | E, false),
        '┐' => (S | W, false),
        '└' => (N | E, false),
        '┘' => (N | W, false),
        '├' => (N | S | E, false),
        '┤' => (N | S | W, false),
        '┬' => (S | E | W, false),
        '┴' => (N | E | W, false),
        '┼' => (N | S | E | W, false),
        '┏' | '╔' => (S | E, true),
        '┓' | '╗' => (S | W, true),
        '┗' | '╚' => (N | E, true),
        '┛' | '╝' => (N | W, true),
        '┣' => (N | S | E, true),
        '┫' => (N | S | W, true),
        '┳' => (S | E | W, true),
        '┻' => (N | E | W, true),
        '╋' => (N | S | E | W, true),
        _ => return None,
    })
}

/// A quarter ellipse as small squares along it, then the two straight arms.
fn round_corner(out: &mut Vec<Rect>, c: &C, which: char) {
    use std::f32::consts::PI;
    let t = thick(c, false);
    let mut r = div(c.w.min(c.h), 2.0) - t;
    r = r.max(2.0);
    let m = div(c.w, 2.0);
    if r > m - 1.0 && m > 2.0 {
        r = m - 1.0;
    }
    let m = div(c.h, 2.0);
    if r > m - 1.0 && m > 2.0 {
        r = m - 1.0;
    }
    let (cx, cy) = (c.cx, c.cy);
    let (ox, oy, a0, a1) = match which {
        '╭' => (cx + r, cy + r, PI, 1.5 * PI),
        '╮' => (cx - r, cy + r, 1.5 * PI, 2.0 * PI),
        '╯' => (cx - r, cy - r, 0.0, 0.5 * PI),
        _ => (cx + r, cy - r, 0.5 * PI, PI),
    };
    let steps = ((a1 - a0).abs() * r + 0.5).trunc().clamp(8.0, 48.0) as usize;
    let half = div(t, 2.0);
    for i in 0..=steps {
        let th = a0 + (a1 - a0) * i as f32 / steps as f32;
        let x = ox + (r * th.cos()).round();
        let y = oy + (r * th.sin()).round();
        out.push(([x - half, y - half, t, t], 1.0));
    }
    match which {
        '╭' => {
            h_stroke(out, cx + r, c.w, cy, t);
            v_stroke(out, cx, cy + r, c.h, t);
        }
        '╮' => {
            h_stroke(out, 0.0, cx - r, cy, t);
            v_stroke(out, cx, cy + r, c.h, t);
        }
        '╯' => {
            h_stroke(out, 0.0, cx - r, cy, t);
            v_stroke(out, cx, 0.0, cy - r, t);
        }
        _ => {
            h_stroke(out, cx + r, c.w, cy, t);
            v_stroke(out, cx, 0.0, cy - r, t);
        }
    }
}

fn box_drawing(out: &mut Vec<Rect>, ch: char, c: &C) -> bool {
    let light = thick(c, false);
    match ch {
        '─' => h_stroke(out, 0.0, c.w, c.cy, light),
        '│' => v_stroke(out, c.cx, 0.0, c.h, light),
        '━' => h_stroke(out, 0.0, c.w, c.cy, thick(c, true)),
        '┃' => v_stroke(out, c.cx, 0.0, c.h, thick(c, true)),
        '═' => {
            h_stroke(out, 0.0, c.w, c.cy - light - 1.0, light);
            h_stroke(out, 0.0, c.w, c.cy + light + 1.0, light);
        }
        '║' => {
            v_stroke(out, c.cx - light - 1.0, 0.0, c.h, light);
            v_stroke(out, c.cx + light + 1.0, 0.0, c.h, light);
        }
        '╭' | '╮' | '╯' | '╰' => round_corner(out, c, ch),
        _ => match join(ch) {
            Some((a, heavy)) => arms(out, c, a, heavy),
            None => return false,
        },
    }
    true
}

fn block(out: &mut Vec<Rect>, ch: char, c: &C) {
    let (w, h) = (c.w, c.h);
    let r = |x: f32, y: f32, rw: f32, rh: f32| ([x, y, rw, rh], 1.0);
    let half_h = div(h, 2.0);
    let half_w = div(w, 2.0);
    out.push(match ch {
        '▀' => r(0.0, 0.0, w, half_h),
        '▄' => r(0.0, half_h, w, h - half_h),
        '▌' => r(0.0, 0.0, half_w, h),
        '▐' => r(half_w, 0.0, w - half_w, h),
        '▔' => r(0.0, 0.0, w, div(h, 8.0).max(1.0)),
        '▏' => r(0.0, 0.0, div(w, 8.0).max(1.0), h),
        '▎' => r(0.0, 0.0, div(w, 4.0).max(1.0), h),
        '▍' => r(0.0, 0.0, div(w * 3.0, 8.0).max(1.0), h),
        // Lower eighths: ▁ ▂ ▃ ▄ ▅ ▆ ▇ (▄ above).
        '▁'..='▇' => {
            let n = ch as u32 - '▁' as u32 + 1;
            let hh = div(h * n as f32, 8.0).max(1.0);
            r(0.0, h - hh, w, hh)
        }
        '░' => ([0.0, 0.0, w, h], 0.25),
        '▒' => ([0.0, 0.0, w, h], 0.5),
        '▓' => ([0.0, 0.0, w, h], 0.75),
        // █ and the rest of the block: Go fills the cell.
        _ => r(0.0, 0.0, w, h),
    });
}

/// A wedge (powerline ▶ ◀), one row of points at a time.
fn triangle(out: &mut Vec<Rect>, c: &C, right: bool) {
    let (w, h) = (c.w, c.h);
    let rows = h.round() as i32;
    let half = div(h, 2.0).max(1.0);
    for row in 0..rows {
        let row = row as f32;
        let span = if row <= half {
            div(w * row, half)
        } else {
            div(w * (h - 1.0 - row), half)
        }
        .max(1.0);
        let x = if right { 0.0 } else { w - span };
        out.push(([x, row, span, 1.0], 1.0));
    }
}

/// The rects for code point `cp` in a `cw` × `ch` cell, or None when the
/// font draws it.
pub fn glyph(cp: u32, cw: f32, ch: f32) -> Option<Vec<Rect>> {
    let c = C {
        w: cw,
        h: ch,
        cx: div(cw, 2.0),
        cy: div(ch, 2.0),
    };
    let r = char::from_u32(cp)?;
    let mut out = Vec::new();
    match cp {
        0x2580..=0x259f => block(&mut out, r, &c),
        0x2500..=0x257f => {
            if !box_drawing(&mut out, r, &c) {
                return None;
            }
        }
        0xe0b0 | 0x25b6 | 0x25b8 | 0x25ba => triangle(&mut out, &c, true),
        0xe0b2 | 0x25c0 | 0x25c2 | 0x25c4 => triangle(&mut out, &c, false),
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_reach_the_cell_edges() {
        let r = glyph('─' as u32, 8.0, 16.0).unwrap();
        assert_eq!(r.len(), 1);
        let ([x, _, w, _], _) = r[0];
        assert_eq!((x, w), (0.0, 8.0));
        let r = glyph('│' as u32, 8.0, 16.0).unwrap();
        let ([_, y, _, h], _) = r[0];
        assert_eq!((y, h), (0.0, 16.0));
    }

    #[test]
    fn corners_join_at_the_centre() {
        let r = glyph('┌' as u32, 8.0, 16.0).unwrap();
        assert_eq!(r.len(), 2);
        // The down arm ends at the bottom edge, the right arm at the right.
        assert!(r.iter().any(|([_, y, _, h], _)| y + h == 16.0));
        assert!(r.iter().any(|([x, _, w, _], _)| x + w == 8.0));
    }

    #[test]
    fn blocks_and_shades() {
        let r = glyph('█' as u32, 8.0, 16.0).unwrap();
        assert_eq!(r, vec![([0.0, 0.0, 8.0, 16.0], 1.0)]);
        let r = glyph('▄' as u32, 8.0, 16.0).unwrap();
        assert_eq!(r, vec![([0.0, 8.0, 8.0, 8.0], 1.0)]);
        let r = glyph('▒' as u32, 8.0, 16.0).unwrap();
        assert_eq!(r[0].1, 0.5);
        assert!(glyph('a' as u32, 8.0, 16.0).is_none());
        // Box-range code points with no rule are left to the font.
        assert!(glyph('╳' as u32, 8.0, 16.0).is_none());
    }
}
