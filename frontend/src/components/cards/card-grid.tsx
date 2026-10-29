import { LayersIcon, PinIcon } from "lucide-react"
import { useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import {
  busy,
  deviceVar,
  type DeviceChip,
  displayName,
  holderLineShows,
  nobodyOn,
  openAsk,
  over,
  percent,
  workLabel,
  type CardSummary,
  type DoneState,
} from "@/lib/cards"
import { plainLine, relativeTime } from "@/lib/irori"
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
import { Faces, TalkBubble } from "./faces"

// Filter is a chip: All, Needs a decision, My devices, Done (the closed
// cards, which load on their own: useDoneCards), or one device
// ("device:<id>", card #22).
export type Filter = "all" | "decide" | "mine" | "done" | `device:${string}`

export const filters: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "decide", label: "Needs a decision" },
  { id: "mine", label: "My devices" },
  { id: "done", label: "Done" },
]

// asFilter is a filter id from outside (the preview's ?filter=), else all.
export function asFilter(id: string | null): Filter {
  return filters.find((f) => f.id === id)?.id ?? "all"
}

// shownCards is the grid under filter f. A subtask that matches brings its
// parent's tile along, before it, so it is seen in context; context holds
// the ids shown only for that.
export function shownCards(
  cards: CardSummary[],
  f: Filter
): { shown: CardSummary[]; context: Set<string> } {
  const keep = new Set(cards.filter((c) => matches(c, f)).map((c) => c.id))
  const context = new Set<string>()
  for (const c of cards) {
    const parent = c.parent?.id
    if (parent && keep.has(c.id) && !keep.has(parent)) context.add(parent)
  }
  return {
    shown: cards.filter((c) => keep.has(c.id) || context.has(c.id)),
    context,
  }
}

export function matches(c: CardSummary, f: Filter): boolean {
  if (f === "done") return over(c)
  if (over(c)) return false
  if (f === "decide") return c.openBlockers > 0
  if (f === "mine") return c.scope === "mine"
  if (f.startsWith("device:")) return c.devices.includes(f.slice(7))
  return true
}

