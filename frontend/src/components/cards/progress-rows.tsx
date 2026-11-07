// ProgressRows is Home's agent progress under Needs you (card #261): one
// compact row per card, with its state (needs.ts RowState: Needs you,
// Stuck, Working, Waiting, Idle, Done), the line that says why, its board
// only when it isn't yours, and its progress. Grouped (card #487), the
// rows come in one headed list per state, in that order.

import { NotYetFold } from "@/components/cards/order"
import { followsOf } from "@/lib/later"
import { Badge } from "@/components/ui/badge"
import { cardRef, PrChip, Spinner } from "@/components/cards/bits"
import { useContext } from "react"
import { percent, withLive, type CardSummary } from "@/lib/cards"
import { LiveRoster } from "@/lib/thread"
import { plainLine } from "@/lib/irori"
import {
  groupRows,
  rowState,
  rowStateLabel,
  type RowLine,
  type RowState,
} from "@/lib/needs"
import { cn } from "@/lib/utils"

const stateVariant: Record<
  RowState,
  "destructive" | "info" | "slate" | "warning" | "success"
> = {
  needs: "destructive",
  stuck: "warning",
  working: "info",
  waiting: "slate",
  idle: "warning",
  later: "slate",
  done: "success",
}

const barTone: Record<RowState, string> = {
  needs: "bg-destructive",
  stuck: "bg-warning",
  working: "bg-info",
  waiting: "bg-muted-foreground",
  idle: "bg-warning",
  later: "bg-muted-foreground",
  done: "bg-success",
}

export function ProgressRows({
  label,
  cards,
  selected,
  onOpen,
  grouped = false,
}: {
  label: string
  cards: CardSummary[]
  selected: string
  onOpen: (id: string) => void
  grouped?: boolean
}) {
  // The live roster's status over the card's (card #317): the card page
  // reads busy the same way, so the two never disagree.
  const live = useContext(LiveRoster)
  if (!cards.length) return null
  const all = cards.map((c) => ({ c, r: rowState(withLive(c, live)) }))
  // Card #570: a card that follows another hangs under it, so the list's
  // shape is the order.
  const byId = new Map(all.map((x) => [x.c.id, x]))
  const under = (id: string) =>
    followsOf(byId.get(id)?.c ?? { id }).flatMap((s) => byId.get(s.id) ?? [])
  const hung = new Set(all.flatMap((x) => under(x.c.id)).map((x) => x.c.id))
  const rows = all.filter((x) => !hung.has(x.c.id))
  const list = (name: string, rs: typeof rows, quiet = false) => (
    // At sm+ the list owns the columns and each row is a subgrid of them
    // (card #586), so the ref column fits this list's widest ref (4.5 to
    // 10rem, see the ref span) and the titles stay lined up. A phone keeps
    // each row's own auto column.
    <ul
      aria-label={name}
      className="grid min-w-0 overflow-hidden glass-panel sm:grid-cols-[max-content_minmax(0,1fr)_minmax(0,16rem)_5.5rem_4.5rem] sm:gap-x-3"
    >
      {rs.map(({ c, r }) => (
        <li
          key={c.id}
          className="min-w-0 border-b border-border/60 last:border-b-0 sm:col-span-full sm:grid sm:grid-cols-subgrid"
        >
          <ProgressRow
            card={c}
            row={r}
            selected={c.id === selected}
            onOpen={onOpen}
            quiet={quiet}
          />
          {under(c.id).map((f) => (
            <ProgressRow
              key={f.c.id}
              card={f.c}
              row={f.r}
              selected={f.c.id === selected}
              onOpen={onOpen}
              quiet
              hangs
            />
          ))}
        </li>
      ))}
    </ul>
  )
  if (!grouped) return list(label, rows)
  return (
    <div className="grid min-w-0 gap-3">
      {groupRows(rows, (x) => x.r.state).map((g) =>
        g.state === "later" ? (
          <NotYetFold key={g.state} count={g.rows.length}>
            {list(`${label}: Not yet`, g.rows, true)}
          </NotYetFold>
        ) : (
          <section key={g.state} className="grid min-w-0 gap-1.5">
            <h3 className="flex items-center gap-2 px-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {rowStateLabel[g.state]}
              <span className="tabular-nums">{g.rows.length}</span>
            </h3>
            {list(`${label}: ${rowStateLabel[g.state]}`, g.rows)}
          </section>
        )
      )}
    </div>
  )
}

function ProgressRow({
  card: c,
  row,
  selected,
  onOpen,
  quiet = false,
  hangs = false,
}: {
  card: CardSummary
  row: RowLine
  selected: boolean
  onOpen: (id: string) => void
  // quiet: no state or bar, dimmed; hangs: under the card it follows.
  quiet?: boolean
  hangs?: boolean
}) {
  const { state, line: said, spin } = row
  const line = hangs ? "" : said
  const p = percent(c.progress)
  const board = c.scope === "board" ? plainLine(c.scopeLabel) : ""
  return (
    <button
      type="button"
      data-card={c.id}
      onClick={() => onOpen(c.id)}
      aria-current={selected || undefined}
      className={cn(
        "grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 px-4 py-2.5 text-left outline-none hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:ring-1 focus-visible:ring-ring sm:col-span-full sm:grid-cols-subgrid",
        selected && "bg-accent/40",
        quiet && "text-muted-foreground opacity-70",
        hangs && "py-1.5"
      )}
    >
      {/* A cross-board ref (agility#1234) widens the list's column up
          to 10rem; past that it is cut, the whole ref on hover. */}
      <span
        className="font-mono text-xs text-muted-foreground tabular-nums sm:max-w-[10rem] sm:min-w-[4.5rem] sm:truncate"
        title={cardRef(c)}
      >
        {hangs && <span aria-hidden>└ </span>}
        {cardRef(c)}
      </span>
      {/* The title gives way, not the board's tag: inside the title's
          ellipsis a phone cut "MLG" off (card #479). */}
      <span
        className="flex min-w-0 items-baseline gap-2 text-sm font-medium"
        title={plainLine(c.title)}
      >
        <span className="min-w-0 truncate">{plainLine(c.title)}</span>
        {board && (
          <span className="shrink-0 text-xs font-normal text-muted-foreground">
            {board}
          </span>
        )}
      </span>
      <span className="col-span-3 col-start-2 row-start-2 flex min-w-0 items-center gap-1.5 text-[12.5px] text-muted-foreground sm:col-span-1 sm:col-start-3 sm:row-start-1">
        {spin && <Spinner className="size-3" />}
        <span
          className={cn(
            "truncate",
            (state === "idle" || state === "stuck") && "text-warning"
          )}
          title={line}
        >
          {line}
        </span>
        <PrChip card={c} />
      </span>
      <span className="col-start-3 row-start-1 justify-self-end sm:col-start-4">
        {!quiet && (
          <Badge variant={stateVariant[state]}>{rowStateLabel[state]}</Badge>
        )}
      </span>
      <span
        className={cn(
          "hidden h-1 overflow-hidden rounded-full sm:col-start-5 sm:row-start-1 sm:block",
          !quiet && "bg-muted"
        )}
        role={p === null ? undefined : "progressbar"}
        aria-label={p === null ? undefined : `${cardRef(c)} progress`}
        aria-valuemin={p === null ? undefined : 0}
        aria-valuemax={p === null ? undefined : 100}
        aria-valuenow={p ?? undefined}
      >
        {p !== null && (
          <span
            className={cn("block h-full", barTone[state])}
            style={{ width: `${p}%` }}
          />
        )}
      </span>
    </button>
  )
}
