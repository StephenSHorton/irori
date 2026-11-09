// ThreadsPanel is a shared board's Threads panel beside its Board (card
// #116). Home's threads moved to the Talk page (card #390); a board's stay
// here until Spaces (Talk 4) bring them into Talk. The thread pieces it
// shows live in components/talk/threads.

import { Ago } from "@/components/common/ago"
import { cardRef } from "@/components/cards/bits"
import { useEffect, useRef, useState } from "react"
import { ChevronDownIcon, ChevronRightIcon, XIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  ThreadCards,
  ThreadView,
  openAfterResolve,
  resolvedShown,
  resolvedToggle,
  stateBadges,
  stateLine,
} from "@/components/talk/threads"
import { type ThreadSummary } from "@/lib/cards"
import { parseTime, plainLine } from "@/lib/irori"

// The board's conversations with the room, open ones on top (the one
// they're in unfolded), resolved ones folded under "Resolved".
export function ThreadsPanel({
  threads,
  open,
  onOpen,
  onHide,
  why = "",
}: {
  threads: ThreadSummary[]
  // Why nobody is free to take a waiting thread (card #172), "" when
  // someone is.
  why?: string
  // The thread unfolded ("" none).
  open: string
  onOpen: (id: string) => void
  onHide: () => void
}) {
  const [showResolvedPick, setShowResolved] = useState(false)
  // A resolved thread opened from a #ref (card #144) unfolds its list.
  const showResolved = resolvedShown(showResolvedPick, open, threads)
  const live = threads.filter((t) => t.state !== "resolved")
  const resolved = threads.filter((t) => t.state === "resolved")
  return (
    <aside
      className="grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-3 overflow-clip p-3 glass-panel"
      aria-label="Threads"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Threads{live.length > 0 && ` · ${live.length} open`}
        </h2>
        <Button
          size="icon-xs"
          variant="ghost"
          aria-label="Hide threads"
          title="Hide threads"
          onClick={onHide}
        >
          <XIcon />
        </Button>
      </div>
      {live.length === 0 && (
        <p className="text-[12.5px] text-muted-foreground">
          No open threads. Ask about anything in the box above: an agent on the
          cards you name answers, or an idle one does.
        </p>
      )}
      {live.map((t) => (
        <ThreadRow
          key={t.id}
          t={t}
          open={open === t.id}
          onToggle={() => onOpen(open === t.id ? "" : t.id)}
          onResolved={() => onOpen(openAfterResolve(open, t.id))}

          why={why}
        />
      ))}
      {resolved.length > 0 && (
        <div className="grid gap-2 border-t border-border pt-3">
          <button
            type="button"
            className="flex items-center gap-1 justify-self-start text-xs text-muted-foreground hover:text-ink"
            onClick={() => {
              const t = resolvedToggle(showResolvedPick, open, threads)
              setShowResolved(t.pick)
              if (t.closeOpen) onOpen("")
            }}
            aria-expanded={showResolved}
          >
            {showResolved ? (
              <ChevronDownIcon className="size-3.5" />
            ) : (
              <ChevronRightIcon className="size-3.5" />
            )}
            Resolved ({resolved.length})
          </button>
          {showResolved &&
            resolved.map((t) => (
              <ThreadRow
                key={t.id}
                t={t}
                open={open === t.id}
                onToggle={() => onOpen(open === t.id ? "" : t.id)}
                onResolved={() => onOpen(openAfterResolve(open, t.id))}

                why={why}
              />
            ))}
        </div>
      )}
    </aside>
  )
}

function ThreadRow({
  t,
  open,
  onToggle,
  onResolved,
  why,
}: {
  t: ThreadSummary
  open: boolean
  onToggle: () => void
  onResolved: () => void
  why: string
}) {
  // Opened from elsewhere (a #ref, card #144), it scrolls into view. It
  // grows as its messages load and arrive: it scrolls again so the reply
  // box stays in view, unless the person has scrolled away (card #564).
  const box = useRef<HTMLElement>(null)
  useEffect(() => {
    const el = box.current
    if (!open || !el) return
    let inView = true
    const seen = new IntersectionObserver(
      ([e]) => {
        inView = e.isIntersecting
      },
      { threshold: 0.5 }
    )
    const grown = new ResizeObserver(() => {
      if (inView) el.scrollIntoView({ block: "nearest" })
    })
    seen.observe(el)
    grown.observe(el)
    return () => {
      seen.disconnect()
      grown.disconnect()
    }
  }, [open])
  const at = parseTime(t.updatedAt)
  const waiting = t.state === "waiting" || t.state === "handed"
  const badge = stateBadges[t.state]
  return (
    <section
      ref={box}
      className={
        open
          ? "grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 rounded-lg border border-primary bg-background/40 px-3 py-2.5"
          : "grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1 rounded-lg border border-border bg-background/40 px-3 py-2.5"
      }
    >
      <button
        type="button"
        className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-0.5 text-left"
        onClick={onToggle}
        aria-expanded={open}
      >
        <span className="flex min-w-0 items-center gap-2">
          {t.short && (
            <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
              {cardRef(t)}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
            {plainLine(t.title)}
          </span>
          {t.private && (
            <Badge
              variant="slate"
              className="h-4 shrink-0 px-1.5 text-[10.5px]"
              title="Only you and your own agents see this thread; it never reaches other members' devices"
            >
              Private
            </Badge>
          )}
          {badge && (
            <Badge
              variant={badge.variant}
              className="h-4 shrink-0 px-1.5 text-[10.5px]"
            >
              {badge.label}
            </Badge>
          )}
          <time
            className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
            dateTime={at?.toISOString()}
          >
            <Ago at={at} />
          </time>
        </span>
        {!open && (
          <span
            className={
              waiting
                ? "line-clamp-2 text-xs text-warning"
                : "line-clamp-2 text-xs text-muted-foreground"
            }
          >
            {stateLine(t, why)}
          </span>
        )}
      </button>
      {t.cards.length > 0 && <ThreadCards cards={t.cards} />}
      {open && (
        <ThreadView
          t={t}

          onResolved={onResolved}
          bound={PanelThreadBound}
        />
      )}
    </section>
  )
}

// How tall an open thread grows in the panel (cards #170, #564): the window
// less the header, the ask box and its row's title, so the row fits the
// column. If a tall draft outgrows that in a tiny window, the thread itself
// scrolls; the padding keeps the box's focus ring from being cut.
const PanelThreadBound =
  "-mx-1 max-h-[max(13rem,calc(100svh-19rem))] overflow-y-auto px-1"
