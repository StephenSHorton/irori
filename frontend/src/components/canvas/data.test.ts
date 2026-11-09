import { describe, expect, test } from "bun:test"

import { AgentRef, CardHolder, CardSummary, TaskState } from "@/lib/models"

import {
  NodeSize,
  drawn,
  groupRects,
  handOffs,
  toGraph,
  withPlaced,
  type CardNode,
  type Graph,
  type Layout,
  type Point,
  type Rect,
} from "./data"
import { areaOrder, layoutGraph, layoutKey } from "./elk"
import { testElk } from "./elk-test-helper"
import { markersOf } from "./node"
import { fits, route, routeAll } from "./route"
import { Weight } from "./weights"

const Now = Date.parse("2026-11-05T12:00:00Z")
const graphOf = (cards: CardSummary[]) => toGraph(cards, undefined, Now)
const autoLayout = async (g: Graph): Promise<Layout> =>
  layoutGraph(g, await testElk())

const busy = (key: string, name = key) =>
  AgentRef({ key, name, status: "busy", live: true })
const idle = (key: string) =>
  AgentRef({ key, name: key, status: "idle", live: true })
const gone = (key: string) =>
  AgentRef({ key, name: key, status: "gone", live: false })

let n = 0
function card(p: Partial<CardSummary> = {}): CardSummary {
  n++
  return CardSummary({
    id: `c${n}`,
    short: `${n}`,
    title: `card ${n}`,
    state: TaskState.Working,
    repoKey: "remote:github.com/org/app",
    updatedAt: "2026-11-05T00:00:00Z",
    progress: { done: 0, total: 0 },
    working: [],
    watching: [],
    talking: [],
    holder: CardHolder({ state: "active" }),
    ...p,
  })
}

const inside = (
  r: Rect,
  p: { x: number; y: number },
  s: { w: number; h: number }
) =>
  p.x >= r.x && p.y >= r.y && p.x + s.w <= r.x + r.w && p.y + s.h <= r.y + r.h

function overlap(a: Rect, b: Rect): boolean {
  return (
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
  )
}

