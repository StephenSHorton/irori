import { LayersIcon, PinIcon, UserIcon } from "lucide-react"
import {
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Loading } from "@/components/ui/loading"
import { TileGridShape } from "@/components/common/loading-shapes"
import {
  busy,
  withLive,
  cards as cardsApi,
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
import { LiveRoster } from "@/lib/thread"
import { plainLine, relativeTime } from "@/lib/irori"
import { TaskState } from "@/lib/models"
import { ownedBy, ownerFilter, type OwnerMember } from "@/lib/owners"
import { presenceLine, type SummaryMember } from "@/lib/boards"
import { stateDot } from "@/lib/devices"
import { cardMatches, matching } from "@/lib/search"
import { spawnLine, type SpawnLine } from "@/lib/spawn-line"
import { cn } from "@/lib/utils"
import {
  AskLine,
  cardTone,
  HolderLine,
  ProgressBar,
  OwnerChip,
  PrChip,
  ScopeBadge,
  SideBadge,
  ShortRef,
  Spinner,
  StatePill,
  cardRef,
  waitTip,
} from "./bits"
import { ConfirmButton } from "@/components/common/confirm-button"
import { ClearDone } from "./clear-done"
import { FACES_MAX, Faces, TalkBubble } from "./faces"

// Filter is a chip: All, Needs a decision, In review (card #191), My
// devices, Closed (done,
// canceled and failed cards, which load on their own: useDoneCards; its
// id stays "done" for old ?filter= links), one device
// ("device:<id>", card #22), or one shared board ("board:<id>", v0.5 §8).
// Home's board row (card #199) adds "private": the cards on no board.
export type Filter =
  | "all"
  | "decide"
  | "review"
  | "mine"
  | "done"
  | "private"
  | `device:${string}`
  | `board:${string}`
  | `owner:${string}`

export const filters: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "decide", label: "Needs a decision" },
  { id: "review", label: "In review" },
  { id: "mine", label: "My devices" },
  { id: "done", label: "Closed" },
]

// asFilter is a filter id from outside (the preview's ?filter=), else all.
export function asFilter(id: string | null): Filter {
  if (id?.startsWith("board:") || id?.startsWith("owner:")) return id as Filter
  if (id === "private") return "private"
  if (id === "closed") return "done"
  return filters.find((f) => f.id === id)?.id ?? "all"
}

// shownCards is the grid under filter f and the search box's query (card
// #171). A subtask that matches brings its parent's tile along, before it,
// so it is seen in context; context holds the ids shown only for that.
export function shownCards(
  cards: CardSummary[],
  f: Filter,
  query = ""
): { shown: CardSummary[]; context: Set<string> } {
  const keep = new Set(
    cards.filter((c) => matches(c, f) && cardMatches(c, query)).map((c) => c.id)
  )
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
  if (f === "review") return c.state === TaskState.InReview
  if (f === "mine") return c.scope === "mine"
  if (f === "private") return c.scope !== "board"
  if (f.startsWith("device:")) return c.devices.includes(f.slice(7))
  if (f.startsWith("board:")) return c.board === f.slice(6)
  if (f.startsWith("owner:")) return ownedBy(c, f)
  return true
}

