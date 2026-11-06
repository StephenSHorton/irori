// The Board's canvas (cards #80, #93, #587): the board as a graph (areas,
// cards, blockers, agents and the edges between them) and where each node
// sits. It reads what the grid already has (CardSummary[] from useCards),
// plus a card's CardDetail when one is to hand, for one blocker node per
// open blocker. A card the person dragged keeps its spot (CardSummary.pos,
// kept in coord so every device shows it); the rest are laid out by
// autoLayout. Nothing here calls the host.
//
// The layout says who is doing what (card #587): each live agent is a hub
// with the cards it holds in a small grid under it, the cards nobody live
// is on sit in their own area, and done cards are tucked in a faded area
// at the end. Nearness means something; nothing grows into a tall column.

import { cardTone, type Tone } from "@/components/cards/bits"
import { agentStatus, type StatusTone } from "@/lib/agent-status"
import {
  BlockerState,
  displayName,
  nobodyOn,
  openAsk,
  over,
  type AgentRef,
  type CardDetail,
  type CardSummary,
} from "@/lib/cards"
import { needsYou } from "@/lib/needs"

// ---- the graph --------------------------------------------------------------

export type Point = { x: number; y: number }
export type Size = { w: number; h: number }
export type Rect = Point & Size

// FaceRef is one face on a card: who, and in which role.
export type FaceRef = {
  key: string
  name: string
  role: "working" | "watching"
  live: boolean
  person: boolean
  color?: string
  agent: AgentRef
}

// GroupNode is an area on the canvas: an agent's hub (the agent and the
// cards it holds), the cards nobody is on, the agents here that hold
// nothing (watching, talking), or the done cards.
export type GroupNode = {
  kind: "group"
  id: string // "group:agent:<key>", "group:unheld", "group:others" or "group:done"
  label: string // "working · 3 cards", "Nobody on these", "Done"
  by: "agent" | "unheld" | "others" | "done"
  key: string // the agent's key, "" for the other two
}

export type CardNode = {
  kind: "card"
  id: string // the card id, so openCard(id) works as is
  label: string // title
  short: string // "#7" style ref without the hash
  state: CardSummary["state"]
  tone: Tone // the grid's status colour (cardTone)
  progress: { done: number; total: number }
  openBlockers: number
  waiting: number // answered blockers waiting on an agent
  parent: string | null // parent card id
  subtasks: number
  group: string // GroupNode id
  repo: string // repoKey, "" for none
  board: string // board id, "" for none
  where: string // the board's name or the repo's, as a person reads it
  // Markers (card #587): it waits on the person, nobody live is on it,
  // nothing happened on it for a while (the host's stale), it is over.
  needs: boolean
  nobody: boolean
  stale: boolean
  done: boolean
  faces: FaceRef[]
  card: CardSummary // the source card, for the real Irori bits (StatePill, Faces)
}

export type BlockerNode = {
  kind: "blocker"
  id: string // "blocker:<cardId>:<blockerId>" ("open" when only the summary is known)
  label: string // the question
  card: string // the blocked card's id
  state: string // BlockerState
  group: string
}

export type AgentNode = {
  kind: "agent"
  id: string // "agent:<key>"
  label: string
  agent: AgentRef
  live: boolean
  person: boolean
  group: string // its hub's GroupNode id, or "group:others"
  status: string // agentStatus's words: "working", "idle", ...
  tone: StatusTone // agentStatus's tone: working pulses
}

export type GraphNode = GroupNode | CardNode | BlockerNode | AgentNode

// talk: two agents wrote to each other on a card in the last few minutes
// (CardSummary.talking, the host's talk window).
export type EdgeKind = "subtask" | "blocker" | "holder" | "watcher" | "talk"

export type GraphEdge = {
  id: string // "<kind>:<source>-><target>"
  kind: EdgeKind
  source: string // parent card / blocker / agent node id
  target: string // child card / blocked card / held card / agent id
  label?: string // a talk line's cards: "#12, #14"
  // A holder line to a card in that agent's own hub: the box already
  // says it, so the canvas shows it only on hover.
  inHub?: boolean
}

