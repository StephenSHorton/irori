// Where the canvas's lines run (card #776). ELK routes the lines inside an
// area as it lays the board out (elk.ts); everything else comes here: a
// line between areas (a second holder, a subtask in another hub, agents
// talking), and any ELK line a dragged card has broken. A line here is
// orthogonal, leaves and enters by whichever sides suit where the two
// nodes actually are, and goes around every other node rather than
// through it (docs/canvas-layout.md, Lines).
//
// The router is a shortest path on a sparse grid: the lines just outside
// each nearby node (Margin out) and the two ends' ports, with a cost per
// bend so a line takes few turns. Pure and deterministic.

import type { Point, Rect } from "./data"

// Margin is how far a line keeps from a node it passes: half the gap
// between two cards, so a line between them runs down the middle.
export const Margin = 12
// A bend costs as much as this much more length.
const Bend = 48
// Nodes this far beyond the two ends' box are left out of the search;
// a line that can't be found inside it is searched for again on all.
const Reach = 360

type Side = 0 | 1 | 2 | 3 // top, right, bottom, left
const Sides: Side[] = [0, 1, 2, 3]
// Unit step out of each side; a move's direction uses the same numbering.
const Out: Point[] = [
  { x: 0, y: -1 },
  { x: 1, y: 0 },
  { x: 0, y: 1 },
  { x: -1, y: 0 },
]

function port(r: Rect, s: Side): Point {
  switch (s) {
    case 0:
      return { x: r.x + r.w / 2, y: r.y }
    case 1:
      return { x: r.x + r.w, y: r.y + r.h / 2 }
    case 2:
      return { x: r.x + r.w / 2, y: r.y + r.h }
    default:
      return { x: r.x, y: r.y + r.h / 2 }
  }
}

function grow(r: Rect, m: number): Rect {
  return { x: r.x - m, y: r.y - m, w: r.w + 2 * m, h: r.h + 2 * m }
}

function meets(a: Rect, b: Rect): boolean {
  return (
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
  )
}

// crosses says the segment a-b passes through r's inside; running along
// its edge or touching a corner doesn't count.
export function crosses(a: Point, b: Point, r: Rect): boolean {
  const x0 = Math.min(a.x, b.x)
  const x1 = Math.max(a.x, b.x)
  const y0 = Math.min(a.y, b.y)
  const y1 = Math.max(a.y, b.y)
  if (x0 === x1) return x0 > r.x && x0 < r.x + r.w && y0 < r.y + r.h && y1 > r.y
  if (y0 === y1) return y0 > r.y && y0 < r.y + r.h && x0 < r.x + r.w && x1 > r.x
  // Not orthogonal: test against the rect as a box, conservatively.
  return x0 < r.x + r.w && x1 > r.x && y0 < r.y + r.h && y1 > r.y
}

function inside(p: Point, r: Rect): boolean {
  return p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h
}

// onEdge says p lies on r's border, give or take a pixel.
function onEdge(p: Point, r: Rect): boolean {
  const e = 1.5
  const inX = p.x >= r.x - e && p.x <= r.x + r.w + e
  const inY = p.y >= r.y - e && p.y <= r.y + r.h + e
  return (
    (inX && (Math.abs(p.y - r.y) <= e || Math.abs(p.y - r.y - r.h) <= e)) ||
    (inY && (Math.abs(p.x - r.x) <= e || Math.abs(p.x - r.x - r.w) <= e))
  )
}

// fits says a line still works where the nodes are now: it starts on the
// source's border, ends on the target's, and passes through no node.
export function fits(
  pts: readonly Point[],
  src: Rect,
  dst: Rect,
  nodes: readonly Rect[]
): boolean {
  if (pts.length < 2) return false
  if (!onEdge(pts[0], src) || !onEdge(pts[pts.length - 1], dst)) return false
  for (let i = 1; i < pts.length; i++)
    for (const r of nodes) if (crosses(pts[i - 1], pts[i], r)) return false
  return true
}

