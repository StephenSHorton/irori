// ProgressRows is Home's agent progress under Needs you (card #261): one
// compact row per card, with one of four states (Needs you, Working,
// Waiting, Done; #259's finding 05), the line that says why, its board
// only when it isn't yours, and its progress.

import { Badge } from "@/components/ui/badge"
import { cardRef, PrChip, Spinner } from "@/components/cards/bits"
import { useContext } from "react"
import { percent, withLive, type CardSummary } from "@/lib/cards"
import { LiveRoster } from "@/lib/thread"
import { plainLine } from "@/lib/irori"
import { rowState, rowStateLabel, type RowState } from "@/lib/needs"
import { cn } from "@/lib/utils"

const stateVariant: Record<RowState, "destructive" | "info" | "slate" | "warning" | "success"> = {
  needs: "destructive",
  working: "info",
  waiting: "slate",
  idle: "warning",
  done: "success",
}

const barTone: Record<RowState, string> = {
  needs: "bg-destructive",
  working: "bg-info",
  waiting: "bg-muted-foreground",
  idle: "bg-warning",
  done: "bg-success",
}

export function ProgressRows({
  label,
  cards,
  selected,
  onOpen,
}: {
  label: string
  cards: CardSummary[]
  selected: string
  onOpen: (id: string) => void
}) {
  if (!cards.length) return null
  return (
    <ul aria-label={label} className="glass-panel grid min-w-0 overflow-hidden">
      {cards.map((c) => (
        <li key={c.id} className="min-w-0 border-b border-border/60 last:border-b-0">
          <ProgressRow card={c} selected={c.id === selected} onOpen={onOpen} />
        </li>
      ))}
    </ul>
  )
}

function ProgressRow({
  card: c,
  selected,
  onOpen,
}: {
  card: CardSummary
  selected: boolean
  onOpen: (id: string) => void
}) {
  // The live roster's status over the card's (card #317): the card page
  // reads busy the same way, so the two never disagree.
  const live = useContext(LiveRoster)
  const { state, line } = rowState(withLive(c, live))
  const p = percent(c.progress)
  const board = c.scope === "board" ? plainLine(c.scopeLabel) : ""
  return (
    <button
      type="button"
      data-card={c.id}
      onClick={() => onOpen(c.id)}
      aria-current={selected || undefined}
      className={cn(
        "grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 px-4 py-2.5 text-left outline-none hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:ring-1 focus-visible:ring-ring sm:grid-cols-[4.5rem_minmax(0,1fr)_minmax(0,16rem)_5.5rem_4.5rem]",
        selected && "bg-accent/40"
      )}
    >
      <span className="font-mono text-xs text-muted-foreground tabular-nums">
        {cardRef(c)}
      </span>
      <span className="min-w-0 truncate text-sm font-medium" title={plainLine(c.title)}>
        {plainLine(c.title)}
        {board && (
          <span className="ml-2 text-xs font-normal text-muted-foreground">{board}</span>
        )}
      </span>
      <span className="col-span-3 col-start-2 row-start-2 flex min-w-0 items-center gap-1.5 text-[12.5px] text-muted-foreground sm:col-span-1 sm:col-start-3 sm:row-start-1">
        {state === "working" && <Spinner className="size-3" />}
        <span className={cn("truncate", state === "idle" && "text-warning")} title={line}>
          {line}
        </span>
        <PrChip card={c} />
      </span>
      <span className="col-start-3 row-start-1 justify-self-end sm:col-start-4">
        <Badge variant={stateVariant[state]}>{rowStateLabel[state]}</Badge>
      </span>
      <span
        className="hidden h-1 overflow-hidden rounded-full bg-muted sm:col-start-5 sm:row-start-1 sm:block"
        role={p === null ? undefined : "progressbar"}
        aria-label={p === null ? undefined : `${cardRef(c)} progress`}
        aria-valuemin={p === null ? undefined : 0}
        aria-valuemax={p === null ? undefined : 100}
        aria-valuenow={p ?? undefined}
      >
        {p !== null && (
          <span className={cn("block h-full", barTone[state])} style={{ width: `${p}%` }} />
        )}
      </span>
    </button>
  )
}