export type Graph = {
  nodes: GraphNode[]
  edges: GraphEdge[]
}

// Laid out: every node but a group has a top-left position and a size; a
// group has the rect around its members. Positions are absolute (React
// Flow subflows want them relative to the group: subtract group.x/y).
export type Layout = {
  pos: Record<string, Point>
  size: Record<string, Size>
  groups: Record<string, Rect>
}

// ---- cards to graph ---------------------------------------------------------

// repoLabel is a repoKey as a person reads it: "remote:github.com/org/repo"
// to "org/repo", a local path to its last folder.
export function repoLabel(repoKey: string): string {
  const k = repoKey.replace(/^remote:/, "").replace(/^local:/, "")
  const gh = k.match(/^[^/]+\.[^/]+\/(.+)$/)
  if (gh) return gh[1]
  return k.split(/[\\/]/).filter(Boolean).pop() ?? k
}

function whereOf(c: CardSummary): string {
  if (c.board) return c.scopeLabel || c.board
  if (c.repoKey) return repoLabel(c.repoKey)
  return ""
}

function faceOf(a: AgentRef, role: FaceRef["role"]): FaceRef {
  return {
    key: a.key,
    name: displayName(a),
    role,
    live: a.live,
    person: a.status === "person",
    color: a.color,
    agent: a,
  }
}

// here says an agent is there to carry a card: running, on a device
// that's online, not gone. The person always is.
export function here(a: AgentRef): boolean {
  if (a.status === "person") return true
  return a.live && a.status !== "gone" && !a.offline && a.status !== "unknown"
}

const UnheldId = "group:unheld"
const OthersId = "group:others"
const DoneId = "group:done"
const hubId = (key: string) => `group:agent:${key}`

// hubOf is where a card sits: done cards in Done, else under its first
// holder that is here, else with the cards nobody is on.
export function hubOf(c: CardSummary): string {
  if (over(c)) return DoneId
  const h = c.working.find(here)
  return h ? hubId(h.key) : UnheldId
}