// FilterBar is the chips over the grid's tabs; the device and board chips
// sit beside it (DeviceBar, BoardBar).
export function FilterBar({
  filter,
  onFilter,
  decide,
  review = 0,
  only,
}: {
  filter: Filter
  onFilter: (f: Filter) => void
  decide: number
  // review: the open cards in review (card #191); its chip shows only
  // while there are some, or while it is the filter.
  review?: number
  // only, when set, are the tabs to show (an empty board: All and Done).
  only?: Filter[]
}) {
  return (
    <div
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Filter cards"
    >
      {filters
        .filter((f) => !only || only.includes(f.id))
        .filter((f) => f.id !== "review" || review > 0 || filter === "review")
        .map((f) => (
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
            {f.id === "review" && review > 0 && (
              <span className="text-xs tabular-nums opacity-80">{review}</span>
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

// BoardBar is the board switcher (v0.5 §8): one chip per shared board, its
// name, narrowing the grid to that board's cards. "All" includes every
// board. On Home (card #199) withPrivate adds Private first: the cards on
// no board.
export function BoardBar({
  filter,
  onFilter,
  boards,
  withPrivate = false,
}: {
  filter: Filter
  onFilter: (f: Filter) => void
  boards: { boardId: string; name: string; tag: string }[]
  withPrivate?: boolean
}) {
  if (!boards.length) return null
  return (
    <div
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Filter cards by space"
    >
      {withPrivate && (
        <Button
          size="sm"
          aria-pressed={filter === "private"}
          variant={filter === "private" ? "secondary" : "ghost"}
          onClick={() => onFilter(filter === "private" ? "all" : "private")}
          title="Your private cards: in no shared space"
        >
          Private
        </Button>
      )}
      {boards.map((b) => {
        const id: Filter = `board:${b.boardId}`
        return (
          <Button
            key={id}
            size="sm"
            aria-pressed={filter === id}
            variant={filter === id ? "secondary" : "ghost"}
            onClick={() => onFilter(filter === id ? "all" : id)}
            title={
              b.tag
                ? `Cards in the space ${plainLine(b.name)} (${plainLine(b.tag)}#…)`
                : `Cards in the space ${plainLine(b.name)}`
            }
          >
            <LayersIcon aria-hidden className="size-3.5 opacity-70" />
            {plainLine(b.name)}
          </Button>
        )
      })}
    </div>
  )
}

// MemberBar narrows a shared board's page to one member's cards (card
// #268): Mine first, then each other member by name. Only a board other
// people are on has it.
//
// Each chip carries the member's presence (card #371): a dot (green
// online, amber seen lately, grey offline) and, on hover, when they were
// last seen and their agents on the board.
export function MemberBar({
  filter,
  onFilter,
  members,
  now,
}: {
  filter: Filter
  onFilter: (f: Filter) => void
  members: (OwnerMember &
    Partial<
      Pick<SummaryMember, "presence" | "lastSeen" | "working" | "free">
    >)[]
  now: Date
}) {
  if (members.length < 2) return null
  const ordered = [...members].sort((a, b) => Number(b.me) - Number(a.me))
  return (
    <div
      className="flex flex-wrap items-center gap-1"
      role="group"
      aria-label="Filter cards by member"
    >
      {ordered.map((m) => {
        const id = ownerFilter(m.personId)
        const name = plainLine(m.name) || "Someone"
        return (
          <Button
            key={id}
            size="sm"
            aria-pressed={filter === id}
            variant={filter === id ? "secondary" : "ghost"}
            onClick={() => onFilter(filter === id ? "all" : id)}
            title={
              (m.me
                ? "Your cards: made by you or your agents"
                : `${name}'s cards: made by them or their agents`) +
              (m.presence
                ? `\n${m.me ? "You" : name}: ${presenceLine(withPresence(m), now)}`
                : "")
            }
          >
            <span className="relative inline-flex">
              <UserIcon aria-hidden className="size-3.5 opacity-70" />
              {m.presence && (
                <span
                  aria-label={presenceLine(withPresence(m), now)}
                  className={cn(
                    "absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full ring-1 ring-background",
                    stateDot(m.presence === "online" ? "connected" : m.presence)
                  )}
                />
              )}
            </span>
            {m.me ? "Mine" : name}
          </Button>
        )
      })}
    </div>
  )
}

// withPresence fills the counts presenceLine reads, for a member from a
// summary that has none.
function withPresence(
  m: OwnerMember &
    Partial<Pick<SummaryMember, "presence" | "lastSeen" | "working" | "free">>
): SummaryMember {
  return {
    ...m,
    presence: m.presence ?? "",
    working: m.working ?? 0,
    free: m.free ?? 0,
  }
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
  picked,
}: {
  cards: CardSummary[]
  selected: string
  // picked: the cards ticked in select mode (card #138), marked like the
  // selected one.
  picked?: Set<string>
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
  const tile = (c: CardSummary) => (
    <div
      key={c.id}
      className={cn("grid", dragging === c.id && "opacity-50")}
      draggable={!!onMove && !!c.pinned}
      onDragStart={
        onMove
          ? (e) => {
              setDragging(c.id)
              e.dataTransfer.setData("text/plain", c.id)
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
        selected={picked ? picked.has(c.id) : c.id === selected}
        onSelect={() => onSelect(c.id)}
        now={now}
        dim={context?.has(c.id) ?? false}
      />
    </div>
  )
  // Masonry (card #113): each column stacks on its own, so a family's
  // subtask tray only lengthens its column; cards go, in order, into
  // whichever column is shortest. A family is its parent's tile with its
  // subtasks as rows in a tray under it.
  return (
    <MasonryGrid
      label={label}
      items={familyGroups(cards).map((f) => ({
        key: f.head.id,
        node:
          f.kids.length === 0 ? (
            tile(f.head)
          ) : (
            <div
              role="group"
              aria-label={`${cardRef(f.head)} and its subtasks`}
              className="grid"
            >
              {tile(f.head)}
              <div className="mx-2 grid overflow-hidden rounded-b-lg border border-t-0 border-border bg-muted/30">
                {f.kids.map((k) => (
                  <SubtaskDetailRow
                    key={k.id}
                    card={k}
                    selected={picked ? picked.has(k.id) : k.id === selected}
                    dim={context?.has(k.id) ?? false}
                    onSelect={() => onSelect(k.id)}
                  />
                ))}
                {f.head.progress.done > 0 && (
                  <span className="flex items-center gap-2 px-3 py-1.5 text-[11px] text-muted-foreground">
                    <span aria-hidden className="text-success">
                      ✓
                    </span>
                    {f.head.progress.done} done
                  </span>
                )}
              </div>
            </div>
          ),
      }))}
    />
  )
}

// familyGroups groups the grid's cards: each subtask whose parent is in the
// list joins its parent, in list order, at the parent's place; any other
// card is a family of its own.
export function familyGroups(
  cards: CardSummary[]
): { head: CardSummary; kids: CardSummary[] }[] {
  const ids = new Set(cards.map((c) => c.id))
  const kids = new Map<string, CardSummary[]>()
  for (const c of cards) {
    const p = c.parent?.id
    if (p && p !== c.id && ids.has(p)) kids.set(p, [...(kids.get(p) ?? []), c])
  }
  return cards
    .filter(
      (c) => !(c.parent?.id && c.parent.id !== c.id && ids.has(c.parent.id))
    )
    .map((c) => ({ head: c, kids: kids.get(c.id) ?? [] }))
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
  const live = useContext(LiveRoster)
  const p = percent(card.progress)
  const closed = over(card)
  const parent = (card.subtasks ?? 0) > 0
  const blocked = card.openBlockers > 0
  const title = plainLine(card.title) || "(untitled)"
  const label = workLabel(withLive(card, live))
  return (
    <button
      type="button"
      data-card={card.id}
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex min-w-0 flex-col gap-2.5 p-3.5 text-left text-sm glass-panel outline-none",
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
      <span className="flex min-w-0 flex-wrap items-center gap-1.5">
        <ShortRef card={card} />
        {card.pinned && (
          <PinIcon className="size-3 text-primary" aria-label="Pinned" />
        )}
        {card.parent && <PartOf parent={card.parent} />}
        <StatePill card={card} />
        <ScopeBadge card={card} />
        <SideBadge card={card} />
        <OwnerChip card={card} />
        <PrChip card={card} />
        {card.working.some((w) => w.conflict) && (
          <Badge
            variant="warning"
            title="Two devices leased the same thing while apart"
          >
            Lease conflict
          </Badge>
        )}
        {!closed && !openAsk(card) && nobodyOn(card) && !card.starting && (
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
        {busy(withLive(card, live)) && <Spinner className="ml-auto" />}
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
        <span
          className={cn(
            "flex gap-1.5 text-xs",
            // Ready for you (card #112) is good news: green, not red.
            card.ready && card.openBlockers === 1
              ? "text-success"
              : "text-destructive"
          )}
        >
          <span aria-hidden>●</span>
          <span className="line-clamp-2">
            {plainLine(card.blockerQuestion)}
          </span>
        </span>
      )}
      {/* Where its holder left off (card #137), dim: a log, not a
          question. */}
      {card.leftOff && !blocked && (
        <span
          className="line-clamp-1 text-xs text-muted-foreground"
          title={plainLine(card.leftOff.text)}
        >
          Left off: {plainLine(card.leftOff.text)}
        </span>
      )}
      {!blocked && !!card.waitingOn && (
        <span className="flex gap-1.5 text-xs text-muted-foreground">
          <span aria-hidden>●</span>
          <span className="line-clamp-2">{waitTip(card)}</span>
        </span>
      )}
      {!blocked && card.subtaskBlocked && (
        <span className="flex gap-1.5 text-xs text-warning">
          <span aria-hidden>●</span>
          <span className="line-clamp-2">{plainLine(card.subtaskBlocked)}</span>
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
            <Faces who={card.working} max={FACES_MAX} />
            {label && <span>{label}</span>}
          </>
        ) : closed ? null : spawnLine(card) ? (
          <SpawnText line={spawnLine(card)!} />
        ) : openAsk(card) ? (
          <span>you asked the room</span>
        ) : card.returned ? (
          <span>nobody on it: {displayName(card.returned)} went away</span>
        ) : (
          <span>nobody on it yet</span>
        )}
        {!closed && card.watching.length > 0 && (
          <>
            <span aria-hidden>·</span>
            <Faces who={card.watching} watching max={FACES_MAX} />
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

// Outcome narrows the Closed filter (card #57).
export type Outcome = "any" | "completed" | "canceled" | "failed"

export const outcomes: { id: Outcome; label: string }[] = [
  { id: "any", label: "Any outcome" },
  { id: "completed", label: "Done" },
  { id: "canceled", label: "Canceled" },
  { id: "failed", label: "Failed" },
]

// ofOutcome keeps the closed cards that ended as o.
export function ofOutcome(cs: CardSummary[], o: Outcome): CardSummary[] {
  return o === "any" ? cs : cs.filter((c) => c.state === o)
}

// DoneGrid is the Closed filter: done, canceled and failed cards, the
// latest first, a page at a time, each with its outcome badge, narrowed
// by outcome.
// DoneGrid is the Closed tab. With a search query (card #171) it shows only
// the loaded closed cards that match.
export function DoneGrid(props: DoneProps) {
  const { done } = props
  // The first read draws the tab's shape (card #598); a failed one ends
  // loading, so DoneBody shows the error. The grid is the board column's
  // gap, which DoneBody's rows had from their parent.
  const first = done.loading && !done.cards.length
  return (
    <Loading
      loading={first}
      label="Loading the closed cards"
      className="min-w-0"
    >
      <div className="grid min-w-0 gap-3.5">
        {first ? <DoneShape /> : <DoneBody {...props} />}
      </div>
    </Loading>
  )
}

type DoneProps = {
  done: DoneState
  query?: string
  selected: string
  onSelect: (id: string) => void
  now: Date
  // byAge: offer clearing by age here, within this scope ("" all, "own"
  // Home's, else a board id; card #138).
  byAge?: string
}

// DoneBody is the Closed tab once its first read is in.
function DoneBody({
  done: all,
  query = "",
  selected,
  onSelect,
  now,
  byAge,
}: DoneProps) {
  const done = { ...all, cards: matching(all.cards, query) }
  const [outcome, setOutcome] = useState<Outcome>("any")
  // Select mode (card #138): clicks tick cards to clear or delete.
  const [picking, setPicking] = useState(false)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const pick = (id: string) =>
    setPicked((p) => {
      const n = new Set(p)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })
  const stopPicking = () => {
    setPicking(false)
    setPicked(new Set())
  }
  if (done.error && !done.cards.length)
    return (
      <p className="px-1 text-[12.5px] text-destructive" role="alert">
        Could not load the closed cards: {done.error}
      </p>
    )
  if (!done.cards.length && !all.cards.length)
    return (
      <p className="px-1 text-[12.5px] text-muted-foreground">
        Nothing closed yet.
      </p>
    )
  if (!done.cards.length)
    return (
      <>
        <p className="px-1 text-[12.5px] text-muted-foreground">
          No cards match
          {done.more ? " among these; Show more looks further back." : "."}
        </p>
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
  const shown = ofOutcome(done.cards, outcome)
  return (
    <>
      <div
        className="flex flex-wrap items-center gap-1"
        role="group"
        aria-label="Filter closed cards by outcome"
      >
        {outcomes.map((o) => (
          <Button
            key={o.id}
            size="xs"
            aria-pressed={outcome === o.id}
            variant={outcome === o.id ? "secondary" : "ghost"}
            onClick={() => setOutcome(o.id)}
          >
            {o.label}
          </Button>
        ))}
        <span className="ml-auto flex items-center gap-2">
          {byAge !== undefined && !picking && (
            <ClearDone compact scope={byAge} onCleared={done.reload} />
          )}
          <Button
            size="xs"
            variant={picking ? "secondary" : "outline"}
            aria-pressed={picking}
            onClick={() => (picking ? stopPicking() : setPicking(true))}
          >
            {picking ? "Done selecting" : "Select"}
          </Button>
        </span>
      </div>
      {picking && (
        <PickBar
          picked={done.cards.filter((c) => picked.has(c.id))}
          onAll={() => setPicked(new Set(shown.map((c) => c.id)))}
          onNone={() => setPicked(new Set())}
          onDone={() => {
            setPicked(new Set())
            done.reload()
          }}
        />
      )}
      {done.error && (
        <p className="px-1 text-[12.5px] text-destructive" role="alert">
          Could not reload the closed cards: {done.error}
        </p>
      )}
      {!shown.length && (
        <p className="px-1 text-[12.5px] text-muted-foreground">
          None {outcome === "completed" ? "done" : outcome} among these
          {done.more ? "; Show more looks further back." : "."}
        </p>
      )}
      <CardGrid
        cards={shown}
        selected={selected}
        onSelect={picking ? pick : onSelect}
        picked={picking ? picked : undefined}
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

// DoneShape is the Closed tab's skeleton: the outcome row, then tiles.
function DoneShape() {
  return (
    <>
      <div className="flex items-center gap-1">
        {outcomes.map((o) => (
          <span key={o.id} className="block h-6 w-20 rounded-md" />
        ))}
        <span className="ml-auto block h-6 w-14 rounded-md" />
      </div>
      <TileGridShape tiles={3} />
    </>
  )
}

// PickBar acts on the closed cards ticked in select mode (card #138):
// clear them from the lists, or delete them for good (your own cards
// only: a shared board's card is withdrawn by its board, not deleted).
function PickBar({
  picked,
  onAll,
  onNone,
  onDone,
}: {
  picked: CardSummary[]
  onAll: () => void
  onNone: () => void
  onDone: () => void
}) {
  const n = picked.length
  const shared = picked.some((c) => c.scope === "board")
  const what = `${n} ${n === 1 ? "card" : "cards"}`
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-2 text-[12.5px] glass-panel">
      <span className="text-ink tabular-nums">
        {n ? `${what} selected` : "Click cards to select them"}
      </span>
      <Button size="xs" variant="ghost" onClick={onAll}>
        Select all shown
      </Button>
      {n > 0 && (
        <Button size="xs" variant="ghost" onClick={onNone}>
          Clear selection
        </Button>
      )}
      <span className="ml-auto flex items-start gap-2">
        <ConfirmButton
          label={n ? `Clear ${n}…` : "Clear…"}
          disabled={!n}
          confirmLabel={`Clear ${what}`}
          question={
            <>
              Clear {what}? They leave the app's lists. Agents still find them
              with search and show. Nothing is deleted.
            </>
          }
          onConfirm={async () => {
            await cardsApi.clearDone(
              0,
              picked.map((c) => c.id)
            )
            onDone()
          }}
        />
        <ConfirmButton
          label="Delete permanently…"
          variant="destructive"
          disabled={!n || shared}
          confirmLabel={`Delete ${what}`}
          question={
            <>
              Delete {what} permanently? They leave Irori on every device, and
              agents no longer find them with search or show. This can't be
              undone.
              {shared && " Cards in a shared space can't be deleted here."}
            </>
          }
          onConfirm={async () => {
            for (const c of picked) await cardsApi.remove(c.id)
            onDone()
          }}
        />
      </span>
    </div>
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
function PartOf({
  parent,
}: {
  parent: { short: string; tag?: string; title: string }
}) {
  return (
    <span
      className="font-mono text-[11px] text-muted-foreground"
      title={`A subtask of ${cardRef(parent)} ${plainLine(parent.title)}`}
    >
      part of {cardRef(parent)}
    </span>
  )
}

// SubtaskDetailRow is a subtask row with detail (card #113):
// line one its ref, title and state; line two who's on it and how far;
// a blocker's question under that.
function SubtaskDetailRow({
  card,
  selected,
  dim,
  onSelect,
}: {
  card: CardSummary
  selected: boolean
  dim: boolean
  onSelect: () => void
}) {
  const blocked = card.openBlockers > 0
  const p = percent(card.progress)
  const who = card.working
  const tone = blocked
    ? "text-destructive"
    : who.length
      ? "text-primary"
      : "text-warning"
  const whoLine = who.length ? (
    <span className="flex min-w-0 items-center gap-1.5">
      <Faces who={who} />
      <span className="truncate">{displayName(who[0])}</span>
    </span>
  ) : spawnLine(card) ? (
    <SpawnText line={spawnLine(card)!} />
  ) : (
    <span className="text-warning">nobody on it</span>
  )
  const bar =
    p !== null ? (
      <span className="flex items-center gap-1.5">
        <span className="h-1 w-14 overflow-hidden rounded-full bg-border">
          <span
            className={cn(
              "block h-full",
              blocked ? "bg-destructive" : "bg-primary"
            )}
            style={{ width: `${p}%` }}
          />
        </span>
        <span className="tabular-nums">{p}%</span>
      </span>
    ) : null
  return (
    <button
      type="button"
      data-card={card.id}
      onClick={onSelect}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "grid gap-1 border-b border-border/60 bg-background/40 px-3 py-2 text-left text-xs hover:bg-muted/60",
        selected && "bg-muted/70",
        dim && "opacity-60"
      )}
    >
      <>
        <span className="flex min-w-0 items-center gap-2">
          <span aria-hidden className={cn("text-[10px]", tone)}>
            ●
          </span>
          <ShortRef card={card} />
          <span className="min-w-0 flex-1 truncate text-ink">
            {plainLine(card.title)}
          </span>
          <span className="flex max-w-[50%] min-w-0">
            <StatePill card={card} />
          </span>
        </span>
        <span className="flex items-center gap-3 pl-4 text-[11px] text-muted-foreground">
          {whoLine}
          {bar}
        </span>
      </>
      {blocked && card.blockerQuestion && (
        <span className={"line-clamp-2 pl-4 text-[11px] text-destructive"}>
          {plainLine(card.blockerQuestion)}
        </span>
      )}
      {!blocked && !!card.waitingOn && (
        <span className="line-clamp-2 pl-4 text-[11px] text-muted-foreground">
          {waitTip(card)}
        </span>
      )}
    </button>
  )
}

// MasonryGrid lays items into as many 260px+ columns as fit; each item
// goes, in order, to the shortest column so far (card #113).
function MasonryGrid({
  label,
  items,
}: {
  label: string
  items: { key: string; node: ReactNode }[]
}) {
  const box = useRef<HTMLElement>(null)
  const refs = useRef(new Map<string, HTMLDivElement>())
  const [width, setWidth] = useState(0)
  const [heights, setHeights] = useState<Record<string, number>>({})
  const gap = 12
  // Re-measured when the set of items changes, so new ones are observed.
  const keys = items.map((it) => it.key).join(" ")
  useLayoutEffect(() => {
    const el = box.current
    if (!el) return
    const measure = () => {
      setWidth(el.clientWidth)
      const h: Record<string, number> = {}
      for (const [k, d] of refs.current) h[k] = d.offsetHeight
      setHeights((old) => (JSON.stringify(old) === JSON.stringify(h) ? old : h))
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    for (const d of refs.current.values()) ro.observe(d)
    return () => ro.disconnect()
  }, [keys])
  const n = Math.max(1, Math.floor((width + gap) / (260 + gap)))
  const cols: { key: string; node: ReactNode }[][] = Array.from(
    { length: n },
    () => []
  )
  const tall = new Array(n).fill(0)
  for (const it of items) {
    let c = 0
    for (let i = 1; i < n; i++) if (tall[i] < tall[c]) c = i
    cols[c].push(it)
    tall[c] += (heights[it.key] ?? 200) + gap
  }
  return (
    <section
      ref={box}
      aria-label={label}
      className="grid gap-3"
      style={{ gridTemplateColumns: `repeat(${n}, minmax(0, 1fr))` }}
    >
      {cols.map((col, i) => (
        <div key={i} className="grid content-start gap-3">
          {col.map((it) => (
            <div
              key={it.key}
              ref={(d) => {
                if (d) refs.current.set(it.key, d)
                else refs.current.delete(it.key)
              }}
              className="grid"
            >
              {it.node}
            </div>
          ))}
        </div>
      ))}
    </section>
  )
}

// SpawnText is a card's spawn state where "nobody on it" would be (card
// #448): starting an agent, waiting on a device's cap, or can't start.
function SpawnText({ line }: { line: SpawnLine }) {
  return (
    <span className={line.tone === "warn" ? "text-warning" : undefined}>
      {line.text}
    </span>
  )
}
