// The Board's canvas (cards #80, #93): the board as a graph (groups, cards,
// blockers, agents and the edges between them) and where each node sits.
// It reads what the grid already has (CardSummary[] from useCards), plus a
// card's CardDetail when one is to hand, for one blocker node per open
// blocker. A card the person dragged keeps its spot (CardSummary.pos, kept
// in coord so every device shows it); the rest are laid out by autoLayout.
// Nothing here calls the host.

import { cardTone, type Tone } from "@/components/cards/bits"
import {
  BlockerState,
  displayName,
  type AgentRef,
  type CardDetail,
  type CardSummary,
} from "@/lib/cards"

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

export type GroupNode = {
  kind: "group"
  id: string // "group:<key>"
  label: string // "sample-org/example-app", or a board's name
  by: "board" | "repo" | "none"
  key: string // the board id or repoKey, "" for none
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
}

export type GraphNode = GroupNode | CardNode | BlockerNode | AgentNode

export type EdgeKind = "subtask" | "blocker" | "holder" | "watcher"

export type GraphEdge = {
  id: string // "<kind>:<source>-><target>"
  kind: EdgeKind
  source: string // parent card / blocker / agent node id
  target: string // child card / blocked card / held card id
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

function groupOf(c: CardSummary): GroupNode {
  if (c.board)
    return {
      kind: "group",
      id: `group:board:${c.board}`,
      label: c.scopeLabel || c.board,
      by: "board",
      key: c.board,
    }
  if (c.repoKey)
    return {
      kind: "group",
      id: `group:repo:${c.repoKey}`,
      label: repoLabel(c.repoKey),
      by: "repo",
      key: c.repoKey,
    }
  return { kind: "group", id: "group:none", label: "No repo", by: "none", key: "" }
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

// toGraph is the board as a graph: a group per board (shared cards) or
// repo, a node per card, one per open blocker, one per agent on a card,
// and edges parent -> subtask, blocker -> card, agent -> card (holder for
// working, watcher for watching). details, when given, supplies each
// card's own blockers; without it a card with open blockers gets one
// blocker node carrying blockerQuestion.
export function toGraph(
  cards: readonly CardSummary[],
  details?: ReadonlyMap<string, CardDetail>
): Graph {
  const nodes: GraphNode[] = []
  const edges: GraphEdge[] = []
  const groups = new Map<string, GroupNode>()
  const agents = new Map<string, AgentNode>()
  const ids = new Set(cards.map((c) => c.id))
  const edge = (kind: EdgeKind, source: string, target: string) =>
    edges.push({ id: `${kind}:${source}->${target}`, kind, source, target })

  for (const c of cards) {
    const g = groupOf(c)
    if (!groups.has(g.id)) groups.set(g.id, g)
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
      group: g.id,
      repo: c.repoKey,
      board: c.board ?? "",
      faces,
      card: c,
    })
    if (parent) edge("subtask", parent, c.id)

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
        nodes.push({ kind: "blocker", id, label: b.question, card: c.id, state: b.state, group: g.id })
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
        group: g.id,
      })
      edge("blocker", id, c.id)
    }

    for (const f of faces) {
      const id = `agent:${f.key}`
      if (!agents.has(id))
        agents.set(id, { kind: "agent", id, label: f.name, agent: f.agent, live: f.live, person: f.person })
      edge(f.role === "working" ? "holder" : "watcher", id, c.id)
    }
  }
  return { nodes: [...groups.values(), ...nodes, ...agents.values()], edges }
}

// ---- default layout ---------------------------------------------------------

export const NodeSize: Record<Exclude<GraphNode["kind"], "group">, Size> = {
  card: { w: 280, h: 132 },
  blocker: { w: 240, h: 72 },
  agent: { w: 150, h: 40 },
}

const Gap = 24 // between stacked nodes
const Indent = 40 // a subtask under its parent
const Pad = 28 // inside a group's rect
const Head = 32 // a group's label strip
const ColGap = 120 // between columns

