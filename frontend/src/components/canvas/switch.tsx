// The Board's view switch (card #93): List (the grid) or Canvas (the board
// as a flow chart you can pan, zoom and arrange). The canvas is its own
// lazy chunk, so React Flow only loads when Canvas is picked. A card's spot
// is kept in coord (cards.place), so every device shows the same layout.

import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react"

import { Button } from "@/components/ui/button"
import {
  cards as cardsApi,
  type CardDetail,
  type CardSummary,
} from "@/lib/cards"
import { previewFlag } from "@/lib/preview"

import { toGraph } from "./data"
import { RecentMs } from "./weights"

export type BoardView = "list" | "canvas"

const views: { id: BoardView; label: string }[] = [
  { id: "list", label: "List" },
  { id: "canvas", label: "Canvas" },
]

const Canvas = lazy(() => import("./reactflow"))

// The picked view is a per-window convenience, kept in this browser.
const viewKey = "irori.board.view"

function asView(v: string | null): BoardView {
  return views.find((x) => x.id === v)?.id ?? "list"
}

// useBoardView is the picked view: ?view=<id> in the preview, else the one
// last picked here, else List.
export function useBoardView(): [BoardView, (v: BoardView) => void] {
  const [view, setView] = useState<BoardView>(() => {
    const q = previewFlag("view")
    if (q) return asView(q)
    try {
      return asView(localStorage.getItem(viewKey))
    } catch {
      return "list"
    }
  })
  const pick = (v: BoardView) => {
    setView(v)
    try {
      localStorage.setItem(viewKey, v)
    } catch {
      // not kept
    }
  }
  return [view, pick]
}

export function ViewSwitch({
  view,
  onView,
}: {
  view: BoardView
  onView: (v: BoardView) => void
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Board view"
    >
      {views.map((v) => (
        <Button
          key={v.id}
          size="sm"
          aria-pressed={view === v.id}
          variant={view === v.id ? "secondary" : "ghost"}
          onClick={() => onView(v.id)}
        >
          {v.label}
        </Button>
      ))}
    </div>
  )
}

// useBlockerDetails loads the page data of every card with open or
// answered blockers, so each blocker is its own node with its question.
// It reloads when one of those cards changes.
function useBlockerDetails(list: CardSummary[]): Map<string, CardDetail> {
  const want = list.filter((c) => c.openBlockers > 0 || c.waiting > 0)
  const key = want.map((c) => `${c.id}@${c.updatedAt}`).join(",")
  const [got, setGot] = useState<Map<string, CardDetail>>(new Map())
  useEffect(() => {
    let live = true
    const ids = key ? key.split(",").map((k) => k.split("@")[0]) : []
    Promise.all(ids.map((id) => cardsApi.detail(id).catch(() => null))).then(
      (ds) => {
        if (!live) return
        const m = new Map<string, CardDetail>()
        for (const d of ds) if (d) m.set(d.id, d)
        setGot(m)
      }
    )
    return () => {
      live = false
    }
  }, [key])
  return got
}

// useRecentExpiry ticks when the next card stops counting as changed in
// the last hour (weights.ts RecentMs), so the canvas reads the clock again
// then rather than at the next unrelated change.
function useRecentExpiry(cards: CardSummary[]): number {
  const [tick, setTick] = useState(0)
  useEffect(() => {
    const now = Date.now()
    let next = Infinity
    for (const c of cards) {
      const end = Date.parse(c.updatedAt) + RecentMs
      if (end > now && end < next) next = end
    }
    if (!Number.isFinite(next)) return
    const t = window.setTimeout(() => setTick((n) => n + 1), next - now + 1000)
    return () => window.clearTimeout(t)
  }, [cards, tick])
  return tick
}

// typing is a key pressed in a field, where F is a letter, not a shortcut.
function typing(t: EventTarget | null): boolean {
  const e = t as HTMLElement | null
  return (
    !!e && (e.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(e.tagName))
  )
}

// BoardCanvas is the canvas over the cards the grid shows. fill: it takes
// the column's remaining height (card #410), so the page never scrolls in
// Canvas; without it (a phone) it has a set height in the scrolling page.
// Full screen lays the same canvas over the whole window, so pan and zoom
// carry over both ways: the corner button or F (while the canvas has
// focus) opens it, Esc or the button closes it.
export function BoardCanvas({
  cards,
  onOpen,
  fill = false,
}: {
  cards: CardSummary[]
  onOpen: (id: string) => void
  fill?: boolean
}) {
  // The grid hands a new array on every render (its clock ticks): build
  // the graph again only when a card's content changed, or a drag in
  // progress would be reset under the pointer.
  const sig = JSON.stringify(cards)
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const stable = useMemo(() => cards, [sig])
  const details = useBlockerDetails(stable)
  const tick = useRecentExpiry(stable)
  const graph = useMemo(
    () => toGraph(stable, details),
    // tick: a card's "changed in the last hour" ran out (weights.ts).
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [stable, details, tick]
  )
  const [full, setFull] = useState(() => previewFlag("full") !== null)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!full) return
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return
      e.preventDefault()
      setFull(false)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [full])
  const onKey = (e: KeyboardEvent) => {
    if (
      e.key.toLowerCase() !== "f" ||
      e.metaKey ||
      e.ctrlKey ||
      e.altKey ||
      typing(e.target)
    )
      return
    e.preventDefault()
    setFull((f) => !f)
  }
  const size = full
    ? "fixed inset-0 z-50 rounded-none! pt-[var(--safe-top)] pb-[var(--safe-bottom)]"
    : fill
      ? "min-h-[280px] flex-[1_1_0]"
      : "h-[calc(100vh-220px)] min-h-[480px]"
  // fixed and relative are one property: give only one.
  const at = full ? "" : "relative "
  return (
    <div
      ref={box}
      tabIndex={-1}
      aria-label="Board canvas"
      data-canvas-full={full ? "" : undefined}
      // A click on the canvas gives it focus, so F reaches it.
      onPointerDownCapture={() => {
        if (!box.current?.contains(document.activeElement))
          box.current?.focus({ preventScroll: true })
      }}
      onKeyDown={onKey}
      style={full ? { background: "var(--background)" } : undefined}
      className={`glass-panel ${at}overflow-hidden outline-none ${size}`}
    >
      <Suspense
        fallback={
          <p className="p-4 text-sm text-muted-foreground">
            Loading the canvas…
          </p>
        }
      >
        <Canvas
          graph={graph}
          onOpen={onOpen}
          onPlace={(id, x, y) => void cardsApi.place(id, x, y)}
          full={full}
          onFull={() => {
            setFull((f) => !f)
            box.current?.focus({ preventScroll: true })
          }}
        />
      </Suspense>
    </div>
  )
}
