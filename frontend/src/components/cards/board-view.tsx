// BoardView is one board's cards as a page (card #161): the header box,
// the filter tabs and device chips, the Threads panel, Pinned, the grid
// or canvas, and Closed. Home is BoardView over your own cards; a shared
// board's page is BoardView over that board's, beside its People panel.
// Each feature lands here once and shows on both.
//
// The filter and view live in App (they survive a card's page and a
// notification's Open); the cards are every live card, and BoardView
// keeps the scope's; Closed comes in already scoped (useDoneCards).

import { OrderPanel } from "@/components/cards/order"
import { laterOf } from "@/lib/later"
import { useState, type ReactNode } from "react"

import {
  BoardBar,
  CardGrid,
  DeviceBar,
  DoneGrid,
  FilterBar,
  MemberBar,
  asFilter,
  matches,
  shownCards,
  type Filter,
} from "@/components/cards/card-grid"
import { SearchBox } from "@/components/cards/search-box"
import { ProgressRows } from "@/components/cards/progress-rows"
import { ThreadsPanel } from "@/components/cards/threads-panel"
import {
  BoardCanvas,
  ViewSwitch,
  type BoardView as View,
} from "@/components/canvas/switch"
import { Badge } from "@/components/ui/badge"
import { useBoards } from "@/lib/boards"
import { useNarrow } from "@/lib/media"
import { TaskState } from "@/lib/models"
import { Button } from "@/components/ui/button"
import {
  cards as cardsApi,
  deviceChips,
  type CardSummary,
  type DoneState,
  type ThreadSummary,
} from "@/lib/cards"

// ScopeOwn is Home's scope: your own cards. Any other scope is a board id.
export const ScopeOwn = "own"

// inScope says whether card c belongs on scope's page. Home (card #199)
// is your private cards and every shared board's card one of your agents
// holds or watches (ours).
export function inScope(c: CardSummary, scope: string): boolean {
  return scope === ScopeOwn
    ? c.scope !== "board" || !!c.ours
    : c.board === scope
}

// homeBoards is Home's board row (card #199): each board with a card on
// Home, in the order they first appear.
export function homeBoards(
  cards: CardSummary[]
): { boardId: string; name: string; tag: string }[] {
  const out: { boardId: string; name: string; tag: string }[] = []
  for (const c of cards) {
    if (
      c.scope !== "board" ||
      !c.board ||
      out.some((b) => b.boardId === c.board)
    )
      continue
    out.push({ boardId: c.board, name: c.scopeLabel, tag: c.tag ?? "" })
  }
  return out
}

// activeFilter is filter as the page can show it: a device chip whose
// device has no cards left, or Mine fall back to All. A board filter and
// Private are Home's (boards: its board row, card #199): kept while that
// board has cards there; elsewhere they fall back to All.
export function activeFilter(
  filter: Filter,
  chips: { id: string }[],
  boards?: { boardId: string }[],
  members?: { personId: string }[]
): Filter {
  // A member chip (card #268) holds only on a board that member is on.
  if (filter.startsWith("owner:"))
    return (members ?? []).length > 1 &&
      members!.some((m) => filter === `owner:${m.personId}`)
      ? filter
      : "all"
  if (
    filter.startsWith("device:") &&
    !chips.some((d) => filter === `device:${d.id}`)
  )
    return "all"
  if (filter === "private") return boards ? filter : "all"
  if (filter.startsWith("board:"))
    return boards?.some((b) => filter === `board:${b.boardId}`) ? filter : "all"
  if (filter === "mine") return "all"
  return filter
}

// hiddenByFilter says what a filter or search is hiding (card #237): the
// open cards that All with no search would show and this view doesn't,
// and how many of them wait on a decision. Null when nothing is hidden,
// on All with no search, or on Closed.
export function hiddenByFilter(
  cards: CardSummary[],
  f: Filter,
  query: string
): { hidden: number; decide: number } | null {
  if (f === "done" || (f === "all" && !query.trim())) return null
  const shown = new Set(shownCards(cards, f, query).shown.map((c) => c.id))
  const hidden = cards.filter((c) => matches(c, "all") && !shown.has(c.id))
  if (!hidden.length) return null
  return {
    hidden: hidden.length,
    decide: hidden.filter((c) => c.openBlockers > 0).length,
  }
}

