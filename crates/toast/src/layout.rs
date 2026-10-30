//! Card geometry in dip (go-toast layout.go). A card is the card color, a
//! 1px border, a kind-colored stripe and icon on the left.

use crate::note::{CORNER_BOTTOM_LEFT, CORNER_TOP_LEFT, CORNER_TOP_RIGHT};

pub const CARD_WIDTH_DIP: f64 = 340.0;
pub const CARD_GAP_DIP: f64 = 8.0;
/// From the work area's edges.
pub const CARD_MARGIN_DIP: f64 = 20.0;

pub const STRIPE_DIP: f64 = 3.0;
pub const ICON_LEFT_DIP: f64 = 15.0;
pub const ICON_SIZE_DIP: f64 = 15.0;
pub const TEXT_LEFT_DIP: f64 = 40.0;
pub const TEXT_RIGHT_DIP: f64 = 14.0;
pub const PAD_TOP_DIP: f64 = 11.0;
pub const PAD_BOTTOM_DIP: f64 = 12.0;
/// The title row (Segoe UI Semibold 14).
pub const TITLE_DIP: f64 = 21.0;
pub const TITLE_SIZE_DIP: f64 = 14.0;
/// Between title and body.
pub const BODY_GAP_DIP: f64 = 1.0;
/// A body line (Segoe UI 12.5).
pub const BODY_LINE_DIP: f64 = 17.0;
pub const BODY_SIZE_DIP: f64 = 12.5;
pub const MAX_BODY_LINES: usize = 2;

/// The x's hit box, in the top-right corner.
pub const CLOSE_SIZE_DIP: f64 = 24.0;
pub const CLOSE_INSET_DIP: f64 = 5.0;
/// The title keeps clear of the x.
pub const TITLE_RIGHT_DIP: f64 = CLOSE_SIZE_DIP + CLOSE_INSET_DIP + 4.0;

/// Above the button row.
pub const BUTTON_GAP_DIP: f64 = 10.0;
pub const BUTTON_DIP: f64 = 26.0;
/// Text inset on each side.
pub const BUTTON_PAD_DIP: f64 = 12.0;
pub const BUTTON_MIN_DIP: f64 = 64.0;
/// Between buttons.
pub const BUTTON_SPACE_DIP: f64 = 8.0;
pub const BUTTON_SIZE_DIP: f64 = 12.5;
pub const BUTTONS_RIGHT_DIP: f64 = TEXT_RIGHT_DIP;

/// The height of a card with `body_lines` lines of body and, with
/// `actions`, a button row. Title only is 44 dip; one body line makes it
/// 62, Toru's original size.
pub fn card_height_dip(body_lines: usize, actions: bool) -> f64 {
    let mut h = PAD_TOP_DIP + TITLE_DIP + PAD_BOTTOM_DIP;
    let lines = body_lines.min(MAX_BODY_LINES);
    if lines > 0 {
        h += BODY_GAP_DIP + lines as f64 * BODY_LINE_DIP;
    }
    if actions {
        h += BUTTON_GAP_DIP + BUTTON_DIP;
    }
    h
}

/// A pixel rectangle, right and bottom exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn has(&self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
}

/// A pixel position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Dip to pixels at scale.
pub fn px(dip: f64, scale: f64) -> i32 {
    (dip * scale + 0.5) as i32
}

/// Places cards of width w and the given heights (oldest first) in the
/// corner of the work area, newest nearest the corner; each card's top-left
/// in the same order.
pub fn stack_origins(
    work: Rect,
    corner: &str,
    w: i32,
    heights: &[i32],
    gap: i32,
    margin: i32,
) -> Vec<Point> {
    let mut out = vec![Point::default(); heights.len()];
    let x = if corner == CORNER_TOP_LEFT || corner == CORNER_BOTTOM_LEFT {
        work.l + margin
    } else {
        work.r - margin - w
    };
    let top = corner == CORNER_TOP_RIGHT || corner == CORNER_TOP_LEFT;
    let mut y = if top {
        work.t + margin
    } else {
        work.b - margin
    };
    for i in (0..heights.len()).rev() {
        let h = heights[i];
        if top {
            out[i] = Point { x, y };
            y += h + gap;
        } else {
            y -= h;
            out[i] = Point { x, y };
            y -= gap;
        }
    }
    out
}

/// What a click lands on: a button index (0, 1), the body or the x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    None,
    Close,
    Body,
    Button(usize),
}

impl Part {
    /// go-toast's int: a button index, -1 body, -2 close, -3 none.
    pub fn code(self) -> i32 {
        match self {
            Part::None => -3,
            Part::Close => -2,
            Part::Body => -1,
            Part::Button(i) => i as i32,
        }
    }
}

/// The x's hit box on a card w px wide.
pub fn close_rect(w: i32, scale: f64) -> Rect {
    let s = px(CLOSE_SIZE_DIP, scale);
    let inset = px(CLOSE_INSET_DIP, scale);
    Rect {
        l: w - inset - s,
        t: inset,
        r: w - inset,
        b: inset + s,
    }
}

/// The button row along the bottom of a card w x h px, given each label's
/// measured text width in px. Buttons read left to right in action order
/// (the primary first) and the row ends at the right padding.
pub fn button_rects(w: i32, h: i32, text_w: &[i32], scale: f64) -> Vec<Rect> {
    if text_w.is_empty() {
        return vec![];
    }
    let min = px(BUTTON_MIN_DIP, scale);
    let widths: Vec<i32> = text_w
        .iter()
        .map(|tw| (tw + 2 * px(BUTTON_PAD_DIP, scale)).max(min))
        .collect();
    let space = px(BUTTON_SPACE_DIP, scale);
    let total: i32 = widths.iter().sum::<i32>() + (widths.len() as i32 - 1) * space;
    let bottom = h - px(PAD_BOTTOM_DIP, scale);
    let top = bottom - px(BUTTON_DIP, scale);
    let mut x = (w - px(BUTTONS_RIGHT_DIP, scale) - total).max(px(TEXT_LEFT_DIP, scale));
    widths
        .iter()
        .map(|bw| {
            let r = Rect {
                l: x,
                t: top,
                r: x + bw,
                b: bottom,
            };
            x += bw + space;
            r
        })
        .collect()
}

/// What (x, y) on a card lands on.
pub fn hit_part(x: i32, y: i32, close: Rect, buttons: &[Rect]) -> Part {
    if close.has(x, y) {
        return Part::Close;
    }
    for (i, b) in buttons.iter().enumerate() {
        if b.has(x, y) {
            return Part::Button(i);
        }
    }
    Part::Body
}
