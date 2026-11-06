import { describe, expect, test } from "bun:test"

import { AgentRef, CardHolder, CardSummary, TaskState } from "@/lib/models"

import {
  NodeSize,
  autoLayout,
  colsFor,
  groupRects,
  handOffs,
  toGraph,
  withPlaced,
  type CardNode,
  type Rect,
} from "./data"
import { markersOf } from "./node"

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
  test("each held card sits in its agent's hub, the rest in their own areas", () => {
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
    const g = toGraph(cards)
    const l = autoLayout(g)
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

  test("no tall column: one agent with 16 cards is a 4 by 4 grid", () => {
    const cards = Array.from({ length: 16 }, () =>
      card({ working: [busy("solo")] })
    )
    const l = autoLayout(toGraph(cards))
    const hub = l.groups["group:agent:solo"]
    expect(colsFor(16, 4)).toBe(4)
    expect(hub.h).toBeLessThan(
      5 * (NodeSize.card.h + 24) + 2 * NodeSize.agent.h
    )
    expect(hub.w).toBeGreaterThan(hub.h)
  })

  test("areas don't overlap, wrap into rows, and done starts its own row", () => {
    const cards = [
      ...Array.from({ length: 30 }, (_, i) =>
        card({ working: [busy(`agent${i % 10}`)] })
      ),
      ...Array.from({ length: 12 }, () => card({ state: TaskState.Submitted })),
      ...Array.from({ length: 5 }, () => card({ state: TaskState.Completed })),
    ]
    const l = autoLayout(toGraph(cards))
    const rects = Object.values(l.groups)
    for (let i = 0; i < rects.length; i++)
      for (let j = i + 1; j < rects.length; j++)
        expect(overlap(rects[i], rects[j])).toBe(false)
    const right = Math.max(...rects.map((r) => r.x + r.w))
    const bottom = Math.max(...rects.map((r) => r.y + r.h))
    // Wider than tall: a map, not a tower.
    expect(right).toBeGreaterThan(bottom)
    const done = l.groups["group:done"]
    expect(done.x).toBe(0)
    for (const [id, r] of Object.entries(l.groups))
      if (id !== "group:done") expect(r.y + r.h).toBeLessThanOrEqual(done.y)
  })

  test("the same cards give the same layout", () => {
    const cards = [
      card({ working: [busy("z")] }),
      card({ working: [busy("y")] }),
      card({ working: [busy("y")], openBlockers: 1, blockerQuestion: "Ship?" }),
    ]
    expect(autoLayout(toGraph(cards))).toEqual(autoLayout(toGraph(cards)))
    // Bigger hub first.
    const l = autoLayout(toGraph(cards))
    expect(l.groups["group:agent:y"].x).toBeLessThan(
      l.groups["group:agent:z"].x
    )
  })

  test("a blocker sits under its card, inside the hub", () => {
    const c = card({
      working: [busy("a")],
      openBlockers: 1,
      blockerQuestion: "Ship?",
    })
    const l = autoLayout(toGraph([c]))
    const b = l.pos[`blocker:${c.id}:open`]
    expect(b.y).toBeGreaterThanOrEqual(l.pos[c.id].y + NodeSize.card.h)
    expect(inside(l.groups["group:agent:a"], b, NodeSize.blocker)).toBe(true)
  })

  test("a kept spot counts only inside the card's own area", () => {
    const a1 = card({ working: [busy("a")] })
    const a2 = card({ working: [busy("a")] })
    const g0 = toGraph([a1, a2])
    const spot = {
      x: autoLayout(g0).pos[a1.id].x + 10,
      y: autoLayout(g0).pos[a1.id].y + 6,
    }
    // Arranged within its hub: kept.
    const inHub = card({ ...a2, pos: spot })
    let g = toGraph([a1, inHub])
    expect(withPlaced(g, autoLayout(g)).pos[inHub.id]).toEqual(spot)
    // Far off (an older layout, or dropped in the open): back in the grid.
    const far = card({ ...a2, pos: { x: 9000, y: 9000 } })
    g = toGraph([a1, far])
    const auto = autoLayout(g)
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
    g = toGraph([a1, handed])
    const l = autoLayout(g)
    expect(withPlaced(g, l).pos[handed.id]).toEqual(l.pos[handed.id])
  })

  test("a second holder sits in the shared strip with a line to the card", () => {
    const c = card({ working: [busy("a"), busy("b")] })
    const g = toGraph([c])
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
  test("waiting on you, nobody on it and stale are marked; done cards aren't", () => {
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
    const g = toGraph(cards)
    const marks = g.nodes
      .filter((x): x is CardNode => x.kind === "card")
      .map((x) => markersOf(x).map((m) => m.id))
    expect(marks).toEqual([["needs"], ["nobody"], ["stale"], []])
  })

  test("agents talking on a card get one line per pair, named by the cards", () => {
    const a = busy("a")
    const b = busy("b")
    const c1 = card({ working: [a], talking: [a, b] })
    const c2 = card({ working: [b], talking: [b, a] })
    const g = toGraph([c1, c2])
    const talk = g.edges.filter((e) => e.kind === "talk")
    expect(talk).toHaveLength(1)
    expect(talk[0].source).toBe("agent:a")
    expect(talk[0].target).toBe("agent:b")
    expect(talk[0].label).toBe(`#${c1.short}, #${c2.short}`)
  })

  test("agents with no card of their own share one strip; in-hub holder lines are marked", () => {
    const w1 = busy("w1")
    const w2 = idle("w2")
    const c = card({ working: [busy("a")], watching: [w1, w2] })
    const g = toGraph([c])
    const l = autoLayout(g)
    const areas = g.nodes.filter((x) => x.kind === "group").map((x) => x.id)
    expect(areas).toEqual(["group:agent:a", "group:others"])
    for (const k of ["agent:w1", "agent:w2"])
      expect(inside(l.groups["group:others"], l.pos[k], NodeSize.agent)).toBe(
        true
      )
    expect(g.edges.find((e) => e.kind === "holder")?.inHub).toBe(true)
  })

  test("agent status tone comes from agentStatus", () => {
    const g = toGraph([
      card({ working: [busy("a")] }),
      card({ working: [idle("b")] }),
    ])
    const tone = (id: string) => g.nodes.find((x) => x.id === id)
    expect(tone("agent:a")).toMatchObject({ tone: "working" })
    expect(tone("agent:b")).toMatchObject({ tone: "idle" })
  })

  test("a card that changes hands is a hand-off", () => {
    const c = card({ working: [busy("a")] })
    const before = toGraph([c])
    const prev = new Map(
      before.nodes
        .filter((x): x is CardNode => x.kind === "card")
        .map((x) => [x.id, x.group])
    )
    expect(handOffs(prev, toGraph([c]))).toEqual([])
    expect(handOffs(prev, toGraph([{ ...c, working: [busy("b")] }]))).toEqual([
      c.id,
    ])
  })
})