describe("canvas layout (card #587)", () => {
  test("each held card sits in its agent's hub, the rest in their own areas", async () => {
    const cards = [
      card({ working: [busy("a")] }),
      card({ working: [busy("a")] }),
      card({ working: [idle("b")] }),
      card({
        state: TaskState.Submitted,
        holder: CardHolder({ state: "nobody" }),
      }),
      card({ working: [gone("c")], holder: CardHolder({ state: "gone" }) }),
      card({ state: TaskState.Completed, working: [busy("a")] }),
    ]
    const g = graphOf(cards)
    const l = await autoLayout(g)
    const byId = new Map(g.nodes.map((x) => [x.id, x]))
    const group = (id: string) => (byId.get(id) as CardNode).group
    expect(group("c" + (n - 5))).toBe("group:agent:a")
    expect(group("c" + (n - 4))).toBe("group:agent:a")
    expect(group("c" + (n - 3))).toBe("group:agent:b")
    expect(group("c" + (n - 2))).toBe("group:unheld")
    // A gone holder doesn't keep a card: nobody is on it.
    expect(group("c" + (n - 1))).toBe("group:unheld")
    expect(group("c" + n)).toBe("group:done")
    // A gone agent gets no node; a done card no agent line.
    expect(byId.has("agent:c")).toBe(false)
    expect(g.edges.some((e) => e.target === "c" + n)).toBe(false)

    for (const x of g.nodes) {
      if (x.kind === "group") continue
      const gid = x.kind === "blocker" ? x.group : x.group
      expect(inside(l.groups[gid], l.pos[x.id], l.size[x.id])).toBe(true)
    }
    // The agent's pill heads its own hub.
    expect(
      inside(l.groups["group:agent:a"], l.pos["agent:a"], NodeSize.agent)
    ).toBe(true)
  })

  test("no tall column: one agent with 16 cards is a grid wider than tall", async () => {
    const cards = Array.from({ length: 16 }, () =>
      card({ working: [busy("solo")] })
    )
    const l = await autoLayout(graphOf(cards))
    const hub = l.groups["group:agent:solo"]
    expect(hub.h).toBeLessThan(
      6 * (NodeSize.card.h + 24) + 2 * NodeSize.agent.h
    )
    expect(hub.w).toBeGreaterThan(hub.h)
  })

  test("areas don't overlap, pack wider than tall, and done comes last", async () => {
    const cards = [
      ...Array.from({ length: 30 }, (_, i) =>
        card({ working: [busy(`agent${i % 10}`)] })
      ),
      ...Array.from({ length: 12 }, () => card({ state: TaskState.Submitted })),
      ...Array.from({ length: 5 }, () => card({ state: TaskState.Completed })),
    ]
    const l = await autoLayout(graphOf(cards))
    const rects = Object.values(l.groups)
    for (let i = 0; i < rects.length; i++)
      for (let j = i + 1; j < rects.length; j++)
        expect(overlap(rects[i], rects[j])).toBe(false)
    const right = Math.max(...rects.map((r) => r.x + r.w))
    const bottom = Math.max(...rects.map((r) => r.y + r.h))
    // Wider than tall: a map, not a tower.
    expect(right).toBeGreaterThan(bottom)
    // Done is packed last: nothing sits below or to the right of it on
    // its own row.
    const done = l.groups["group:done"]
    for (const [id, r] of Object.entries(l.groups))
      if (id !== "group:done")
        expect(r.y < done.y || (r.y <= done.y + done.h && r.x < done.x)).toBe(
          true
        )
  })

  test("the same cards give the same layout", async () => {
    const cards = [
      card({ working: [busy("z")] }),
      card({ working: [busy("y")] }),
      card({ working: [busy("y")], openBlockers: 1, blockerQuestion: "Ship?" }),
    ]
    expect(await autoLayout(graphOf(cards))).toEqual(
      await autoLayout(graphOf(cards))
    )
    // Bigger hub first.
    const l = await autoLayout(graphOf(cards))
    expect(l.groups["group:agent:y"].x).toBeLessThan(
      l.groups["group:agent:z"].x
    )
  })

  test("a blocker sits under its card, inside the hub", async () => {
    const c = card({
      working: [busy("a")],
      openBlockers: 1,
      blockerQuestion: "Ship?",
    })
    const l = await autoLayout(graphOf([c]))
    const b = l.pos[`blocker:${c.id}:open`]
    expect(b.y).toBeGreaterThanOrEqual(l.pos[c.id].y + NodeSize.card.h)
    expect(inside(l.groups["group:agent:a"], b, NodeSize.blocker)).toBe(true)
  })

  test("a kept spot counts only inside the card's own area", async () => {
    const a1 = card({ working: [busy("a")] })
    const a2 = card({ working: [busy("a")] })
    const g0 = graphOf([a1, a2])
    const spot = {
      x: (await autoLayout(g0)).pos[a1.id].x + 10,
      y: (await autoLayout(g0)).pos[a1.id].y + 6,
    }
    // Arranged within its hub: kept.
    const inHub = card({ ...a2, pos: spot })
    let g = graphOf([a1, inHub])
    expect(withPlaced(g, await autoLayout(g)).pos[inHub.id]).toEqual(spot)
    // Far off (an older layout, or dropped in the open): back in the grid.
    const far = card({ ...a2, pos: { x: 9000, y: 9000 } })
    g = graphOf([a1, far])
    const auto = await autoLayout(g)
    expect(withPlaced(g, auto).pos[far.id]).toEqual(auto.pos[far.id])
    // While dragged far, it floats rather than stretching the box.
    const dragged = { ...auto.pos, [far.id]: { x: 9000, y: 9000 } }
    expect(
      groupRects(g, dragged, auto, auto.groups)["group:agent:a"].w
    ).toBeLessThan(2000)
    expect(groupRects(g, dragged, auto)["group:agent:a"].w).toBeGreaterThan(
      9000
    )
    // Handed to b: its spot in a's hub no longer counts.
    const handed = card({ ...a2, working: [busy("b")], pos: spot })
    g = graphOf([a1, handed])
    const l = await autoLayout(g)
    expect(withPlaced(g, l).pos[handed.id]).toEqual(l.pos[handed.id])
  })

  test("a second holder sits in the shared strip with a line to the card", async () => {
    const c = card({ working: [busy("a"), busy("b")] })
    const g = graphOf([c])
    expect(g.nodes.find((x) => x.id === "agent:b")).toMatchObject({
      group: "group:others",
    })
    expect(
      g.edges.some(
        (e) => e.kind === "holder" && e.source === "agent:b" && !e.inHub
      )
    ).toBe(true)
  })
})

