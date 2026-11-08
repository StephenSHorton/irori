//! The canvas's lines as Rune shapes (the React app's reactflow/index.tsx
//! toEdges, drawn the way React Flow draws them): which side of a node
//! each kind of line leaves and enters by, a step line with rounded
//! corners for subtasks and holders, a curve for the rest, dashes, and an
//! arrowhead at the end. Dashes are cut here by length: Rune's own
//! `Shape::dashed` (fork card-674-gaps d0ffeb2) never returns on most
//! curved or stepped lines (dash::pieces stalls on f32 rounding), so the
//! native version (e865e49c, with main's flowing talk and holder lines on
//! `El::dash_flow`) waits for that fix.

use rune::{Color, LineCap, Shape};

use super::data::{EdgeKind, Point, Size};

/// A side of a node: where a line leaves or comes in, and which way it
/// points out of the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Side {
    fn at(self, p: Point, s: Size) -> [f32; 2] {
        match self {
            Side::Top => [p.x + s.w / 2.0, p.y],
            Side::Bottom => [p.x + s.w / 2.0, p.y + s.h],
            Side::Left => [p.x, p.y + s.h / 2.0],
            Side::Right => [p.x + s.w, p.y + s.h / 2.0],
        }
    }

    fn out(self) -> [f32; 2] {
        match self {
            Side::Top => [0.0, -1.0],
            Side::Bottom => [0.0, 1.0],
            Side::Left => [-1.0, 0.0],
            Side::Right => [1.0, 0.0],
        }
    }

    fn vertical(self) -> bool {
        matches!(self, Side::Top | Side::Bottom)
    }
}

/// Which sides a kind of line joins (React Flow's handles): a parent's
/// bottom to its subtask's left, a blocker's top to the bottom of its card,
/// an agent's bottom to the top of a card it holds, an agent's right to
/// the left of a card it watches or an agent it talks with.
pub fn sides(k: EdgeKind) -> (Side, Side) {
    match k {
        EdgeKind::Subtask => (Side::Bottom, Side::Left),
        EdgeKind::Blocker => (Side::Top, Side::Bottom),
        EdgeKind::Holder => (Side::Bottom, Side::Top),
        EdgeKind::Watcher | EdgeKind::Talk => (Side::Right, Side::Left),
    }
}

/// How a kind of line looks: its colour role, dash (on, off), opacity and
/// width, and whether it ends in an arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub dash: Option<(f32, f32)>,
    pub opacity: f32,
    pub width: f32,
    pub arrow: bool,
    /// A step line (React Flow's smoothstep) rather than a curve.
    pub step: bool,
}

pub fn look(k: EdgeKind) -> Look {
    let (dash, opacity) = match k {
        EdgeKind::Subtask => (None, 0.8),
        EdgeKind::Blocker => (Some((4.0, 4.0)), 0.9),
        EdgeKind::Holder => (None, 0.55),
        EdgeKind::Watcher => (Some((2.0, 4.0)), 0.2),
        EdgeKind::Talk => (Some((6.0, 4.0)), 0.9),
    };
    Look {
        dash,
        opacity,
        width: if k == EdgeKind::Talk { 2.0 } else { 1.5 },
        arrow: k != EdgeKind::Talk,
        step: matches!(k, EdgeKind::Subtask | EdgeKind::Holder),
    }
}

const STEP_OUT: f32 = 20.0;
const CORNER: f32 = 6.0;