// simplify drops repeated points and the middle of straight runs.
export function simplify(pts: readonly Point[]): Point[] {
  const out: Point[] = []
  for (const p of pts) {
    const last = out[out.length - 1]
    if (last && last.x === p.x && last.y === p.y) continue
    if (out.length >= 2) {
      const a = out[out.length - 2]
      const b = last
      if ((a.x === b.x && b.x === p.x) || (a.y === b.y && b.y === p.y)) {
        // Straight on (or straight back, which simplify also folds).
        out[out.length - 1] = p
        continue
      }
    }
    out.push(p)
  }
  return out
}

// A small binary heap on [cost, state].
class Heap {
  private a: [number, number][] = []
  get size() {
    return this.a.length
  }
  push(c: number, s: number) {
    const a = this.a
    a.push([c, s])
    let i = a.length - 1
    while (i > 0) {
      const p = (i - 1) >> 1
      if (!less([c, s], a[p])) break
      a[i] = a[p]
      i = p
    }
    a[i] = [c, s]
  }
  pop(): [number, number] {
    const a = this.a
    const top = a[0]
    const last = a.pop()!
    if (a.length) {
      a[0] = last
      let i = 0
      for (;;) {
        const l = 2 * i + 1
        const r = l + 1
        let m = i
        if (l < a.length && less(a[l], a[m])) m = l
        if (r < a.length && less(a[r], a[m])) m = r
        if (m === i) break
        ;[a[i], a[m]] = [a[m], a[i]]
        i = m
      }
    }
    return top
  }
}

function less(x: [number, number], y: [number, number]): boolean {
  return x[0] < y[0] || (x[0] === y[0] && x[1] < y[1])
}

