import { LayersIcon } from "lucide-react"

import { Card } from "@/components/ui/card"
import { percent, type CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { cn } from "@/lib/utils"
import { ProgressBar, ScopeBadge, StatePill } from "./bits"
import { Faces } from "./faces"

export type Filter = "all" | "decide" | "mine"

export const filters: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "decide", label: "Needs a decision" },
  { id: "mine", label: "My devices" },
]

export function matches(c: CardSummary, f: Filter): boolean {
  if (f === "decide") return c.openBlockers > 0
  if (f === "mine") return c.scope === "mine"
  return true
}

// FilterBar is the chips over the grid. Shared boards join it in v0.4.
export function FilterBar({
  filter,
  onFilter,
  decide,
}: {
  filter: Filter
  onFilter: (f: Filter) => void
  decide: number
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-2"
      role="group"
      aria-label="Filter cards"
    >
      {filters.map((f) => (
        <button
          key={f.id}
          type="button"
          aria-pressed={filter === f.id}
          onClick={() => onFilter(f.id)}
          className={cn(
            "inline-flex h-7 items-center gap-1.5 rounded-full border px-3 text-[12.5px] transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/50",
            filter === f.id
              ? "border-transparent bg-accent font-medium text-primary"
              : "border-border bg-card text-muted-foreground hover:text-ink"
          )}
        >
          {f.label}
          {f.id === "decide" && decide > 0 && (
            <span className="rounded-full bg-destructive/10 px-1.5 text-[11px] text-destructive tabular-nums">
              {decide}
            </span>
          )}
        </button>
      ))}
    </div>
  )
}

// CardGrid is the cards of shared work, one button each.
export function CardGrid({
  cards,
  selected,
  onSelect,
}: {
  cards: CardSummary[]
  selected: string
  onSelect: (id: string) => void
}) {
  return (
    <section
      aria-label="Shared work"
      className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3"
    >
      {cards.map((c) => (
        <CardTile
          key={c.id}
          card={c}
          selected={c.id === selected}
          onSelect={() => onSelect(c.id)}
        />
      ))}
    </section>
  )
}

function CardTile({
  card,
  selected,
  onSelect,
}: {
  card: CardSummary
  selected: boolean
  onSelect: () => void
}) {
  const p = percent(card.progress)
  const blocked = card.openBlockers > 0
  const title = plainLine(card.title) || "(untitled)"
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex min-w-0 flex-col gap-2.5 rounded-[14px] border border-border bg-card p-3.5 text-left text-sm glass-panel transition-[border-color,box-shadow] outline-none hover:border-primary/40 focus-visible:ring-3 focus-visible:ring-ring/50",
        selected && "border-primary ring-3 ring-accent hover:border-primary"
      )}
    >
      <span className="flex flex-wrap items-center gap-1.5">
        <StatePill card={card} />
        <ScopeBadge card={card} />
      </span>
      <span className="text-[14.5px] leading-snug font-semibold text-balance text-ink">
        {title}
      </span>
      {blocked && card.blockerQuestion && (
        <span className="flex gap-1.5 text-[12.5px] text-destructive">
          <span aria-hidden>●</span>
          <span className="line-clamp-2">
            {plainLine(card.blockerQuestion)}
          </span>
        </span>
      )}
      <ProgressBar
        progress={card.progress}
        blocked={blocked}
        label={`${p ?? 0}% complete`}
      />
      <span className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
        {p !== null && (
          <>
            <span className="tabular-nums">{p}%</span>
            <span aria-hidden>·</span>
          </>
        )}
        {card.working.length > 0 ? (
          <>
            <Faces who={card.working} />
            <span>working</span>
          </>
        ) : (
          <span>nobody on it yet</span>
        )}
        {card.watching.length > 0 && (
          <>
            <span aria-hidden>·</span>
            <Faces who={card.watching} watching />
            <span>watching</span>
          </>
        )}
      </span>
    </button>
  )
}

// EmptyCards explains where cards come from when there are none.
export function EmptyCards() {
  return (
    <Card className="items-center px-6 py-12! text-center">
      <LayersIcon className="size-8 text-muted-foreground" aria-hidden />
      <div className="flex max-w-md flex-col gap-2">
        <h2 className="text-base font-medium text-ink">No shared work yet</h2>
        <p className="text-muted-foreground">
          Agents put a card here when their work should be shared: what they're
          on, how far along it is, who else is watching, and what they need you
          to decide. Plans and backlogs stay in the project's own tracker.
        </p>
      </div>
    </Card>
  )
}