const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`

// toGraph is the board as a graph: a hub per agent that is here (the
// person too, when they hold a card), an area for the cards nobody is on
// and one for done cards; a node per card and per open blocker; edges
// parent -> subtask, blocker -> card, agent -> card (holder for working,
// watcher for watching) and agent -> agent for agents talking on a card.
// details, when given, supplies each card's own blockers; without it a
// card with open blockers gets one blocker node carrying blockerQuestion.
// Agents that aren't here get no node: their faces stay on the card.
export function toGraph(
  cards: readonly CardSummary[],
  details?: ReadonlyMap<string, CardDetail>
): Graph {
  const nodes: GraphNode[] = []
  const edges: GraphEdge[] = []
  const agents = new Map<string, AgentNode>()
  const held = new Map<string, number>() // hub id -> open cards in it
  const ids = new Set(cards.map((c) => c.id))
  const seen = new Set<string>()
  const edge = (kind: EdgeKind, source: string, target: string, label?: string) => {
    const id = `${kind}:${source}->${target}`
    if (seen.has(id)) return
    seen.add(id)
    edges.push({ id, kind, source, target, ...(label ? { label } : {}) })
  }
  const agentNode = (a: AgentRef): AgentNode => {
    const id = `agent:${a.key}`
    let n = agents.get(id)
    if (!n) {
      const st = agentStatus(a)
      n = {
        kind: "agent",
        id,
        label: displayName(a),
        agent: a,
        live: a.live,
        person: a.status === "person",
        group: hubId(a.key),
        status: st.text,
        tone: st.tone,
      }
      agents.set(id, n)
    }
    return n
  }

  let unheld = 0
  let done = 0
  const talk = new Map<string, { a: string; b: string; cards: string[] }>()
  for (const c of cards) {
    const g = hubOf(c)
    const isDone = g === DoneId
    if (g === UnheldId) unheld++
    else if (isDone) done++
    else held.set(g, (held.get(g) ?? 0) + 1)
    const faces = [
      ...c.working.map((a) => faceOf(a, "working")),
      ...c.watching.map((a) => faceOf(a, "watching")),
    ]
    const parent = c.parent && ids.has(c.parent.id) ? c.parent.id : null
    nodes.push({
      kind: "card",
      id: c.id,
      label: c.title,
      short: c.short,
      state: c.state,
      tone: cardTone(c),
      progress: c.progress,
      openBlockers: c.openBlockers,
      waiting: c.waiting,
      parent,
      subtasks: c.subtasks ?? 0,
      group: g,
      repo: c.repoKey,
      board: c.board ?? "",
      where: whereOf(c),
      needs: needsYou(c),
      // The grid's rule for its Nobody on it badge.
      nobody: !isDone && !openAsk(c) && nobodyOn(c) && !c.starting,
      stale: !isDone && !!c.stale,
      done: isDone,
      faces,
      card: c,
    })
    if (parent) edge("subtask", parent, c.id)
    // A done card is tucked away: no blockers, no agent lines.
    if (isDone) continue

    const open = details
      ?.get(c.id)
      ?.blockers.filter(
        (b) =>
          !b.resolved &&
          // A wait on other work isn't a question (card #481).
          !b.waitsOn &&
          b.state !== BlockerState.Resolved &&
          b.state !== BlockerState.Closed
      )
    if (open?.length) {
      for (const b of open) {
        const id = `blocker:${c.id}:${b.id}`
        nodes.push({ kind: "blocker", id, label: b.question, card: c.id, state: b.state, group: g })
        edge("blocker", id, c.id)
      }
    } else if (c.openBlockers > 0 || c.waiting > 0) {
      const id = `blocker:${c.id}:open`
      nodes.push({
        kind: "blocker",
        id,
        label:
          c.blockerQuestion ||
          `${c.waiting} answer${c.waiting === 1 ? "" : "s"} waiting on an agent`,
        card: c.id,
        state: c.openBlockers > 0 ? BlockerState.Open : BlockerState.Answered,
        group: g,
      })
      edge("blocker", id, c.id)
    }

    for (const f of faces) {
      if (!here(f.agent)) continue
      // You watching every card would be a line to each: your face on
      // the card says it.
      if (f.person && f.role === "watching") continue
      const a = agentNode(f.agent)
      edge(f.role === "working" ? "holder" : "watcher", a.id, c.id)
    }

    // Agents writing to each other on this card: a line per pair.
    const talking = (c.talking ?? []).filter((a) => here(a) && a.status !== "person")
    for (let i = 0; i < talking.length; i++)
      for (let j = i + 1; j < talking.length; j++) {
        const [a, b] = [talking[i].key, talking[j].key].sort()
        if (a === b) continue
        const k = `${a}\n${b}`
        const t = talk.get(k) ?? { a, b, cards: [] }
        t.cards.push(`#${c.short.replace(/^#/, "")}`)
        talk.set(k, t)
      }
    for (const a of talking) agentNode(a)
  }
  for (const t of talk.values())
    edge("talk", `agent:${t.a}`, `agent:${t.b}`, t.cards.join(", "))

  // An agent that holds a card here (or you) has a hub; the rest that
  // are here (watching, talking) share one strip.
  const hubs: GroupNode[] = []
  let others = 0
  for (const a of agents.values()) {
    const n = held.get(a.group) ?? 0
    if (!n && !a.person) {
      a.group = OthersId
      others++
      continue
    }
    hubs.push({
      kind: "group",
      id: a.group,
      label: [a.person ? "You" : a.status, plural(n, "card")].filter(Boolean).join(" · "),
      by: "agent",
      key: a.agent.key,
    })
  }
  const groupOf = new Map<string, string>()
  for (const x of nodes) if (x.kind === "card") groupOf.set(x.id, x.group)
  for (const e of edges)
    if (e.kind === "holder" && groupOf.get(e.target) === agents.get(e.source)?.group) e.inHub = true
  const areas: GroupNode[] = []
  if (unheld)
    areas.push({ kind: "group", id: UnheldId, label: `Nobody on these · ${unheld}`, by: "unheld", key: "" })
  if (others)
    areas.push({
      kind: "group",
      id: OthersId,
      label: `Also here, on no card of their own · ${others}`,
      by: "others",
      key: "",
    })
  if (done) areas.push({ kind: "group", id: DoneId, label: `Done · ${done}`, by: "done", key: "" })
  return { nodes: [...hubs, ...areas, ...nodes, ...agents.values()], edges }
}

