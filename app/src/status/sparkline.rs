//! A small area-and-line graph of the newest readings (card #599): the
//! React app's components/meters/sparkline.tsx as a Rune [`Shape`].
//! Readings fill from the right, as Task Manager's do, across `slots`
//! places; a missing reading draws as 0.

use rune::{Color, El, Shape, canvas};

/// The top of the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Max {
    /// A fixed top (100 for percents).
    Fixed(f64),
    /// A rate, scaled to the largest reading shown.
    Auto,
}

const W: f32 = 100.0;
const H: f32 = 100.0;

/// The line's points in a 100 × 100 box, and where the first one sits
/// (for the area's left edge).
pub fn points(values: &[Option<f64>], slots: usize, max: Max) -> (Vec<[f32; 2]>, f32) {
    let n = slots.max(2);
    let shown = &values[values.len().saturating_sub(n)..];
    let top = match max {
        Max::Fixed(m) => m,
        Max::Auto => shown.iter().map(|v| v.unwrap_or(0.0)).fold(1e-9, f64::max) * 1.1,
    };
    let step = W / (n - 1) as f32;
    let start = (n - shown.len()) as f32;
    let pts = shown
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let v = v.unwrap_or(0.0).clamp(0.0, top);
            let y = H - (v / top) as f32 * H;
            [(start + i as f32) * step, y]
        })
        .collect();
    (pts, start * step)
}

/// The graph, `width` × `height` points, in `color`.
pub fn sparkline(
    values: &[Option<f64>],
    slots: usize,
    max: Max,
    width: f32,
    height: f32,
    color: Color,
) -> El {
    let (pts, left) = points(values, slots, max);
    let mut shape = Shape::new().view_box(W, H);
    if pts.len() > 1 {
        let mut area = pts.clone();
        area.push([W, H]);
        area.push([left, H]);
        shape = shape.fill(area, color.alpha(0.16)).stroke(pts, 1.25, color);
    }
    canvas(shape).size(width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_fill_from_the_right() {
        let (pts, left) = points(&[Some(50.0), Some(100.0)], 5, Max::Fixed(100.0));
        assert_eq!(pts, vec![[75.0, 50.0], [100.0, 0.0]]);
        assert_eq!(left, 75.0);
    }

    #[test]
    fn a_missing_reading_is_zero_and_auto_scales_to_the_top() {
        let (pts, _) = points(&[None, Some(10.0)], 2, Max::Auto);
        assert_eq!(pts[0], [0.0, 100.0]);
        // 10 of a top of 11.
        assert!((pts[1][1] - (100.0 - 100.0 / 1.1)).abs() < 0.01);
    }

    #[test]
    fn only_the_newest_slots_show() {
        let v: Vec<Option<f64>> = (0..10).map(|i| Some(i as f64)).collect();
        let (pts, left) = points(&v, 4, Max::Fixed(100.0));
        assert_eq!(pts.len(), 4);
        assert_eq!(left, 0.0);
        assert_eq!(pts[3][0], 100.0);
    }
}