describe("canvas markers and lines (card #587)", () => {
  test("waiting on you, nobody on it and stale are marked; done cards aren't", async () => {
    const cards = [
      card({ working: [busy("a")], openBlockers: 1 }),
      card({ working: [gone("g")], holder: CardHolder({ state: "gone" }) }),
      card({
        working: [idle("a")],
        stale: true,
        holder: CardHolder({ state: "idle" }),
      }),
      card({ state: TaskState.Completed, stale: true }),
    ]
    const g = graphOf(cards)
    const marks = g.nodes
      .filter((x): x is CardNode => x.kind === "card")
      .map((x) => markersOf(x).map((m) => m.id))
    expect(marks).toEqual([["needs"], ["nobody"], ["stale"], []])
  })

  test("agents talking on a card get one line per pair, named by the cards", async () => {
    const a = busy("a")
    const b = busy("b")
    const c1 = card({ working: [a], talking: [a, b] })
    const c2 = card({ working: [b], talking: [b, a] })
    const g = graphOf([c1, c2])
    const talk = g.edges.filter((e) => e.kind === "talk")
    expect(talk).toHaveLength(1)
    expect(talk[0].source).toBe("agent:a")
    expect(talk[0].target).toBe("agent:b")
    expect(talk[0].label).toBe(`#${c1.short}, #${c2.short}`)
  })

  test("agents with no card of their own share one strip; in-hub holder lines are marked", async () => {
    const w1 = busy("w1")
    const w2 = idle("w2")
    const c = card({ working: [busy("a")], watching: [w1, w2] })
    const g = graphOf([c])
    const l = await autoLayout(g)
    const areas = g.nodes.filter((x) => x.kind === "group").map((x) => x.id)
    expect(areas).toEqual(["group:agent:a", "group:others"])
    for (const k of ["agent:w1", "agent:w2"])
      expect(inside(l.groups["group:others"], l.pos[k], NodeSize.agent)).toBe(
        true
      )
    expect(g.edges.find((e) => e.kind === "holder")?.inHub).toBe(true)
  })

  test("agent status tone comes from agentStatus", async () => {
    const g = graphOf([
      card({ working: [busy("a")] }),
      card({ working: [idle("b")] }),
    ])
    const tone = (id: string) => g.nodes.find((x) => x.id === id)
    expect(tone("agent:a")).toMatchObject({ tone: "working" })
    expect(tone("agent:b")).toMatchObject({ tone: "idle" })
  })

  test("a card that changes hands is a hand-off", async () => {
    const c = card({ working: [busy("a")] })
    const before = graphOf([c])
    const prev = new Map(
      before.nodes
        .filter((x): x is CardNode => x.kind === "card")
        .map((x) => [x.id, x.group])
    )
    expect(handOffs(prev, graphOf([c]))).toEqual([])
    expect(handOffs(prev, graphOf([{ ...c, working: [busy("b")] }]))).toEqual([
      c.id,
    ])
  })
})

// ---- card #776: ELK, weights and lines ----------------------------------

// rectsOf is every node's box in a layout (what the lines go around).
function rectsOf(l: Layout): Record<string, Rect> {
  const out: Record<string, Rect> = {}
  for (const [id, p] of Object.entries(l.pos)) {
    const sz = l.size[id]
    if (sz) out[id] = { x: p.x, y: p.y, w: sz.w, h: sz.h }
  }
  return out
}

// linesOf is every line the canvas draws, routed where the nodes are.
function linesOf(g: Graph, l: Layout): Record<string, Point[]> {
  return routeAll(
    drawn(g).filter((e) => e.kind !== "watcher"),
    rectsOf(l),
    l.routes
  )
}

// throughCards is each line that runs through a node it doesn't join.
function throughCards(
  g: Graph,
  l: Layout,
  lines: Record<string, Point[]>
): string[] {
  const rects = rectsOf(l)
  const bad: string[] = []
  for (const e of drawn(g)) {
    const pts = lines[e.id]
    if (!pts) continue
    const others = Object.entries(rects)
      .filter(([id]) => id !== e.source && id !== e.target)
      .map(([, r]) => r)
    if (!fits(pts, rects[e.source], rects[e.target], others)) bad.push(e.id)
  }
  return bad
}

