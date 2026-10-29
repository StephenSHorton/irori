import { LayersIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import {
  busy,
  openAsk,
  percent,
  workLabel,
  type CardSummary,
} from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { cn } from "@/lib/utils"
import {
  AskLine,
  cardTone,
  HolderLine,
  ProgressBar,
  ScopeBadge,
  ShortRef,
  Spinner,
  StatePill,
} from "./bits"
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
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Filter cards"
    >
      {filters.map((f) => (
        <Button
          key={f.id}
          size="sm"
          aria-pressed={filter === f.id}
          variant={filter === f.id ? "default" : "ghost"}
          onClick={() => onFilter(f.id)}
        >
          {f.label}
          {f.id === "decide" && decide > 0 && (
            <span
              className={cn(
                "text-xs tabular-nums",
                filter === f.id ? "opacity-80" : "text-destructive"
              )}
            >
              {decide}
            </span>
          )}
        </Button>
      ))}
    </div>
  )
}

// CardGrid is the cards of shared work, one button each; a button opens
// its card's page. selected is the card last opened, marked on the way
// back so the person can see where they were.
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
  const label = workLabel(card)
  return (
    <button
      type="button"
      data-card={card.id}
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "glass-panel flex min-w-0 flex-col gap-2.5 p-3.5 text-left text-sm outline-none",
        "transition-[border-color,box-shadow,transform] duration-150",
        "hover:-translate-y-0.5 hover:border-primary/70 hover:shadow-[0_0_0_1px_var(--color-ring),0_12px_28px_oklch(0_0_0/0.45)] focus-visible:ring-1 focus-visible:ring-ring",
        selected &&
          "border-primary! shadow-[0_0_0_1px_var(--color-ring),0_8px_30px_oklch(0_0_0/0.45)]"
      )}
    >
      <span className="flex flex-wrap items-center gap-1.5">
        <ShortRef card={card} />
        <StatePill card={card} />
        <ScopeBadge card={card} />
        {card.working.some((w) => w.conflict) && (
          <Badge
            variant="warning"
            title="Two devices leased the same thing while apart"
          >
            Lease conflict
          </Badge>
        )}
        {busy(card) && <Spinner className="ml-auto" />}
      </span>
      <span className="text-sm leading-snug font-medium text-balance text-ink">
        {title}
      </span>
      {blocked && card.blockerQuestion && (
        <span className="flex gap-1.5 text-xs text-destructive">
          <span aria-hidden>●</span>
          <span className="line-clamp-2">
            {plainLine(card.blockerQuestion)}
          </span>
        </span>
      )}
      {card.ask && openAsk(card) ? (
        <AskLine ask={card.ask} />
      ) : p === null ? (
        <HolderLine holder={card.holder} />
      ) : (
        <ProgressBar
          progress={card.progress}
          label={`${p ?? 0}% complete`}
          tone={cardTone(card)}
        />
      )}
      <span className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
        {p !== null && (
          <>
            <span className="tabular-nums">{p}%</span>
            <span aria-hidden>·</span>
          </>
        )}
        {card.working.length > 0 ? (
          <>
            <Faces who={card.working} />
            {label && <span>{label}</span>}
          </>
        ) : openAsk(card) ? (
          <span>you asked the room</span>
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
    <Card className="items-center p-10! text-center">
      <LayersIcon className="size-10 text-muted-foreground" aria-hidden />
      <div className="flex max-w-md flex-col gap-2">
        <h2 className="font-medium text-ink">No shared work yet</h2>
        <p className="text-sm text-muted-foreground">
          Agents put a card here when their work should be shared: what they're
          on, how far along it is, who else is watching, and what they need you
          to decide. Plans and backlogs stay in the project's own tracker. Ask
          the room above to give the agents something to start now.
        </p>
      </div>
    </Card>
  )
}
