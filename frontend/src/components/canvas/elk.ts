// The canvas layout on ELK (card #776; it replaces #587's hand-written
// grid). docs/canvas-layout.md says what it does and why; weights.ts has
// the weights. Here the board becomes an ELK graph and back:
//
// - Each area (an agent's hub, the cards nobody is on, the agents here
//   with no card of their own, done) is an ELK compound node laid out on
//   its own (SEPARATE_CHILDREN) by "layered", top to bottom: a subtask
//   sits under its parent with an orthogonal line ELK routes around the
//   other cards, and cards with no line between them are packed in rows
//   about as wide as the window, so nothing grows into a tall column.
// - The areas are packed by "rectpacking", in order: you first, then the
//   hubs by how much is going on in them, the agents talking side by side,
//   then nobody's cards, the others strip, and done last.
// - Everything goes in sorted by weight then card number, never by the
//   order the cards came in, so every device lays out the same picture.
//
// Agent pills and blockers stay out of ELK: a hub's pill heads it (the
// hub's padding leaves it room) and a card's blockers sit under it (its
// ELK box is that much taller), as followers() places them after a drag.
//
// ELK itself runs in a web worker in the app (elk-worker.ts); tests hand
// layoutGraph an in-process one.

import type { ElkExtendedEdge, ElkNode } from "elkjs/lib/elk-api"

import {
  BlockerGap,
  Gap,
  Head,
  NodeSize,
  Pad,
  AreaGap,
  followers,
  type AgentNode,
  type CardNode,
  type Graph,
  type GroupNode,
  type Layout,
  type Point,
} from "./data"
import { AreaWeight, byProminence } from "./weights"

// Elk is what layoutGraph needs of ELK: elkjs's own ELK class fits.
export type Elk = { layout(graph: ElkNode): Promise<ElkNode> }

// A hub's top: its label strip, then its agent's pill.
const HubTop = Head + Pad + NodeSize.agent.h + Gap
// Areas pack to about the window's shape.
const Aspect = 2.2

