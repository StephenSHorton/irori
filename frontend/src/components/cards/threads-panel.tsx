// ThreadsPanel is a shared board's Threads panel beside its Board (card
// #116). Home's threads moved to the Talk page (card #390); a board's stay
// here until Spaces (Talk 4) bring them into Talk. The thread pieces it
// shows live in components/talk/threads.

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
import { parseTime, relativeTime, plainLine } from "@/lib/irori"

// The board's conversations with the room, open ones on top (the one
// they're in unfolded), resolved ones folded under "Resolved".
export function ThreadsPanel({
  threads,
  open,
  onOpen,
  onHide,
  now,
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
  now: Date
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
          now={now}
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
                now={now}
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
  now,
  why,
}: {
  t: ThreadSummary
  open: boolean
  onToggle: () => void
  onResolved: () => void
  now: Date
  why: string
}) {
  // Opened from elsewhere (a #ref, card #144), it scrolls into view.
  const box = useRef<HTMLElement>(null)
  useEffect(() => {
    if (open) box.current?.scrollIntoView({ block: "nearest" })
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
              #{plainLine(t.short)}
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
            {relativeTime(at, now)}
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
      {open && <ThreadView t={t} now={now} onResolved={onResolved} scroll={PanelThreadMax} />}
    </section>
  )
}


// How tall an open thread's messages grow in the panel before they scroll
// (card #170): the window less the header, the ask box and its own lines.
const PanelThreadMax = "max-h-[max(6rem,calc(100svh-30rem))]"
