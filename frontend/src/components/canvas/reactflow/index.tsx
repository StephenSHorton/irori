// The Board's canvas on React Flow (@xyflow/react; picked on card #80,
// shipped on #93, made a map of who's doing what on #587). Nodes are React
// components, so a card here is the grid's own card body (StatePill, Faces,
// ProgressBar) with its markers on top. Pan, zoom, drag, minimap and
// controls come built in. A dragged card keeps its spot in coord (onPlace)
// and its blockers move with it; each area (an agent's hub, the cards
// nobody is on, done) is a box drawn behind its members, sized to them as
// they move. Agents can be moved for a look, but their spots aren't kept.
//
// Motion (card #587): a working agent pulses, its holder lines flow, talk
// lines between agents flow, and a card that changes hands flashes once.
// None of it runs under Reduce Motion.

import "@xyflow/react/dist/style.css"
import "./flow.css"

import { Maximize2, Minimize2 } from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Position,
  ReactFlow,
  applyNodeChanges,
  useReactFlow,
  useStore,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
} from "@xyflow/react"

import { Button } from "@/components/ui/button"
import { useMedia, useNarrow } from "@/lib/media"
import { useReducedMotion } from "@/lib/motion"
import { cn } from "@/lib/utils"

import {
  followers,
  groupRects,
  handOffs,
  type AgentNode,
  type BlockerNode,
  type CanvasProps,
  type CardNode,
  type Graph,
  type GraphEdge,
  type GroupNode,
  type Layout,
  type Point,
  type Rect,
} from "../data"
import { AgentBody, BlockerBody, CardBody, CardMarkers, openTarget, toneVar } from "../node"

type CardData = { n: CardNode }
type BlockerData = { n: BlockerNode }
type AgentData = { n: AgentNode }
type GroupData = { n: GroupNode }

const hidden = "!size-1.5 !min-h-0 !min-w-0 !border-0 !bg-transparent"

function CardNodeView({ data, selected }: NodeProps<Node<CardData>>) {
  return (
    <>
      <Handle type="target" position={Position.Top} id="top" className={hidden} />
      <Handle type="target" position={Position.Left} id="left" className={hidden} />
      <Handle type="target" position={Position.Bottom} id="below" className={hidden} />
      <CardMarkers n={data.n} />
      <CardBody n={data.n} selected={selected} />
      <Handle type="source" position={Position.Bottom} id="bottom" className={hidden} />
    </>
  )
}

function BlockerNodeView({ data }: NodeProps<Node<BlockerData>>) {
  return (
    <>
      <BlockerBody n={data.n} />
      <Handle type="source" position={Position.Top} id="top" className={hidden} />
    </>
  )
}

function AgentNodeView({ data }: NodeProps<Node<AgentData>>) {
  return (
    <>
      <Handle type="target" position={Position.Left} id="talk-in" className={hidden} />
      <AgentBody n={data.n} />
      <Handle type="source" position={Position.Bottom} id="bottom" className={hidden} />
      <Handle type="source" position={Position.Right} id="right" className={hidden} />
    </>
  )
}

// An area's box: an agent's hub plain, the cards nobody is on in amber
// (what nobody is working shows from across the canvas), done faint.
const areaLook: Record<GroupNode["by"], { box: string; label: string }> = {
  agent: { box: "border-border/70 bg-foreground/[0.02]", label: "text-muted-foreground" },
  unheld: { box: "border-warning/60 bg-warning/[0.04]", label: "text-warning" },
  others: { box: "border-border/40 bg-transparent", label: "text-muted-foreground" },
  done: { box: "border-border/40 bg-transparent opacity-70", label: "text-muted-foreground/70" },
}

function GroupNodeView({ data }: NodeProps<Node<GroupData>>) {
  const look = areaLook[data.n.by]
  return (
    <div className={cn("h-full w-full border border-dashed", look.box)}>
      <span
        className={cn(
          "absolute top-2 left-3 text-xs font-medium tracking-wide uppercase",
          look.label
        )}
      >
        {data.n.label}
      </span>
    </div>
  )
}

// Defined once, outside the component: React Flow warns and remounts
// every node when nodeTypes is a new object each render.
const nodeTypes = {
  card: CardNodeView,
  blocker: BlockerNodeView,
  agent: AgentNodeView,
  group: GroupNodeView,
}

// toNodes is every card, blocker and agent at its spot (areas are drawn
// from these on each render, see groupNodes). Done cards are faded.
function toNodes(graph: Graph, layout: Layout): Node[] {
  const out: Node[] = []
  for (const n of graph.nodes) {
    if (n.kind === "group") continue
    const p = layout.pos[n.id]
    const s = layout.size[n.id]
    if (!p || !s) continue
    const faded = n.kind === "card" && n.done
    out.push({
      id: n.id,
      type: n.kind,
      position: p,
      style: { width: s.w, height: s.h, ...(faded ? { opacity: 0.45 } : {}) },
      data: { n },
      // A blocker follows its card.
      draggable: n.kind !== "blocker",
    })
  }
  return out
}