// handOffs is the cards whose live holder changed between two graphs: a
// card that came to an agent, or moved from one to another.
export function handOffs(prev: Map<string, string>, graph: Graph): string[] {
  const out: string[] = []
  for (const n of graph.nodes) {
    if (n.kind !== "card" || n.done) continue
    const was = prev.get(n.id)
    if (was !== undefined && was !== n.group && n.group.startsWith("group:agent:")) out.push(n.id)
  }
  return out
}

// ---- default layout ---------------------------------------------------------

export const NodeSize: Record<Exclude<GraphNode["kind"], "group">, Size> = {
  card: { w: 280, h: 132 },
  blocker: { w: 240, h: 72 },
  agent: { w: 180, h: 40 },
}

const Gap = 24 // between cards in a grid
const Pad = 24 // inside an area's rect
const Head = 32 // an area's label strip
const AreaGap = 72 // between areas
// Areas wrap into rows about this many times wider than tall, like the
// window the canvas sits in, so the map fills it instead of a column.
const Aspect = 2.2
const MinRow = 1600 // a small board stays on one row
const BlockerInset = 20 // a blocker under its card, set in a little
const BlockerGap = 8 // between a card and its blockers, and between blockers
const HubCols = 4 // an agent's grid is at most this wide
const AreaCols = 6 // the unheld and done grids
const PillCols = 6 // agents in the others strip
const PillGap = 12

// colsFor is how wide a grid of n cards is: about square, so a hub of 16
// cards is 4 by 4, never 16 tall.
export function colsFor(n: number, max: number): number {
  return Math.max(1, Math.min(max, Math.ceil(Math.sqrt(n))))
}

// blockerSpot is where a card's i-th blocker sits: under the card, so the
// grid's next column stays clear.
function blockerSpot(card: Point, i: number): Point {
  return {
    x: card.x + BlockerInset,
    y: card.y + NodeSize.card.h + BlockerGap + i * (NodeSize.blocker.h + BlockerGap),
  }
}

// depthFirst orders an area's cards so each subtask follows its parent,
// keeping the input order (the grid's) otherwise.
function depthFirst(cards: CardNode[]): CardNode[] {
  const inSet = new Set(cards.map((c) => c.id))
  const kids = new Map<string, CardNode[]>()
  for (const c of cards)
    if (c.parent && inSet.has(c.parent)) kids.set(c.parent, [...(kids.get(c.parent) ?? []), c])
  const out: CardNode[] = []
  const visit = (c: CardNode) => {
    out.push(c)
    for (const k of kids.get(c.id) ?? []) visit(k)
  }
  for (const c of cards) if (!c.parent || !inSet.has(c.parent)) visit(c)
  return out
}

type Placed = { w: number; h: number; at: [string, Point, Size][] }