function search(
  src: Rect,
  dst: Rect,
  others: readonly Rect[],
  area: Rect | null
): Point[] | null {
  const M = Margin
  const near = area ? others.filter((o) => meets(o, area)) : others
  const blocks = [...near, src, dst].map((r) => grow(r, M))
  const ends = (r: Rect) =>
    Sides.map((s) => {
      const p = port(r, s)
      return { s, p, q: { x: p.x + Out[s].x * M, y: p.y + Out[s].y * M } }
    }).filter((e) => !blocks.some((b) => inside(e.q, b)))
  const starts = ends(src)
  const goals = ends(dst)
  if (!starts.length || !goals.length) return null

  const xsSet = new Set<number>()
  const ysSet = new Set<number>()
  for (const b of blocks) {
    xsSet.add(b.x).add(b.x + b.w)
    ysSet.add(b.y).add(b.y + b.h)
  }
  for (const e of [...starts, ...goals]) {
    xsSet.add(e.q.x)
    ysSet.add(e.q.y)
  }
  // A line between the two, if there's room for one.
  const cs = { x: src.x + src.w / 2, y: src.y + src.h / 2 }
  const cd = { x: dst.x + dst.w / 2, y: dst.y + dst.h / 2 }
  xsSet.add((cs.x + cd.x) / 2)
  ysSet.add((cs.y + cd.y) / 2)
  if (area) {
    xsSet.add(area.x).add(area.x + area.w)
    ysSet.add(area.y).add(area.y + area.h)
  }
  const xs = [...xsSet].sort((a, b) => a - b)
  const ys = [...ysSet].sort((a, b) => a - b)
  const X = xs.length
  const Y = ys.length
  const at = (i: number, j: number) => ({ x: xs[i], y: ys[j] })
  const free = new Uint8Array(X * Y)
  for (let i = 0; i < X; i++)
    for (let j = 0; j < Y; j++) {
      const p = at(i, j)
      free[j * X + i] = blocks.some((b) => inside(p, b)) ? 0 : 1
    }
  // open[k][d]: the step from node k in direction d is clear.
  const stepOk = (i: number, j: number, d: number): number => {
    const ni = i + (d === 1 ? 1 : d === 3 ? -1 : 0)
    const nj = j + (d === 2 ? 1 : d === 0 ? -1 : 0)
    if (ni < 0 || nj < 0 || ni >= X || nj >= Y) return -1
    if (!free[nj * X + ni]) return -1
    const a = at(i, j)
    const b = at(ni, nj)
    if (blocks.some((r) => crosses(a, b, r))) return -1
    return nj * X + ni
  }
  const idx = (p: Point) => ys.indexOf(p.y) * X + xs.indexOf(p.x)
  const goalAt = new Map<number, Side[]>()
  for (const g of goals) {
    const k = idx(g.q)
    goalAt.set(k, [...(goalAt.get(k) ?? []), g.s])
  }

  // States are node*4+dir (the direction the line is moving); the goal is
  // one more state past them.
  const N = X * Y * 4
  const GOAL = N
  const cost = new Float64Array(N + 1).fill(Infinity)
  const prev = new Int32Array(N + 1).fill(-1)
  const goalSide = new Int8Array(1).fill(-1)
  const heap = new Heap()
  for (const s of starts) {
    const st = idx(s.q) * 4 + s.s
    if (M < cost[st]) {
      cost[st] = M
      prev[st] = -2 - s.s // a start: which side it left by
      heap.push(M, st)
    }
  }
  while (heap.size) {
    const [c, st] = heap.pop()
    if (c > cost[st]) continue
    if (st === GOAL) break
    const k = st >> 2
    const dir = st & 3
    const sides = goalAt.get(k)
    if (sides)
      for (const s of sides) {
        // Into the target: moving against its side's outward step.
        const into = (s + 2) % 4
        const total = c + M + (dir === into ? 0 : Bend)
        if (total < cost[GOAL]) {
          cost[GOAL] = total
          prev[GOAL] = st
          goalSide[0] = s
          heap.push(total, GOAL)
        }
      }
    const i = k % X
    const j = (k - i) / X
    for (let d = 0; d < 4; d++) {
      if (d === (dir + 2) % 4) continue // no turning back
      const nk = stepOk(i, j, d)
      if (nk < 0) continue
      const a = at(i, j)
      const b = at(nk % X, (nk - (nk % X)) / X)
      const len = Math.abs(a.x - b.x) + Math.abs(a.y - b.y)
      const nc = c + len + (d === dir ? 0 : Bend)
      const ns = nk * 4 + d
      if (nc < cost[ns]) {
        cost[ns] = nc
        prev[ns] = st
        heap.push(nc, ns)
      }
    }
  }
  if (prev[GOAL] < 0) return null
  const path: Point[] = [port(dst, goalSide[0] as Side)]
  let st = prev[GOAL]
  let startSide: Side = 0
  while (st >= 0) {
    const k = st >> 2
    const i = k % X
    path.push(at(i, (k - i) / X))
    const p = prev[st]
    if (p <= -2) startSide = (-2 - p) as Side
    st = p
  }
  path.push(port(src, startSide))
  return simplify(path.reverse())
}

// route is a line from src to dst that goes around others: the shortest
// with few bends, by the sides that suit. If no such line exists (the
// nodes overlap), it is the straight line between their nearest sides.
export function route(src: Rect, dst: Rect, others: readonly Rect[]): Point[] {
  const x0 = Math.min(src.x, dst.x) - Reach
  const y0 = Math.min(src.y, dst.y) - Reach
  const area = {
    x: x0,
    y: y0,
    w: Math.max(src.x + src.w, dst.x + dst.w) + Reach - x0,
    h: Math.max(src.y + src.h, dst.y + dst.h) + Reach - y0,
  }
  return (
    search(src, dst, others, area) ??
    search(src, dst, others, null) ??
    straight(src, dst)
  )
}