// groupNodes is a box behind each area's members, where they sit now.
function groupNodes(graph: Graph, layout: Layout, anchor: Record<string, Rect>, nodes: Node[]): Node[] {
  const pos: Record<string, Point> = {}
  for (const n of nodes) pos[n.id] = n.position
  const rects = groupRects(graph, pos, layout, anchor)
  const out: Node[] = []
  for (const g of graph.nodes) {
    if (g.kind !== "group") continue
    const r = rects[g.id]
    if (!r) continue
    out.push({
      id: g.id,
      type: "group",
      position: { x: r.x, y: r.y },
      style: { width: r.w, height: r.h },
      // Rebuilt each render, so never measured by React Flow (which hides
      // a node until it is): give it its size.
      width: r.w,
      height: r.h,
      measured: { width: r.w, height: r.h },
      data: { n: g },
      draggable: false,
      selectable: false,
      focusable: false,
      zIndex: -1,
    })
  }
  return out
}

const edgeStyle: Record<GraphEdge["kind"], { stroke: string; dash?: string; opacity: number }> = {
  subtask: { stroke: "var(--muted-foreground)", opacity: 0.8 },
  blocker: { stroke: "var(--destructive)", dash: "4 4", opacity: 0.9 },
  holder: { stroke: "var(--primary)", opacity: 0.55 },
  watcher: { stroke: "var(--muted-foreground)", dash: "2 4", opacity: 0.2 },
  talk: { stroke: "var(--info)", dash: "6 4", opacity: 0.9 },
}

const handlesFor: Record<GraphEdge["kind"], { sourceHandle: string; targetHandle: string }> = {
  subtask: { sourceHandle: "bottom", targetHandle: "left" },
  blocker: { sourceHandle: "top", targetHandle: "below" },
  holder: { sourceHandle: "bottom", targetHandle: "top" },
  watcher: { sourceHandle: "right", targetHandle: "left" },
  talk: { sourceHandle: "right", targetHandle: "talk-in" },
}

// toEdges is every line. still: Reduce Motion is on, so nothing flows.
// A holder line flows while its agent works; a talk line always flows.
// A holder line inside its agent's hub (the box says it) and a watcher
// line wait for a hover (see Canvas), so the map isn't a web.
export function toEdges(graph: Graph, still: boolean): Edge[] {
  const working = new Set(
    graph.nodes
      .filter((n): n is AgentNode => n.kind === "agent" && (n.tone === "working" || n.tone === "job"))
      .map((n) => n.id)
  )
  return graph.edges.map((e) => {
    const s = edgeStyle[e.kind]
    const flows =
      !still && (e.kind === "talk" || (e.kind === "holder" && working.has(e.source)))
    return {
      id: e.id,
      source: e.source,
      target: e.target,
      ...handlesFor[e.kind],
      type: e.kind === "subtask" || e.kind === "holder" ? "smoothstep" : "default",
      animated: flows,
      hidden: e.kind === "watcher" || !!e.inHub,
      label: e.label,
      labelStyle: e.label ? { fill: "var(--info)", fontSize: 11 } : undefined,
      labelBgStyle: e.label ? { fill: "var(--card)" } : undefined,
      zIndex: e.kind === "talk" ? 5 : 0,
      style: {
        stroke: s.stroke,
        strokeDasharray: s.dash,
        strokeWidth: e.kind === "talk" ? 2 : 1.5,
        opacity: s.opacity,
      },
      markerEnd:
        e.kind === "talk"
          ? undefined
          : { type: MarkerType.ArrowClosed, color: s.stroke, width: 16, height: 16 },
    }
  })
}

function holders(graph: Graph): Map<string, string> {
  const m = new Map<string, string>()
  for (const n of graph.nodes) if (n.kind === "card") m.set(n.id, n.group)
  return m
}

// useFlash is the cards that just changed hands, for a moment (card
// #587); none under Reduce Motion.
function useFlash(graph: Graph, still: boolean): ReadonlySet<string> {
  const last = useRef<Map<string, string> | null>(null)
  const [flash, setFlash] = useState<ReadonlySet<string>>(new Set())
  useEffect(() => {
    const prev = last.current
    last.current = holders(graph)
    if (!prev || still) return
    const moved = handOffs(prev, graph)
    if (!moved.length) return
    setFlash(new Set(moved))
    const t = window.setTimeout(() => setFlash(new Set()), 1600)
    return () => window.clearTimeout(t)
  }, [graph, still])
  return flash
}

// KeepCenter holds the middle of the view still when the canvas changes
// size (a window resize, full screen on or off: card #410): React Flow
// keeps the top-left corner, so the view would slide.
function KeepCenter() {
  const w = useStore((s) => s.width)
  const h = useStore((s) => s.height)
  const { getViewport, setViewport } = useReactFlow()
  const last = useRef({ w: 0, h: 0 })
  useEffect(() => {
    const prev = last.current
    last.current = { w, h }
    if (!prev.w || !prev.h || !w || !h) return
    if (prev.w === w && prev.h === h) return
    const v = getViewport()
    void setViewport({ x: v.x + (w - prev.w) / 2, y: v.y + (h - prev.h) / 2, zoom: v.zoom })
  }, [w, h, getViewport, setViewport])
  return null
}