// FilterBar is the chips over the grid. Shared boards join it in v0.4.
export function FilterBar({
  filter,
  onFilter,
  decide,
  only,
}: {
  filter: Filter
  onFilter: (f: Filter) => void
  decide: number
  // only, when set, are the tabs to show (an empty board: All and Done).
  only?: Filter[]
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Filter cards"
    >
      {filters.filter((f) => !only || only.includes(f.id)).map((f) => (
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

// DeviceBar is a chip per device, to narrow the grid to one device's cards
// (card #22). It is its own row of chips, apart from FilterBar's tabs, so
// other chip rows (shared boards) can sit beside the tabs too.
export function DeviceBar({
  filter,
  onFilter,
  devices,
}: {
  filter: Filter
  onFilter: (f: Filter) => void
  devices: DeviceChip[]
}) {
  if (!devices.length) return null
  return (
    <div
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Filter cards by device"
    >
      {devices.map((d) => {
        const id: Filter = `device:${d.id}`
        return (
          <Button
            key={id}
            size="sm"
            aria-pressed={filter === id}
            variant={filter === id ? "secondary" : "ghost"}
            onClick={() => onFilter(filter === id ? "all" : id)}
            title={`Cards its agents are on: ${d.name}`}
          >
            <span
              aria-hidden
              className="inline-block size-2 rounded-full"
              style={{ background: deviceVar(d.color) }}
            />
            {d.name}
          </Button>
        )
      })}
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
  now,
  context,
  label = "Shared work",
  onMove,
}: {
  cards: CardSummary[]
  selected: string
  onSelect: (id: string) => void
  now?: Date
  // context: cards shown only because a subtask of theirs matched the
  // filter; they show dimmed.
  context?: Set<string>
  label?: string
  // onMove, when set, makes the tiles draggable: dropping card id on
  // another puts it in that card's place (before it when it came from
  // later in the list, after it when from earlier). before is the card it
  // now goes in front of, "" for last.
  onMove?: (id: string, before: string) => void
}) {
  const [dragging, setDragging] = useState("")
  // Only pinned cards move; a pinned parent's subtasks ride along.
  const pins = cards.filter((c) => c.pinned)
  const drop = (target: string) => {
    const from = pins.findIndex((c) => c.id === dragging)
    const to = pins.findIndex((c) => c.id === target)
    setDragging("")
    if (!onMove || from < 0 || to < 0 || from === to) return
    // Moving down lands after the target: in front of the card after it.
    const before = from < to ? (pins[to + 1]?.id ?? "") : target
    onMove(dragging, before)
  }
  return (
    <section
      aria-label={label}
      className="grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-3"
    >
      {cards.map((c) => (
        <div
          key={c.id}
          className={cn("grid", dragging === c.id && "opacity-50")}
          draggable={!!onMove && !!c.pinned}
          onDragStart={
            onMove
              ? (e) => {
                  setDragging(c.id)
                  e.dataTransfer.effectAllowed = "move"
                }
              : undefined
          }
          onDragEnd={onMove ? () => setDragging("") : undefined}
          onDragOver={
            onMove && dragging && c.pinned
              ? (e) => {
                  e.preventDefault()
                  e.dataTransfer.dropEffect = "move"
                }
              : undefined
          }
          onDrop={
            onMove && dragging && c.pinned
              ? (e) => {
                  e.preventDefault()
                  drop(c.id)
                }
              : undefined
          }
        >
          <CardTile
            card={c}
            selected={c.id === selected}
            onSelect={() => onSelect(c.id)}
            now={now}
            dim={context?.has(c.id) ?? false}
          />
        </div>
      ))}
    </section>
  )
}

function CardTile({
  card,
  selected,
  onSelect,
  now,
  dim,
}: {
  card: CardSummary
  selected: boolean
  onSelect: () => void
  now?: Date
  dim: boolean
}) {
  const p = percent(card.progress)
  const closed = over(card)
  const parent = (card.subtasks ?? 0) > 0
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
          "border-primary! shadow-[0_0_0_1px_var(--color-ring),0_8px_30px_oklch(0_0_0/0.45)]",
        // The accent edge is 2px wider than the border: take it off the
        // padding so the tile's contents don't move.
        card.accent && "pl-3",
        dim && "opacity-60"
      )}
      // A card everyone on is on one device wears its colour on the left
      // edge (card #22); the status colours stay where they mean something.
      style={
        card.accent
          ? {
              borderLeft: `3px solid ${deviceVar(card.accent)}`,
            }
          : undefined
      }
    >
      <span className="flex flex-wrap items-center gap-1.5">
        <ShortRef card={card} />
        {card.pinned && (
          <PinIcon
            className="size-3 text-primary"
            aria-label="Pinned"
          />
        )}
        {card.parent && <PartOf parent={card.parent} />}
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
        {!closed && !openAsk(card) && nobodyOn(card) && (
          <Badge
            variant="warning"
            title="The agent on it went away; nobody is working it"
          >
            Nobody on it
          </Badge>
        )}
        {card.stale && (
          <Badge
            variant="secondary"
            title="Nothing has happened on it for a while, and nobody is busy on it"
          >
            Stale
          </Badge>
        )}
        {busy(card) && <Spinner className="ml-auto" />}
      </span>
      {card.talking.length > 1 && (
        <span className="-mt-1 flex">
          <TalkBubble who={card.talking} />
        </span>
      )}
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
      {!blocked && card.subtaskBlocked && (
        <span className="flex gap-1.5 text-xs text-warning">
          <span aria-hidden>●</span>
          <span className="line-clamp-2">
            {plainLine(card.subtaskBlocked)}
          </span>
        </span>
      )}
      {card.ask && (openAsk(card) || closed) ? (
        <AskLine ask={card.ask} />
      ) : closed && p === null ? null : p === null ? (
        <HolderLine holder={card.holder} returned={card.returned} />
      ) : (
        <>
          <ProgressBar
            progress={card.progress}
            label={
              parent
                ? `${card.progress.done} of ${card.progress.total} subtasks done`
                : `${p ?? 0}% complete`
            }
            tone={cardTone(card)}
          />
          {!closed && holderLineShows(card) && (
            <HolderLine holder={card.holder} returned={card.returned} />
          )}
        </>
      )}
      <span className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
        {p !== null && (
          <>
            <span className="tabular-nums">
              {parent
                ? `${card.progress.done}/${card.progress.total} subtasks`
                : `${p}%`}
            </span>
            <span aria-hidden>·</span>
          </>
        )}
        {card.working.length > 0 ? (
          <>
            <Faces who={card.working} />
            {label && <span>{label}</span>}
          </>
        ) : closed ? null : openAsk(card) ? (
          <span>you asked the room</span>
        ) : card.returned ? (
          <span>nobody on it: {displayName(card.returned)} went away</span>
        ) : (
          <span>nobody on it yet</span>
        )}
        {!closed && card.watching.length > 0 && (
          <>
            <span aria-hidden>·</span>
            <Faces who={card.watching} watching />
            <span>watching</span>
          </>
        )}
        {closed && now && (
          // The card's last change, which is most often its close but
          // not always (a link added after), so it says "updated".
          <span
            className="ml-auto tabular-nums"
            title={`Last changed ${new Date(card.updatedAt as string).toLocaleString()}`}
          >
            updated {relativeTime(new Date(card.updatedAt as string), now)}
          </span>
        )}
      </span>
    </button>
  )
}

// DoneGrid is the Done filter: closed cards, the latest first, a page at a
// time.
export function DoneGrid({
  done,
  selected,
  onSelect,
  now,
}: {
  done: DoneState
  selected: string
  onSelect: (id: string) => void
  now: Date
}) {
  if (done.error && !done.cards.length)
    return (
      <p className="px-1 text-[12.5px] text-destructive" role="alert">
        Could not load the done cards: {done.error}
      </p>
    )
  if (done.loading && !done.cards.length)
    return <p className="px-1 text-sm text-muted-foreground">Loading…</p>
  if (!done.cards.length)
    return (
      <p className="px-1 text-[12.5px] text-muted-foreground">
        Nothing done yet.
      </p>
    )
  return (
    <>
      {done.error && (
        <p className="px-1 text-[12.5px] text-destructive" role="alert">
          Could not reload the done cards: {done.error}
        </p>
      )}
      <CardGrid
        cards={done.cards}
        selected={selected}
        onSelect={onSelect}
        now={now}
      />
      {done.more && (
        <Button
          size="sm"
          variant="ghost"
          className="self-center"
          onClick={done.loadMore}
        >
          Show more
        </Button>
      )}
    </>
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

// PartOf is a subtask's "part of #12" chip, naming its parent card.
function PartOf({ parent }: { parent: { short: string; title: string } }) {
  return (
    <span
      className="font-mono text-[11px] text-muted-foreground"
      title={`A subtask of #${plainLine(parent.short)} ${plainLine(parent.title)}`}
    >
      part of #{plainLine(parent.short)}
    </span>
  )
}