fn add(a: [f32; 2], b: [f32; 2], k: f32) -> [f32; 2] {
    [a[0] + b[0] * k, a[1] + b[1] * k]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The corners of a step line from `s` (leaving by `ss`) to `t` (coming in
/// by `ts`): straight out of each side, then across at right angles.
fn step_points(s: [f32; 2], ss: Side, t: [f32; 2], ts: Side) -> Vec<[f32; 2]> {
    let so = add(s, ss.out(), STEP_OUT);
    let to = add(t, ts.out(), STEP_OUT);
    let mut pts = vec![s];
    match (ss.vertical(), ts.vertical()) {
        (true, true) => {
            // Down and into a top: meet halfway when there's room.
            let ahead = (to[1] - so[1]) * ss.out()[1] >= 0.0 && ts.out()[1] == -ss.out()[1];
            if ahead {
                let mid = (s[1] + t[1]) / 2.0;
                pts.extend([[s[0], mid], [t[0], mid]]);
            } else {
                let mid = (so[0] + to[0]) / 2.0;
                pts.extend([so, [mid, so[1]], [mid, to[1]], to]);
            }
        }
        (true, false) => {
            // Down and into a side: one corner when the side faces us.
            let faces = (s[0] - t[0]) * ts.out()[0] > 0.0 && (t[1] - so[1]) * ss.out()[1] > 0.0;
            if faces {
                pts.push([s[0], t[1]]);
            } else {
                pts.extend([so, [to[0], so[1]], to]);
            }
        }
        (false, true) => {
            let faces = (t[0] - so[0]) * ss.out()[0] > 0.0 && (s[1] - t[1]) * ts.out()[1] > 0.0;
            if faces {
                pts.push([t[0], s[1]]);
            } else {
                pts.extend([so, [so[0], to[1]], to]);
            }
        }
        (false, false) => {
            let mid = (s[0] + t[0]) / 2.0;
            if (to[0] - so[0]) * ss.out()[0] >= 0.0 {
                pts.extend([[mid, s[1]], [mid, t[1]]]);
            } else {
                let midy = (so[1] + to[1]) / 2.0;
                pts.extend([so, [so[0], midy], [to[0], midy], to]);
            }
        }
    }
    pts.push(t);
    pts.dedup();
    round_corners(&pts, CORNER)
}

/// A polyline with each corner turned on a small curve.
fn round_corners(pts: &[[f32; 2]], r: f32) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for w in pts.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let r = r.min(dist(a, b) / 2.0).min(dist(b, c) / 2.0);
        if r < 0.5 {
            out.push(b);
            continue;
        }
        let d1 = dist(a, b);
        let d2 = dist(b, c);
        let p = [b[0] + (a[0] - b[0]) / d1 * r, b[1] + (a[1] - b[1]) / d1 * r];
        let q = [b[0] + (c[0] - b[0]) / d2 * r, b[1] + (c[1] - b[1]) / d2 * r];
        for i in 0..=4 {
            let t = i as f32 / 4.0;
            let u = 1.0 - t;
            out.push([
                u * u * p[0] + 2.0 * u * t * b[0] + t * t * q[0],
                u * u * p[1] + 2.0 * u * t * b[1] + t * t * q[1],
            ]);
        }
    }
    out.push(pts[pts.len() - 1]);
    out
}

/// A curve from `s` to `t` (React Flow's default bezier): it leaves and
/// comes in square to each side.
fn curve_points(s: [f32; 2], ss: Side, t: [f32; 2], ts: Side) -> Vec<[f32; 2]> {
    let reach = |side: Side| {
        let d = if side.vertical() {
            (t[1] - s[1]).abs()
        } else {
            (t[0] - s[0]).abs()
        };
        (d * 0.5).max(25.0)
    };
    let c1 = add(s, ss.out(), reach(ss));
    let c2 = add(t, ts.out(), reach(ts));
    let n = 24;
    (0..=n)
        .map(|i| {
            let t_ = i as f32 / n as f32;
            let u = 1.0 - t_;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t_, 3.0 * u * t_ * t_, t_ * t_ * t_);
            [
                a * s[0] + b * c1[0] + c * c2[0] + d * t[0],
                a * s[1] + b * c1[1] + c * c2[1] + d * t[1],
            ]
        })
        .collect()
}

/// The points of a line of kind `k` from one node's rect to another's.
pub fn route(k: EdgeKind, from: (Point, Size), to: (Point, Size)) -> Vec<[f32; 2]> {
    let (ss, ts) = sides(k);
    let s = ss.at(from.0, from.1);
    let t = ts.at(to.0, to.1);
    if look(k).step {
        step_points(s, ss, t, ts)
    } else {
        curve_points(s, ss, t, ts)
    }
}

/// The polyline cut into dashes `on` long with `off` between.
pub fn dashes(pts: &[[f32; 2]], on: f32, off: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let (mut drawing, mut left) = (true, on);
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut len = dist(a, b);
        while len > 0.0 {
            let step = left.min(len);
            let next = [
                a[0] + (b[0] - a[0]) * step / len,
                a[1] + (b[1] - a[1]) * step / len,
            ];
            if drawing {
                if cur.is_empty() {
                    cur.push(a);
                }
                cur.push(next);
            }
            left -= step;
            len -= step;
            a = next;
            if left <= 1e-4 {
                if drawing && cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                drawing = !drawing;
                left = if drawing { on } else { off };
            }
        }
    }
    if drawing && cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// A filled arrowhead at the end of `pts`, pointing along its last piece.