export default function Canvas({ graph, layout, auto, onOpen, onPlace, full, onFull }: CanvasProps) {
  const start = useMemo(() => toNodes(graph, layout), [graph, layout])
  const [nodes, setNodes] = useState<Node[]>(start)
  const [seed, setSeed] = useState(start)
  if (seed !== start) {
    // The cards (or a kept spot) changed: start from the new layout
    // (render-time reset, the React-recommended way to derive state).
    setSeed(start)
    setNodes(start)
  }

  // A card dragged carries its blockers; dropped, its spot is kept.
  const onNodesChange = useCallback(
    (changes: NodeChange[]) => {
      setNodes((ns) => {
        const next = applyNodeChanges(changes, ns)
        const cardMoved = changes.some(
          (c) => c.type === "position" && c.position && next.find((n) => n.id === c.id)?.type === "card"
        )
        if (!cardMoved) return next
        const pos: Record<string, Point> = {}
        for (const n of next) pos[n.id] = n.position
        const follow = followers(graph, pos)
        return next.map((n) =>
          n.type === "blocker" && follow[n.id] ? { ...n, position: follow[n.id] } : n
        )
      })
      for (const c of changes) {
        if (c.type !== "position" || c.dragging !== false || !c.position) continue
        const n = graph.nodes.find((x) => x.id === c.id)
        if (n?.kind === "card") onPlace(n.id, c.position.x, c.position.y)
      }
    },
    [graph, onPlace]
  )

  const still = useReducedMotion()
  const flash = useFlash(graph, still)

  // Hover an agent or a card to bring its lines to the front, full
  // strength, and fade the rest a little.
  const [hover, setHover] = useState<string | null>(null)
  const base = useMemo(() => toEdges(graph, still), [graph, still])
  const edges = useMemo(
    () =>
      hover === null
        ? base
        : base.map((e) => {
            const lit = e.source === hover || e.target === hover
            const o = Number(e.style?.opacity ?? 1)
            return {
              ...e,
              hidden: lit ? false : e.hidden,
              zIndex: lit ? 10 : e.zIndex,
              style: { ...e.style, opacity: lit ? 1 : o * 0.5 },
            }
          }),
    [base, hover]
  )
  const shown = useMemo(
    () =>
      flash.size
        ? nodes.map((n) => (flash.has(n.id) ? { ...n, className: "irori-handoff" } : n))
        : nodes,
    [nodes, flash]
  )
  const all = useMemo(
    () => [...groupNodes(graph, layout, auto.groups, nodes), ...shown],
    [graph, layout, auto.groups, nodes, shown]
  )
  const placed = graph.nodes.some((n) => n.kind === "card" && n.card.pos)
  // A touch screen (card #227): a finger pans and pinches, so nodes don't
  // drag (a pan that starts on a card would move it), and a phone has no
  // room for the minimap.
  const touch = useMedia("(pointer: coarse)")
  const narrow = useNarrow()

  return (
    <div className="irori-flow h-full w-full">
      <ReactFlow
        nodes={all}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onNodeMouseEnter={(_, n) => setHover(n.id)}
        onNodeMouseLeave={() => setHover(null)}
        onNodeClick={(_, node) => {
          const n = (node.data as { n: { kind: string; id: string; card?: string } }).n
          const id = openTarget(n)
          if (id) onOpen(id)
        }}
        colorMode="dark"
        fitView
        fitViewOptions={{ padding: 0.08 }}
        minZoom={0.1}
        maxZoom={1.75}
        nodesConnectable={false}
        nodesDraggable={!touch}
        elevateNodesOnSelect={false}
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1} color="var(--border)" />
        {!narrow && (
        <MiniMap
          pannable
          zoomable
          nodeColor={(n) => {
            const d = (n.data as { n?: CardNode | GroupNode }).n
            if (d?.kind === "card") return toneVar[d.tone]
            if (n.type === "blocker") return "var(--destructive)"
            if (d?.kind === "group") return "transparent"
            return "var(--primary)"
          }}
          maskColor="color-mix(in oklch, var(--background) 70%, transparent)"
          style={{ background: "var(--card)" }}
        />
        )}
        <Controls showInteractive={false} />
        <KeepCenter />
      </ReactFlow>
      <div className="absolute top-3 right-3 z-10 flex items-center gap-2">
      {placed && (
        <Button
          size="sm"
          variant="secondary"
          title="Put every card back where Irori lays it out"
          onClick={() => {
            for (const n of graph.nodes) {
              const p = auto.pos[n.id]
              if (n.kind === "card" && n.card.pos && p) onPlace(n.id, p.x, p.y)
            }
          }}
        >
          Tidy up
        </Button>
      )}
      {onFull && (
        <Button
          size="icon-sm"
          variant="secondary"
          aria-pressed={!!full}
          aria-label={full ? "Exit full screen" : "Full screen"}
          title={full ? "Exit full screen (Esc)" : "Full screen (F)"}
          onClick={onFull}
        >
          {full ? <Minimize2 /> : <Maximize2 />}
        </Button>
      )}
      </div>
    </div>
  )
}