// straight joins the two sides that face each other most.
export function straight(src: Rect, dst: Rect): Point[] {
  const cs = { x: src.x + src.w / 2, y: src.y + src.h / 2 }
  const cd = { x: dst.x + dst.w / 2, y: dst.y + dst.h / 2 }
  const dx = cd.x - cs.x
  const dy = cd.y - cs.y
  const [a, b]: [Side, Side] =
    Math.abs(dx) * src.h > Math.abs(dy) * src.w
      ? dx > 0
        ? [1, 3]
        : [3, 1]
      : dy > 0
        ? [2, 0]
        : [0, 2]
  return [port(src, a), port(dst, b)]
}

// blockerLine is a blocker's line to its card: out of the blocker's left
// side and up the strip its inset leaves clear under the card, so it
// never runs through the blockers stacked above it.
export function blockerLine(blocker: Rect, card: Rect): Point[] | null {
  const x = card.x + Math.min(10, (blocker.x - card.x) / 2)
  if (blocker.x - card.x < 8 || blocker.y < card.y + card.h) return null
  const y = blocker.y + blocker.h / 2
  return [
    { x: blocker.x, y },
    { x, y },
    { x, y: card.y + card.h },
  ]
}

export type Line = { id: string; kind: string; source: string; target: string }

// routeAll is every line's points where the nodes are now: a blocker's
// own line, ELK's line while it still fits, else one routed here.
export function routeAll(
  lines: readonly Line[],
  rects: Readonly<Record<string, Rect>>,
  elk: Readonly<Record<string, Point[]>>
): Record<string, Point[]> {
  const all = Object.entries(rects)
  const out: Record<string, Point[]> = {}
  for (const e of lines) {
    const s = rects[e.source]
    const t = rects[e.target]
    if (!s || !t) continue
    const others = all
      .filter(([id]) => id !== e.source && id !== e.target)
      .map(([, r]) => r)
    if (e.kind === "blocker") {
      const b = blockerLine(s, t)
      if (b) {
        out[e.id] = b
        continue
      }
    }
    const given = elk[e.id]
    out[e.id] =
      given && fits(given, s, t, [...others, s, t])
        ? given
        : route(s, t, others)
  }
  return out
}

// roundedPath is an SVG path through the points with its corners rounded
// (like React Flow's smoothstep), and the middle of the line for a label.
export function roundedPath(
  pts: readonly Point[],
  radius = 8
): { d: string; mid: Point } {
  if (pts.length < 2) return { d: "", mid: pts[0] ?? { x: 0, y: 0 } }
  let d = `M ${pts[0].x} ${pts[0].y}`
  for (let i = 1; i < pts.length - 1; i++) {
    const a = pts[i - 1]
    const b = pts[i]
    const c = pts[i + 1]
    const r = Math.min(
      radius,
      Math.hypot(b.x - a.x, b.y - a.y) / 2,
      Math.hypot(c.x - b.x, c.y - b.y) / 2
    )
    const t1 = towards(b, a, r)
    const t2 = towards(b, c, r)
    d += ` L ${t1.x} ${t1.y} Q ${b.x} ${b.y} ${t2.x} ${t2.y}`
  }
  const last = pts[pts.length - 1]
  d += ` L ${last.x} ${last.y}`
  return { d, mid: middle(pts) }
}

function towards(from: Point, to: Point, r: number): Point {
  const len = Math.hypot(to.x - from.x, to.y - from.y) || 1
  return {
    x: from.x + ((to.x - from.x) / len) * r,
    y: from.y + ((to.y - from.y) / len) * r,
  }
}

// middle is the point halfway along the line.
function middle(pts: readonly Point[]): Point {
  let total = 0
  for (let i = 1; i < pts.length; i++)
    total += Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y)
  let left = total / 2
  for (let i = 1; i < pts.length; i++) {
    const seg = Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y)
    if (seg >= left) return towards(pts[i - 1], pts[i], left)
    left -= seg
  }
  return pts[pts.length - 1]
}