// placeArea lays one area out from (0, 0): its label strip, its agents
// (a hub's one, or the others strip's several, in rows), then its cards
// in a grid, each card's blockers under it.
function placeArea(
  agents: AgentNode[],
  cards: CardNode[],
  blockersOf: Map<string, BlockerNode[]>,
  maxCols: number
): Placed {
  const at: Placed["at"] = []
  let y = Head + Pad
  let w = 0
  const pillCols = Math.min(PillCols, agents.length)
  for (let r = 0; r * pillCols < agents.length; r++) {
    agents
      .slice(r * pillCols, (r + 1) * pillCols)
      .forEach((a, i) => at.push([a.id, { x: Pad + i * (NodeSize.agent.w + PillGap), y }, NodeSize.agent]))
    w = Math.max(w, pillCols * NodeSize.agent.w + (pillCols - 1) * PillGap)
    y += NodeSize.agent.h + PillGap
  }
  if (agents.length) y += Gap - PillGap
  const cols = colsFor(cards.length, maxCols)
  for (let r = 0; r * cols < cards.length; r++) {
    const row = cards.slice(r * cols, (r + 1) * cols)
    let rowH = 0
    row.forEach((c, i) => {
      const p = { x: Pad + i * (NodeSize.card.w + Gap), y }
      at.push([c.id, p, NodeSize.card])
      const bs = blockersOf.get(c.id) ?? []
      bs.forEach((b, j) => at.push([b.id, blockerSpot(p, j), NodeSize.blocker]))
      const h = NodeSize.card.h + bs.length * (NodeSize.blocker.h + BlockerGap)
      rowH = Math.max(rowH, h)
    })
    w = Math.max(w, row.length * NodeSize.card.w + (row.length - 1) * Gap)
    y += rowH + Gap
  }
  if (cards.length || agents.length) y -= Gap
  return { w: w + 2 * Pad, h: y + Pad, at }
}

// autoLayout is the starting layout, no library (card #587): a hub per
// agent (biggest first, then by name), then the cards nobody is on, packed
// in rows that wrap; the done cards start a row of their own at the end.
// Deterministic, so every device starts from the same picture.
export function autoLayout(g: Graph): Layout {
  const pos: Layout["pos"] = {}
  const size: Layout["size"] = {}
  const groups: Layout["groups"] = {}
  const cards = g.nodes.filter((n): n is CardNode => n.kind === "card")
  const blockersOf = new Map<string, BlockerNode[]>()
  for (const n of g.nodes)
    if (n.kind === "blocker") blockersOf.set(n.card, [...(blockersOf.get(n.card) ?? []), n])
  const agentsOf = new Map<string, AgentNode[]>()
  for (const n of g.nodes)
    if (n.kind === "agent") agentsOf.set(n.group, [...(agentsOf.get(n.group) ?? []), n])
  const agentOf = (id: string) => agentsOf.get(id)?.[0]
  const members = new Map<string, CardNode[]>()
  for (const c of cards) members.set(c.group, [...(members.get(c.group) ?? []), c])
  const count = (id: string) => members.get(id)?.length ?? 0

  const areas = g.nodes.filter((n): n is GroupNode => n.kind === "group")
  const hubs = areas
    .filter((a) => a.by === "agent")
    .sort((a, b) => {
      const pa = agentOf(a.id)?.person ? 1 : 0
      const pb = agentOf(b.id)?.person ? 1 : 0
      return (
        pb - pa || // you first
        count(b.id) - count(a.id) ||
        (agentOf(a.id)?.label ?? "").localeCompare(agentOf(b.id)?.label ?? "") ||
        a.key.localeCompare(b.key)
      )
    })
  const order = [
    ...hubs,
    ...areas.filter((a) => a.by === "unheld"),
    ...areas.filter((a) => a.by === "others"),
    ...areas.filter((a) => a.by === "done"),
  ]

  const placedAreas = order.map((a) => ({
    a,
    placed: placeArea(
      agentsOf.get(a.id) ?? [],
      depthFirst(members.get(a.id) ?? []),
      blockersOf,
      a.by === "agent" ? HubCols : AreaCols
    ),
  }))
  const room = placedAreas
    .filter((p) => p.a.by !== "done")
    .reduce((s, p) => s + (p.placed.w + AreaGap) * (p.placed.h + AreaGap), 0)
  const rowWidth = Math.max(
    MinRow,
    ...placedAreas.map((p) => p.placed.w),
    Math.sqrt(room * Aspect)
  )

  let x = 0
  let y = 0
  let rowH = 0
  for (const { a, placed } of placedAreas) {
    const fresh = a.by === "done" // done starts its own row
    if (x > 0 && (fresh || x + placed.w > rowWidth)) {
      x = 0
      y += rowH + AreaGap
      rowH = 0
    }
    groups[a.id] = { x, y, w: placed.w, h: placed.h }
    for (const [id, p, s] of placed.at) {
      pos[id] = { x: x + p.x, y: y + p.y }
      size[id] = s
    }
    x += placed.w + AreaGap
    rowH = Math.max(rowH, placed.h)
  }
  return { pos, size, groups }
}