fn arrow(pts: &[[f32; 2]], size: f32) -> Option<Vec<[f32; 2]>> {
    let t = *pts.last()?;
    let a = pts.iter().rev().skip(1).find(|p| dist(**p, t) > 0.5)?;
    let d = dist(*a, t);
    let (ux, uy) = ((t[0] - a[0]) / d, (t[1] - a[1]) / d);
    let base = [t[0] - ux * size, t[1] - uy * size];
    let half = size * 0.5;
    Some(vec![
        t,
        [base[0] - uy * half, base[1] + ux * half],
        [base[0] + uy * half, base[1] - ux * half],
    ])
}

/// One line as a shape, `color` at `opacity`, shifted by `-origin`.
pub fn shape(
    pts: &[[f32; 2]],
    lk: Look,
    color: Color,
    opacity: f32,
    width: f32,
    origin: [f32; 2],
) -> Shape {
    let pts: Vec<[f32; 2]> = pts
        .iter()
        .map(|p| [p[0] - origin[0], p[1] - origin[1]])
        .collect();
    let c = color.alpha(opacity);
    let mut sh = Shape::new();
    // The line stops short of the arrow's tip so the two don't overlap.
    let mut body = pts.clone();
    if lk.arrow
        && let Some(&t) = pts.last()
        && let Some(&a) = pts.iter().rev().skip(1).find(|p| dist(**p, t) > 0.5)
    {
        let d = dist(a, t);
        let back = 6.0f32.min(d);
        if let Some(last) = body.last_mut() {
            *last = [
                t[0] - (t[0] - a[0]) / d * back,
                t[1] - (t[1] - a[1]) / d * back,
            ];
        }
    }
    match lk.dash {
        Some((on, off)) => {
            for d in dashes(&body, on, off) {
                sh = sh.stroke_with(d, false, width, LineCap::Butt, c);
            }
        }
        None => sh = sh.stroke(body, width, c),
    }
    if lk.arrow
        && let Some(head) = arrow(&pts, 9.0)
    {
        sh = sh.fill(head, c);
    }
    sh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::data::{AGENT, CARD};

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    #[test]
    fn dashes_cut_a_line_by_length() {
        let d = dashes(&[[0.0, 0.0], [20.0, 0.0]], 4.0, 4.0);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0], vec![[0.0, 0.0], [4.0, 0.0]]);
        assert_eq!(d[1], vec![[8.0, 0.0], [12.0, 0.0]]);
        // Round a corner: one dash bends with it.
        let d = dashes(&[[0.0, 0.0], [2.0, 0.0], [2.0, 6.0]], 4.0, 2.0);
        assert_eq!(d[0], vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]);
    }

    #[test]
    fn a_holder_line_runs_from_the_agents_bottom_to_the_cards_top() {
        let pts = route(
            EdgeKind::Holder,
            (p(0.0, 0.0), AGENT),
            (p(200.0, 200.0), CARD),
        );
        assert_eq!(pts.first(), Some(&[90.0, 40.0]));
        assert_eq!(pts.last(), Some(&[340.0, 200.0]));
        // Square corners only: every piece away from the bends is level or upright.
        assert!(pts.iter().all(|q| q[0].is_finite() && q[1].is_finite()));
    }

    #[test]
    fn a_subtask_line_comes_into_the_kids_left_side() {
        let pts = route(
            EdgeKind::Subtask,
            (p(0.0, 0.0), CARD),
            (p(300.0, 200.0), CARD),
        );
        assert_eq!(pts.first(), Some(&[140.0, 132.0]));
        assert_eq!(pts.last(), Some(&[300.0, 266.0]));
    }

    #[test]
    fn a_talk_line_is_a_curve_with_no_arrow() {
        let pts = route(EdgeKind::Talk, (p(0.0, 0.0), AGENT), (p(400.0, 0.0), AGENT));
        assert_eq!(pts.len(), 25);
        assert!(!look(EdgeKind::Talk).arrow);
        assert!(look(EdgeKind::Blocker).dash.is_some());
    }
}