// Home remembers its last board pick per viewer (card #199): All, Private
// or one board, kept in this window's localStorage under homeFilterKey.
// The other tabs (Needs a decision, Closed) are not remembered.
export const homeFilterKey = "irori.homeFilter"

// rememberedFilter says whether f is a pick Home keeps.
export function rememberedFilter(f: Filter): boolean {
  return f === "all" || f === "private" || f.startsWith("board:")
}

// savedHomeFilter is the pick Home starts on from what was saved: a
// remembered pick, else All.
export function savedHomeFilter(saved: string | null): Filter {
  if (!saved) return "all"
  const f = asFilter(saved)
  return rememberedFilter(f) ? f : "all"
}

export type ThreadsProps = {
  list: ThreadSummary[]
  open: string
  onOpen: (id: string) => void
  hidden: boolean
  onHide: (hidden: boolean) => void
  // why says why a waiting thread has no agent yet (card #172).
  why?: string
}

export function BoardView({
  scope,
  cards,
  done,
  filter,
  onFilter,
  view,
  onView,
  query,
  onQuery,
  selected,
  onOpen,
  now,
  header,
  notice,
  empty,
  nothingDecide,
  threads,
  aside,
  lead,
  rows = false,
  listed,
}: {
  scope: string
  cards: CardSummary[]
  done: DoneState
  filter: Filter
  onFilter: (f: Filter) => void
  view: View
  onView: (v: View) => void
  // query is the search box's words (card #171), kept in App like the view.
  query: string
  onQuery: (q: string) => void
  selected: string
  onOpen: (id: string) => void
  now: Date
  // header is the box above the board (Ask about anything…).
  header?: ReactNode
  // notice sits under the filters (Home: a board's decisions waiting).
  notice?: ReactNode
  // empty is what an empty board shows.
  empty: ReactNode
  // nothingDecide is Needs a decision with nothing in it.
  nothingDecide?: string
  threads?: ThreadsProps
  // aside is a right column under the Threads panel (a board's People).
  aside?: ReactNode
  // lead leads the column, above the filters (Home: Needs you, card #261).
  lead?: ReactNode
  // rows: the list is compact progress rows instead of the grid (Home,
  // card #261).
  rows?: boolean
  // listed are the cards lead already shows; All leaves them out of the
  // rows.
  listed?: ReadonlySet<string>
}) {
  const mine = cards.filter((c) => inScope(c, scope))
  const chips = deviceChips(mine)
  // Home's board row (card #199); a board page has none.
  const rowBoards = scope === ScopeOwn ? homeBoards(mine) : undefined
  const { boards } = useBoards()
  // A shared board's page narrows by member (card #268).
  const members =
    scope === ScopeOwn
      ? undefined
      : (boards.find((b) => b.boardId === scope)?.members ?? undefined)
  const active = activeFilter(filter, chips, rowBoards, members)
  const { shown: all, context } = shownCards(mine, active, query)
  // On All with no search, a card the lead lists (Needs you) isn't
  // listed again below it.
  const shown =
    rows && listed && active === "all" && !query.trim()
      ? all.filter((c) => !listed.has(c.id))
      : all
  const hiddenLine = hiddenByFilter(mine, active, query)
  // The Pinned group: pinned cards in their order, each pinned parent with
  // its subtasks (as BoardService.Cards lists them).
  const pinnedGroup = shown.filter(
    (c) =>
      c.pinned ||
      (c.parent && shown.some((p) => p.id === c.parent?.id && p.pinned))
  )
  const inPinned = new Set(pinnedGroup.map((c) => c.id))
  const decide = mine.filter((c) => c.openBlockers > 0).length
  const review = mine.filter((c) => c.state === TaskState.InReview).length

  const openThreads =
    threads?.list.filter((t) => t.state !== "resolved").length ?? 0
  // A phone or a narrow window (card #227): one column, and Threads is a
  // full-screen sheet its button opens (opening a thread opens it too).
  const narrow = useNarrow()
  const openThread = threads?.open ?? ""
  const [sheet, setSheet] = useState(() => narrow && !!openThread)
  const [sheetFor, setSheetFor] = useState(openThread)
  if (sheetFor !== openThread) {
    setSheetFor(openThread)
    if (narrow && openThread) setSheet(true)
  }
  const showThreads =
    !narrow &&
    !!threads &&
    !threads.hidden &&
    (threads.list.length > 0 || !!threads.open)
  // Home asks for the panel (card #261): it starts hidden and opens with
  // a thread, so it isn't a fixed column.
  const threadsButton =
    !!threads &&
    (narrow
      ? threads.list.length > 0 || !!threads.open
      : !showThreads && threads.list.length > 0)
  // The app is exactly the window's height (card #170): beside the Board
  // the Threads column scrolls on its own; without it the Board does.
  const panel = showThreads && threads && (
    <ThreadsPanel
      threads={threads.list}
      open={threads.open}
      onOpen={threads.onOpen}
      onHide={() => threads.onHide(true)}
      now={now}
      why={threads.why}
    />
  )
  // Canvas fills what's left of the window (card #410): the column
  // doesn't scroll, the canvas takes its remaining height and a long lead
  // (Needs you) scrolls in a box of its own. A phone keeps one scrolling
  // column with a set-height canvas.
  const fill = !narrow && view === "canvas" && active !== "done" && mine.length > 0
  const column = fill ? fillColumn : boardColumn
  const withThreads = (main: ReactNode) =>
    narrow ? (
      <div className="-mx-1 grid min-h-0 min-w-0 flex-1 grid-cols-[minmax(0,1fr)] content-start gap-4 overflow-y-auto px-1 pb-10">
        <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-3.5">
          {main}
        </div>
        {aside}
        {sheet && threads && (
          <div className="fixed inset-0 z-40 overflow-y-auto bg-background p-2 pt-[calc(var(--safe-top)+0.5rem)] pb-[calc(var(--safe-bottom)+2.5rem)]">
            <ThreadsPanel
              threads={threads.list}
              open={threads.open}
              onOpen={threads.onOpen}
              onHide={() => setSheet(false)}
              now={now}
              why={threads.why}
            />
          </div>
        )}
      </div>
    ) : aside ? (
      <div className="flex min-h-0 flex-1 flex-col gap-4 lg:flex-row">
        <div className={column}>{main}</div>
        <div className="-mr-1 grid min-h-0 shrink-0 content-start gap-3.5 overflow-y-auto pr-1 pb-4 lg:w-[360px]">
          {panel}
          {aside}
        </div>
      </div>
    ) : showThreads && threads ? (
      <div className="flex min-h-0 flex-1 gap-4">
        <div className={column}>{main}</div>
        <div className="-mr-1 min-h-0 w-[380px] shrink-0 overflow-y-auto pr-1 pb-4">
          <ThreadsPanel
            threads={threads.list}
            open={threads.open}
            onOpen={threads.onOpen}
            onHide={() => threads.onHide(true)}
            now={now}
            why={threads.why}
          />
        </div>
      </div>
    ) : (
      <div className={column}>{main}</div>
    )

  const threadsBtn = threads && threadsButton && (
    <Button
      size="sm"
      variant="ghost"
      onClick={() => (narrow ? setSheet(true) : threads.onHide(false))}
      title="Show the Threads panel"
    >
      Threads
      {openThreads > 0 && (
        <Badge variant="slate" className="tabular-nums">
          {openThreads}
        </Badge>
      )}
    </Button>
  )

  if (mine.length === 0 && filter !== "done")
    return (
      <>
        {header}
        {withThreads(
          <>
            <FilterBar
              filter={filter}
              onFilter={onFilter}
              decide={0}
              only={["all", "done"]}
            />
            {threadsBtn && (
              <span className="flex justify-end">{threadsBtn}</span>
            )}
            {lead}
            {notice}
            {empty}
          </>
        )}
      </>
    )

  return (
    <>
      {header}
      {withThreads(
        <>
          {fill && lead ? (
            <div className="max-h-[30%] min-h-0 overflow-y-auto">{lead}</div>
          ) : (
            lead
          )}
          {rows && (
            <h2 className="px-1 pt-2 text-base font-semibold">Agent progress</h2>
          )}
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <FilterBar
              filter={active}
              onFilter={onFilter}
              decide={decide}
              review={review}
              only={["all", "decide", "review", "done"]}
            />
            <DeviceBar filter={active} onFilter={onFilter} devices={chips} />
            {members && (
              <MemberBar filter={active} onFilter={onFilter} members={members} now={now} />
            )}
            {rowBoards && (
              <BoardBar
                filter={active}
                onFilter={onFilter}
                boards={rowBoards}
                withPrivate
              />
            )}
            <span className="ml-auto flex flex-wrap items-center gap-2">
              {threadsBtn}
              <SearchBox query={query} onQuery={onQuery} />
              <ViewSwitch view={view} onView={onView} />
            </span>
          </div>
          {notice}
          {hiddenLine && (
            <p
              className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-md border border-warning/40 bg-warning/10 px-2.5 py-1.5 text-[12.5px] text-foreground"
              role="status"
            >
              <span>
                {query.trim() ? "A search" : "A filter"} is hiding{" "}
                {hiddenLine.hidden} {hiddenLine.hidden === 1 ? "card" : "cards"}
                {hiddenLine.decide > 0 && (
                  <>
                    ,{" "}
                    <b className="text-warning">
                      {hiddenLine.decide}{" "}
                      {hiddenLine.decide === 1 ? "needs" : "need"} a decision
                    </b>
                  </>
                )}
              </span>
              <Button
                size="sm"
                variant="ghost"
                className="h-6 px-2 text-[12.5px]"
                onClick={() => {
                  onFilter("all")
                  onQuery("")
                }}
              >
                Show all
              </Button>
            </p>
          )}
          {active === "done" ? (
            <DoneGrid
              done={done}
              byAge={scope}
              query={query}
              selected={selected}
              onSelect={onOpen}
              now={now}
            />
          ) : view === "canvas" ? (
            <BoardCanvas cards={shown} onOpen={onOpen} fill={fill} />
          ) : shown.length && rows ? (
            <>
              <OrderPanel cards={shown} onOpen={onOpen} />
              {pinnedGroup.length > 0 && (
                <h3 className="px-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
                  Pinned
                </h3>
              )}
              {pinnedGroup.length > 0 && (
                <ProgressRows
                  label="Pinned cards"
                  cards={pinnedGroup}
                  selected={selected}
                  onOpen={onOpen}
                />
              )}
              <ProgressRows
                label={pinnedGroup.length ? "Everything else" : "Cards"}
                cards={shown.filter((c) => !inPinned.has(c.id))}
                selected={selected}
                onOpen={onOpen}
                grouped
              />
            </>
          ) : shown.length ? (
            <>
              <OrderPanel cards={shown} onOpen={onOpen} />
              {pinnedGroup.length > 0 && (
                <>
                  <h2 className="px-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
                    Pinned
                  </h2>
                  <CardGrid
                    label="Pinned cards"
                    cards={pinnedGroup}
                    context={context}
                    selected={selected}
                    onSelect={onOpen}
                    onMove={(id, before) => void cardsApi.movePin(id, before)}
                  />
                  <h2 className="px-1 pt-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
                    Everything else
                  </h2>
                </>
              )}
              <CardGrid
                cards={shown.filter((c) => !inPinned.has(c.id) && !laterOf(c))}
                context={context}
                selected={selected}
                onSelect={onOpen}
              />
              {shown.some((c) => laterOf(c)) && (
                <>
                  <h2 className="flex items-center gap-2 px-1 pt-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
                    Later
                    <span className="tabular-nums">
                      {shown.filter((c) => laterOf(c)).length}
                    </span>
                  </h2>
                  <CardGrid
                    label="Later"
                    cards={shown.filter((c) => laterOf(c))}
                    context={context}
                    selected={selected}
                    onSelect={onOpen}
                  />
                </>
              )}
            </>
          ) : (
            <p className="px-1 text-[12.5px] text-muted-foreground">
              {query.trim()
                ? "No cards match."
                : rows && all.length
                  ? "Nothing else in motion."
                : active === "decide" && nothingDecide
                  ? nothingDecide
                  : "Nothing here with this filter."}
            </p>
          )}
        </>
      )}
    </>
  )
}

// The Board's column (card #170): it scrolls by itself under the box.
// The 4px gutter keeps a selected card's ring clear of the clip. Its rows
// are max-content (auto-rows-max): the column has a set height, and an
// auto row would shrink a child that clips (Home's progress rows, a
// glass panel with overflow hidden) to nothing once the column is full,
// leaving it unscrollable (card #301).
const boardColumn =
  "-mx-1 grid min-h-0 min-w-0 flex-1 auto-rows-max content-start gap-3.5 overflow-y-auto px-1 pb-10"

// The column in Canvas (card #410): a flex column exactly the space left,
// that never scrolls. Its rows keep their height (shrink-0); the canvas
// grows into the rest.
const fillColumn =
  "-mx-1 flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 overflow-hidden px-1 pt-1 pb-4 *:shrink-0"