// ---- placed cards -----------------------------------------------------------

// withPlaced is the layout with every placed card at its kept spot: its
// blockers move with it (under it, as autoLayout puts them), and each
// area's rect grows or shrinks to hold its members where they are now.
export function withPlaced(g: Graph, l: Layout): Layout {
  const pos = { ...l.pos }
  for (const n of g.nodes) {
    if (n.kind !== "card" || !n.card.pos) continue
    pos[n.id] = { x: n.card.pos.x, y: n.card.pos.y }
  }
  return { ...l, pos: followers(g, pos), groups: groupRects(g, pos, l, l.groups) }
}

// followers puts each blocker under its card's current spot.
export function followers(g: Graph, pos: Record<string, Point>): Record<string, Point> {
  const out = { ...pos }
  const seen = new Map<string, number>()
  for (const n of g.nodes) {
    if (n.kind !== "blocker") continue
    const c = out[n.card]
    if (!c) continue
    const i = seen.get(n.card) ?? 0
    seen.set(n.card, i + 1)
    out[n.id] = blockerSpot(c, i)
  }
  return out
}

// Reach is how far outside its area's laid-out rect a kept card may sit
// and still count in it: a card dragged within its hub grows the box; one
// dragged far away (or kept from an older layout) floats on its own
// rather than stretching the box across the canvas.
const Reach = 160

function near(p: Point, r: Rect | undefined): boolean {
  if (!r) return true
  return (
    p.x >= r.x - Reach - NodeSize.card.w &&
    p.x <= r.x + r.w + Reach &&
    p.y >= r.y - Reach - NodeSize.card.h &&
    p.y <= r.y + r.h + Reach
  )
}

// groupRects is each area's rect around its agent, cards and blockers as
// they sit now; an area with none keeps the rect it had. anchor, when
// given, is where each area was laid out: a card far from it doesn't
// count (see Reach), nor do its blockers.
export function groupRects(
  g: Graph,
  pos: Record<string, Point>,
  l: Layout,
  anchor?: Record<string, Rect>
): Record<string, Rect> {
  const out: Record<string, Rect> = { ...l.groups }
  const box = new Map<string, { x0: number; y0: number; x1: number; y1: number }>()
  const away = new Set<string>()
  if (anchor)
    for (const n of g.nodes)
      if (n.kind === "card" && pos[n.id] && !near(pos[n.id], anchor[n.group])) away.add(n.id)
  for (const n of g.nodes) {
    if (n.kind !== "card" && n.kind !== "blocker" && n.kind !== "agent") continue
    if (away.has(n.kind === "blocker" ? n.card : n.id)) continue
    const p = pos[n.id]
    if (!p) continue
    const s = NodeSize[n.kind]
    const b = box.get(n.group) ?? { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity }
    box.set(n.group, {
      x0: Math.min(b.x0, p.x),
      y0: Math.min(b.y0, p.y),
      x1: Math.max(b.x1, p.x + s.w),
      y1: Math.max(b.y1, p.y + s.h),
    })
  }
  for (const [id, b] of box)
    out[id] = { x: b.x0 - Pad, y: b.y0 - Head - Pad, w: b.x1 - b.x0 + 2 * Pad, h: b.y1 - b.y0 + Head + 2 * Pad }
  return out
}

// ---- what every canvas takes ------------------------------------------------

export type CanvasProps = {
  graph: Graph
  // The auto layout with placed cards laid over (withPlaced).
  layout: Layout
  // The auto layout alone, for Tidy up.
  auto: Layout
  onOpen: (id: string) => void // App's openCard: the real card page
  onPlace: (id: string, x: number, y: number) => void // keep a card's spot
  // Full screen (card #410): whether the canvas fills the window, and
  // the button that toggles it.
  full?: boolean
  onFull?: () => void
}