function cmp(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

// areaOrder is the areas as they are packed (see the top of this file).
export function areaOrder(g: Graph): GroupNode[] {
  const cards = g.nodes.filter((n): n is CardNode => n.kind === "card")
  const agentOf = new Map<string, AgentNode>()
  for (const n of g.nodes)
    if (n.kind === "agent" && n.group.startsWith("group:agent:"))
      agentOf.set(n.group, n)
  const members = new Map<string, CardNode[]>()
  for (const c of cards)
    members.set(c.group, [...(members.get(c.group) ?? []), c])
  const areaScore = (a: GroupNode): number => {
    const ag = agentOf.get(a.id)
    // A hub counts as much as its most prominent card (every hub holds
    // one), so a hub of stale cards falls behind.
    const scores = (members.get(a.id) ?? []).map((c) => c.score)
    const top = scores.length ? Math.max(...scores) : 0
    const tone = ag?.tone
    return (
      top +
      (ag?.person ? AreaWeight.you : 0) +
      (tone === "working" || tone === "job" ? AreaWeight.working : 0) +
      (tone === "idle" ? AreaWeight.idle : 0)
    )
  }
  const label = (a: GroupNode) => agentOf.get(a.id)?.label ?? ""
  const count = (a: GroupNode) => members.get(a.id)?.length ?? 0
  const areas = g.nodes.filter((n): n is GroupNode => n.kind === "group")
  const hubs = areas
    .filter((a) => a.by === "agent")
    .sort(
      (a, b) =>
        areaScore(b) - areaScore(a) ||
        count(b) - count(a) ||
        // Code-unit order, never the locale's: every device agrees.
        cmp(label(a), label(b)) ||
        cmp(a.key, b.key)
    )
  // Agents talking pull together: a hub's talk partner comes right after
  // it (the first time either comes up), so the line between them is
  // short.
  const talks = new Map<string, string[]>()
  for (const e of g.edges) {
    if (e.kind !== "talk") continue
    const a = `group:agent:${e.source.slice("agent:".length)}`
    const b = `group:agent:${e.target.slice("agent:".length)}`
    talks.set(a, [...(talks.get(a) ?? []), b])
    talks.set(b, [...(talks.get(b) ?? []), a])
  }
  const byId = new Map(hubs.map((h) => [h.id, h]))
  const placed = new Set<string>()
  const ordered: GroupNode[] = []
  for (const h of hubs) {
    if (placed.has(h.id)) continue
    placed.add(h.id)
    ordered.push(h)
    const rank = (id: string) => hubs.findIndex((x) => x.id === id)
    const partners = (talks.get(h.id) ?? [])
      .filter((id) => byId.has(id) && !placed.has(id))
      .sort((a, b) => rank(a) - rank(b))
    for (const p of partners) {
      placed.add(p)
      ordered.push(byId.get(p)!)
    }
  }
  return [
    ...ordered,
    ...areas.filter((a) => a.by === "unheld"),
    ...areas.filter((a) => a.by === "others"),
    ...areas.filter((a) => a.by === "done"),
  ]
}

// cardOrder is an area's cards as ELK gets them: each tree (a card and
// its subtasks in the area) as a whole, the most prominent tree first,
// each parent before its subtasks, and subtasks by prominence too.
export function cardOrder(cards: CardNode[]): CardNode[] {
  const inSet = new Set(cards.map((c) => c.id))
  const kids = new Map<string, CardNode[]>()
  for (const c of cards)
    if (c.parent && inSet.has(c.parent))
      kids.set(c.parent, [...(kids.get(c.parent) ?? []), c])
  for (const k of kids.values()) k.sort(byProminence)
  // A tree counts as much as its most prominent card.
  const best = new Map<string, number>()
  const treeScore = (c: CardNode): number => {
    const got = best.get(c.id)
    if (got !== undefined) return got
    best.set(c.id, c.score) // a cycle stops here
    const s = Math.max(c.score, ...(kids.get(c.id) ?? []).map(treeScore))
    best.set(c.id, s)
    return s
  }
  const roots = cards
    .filter((c) => !c.parent || !inSet.has(c.parent))
    .map((c) => ({ c, score: treeScore(c), short: c.short, id: c.id }))
    .sort(byProminence)
  const out: CardNode[] = []
  const seen = new Set<string>()
  const visit = (c: CardNode) => {
    if (seen.has(c.id)) return
    seen.add(c.id)
    out.push(c)
    for (const k of kids.get(c.id) ?? []) visit(k)
  }
  for (const r of roots) visit(r.c)
  // Cards on a parent cycle have no root: add them in order.
  for (const c of [...cards].sort(byProminence)) visit(c)
  return out
}

function blockerCount(g: Graph): Map<string, number> {
  const m = new Map<string, number>()
  for (const n of g.nodes)
    if (n.kind === "blocker") m.set(n.card, (m.get(n.card) ?? 0) + 1)
  return m
}

const areaOptions = (a: GroupNode): Record<string, string> => ({
  "elk.algorithm": "layered",
  "elk.direction": "DOWN",
  "elk.padding": `[top=${a.by === "agent" ? HubTop : Head + Pad},left=${Pad},bottom=${Pad},right=${Pad}]`,
  // A hub about square; the big areas lie wide, like the window.
  "elk.aspectRatio": a.by === "agent" ? "1.6" : "2.4",
  "elk.separateConnectedComponents": "true",
  "elk.spacing.componentComponent": `${Gap}`,
  "elk.spacing.nodeNode": `${Gap}`,
  "elk.layered.spacing.nodeNodeBetweenLayers": `${Gap + 16}`,
  "elk.spacing.edgeNode": "12",
  "elk.spacing.edgeEdge": "8",
  "elk.edgeRouting": "ORTHOGONAL",
  "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
  "elk.layered.considerModelOrder.components": "MODEL_ORDER",
  "elk.layered.crossingMinimization.forceNodeModelOrder": "true",
  "elk.layered.nodePlacement.strategy": "BRANDES_KOEPF",
  "elk.randomSeed": "1",
})

// toElk is the board as an ELK graph (see the top of this file).
export function toElk(g: Graph): ElkNode {
  const order = areaOrder(g)
  const blockers = blockerCount(g)
  const cards = g.nodes.filter((n): n is CardNode => n.kind === "card")
  const areaOf = new Map<string, string>()
  const children = order.map((a): ElkNode => {
    const inArea = cardOrder(cards.filter((c) => c.group === a.id))
    const kids: ElkNode[] = inArea.map((c) => {
      areaOf.set(c.id, a.id)
      const b = blockers.get(c.id) ?? 0
      return {
        id: c.id,
        width: NodeSize.card.w,
        height: NodeSize.card.h + b * (NodeSize.blocker.h + BlockerGap),
      }
    })
    // The others strip is agents only: ELK packs their pills, by name
    // (not by the order the cards came in, which differs by device).
    if (a.by === "others")
      for (const n of g.nodes
        .filter((n): n is AgentNode => n.kind === "agent" && n.group === a.id)
        .sort((x, y) => cmp(x.label, y.label) || cmp(x.id, y.id)))
        kids.push({
          id: n.id,
          width: NodeSize.agent.w,
          height: NodeSize.agent.h,
        })
    const edges: ElkExtendedEdge[] = []
    for (const e of g.edges)
      if (
        e.kind === "subtask" &&
        areaOf.get(e.source) === a.id &&
        areaOf.get(e.target) === a.id
      )
        edges.push({ id: e.id, sources: [e.source], targets: [e.target] })
    return {
      id: a.id,
      layoutOptions: {
        ...areaOptions(a),
        // Wide enough for its pill and label.
        "elk.nodeSize.constraints": "MINIMUM_SIZE",
        "elk.nodeSize.minimum": `(${NodeSize.agent.w + 2 * Pad},${HubTop})`,
      },
      children: kids,
      edges,
    }
  })
  return {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "rectpacking",
      "elk.hierarchyHandling": "SEPARATE_CHILDREN",
      "elk.aspectRatio": `${Aspect}`,
      "elk.spacing.nodeNode": `${AreaGap}`,
      "elk.padding": "[top=0,left=0,bottom=0,right=0]",
      "elk.json.edgeCoords": "ROOT",
      "elk.json.shapeCoords": "ROOT",
      "elk.randomSeed": "1",
    },
    children,
    edges: [],
  }
}