// loopsBack says a line turns back on itself: it moves both ways along x,
// or both ways along y.
function loopsBack(pts: Point[]): boolean {
  let sx = 0
  let sy = 0
  for (let i = 1; i < pts.length; i++) {
    const dx = Math.sign(pts[i].x - pts[i - 1].x)
    const dy = Math.sign(pts[i].y - pts[i - 1].y)
    if (dx && sx && dx !== sx) return true
    if (dy && sy && dy !== sy) return true
    sx ||= dx
    sy ||= dy
  }
  return false
}

// The board in #774's screenshot: one agent, a parent and two subtasks.
function board774() {
  const lead = busy("k-lead", "robo-jam-lead")
  const p = card({ working: [lead], subtasks: 2 })
  const s1 = card({ working: [lead], parent: { id: p.id } as never })
  const s2 = card({ working: [lead], parent: { id: p.id } as never })
  return { cards: [p, s1, s2], p, s1, s2 }
}

describe("canvas layout on ELK (card #776)", () => {
  test("#774's board: no line through a card, no subtask line loops back", async () => {
    const { cards, p, s1, s2 } = board774()
    const g = graphOf(cards)
    const l = await autoLayout(g)
    const lines = linesOf(g, l)
    // In-hub holder lines aren't drawn at all.
    expect(drawn(g).some((e) => e.kind === "holder")).toBe(false)
    expect(throughCards(g, l, lines)).toEqual([])
    for (const s of [s1, s2]) {
      const pts = lines[`subtask:${p.id}->${s.id}`]
      expect(pts).toBeDefined()
      expect(loopsBack(pts)).toBe(false)
      // Down from the parent into the subtask's top.
      expect(pts[0].y).toBe(l.pos[p.id].y + NodeSize.card.h)
      expect(pts[pts.length - 1].y).toBe(l.pos[s.id].y)
    }
    // Two rows, as in the screenshot: the parent, then both subtasks side
    // by side under it.
    expect(l.pos[s1.id].y).toBeGreaterThan(l.pos[p.id].y)
    expect(l.pos[s2.id].y).toBe(l.pos[s1.id].y)
    expect(l.pos[s2.id].x).not.toBe(l.pos[s1.id].x)
  })

  test("a card moved by hand keeps every line out of the cards", async () => {
    const { cards, p, s1, s2 } = board774()
    const g0 = graphOf(cards)
    const l0 = await autoLayout(g0)
    // The person drags the parent between its two subtasks' row and puts
    // the second subtask to its right.
    const moved = [
      { ...p, pos: { x: l0.pos[s2.id].x, y: l0.pos[p.id].y } },
      s1,
      { ...s2, pos: { x: l0.pos[s1.id].x, y: l0.pos[p.id].y } },
    ]
    const g = graphOf(moved)
    const l = withPlaced(g, await autoLayout(g))
    const lines = linesOf(g, l)
    expect(throughCards(g, l, lines)).toEqual([])
    for (const id of Object.keys(lines))
      if (id.startsWith("subtask:")) expect(loopsBack(lines[id])).toBe(false)
  })

  test("the sample board: no line runs through a card", async () => {
    const w = globalThis as { window?: unknown }
    const had = w.window !== undefined
    if (!had) w.window = { location: { search: "" } }
    let sample: CardSummary[]
    try {
      const { sampleCards } = await import("@/lib/sample-cards")
      sample = sampleCards(new Date(Now))
    } finally {
      if (!had) delete w.window
    }
    const g = graphOf(sample)
    const l = await autoLayout(g)
    expect(throughCards(g, l, linesOf(g, l))).toEqual([])
  })

  test("the same cards in any order give the same layout", async () => {
    const a = busy("a")
    const b = busy("b")
    const { cards } = board774()
    const all = [
      ...cards,
      card({ working: [a], openBlockers: 1, blockerQuestion: "Ship?" }),
      card({ working: [a], talking: [a, b] }),
      card({ working: [b] }),
      card({ state: TaskState.Submitted }),
      card({ state: TaskState.Completed }),
    ]
    const l1 = await autoLayout(graphOf(all))
    const l2 = await autoLayout(graphOf([...all].reverse()))
    const l3 = await autoLayout(
      graphOf([all[3], all[6], all[0], all[7], all[2], all[5], all[1], all[4]])
    )
    expect(l2).toEqual(l1)
    expect(l3).toEqual(l1)
    expect(layoutKey(graphOf([...all].reverse()))).toBe(layoutKey(graphOf(all)))
  })

  test("a title or steps changing doesn't lay the board out again", () => {
    const c = card({ working: [busy("a")] })
    const k = layoutKey(graphOf([c]))
    expect(
      layoutKey(
        graphOf([{ ...c, title: "new", progress: { done: 1, total: 3 } }])
      )
    ).toBe(k)
    expect(layoutKey(graphOf([{ ...c, openBlockers: 1 }]))).not.toBe(k)
  })

  test("weights: what needs you comes first, stale cards go back", async () => {
    const a = busy("a")
    const plain = card({ working: [idle("q")] })
    const stale = card({ working: [idle("q")], stale: true })
    const needs = card({ working: [idle("q")], openBlockers: 1 })
    const recent = card({
      working: [idle("q")],
      updatedAt: new Date(Now - 10 * 60 * 1000).toISOString(),
    })
    const g = graphOf([stale, plain, recent, needs, card({ working: [a] })])
    const score = (id: string) =>
      (g.nodes.find((x) => x.id === id) as CardNode).score
    expect(score(needs.id)).toBe(Weight.needsYou)
    expect(score(recent.id)).toBe(Weight.recent)
    expect(score(plain.id)).toBe(0)
    expect(score(stale.id)).toBe(Weight.stale)
    const l = await autoLayout(g)
    // Reading order in q's hub: row by row, left to right.
    const order = [needs, recent, plain, stale].map((c) => l.pos[c.id])
    for (let i = 1; i < order.length; i++) {
      const [p, q] = [order[i - 1], order[i]]
      expect(p.y < q.y || (p.y === q.y && p.x < q.x)).toBe(true)
    }
    // What needs you leads its hub, and so the hub: q's comes first.
    expect(areaOrder(g)[0].id).toBe("group:agent:q")
    // Otherwise a working agent's hub comes before an idle one's, bigger
    // or not.
    const h = graphOf([
      card({ working: [idle("q")] }),
      card({ working: [idle("q")] }),
      card({ working: [a] }),
    ])
    expect(areaOrder(h).map((x) => x.id)).toEqual([
      "group:agent:a",
      "group:agent:q",
    ])
  })

  test("weights: a hub of stale cards falls behind a hub of plain ones", () => {
    const g = graphOf([
      card({ working: [idle("old")], stale: true }),
      card({ working: [idle("new")] }),
    ])
    expect(areaOrder(g).map((x) => x.id)).toEqual([
      "group:agent:new",
      "group:agent:old",
    ])
  })

  test("why: a subtask held elsewhere doesn't claim to sit under its parent", () => {
    const p = card({ working: [busy("a")] })
    const s1 = card({ working: [busy("b")], parent: { id: p.id } as never })
    const g = graphOf([p, s1])
    const why = (g.nodes.find((x) => x.id === s1.id) as CardNode).why
    expect(why.map((r) => r.text)).toContain(
      `A subtask of #${p.short}, which sits with its own holder`
    )
  })

  test("weights: agents talking sit side by side, done comes last", () => {
    const [a, b, c] = [busy("a"), busy("b"), busy("c")]
    const g = graphOf([
      card({ working: [a], openBlockers: 1 }), // a leads
      card({ working: [b], openBlockers: 1 }), // b next on its own
      card({ working: [c], talking: [c, a] }), // c talks with a
      card({ state: TaskState.Completed }),
      card({ state: TaskState.Submitted }),
    ])
    expect(areaOrder(g).map((x) => x.id)).toEqual([
      "group:agent:a",
      "group:agent:c",
      "group:agent:b",
      "group:unheld",
      "group:done",
    ])
  })

  test("why it's here: where first, then what put it forward", () => {
    const { cards, p, s1 } = board774()
    const g = graphOf([
      { ...cards[0], openBlockers: 1 },
      ...cards.slice(1),
      card({ state: TaskState.Completed }),
    ])
    const why = (id: string) =>
      (g.nodes.find((x) => x.id === id) as CardNode).why.map((r) => r.text)
    expect(why(p.id)).toEqual([
      "In robo-jam-lead's hub: robo-jam-lead holds it",
      "Near the front: it waits on you",
      "Near the front: an agent is working on it now",
    ])
    expect(why(s1.id).slice(0, 2)).toEqual([
      "In robo-jam-lead's hub: robo-jam-lead holds it",
      `Under #${p.short}: it's a subtask of it`,
    ])
    expect(why("c" + n)).toEqual([
      "In Done, at the end and faded: it's finished",
    ])
  })
})