// autoLayout is the starting layout, no library: agents in a column on the
// left, then one column per group (biggest first). In a group, each root
// card is followed by its subtasks indented under it (depth first, as the
// grid orders them), with the card's blockers to its right. Deterministic,
// so all three canvases start from the same picture.
export function autoLayout(g: Graph): Layout {
  const pos: Layout["pos"] = {}
  const size: Layout["size"] = {}
  const groups: Layout["groups"] = {}
  const cards = g.nodes.filter((n): n is CardNode => n.kind === "card")
  const blockers = g.nodes.filter((n): n is BlockerNode => n.kind === "blocker")
  const agentNodes = g.nodes.filter((n): n is AgentNode => n.kind === "agent")
  const groupNodes = g.nodes.filter((n): n is GroupNode => n.kind === "group")

  const blockersOf = new Map<string, BlockerNode[]>()
  for (const b of blockers) blockersOf.set(b.card, [...(blockersOf.get(b.card) ?? []), b])
  const childrenOf = new Map<string, CardNode[]>()
  for (const c of cards)
    if (c.parent) childrenOf.set(c.parent, [...(childrenOf.get(c.parent) ?? []), c])

  // Agents: one column at x = 0.
  let ay = Head + Pad
  for (const a of agentNodes) {
    pos[a.id] = { x: 0, y: ay }
    size[a.id] = NodeSize.agent
    ay += NodeSize.agent.h + Gap / 2
  }

  const members = (gid: string) => cards.filter((c) => c.group === gid)
  const ordered = [...groupNodes].sort(
    (a, b) => members(b.id).length - members(a.id).length || a.label.localeCompare(b.label)
  )
  let x = NodeSize.agent.w + ColGap
  for (const grp of ordered) {
    const inGroup = members(grp.id)
    const inSet = new Set(inGroup.map((c) => c.id))
    const roots = inGroup.filter((c) => !c.parent || !inSet.has(c.parent))
    const x0 = x + Pad
    let y = Head + Pad
    let right = x0
    const place = (c: CardNode, depth: number) => {
      const cx = x0 + depth * Indent
      pos[c.id] = { x: cx, y }
      size[c.id] = NodeSize.card
      let bx = cx + NodeSize.card.w + Gap
      let rowH = NodeSize.card.h
      for (const b of blockersOf.get(c.id) ?? []) {
        pos[b.id] = { x: bx, y }
        size[b.id] = NodeSize.blocker
        bx += NodeSize.blocker.w + Gap
        rowH = Math.max(rowH, NodeSize.blocker.h)
      }
      right = Math.max(right, bx - Gap)
      y += rowH + Gap
      for (const k of childrenOf.get(c.id) ?? []) if (inSet.has(k.id)) place(k, depth + 1)
    }
    for (const r of roots) place(r, 0)
    const w = right - x + Pad
    groups[grp.id] = { x, y: 0, w, h: y - Gap + Pad }
    x += w + ColGap
  }
  return { pos, size, groups }
}

// ---- placed cards -----------------------------------------------------------

// withPlaced is the layout with every placed card at its kept spot: its
// blockers move with it (to its right, as autoLayout puts them), and each
// group's rect grows or shrinks to hold its members where they are now.
export function withPlaced(g: Graph, l: Layout): Layout {
  const pos = { ...l.pos }
  for (const n of g.nodes) {
    if (n.kind !== "card" || !n.card.pos) continue
    pos[n.id] = { x: n.card.pos.x, y: n.card.pos.y }
  }
  return { ...l, pos: followers(g, pos), groups: groupRects(g, pos, l) }
}

// followers puts each blocker beside its card's current spot.
export function followers(g: Graph, pos: Record<string, Point>): Record<string, Point> {
  const out = { ...pos }
  const seen = new Map<string, number>()
  for (const n of g.nodes) {
    if (n.kind !== "blocker") continue
    const c = out[n.card]
    if (!c) continue
    const i = seen.get(n.card) ?? 0
    seen.set(n.card, i + 1)
    out[n.id] = {
      x: c.x + NodeSize.card.w + Gap + i * (NodeSize.blocker.w + Gap),
      y: c.y,
    }
  }
  return out
}

// groupRects is each group's rect around its cards and blockers as they
// sit now; a group with none keeps the rect it had.
export function groupRects(
  g: Graph,
  pos: Record<string, Point>,
  l: Layout
): Record<string, Rect> {
  const out: Record<string, Rect> = { ...l.groups }
  const box = new Map<string, { x0: number; y0: number; x1: number; y1: number }>()
  for (const n of g.nodes) {
    if (n.kind !== "card" && n.kind !== "blocker") continue
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