// fromElk reads ELK's answer back as a Layout: every card where ELK put
// it, its blockers under it, each hub's pill at its head, each area's
// rect, and ELK's line for each edge inside an area.
export function fromElk(g: Graph, res: ElkNode): Layout {
  const pos: Layout["pos"] = {}
  const size: Layout["size"] = {}
  const groups: Layout["groups"] = {}
  const routes: Layout["routes"] = {}
  const r = (v: number | undefined) => Math.round(v ?? 0)
  for (const a of res.children ?? []) {
    groups[a.id] = { x: r(a.x), y: r(a.y), w: r(a.width), h: r(a.height) }
    for (const c of a.children ?? []) pos[c.id] = { x: r(c.x), y: r(c.y) }
    for (const e of a.edges ?? []) {
      const s = (e as ElkExtendedEdge).sections
      if (s?.length !== 1) continue
      const pts: Point[] = [
        s[0].startPoint,
        ...(s[0].bendPoints ?? []),
        s[0].endPoint,
      ].map((p) => ({ x: r(p.x), y: r(p.y) }))
      routes[e.id] = pts
    }
  }
  for (const n of g.nodes) {
    if (n.kind === "group") continue
    size[n.id] = NodeSize[n.kind]
    // A hub's pill heads it.
    if (n.kind === "agent" && !pos[n.id] && groups[n.group])
      pos[n.id] = {
        x: groups[n.group].x + Pad,
        y: groups[n.group].y + Head + Pad,
      }
  }
  return { pos: followers(g, pos), size, groups, routes }
}

// layoutGraph lays the board out on ELK.
export async function layoutGraph(g: Graph, elk: Elk): Promise<Layout> {
  if (!g.nodes.some((n) => n.kind === "group"))
    return { pos: {}, size: {}, groups: {}, routes: {} }
  return fromElk(g, await elk.layout(toElk(g)))
}

// layoutKey is everything the layout depends on, and nothing else: a new
// key means lay out again. A card's title or steps changing keeps the
// picture still. steady leaves out what only reorders (the weights, and
// whether an agent is working or idle), which the canvas lets settle a
// while before it moves anything (reactflow/index.tsx).
export function layoutKey(g: Graph, steady = false): string {
  const parts: string[] = []
  for (const n of g.nodes) {
    if (n.kind === "card")
      parts.push(
        `c|${n.id}|${n.group}|${n.parent ?? ""}|${steady ? "" : n.score}|${n.short}`
      )
    else if (n.kind === "blocker") parts.push(`b|${n.id}|${n.card}`)
    else if (n.kind === "agent")
      parts.push(
        `a|${n.id}|${n.group}|${steady ? "" : n.tone}|${n.person ? 1 : 0}|${n.label}`
      )
    else parts.push(`g|${n.id}|${n.by}`)
  }
  for (const e of g.edges)
    if (e.kind === "subtask" || e.kind === "talk") parts.push(`e|${e.id}`)
  return parts.sort().join("\n")
}