describe("canvas lines (card #776)", () => {
  const box = (x: number, y: number): Rect => ({ x, y, w: 280, h: 132 })

  test("a line goes around a card in its way, not through it", () => {
    const a = box(0, 0)
    const b = box(608, 0)
    const mid = box(304, 0)
    const pts = route(a, b, [mid])
    expect(fits(pts, a, b, [mid])).toBe(true)
    // Over the top of it and down: it bends, but never doubles back.
    expect(pts.length).toBeGreaterThan(2)
    for (let i = 1; i < pts.length; i++)
      expect(pts[i].x).toBeGreaterThanOrEqual(pts[i - 1].x)
  })

  test("sides follow where the cards are", () => {
    const a = box(0, 0)
    // Right of it: out of the right side, into the left.
    let pts = route(a, box(400, 0), [])
    expect(pts[0]).toEqual({ x: 280, y: 66 })
    expect(pts[pts.length - 1]).toEqual({ x: 400, y: 66 })
    // Below it: out of the bottom, into the top.
    pts = route(a, box(0, 300), [])
    expect(pts[0]).toEqual({ x: 140, y: 132 })
    expect(pts[pts.length - 1]).toEqual({ x: 140, y: 300 })
    // Above it: out of the top.
    pts = route(a, box(0, -300), [])
    expect(pts[0]).toEqual({ x: 140, y: 0 })
  })

  test("a tall stack past the search window doesn't let a line through", () => {
    // Review of #776: a column of boxes 8 px apart reaching past the
    // window, so the only way round is outside it.
    const a = box(0, 0)
    const b = box(700, 0)
    const wall: Rect[] = []
    for (let y = -430; y <= 1200; y += 80)
      wall.push({ x: 400, y, w: 240, h: 72 })
    const pts = route(a, b, wall)
    expect(fits(pts, a, b, [...wall, a, b])).toBe(true)
  })

  test("a big scattered board: every line clear, and kept while nothing moves", () => {
    const cards: Rect[] = []
    for (let i = 0; i < 60; i++)
      cards.push(
        box(
          (i % 10) * 330 + ((i * 37) % 23),
          Math.floor(i / 10) * 190 + ((i * 53) % 29)
        )
      )
    const lines = Array.from({ length: 20 }, (_, i) => ({
      id: `l${i}`,
      kind: "subtask",
      source: `n${i}`,
      target: `n${59 - i}`,
    }))
    const rects = Object.fromEntries(cards.map((r, i) => [`n${i}`, r]))
    const out = routeAll(lines, rects, {})
    for (const l of lines) {
      const others = cards.filter(
        (_, i) => `n${i}` !== l.source && `n${i}` !== l.target
      )
      expect(fits(out[l.id], rects[l.source], rects[l.target], others)).toBe(
        true
      )
    }
    // A second pass with nothing moved (a drag frame elsewhere) keeps
    // every line as it was, without routing it again.
    const again = routeAll(lines, rects, {}, out)
    for (const l of lines) expect(again[l.id]).toBe(out[l.id])
  })

  test("ELK's line is kept while it fits, rerouted once a card is in its way", () => {
    const a = box(0, 0)
    const b = box(0, 300)
    const given = [
      { x: 140, y: 132 },
      { x: 140, y: 300 },
    ]
    const lines = [{ id: "e", kind: "subtask", source: "a", target: "b" }]
    expect(routeAll(lines, { a, b }, { e: given }).e).toEqual(given)
    const out = routeAll(lines, { a, b, c: box(60, 160) }, { e: given }).e
    expect(out).not.toEqual(given)
    expect(fits(out, a, b, [box(60, 160)])).toBe(true)
  })
})
