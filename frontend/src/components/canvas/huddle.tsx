// A huddle's canvas (card #1052; docs/design/call-agents.md §3, §8): its
// own React Flow view, apart from the Board's. People head it, each with
// their assistant; the items they and the assistants wrote sit in a column
// under whoever asked. Pan, zoom and drag come from React Flow; a drop
// saves the spot on the item (a `call.item.set`), so every member sees it.
//
// It shares only flow.css and KeepCenter with the Board's canvas. What it
// takes care to avoid is in docs/canvas-layout.md ("Huddle canvas") and
// lib/huddle-canvas.ts: the graph is built from the items alone, a dragged
// node is never reset by an incoming item, and the view refits only while
// untouched and only when a column comes.

import "@xyflow/react/dist/style.css"
import "./reactflow/flow.css"

import {
  createContext,
  memo,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  applyNodeChanges,
  useNodesInitialized,
  useReactFlow,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
} from "@xyflow/react"
import { BotIcon, CheckIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { hueOf, initials } from "@/components/talk/huddle"
import { defaultAssistantName } from "@/lib/assistant-label"
import type { TalkTranscript } from "@/lib/call"
import {
  ItemWidth,
  Placer,
  buildCanvas,
  canSettle,
  canvasKey,
  clamped,
  isLooking,
  kindWord,
  reconcile,
  settleWord,
  shouldRefit,
  type CItem,
  type CNode,
  type CanvasGraph,
  type Head,
  type Point,
} from "@/lib/huddle-canvas"
import { setCanvasItem } from "@/lib/huddles"
import { errorText } from "@/lib/talk"
import { useMedia } from "@/lib/media"
import { cn } from "@/lib/utils"

import { KeepCenter } from "./reactflow"

// ---- what the nodes can do -----------------------------------------------------

type Acts = {
  expanded: ReadonlySet<string>
  toggle: (id: string) => void
  settle: (id: string, status: "settled" | "dropped") => void
  busy: ReadonlySet<string>
}

const ActsContext = createContext<Acts>({
  expanded: new Set(),
  toggle: () => {},
  settle: () => {},
  busy: new Set(),
})

// ---- nodes -------------------------------------------------------------------------

const stateDot: Record<string, string> = {
  connected: "bg-success",
  "not connected": "bg-muted-foreground",
  reconnecting: "bg-warning",
}

const HeadNode = memo(function HeadNode({ data }: NodeProps) {
  const head = (data as { head: Head }).head
  const label =
    head.label ||
    (head.mine ? "Your assistant" : defaultAssistantName(head.name))
  return (
    <div
      className="flex h-[72px] items-center gap-2.5 px-3 glass-panel"
      style={{ width: ItemWidth }}
      data-head={head.name}
    >
      <span
        className="flex size-8 shrink-0 items-center justify-center rounded-full text-[12px] font-semibold text-background"
        style={{ background: hueOf(head.name) }}
        aria-hidden
      >
        {initials(head.name)}
      </span>
      <div className="min-w-0">
        <p className="truncate text-[13px] font-semibold text-ink">
          {head.name}
        </p>
        <p className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <BotIcon className="size-3 shrink-0" aria-hidden />
          <span className="truncate">{label}</span>
          {head.state && (
            <span
              role="img"
              aria-label={head.state}
              title={head.why ? `${head.state}: ${head.why}` : head.state}
              className={cn(
                "size-1.5 shrink-0 rounded-full",
                stateDot[head.state] ?? "bg-muted-foreground"
              )}
            />
          )}
        </p>
      </div>
      <Handle
        id="s-b"
        type="source"
        position={Position.Bottom}
        className="opacity-0"
      />
    </div>
  )
})

const kindTone: Record<string, string> = {
  question: "var(--info)",
  answer: "var(--success)",
  decision: "var(--primary)",
  task: "var(--warning)",
  note: "var(--muted-foreground)",
  summary: "var(--primary)",
}

const ItemNode = memo(function ItemNode({ id, data }: NodeProps) {
  const c = (data as { c: CItem }).c
  const { expanded, toggle, settle, busy } = useContext(ActsContext)
  const { item } = c
  const open = expanded.has(id)
  const long = clamped(item.text)
  const looking = isLooking(item)
  const dropped = item.status === "dropped"
  const who = item.assistant ? item.label || "Assistant" : c.group
  return (
    <div
      data-item={item.itemId}
      data-kind={item.kind}
      data-assistant={item.assistant ? "true" : undefined}
      className={cn(
        "flex flex-col gap-1.5 border-l-2 p-2.5 text-left glass-panel",
        dropped && "opacity-55"
      )}
      style={{
        width: ItemWidth,
        borderLeftColor: kindTone[item.kind] ?? kindTone.note,
      }}
    >
      <div className="flex min-w-0 items-center gap-1.5 text-[11px]">
        <span className="shrink-0 font-semibold text-ink">
          {kindWord(item.kind)}
        </span>
        {c.status && (
          <span className="min-w-0 truncate text-muted-foreground">
            · {c.status}
          </span>
        )}
      </div>
      <p className="flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground">
        {item.assistant && (
          <BotIcon className="size-3 shrink-0" aria-label="Assistant" />
        )}
        <span className="truncate">{who}</span>
      </p>
      <p
        className={cn(
          "text-[12.5px] leading-snug break-words whitespace-pre-wrap text-ink-2",
          !open && "line-clamp-3",
          looking && "text-muted-foreground italic motion-safe:animate-pulse",
          dropped && "line-through"
        )}
      >
        {item.text}
      </p>
      {(long || canSettle(item)) && (
        <div className="nodrag nopan flex flex-wrap items-center gap-1">
          {long && (
            <button
              type="button"
              className="rounded text-[11px] text-primary hover:underline"
              aria-expanded={open}
              onClick={() => toggle(id)}
            >
              {open ? "Less" : "More"}
            </button>
          )}
          {canSettle(item) && (
            <span className="ml-auto flex gap-1">
              <Button
                size="xs"
                variant="secondary"
                disabled={busy.has(id)}
                onClick={() => settle(id, "settled")}
              >
                <CheckIcon />
                {settleWord(item.kind)}
              </Button>
              <Button
                size="xs"
                variant="ghost"
                disabled={busy.has(id)}
                onClick={() => settle(id, "dropped")}
              >
                <XIcon />
                Drop
              </Button>
            </span>
          )}
        </div>
      )}
      <Handle
        id="t-t"
        type="target"
        position={Position.Top}
        className="opacity-0"
      />
      <Handle
        id="t-l"
        type="target"
        position={Position.Left}
        className="opacity-0"
      />
      <Handle
        id="t-r"
        type="target"
        position={Position.Right}
        className="opacity-0"
      />
      <Handle
        id="s-b"
        type="source"
        position={Position.Bottom}
        className="opacity-0"
      />
      <Handle
        id="s-l"
        type="source"
        position={Position.Left}
        className="opacity-0"
      />
      <Handle
        id="s-r"
        type="source"
        position={Position.Right}
        className="opacity-0"
      />
    </div>
  )
})

const nodeTypes = { head: HeadNode, item: ItemNode }

// ---- graph to React Flow ---------------------------------------------------------------

type FlowNode = Node & { position: Point }

function flowNodes(g: CanvasGraph, draggable: boolean): FlowNode[] {
  return g.nodes.map((n: CNode) =>
    n.type === "head"
      ? {
          id: n.id,
          type: "head",
          position: n.pos,
          data: { head: n.head },
          draggable: false,
          selectable: false,
          zIndex: 0,
        }
      : {
          id: n.id,
          type: "item",
          position: n.pos,
          data: { c: n.c },
          draggable,
          zIndex: 1,
        }
  )
}

function flowEdges(g: CanvasGraph, pos: ReadonlyMap<string, Point>): Edge[] {
  return g.edges.map((e) => {
    const a = pos.get(e.source)
    const b = pos.get(e.target)
    // The sides that face each other; one column apart is bottom to top.
    let sh = "s-b"
    let th = "t-t"
    if (a && b) {
      if (b.x - a.x > 40) {
        sh = "s-r"
        th = "t-l"
      } else if (a.x - b.x > 40) {
        sh = "s-l"
        th = "t-r"
      }
    }
    return {
      id: e.id,
      source: e.source,
      target: e.target,
      sourceHandle: sh,
      targetHandle: th,
      type: "default",
      selectable: false,
      markerEnd: e.refs
        ? undefined
        : { type: MarkerType.ArrowClosed, color: "var(--muted-foreground)" },
      style: {
        stroke: "var(--muted-foreground)",
        strokeWidth: 1.25,
        strokeDasharray: e.refs ? "4 4" : undefined,
        opacity: e.refs ? 0.6 : 0.85,
      },
    }
  })
}

// Fit fits the whole canvas while the person hasn't touched the view, and
// only when a column comes or goes (never for an item landing in one).
function Fit({
  columns,
  touched,
}: {
  columns: number
  touched: { current: boolean }
}) {
  const { fitView } = useReactFlow()
  // Nodes have no size until they are measured: fitting before that fits
  // nothing.
  const measured = useNodesInitialized()
  const fitted = useRef(0)
  useEffect(() => {
    if (!measured) return
    if (
      !shouldRefit({
        touched: touched.current,
        columns,
        fitted: fitted.current,
      })
    )
      return
    fitted.current = columns
    const t = window.setTimeout(
      () => void fitView({ padding: 0.1, maxZoom: 1 }),
      50
    )
    return () => window.clearTimeout(t)
  }, [columns, fitView, touched, measured])
  return null
}

// useDarkClass is whether the page is dark, so React Flow's own controls
// follow it.
function useDarkClass(): boolean {
  const [dark, setDark] = useState(() =>
    document.documentElement.classList.contains("dark")
  )
  useEffect(() => {
    const el = document.documentElement
    const read = () => setDark(el.classList.contains("dark"))
    const mo = new MutationObserver(read)
    mo.observe(el, { attributes: true, attributeFilter: ["class"] })
    return () => mo.disconnect()
  }, [])
  return dark
}

// HuddleCanvas draws huddle `callId`'s canvas from its transcript's items.
export function HuddleCanvas({
  callId,
  t,
  people,
  onError,
  onChanged,
}: {
  callId: string
  t: Pick<TalkTranscript, "assistants" | "canvas">
  people: readonly string[]
  onError: (text: string) => void
  // A write went through: read the transcript again.
  onChanged: () => void
}) {
  // The graph depends on the items alone: the key is their content, so a
  // transcript read again (or a line arriving) changes nothing here.
  const key = canvasKey(t, people)
  const [placer] = useState(() => new Placer())
  const graph = useMemo(
    () => buildCanvas(placer, t, people),
    // The key is the content of t and people.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key, placer]
  )
  const touch = useMedia("(pointer: coarse)")
  const built = useMemo(() => flowNodes(graph, !touch), [graph, touch])

  const [nodes, setNodes] = useState<FlowNode[]>(built)
  const dragging = useRef(new Set<string>())
  const pending = useRef(new Map<string, Point>())
  // The graph changed: show it, except where the person's hand is. This is
  // an effect, not a reset in render, so a drag in progress is never lost.
  useEffect(() => {
    setNodes((prev) =>
      reconcile(prev, built, dragging.current, pending.current)
    )
  }, [built])

  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setNodes((ns) => applyNodeChanges(changes, ns) as FlowNode[])
  }, [])

  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set())
  const [busy, setBusy] = useState<ReadonlySet<string>>(new Set())
  const toggle = useCallback(
    (id: string) =>
      setExpanded((s) => {
        const n = new Set(s)
        if (!n.delete(id)) n.add(id)
        return n
      }),
    []
  )
  const save = useCallback(
    (id: string, change: { status?: string; x?: number; y?: number }) => {
      setBusy((s) => new Set(s).add(id))
      setCanvasItem(callId, id, change).then(
        () => {
          setBusy((s) => {
            const n = new Set(s)
            n.delete(id)
            return n
          })
          onChanged()
        },
        (err: unknown) => {
          pending.current.delete(id)
          setBusy((s) => {
            const n = new Set(s)
            n.delete(id)
            return n
          })
          onError(errorText(err))
        }
      )
    },
    [callId, onChanged, onError]
  )
  const acts = useMemo<Acts>(
    () => ({
      expanded,
      toggle,
      busy,
      settle: (id, status) => save(id, { status }),
    }),
    [expanded, toggle, busy, save]
  )

  const edges = useMemo(
    () => flowEdges(graph, new Map(built.map((n) => [n.id, n.position]))),
    [graph, built]
  )
  // An expanded item rises over the ones under it.
  const shown = useMemo(
    () =>
      expanded.size
        ? nodes.map((n) => (expanded.has(n.id) ? { ...n, zIndex: 5 } : n))
        : nodes,
    [nodes, expanded]
  )

  const touched = useRef(false)
  const dark = useDarkClass()

  return (
    <ActsContext.Provider value={acts}>
      <div className="irori-flow h-full w-full" data-testid="huddle-canvas">
        <ReactFlow
          nodes={shown}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onNodeDragStart={(_, n) => {
            touched.current = true
            dragging.current.add(n.id)
          }}
          onNodeDragStop={(_, n) => {
            dragging.current.delete(n.id)
            const p = {
              x: Math.round(n.position.x),
              y: Math.round(n.position.y),
            }
            pending.current.set(n.id, p)
            save(n.id, p)
          }}
          onMoveStart={(e) => {
            if (e) touched.current = true
          }}
          colorMode={dark ? "dark" : "light"}
          minZoom={0.2}
          maxZoom={1.5}
          nodesConnectable={false}
          nodesDraggable={!touch}
          elevateNodesOnSelect={false}
          proOptions={{ hideAttribution: true }}
        >
          <Background
            variant={BackgroundVariant.Dots}
            gap={22}
            size={1}
            color="var(--border)"
          />
          <Controls showInteractive={false} />
          <KeepCenter />
          <Fit columns={graph.columns} touched={touched} />
        </ReactFlow>
      </div>
    </ActsContext.Provider>
  )
}

export default HuddleCanvas
