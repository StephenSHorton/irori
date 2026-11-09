// A space's boards (card #558): a space is a group of people, a board is a
// list of tasks. SpaceBoards is what a space opens on: a tile per board,
// each opening that board's list, and Add board. ListHeaderBar is the
// header over one open board: which space it's in, and renaming or
// deleting the board. Leave, Invite and Space settings stay on the space.
// Personal has the same page and list, over the person's own log.

import { useEffect, useState, type FormEvent } from "react"
import { ChevronRightIcon, EllipsisIcon } from "lucide-react"

import { BoardView, ScopeOwn } from "@/components/cards/board-view"
import type { Filter } from "@/components/cards/card-grid"
import type { BoardView as View } from "@/components/canvas/switch"
import { ConfirmDialog } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import {
  boards as actions,
  MaxBoardName,
  reloadBoards,
  usePersonalBoards,
  type BoardSummary,
  type SpaceBoard,
} from "@/lib/boards"
import type { CardSummary, DoneState } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { go } from "@/lib/nav"
import { boardsOf, cardsOnPersonal, Personal } from "@/lib/spaces"
import { errorText } from "@/lib/talk"

import { boardSegs, RecentCards, StateBar } from "./board-tiles"

// cardsOnBoard is the space's cards on one of its boards, by the board's
// id: a card with no inner board (or the space's own id) is on the first
// board, as are the cards from before spaces.
export function cardsOnBoard(
  b: BoardSummary,
  cards: CardSummary[],
  boardId: string
): CardSummary[] {
  const first = boardsOf(b)[0].boardId
  const on = (c: CardSummary) =>
    !c.spaceBoard || c.spaceBoard === b.boardId ? first : c.spaceBoard
  return cards.filter((c) => c.board === b.boardId && on(c) === boardId)
}

// recent is a board's newest few cards.
function recent(cards: CardSummary[]): CardSummary[] {
  return [...cards]
    .sort((x, y) =>
      y.updatedAt > x.updatedAt ? 1 : y.updatedAt < x.updatedAt ? -1 : 0
    )
    .slice(0, 3)
}

// BoardTile is one board in a space: its name and tag, its cards by state,
// and its newest few. onOpen opens its list.
function BoardTile({
  board: x,
  isDefault,
  cards,
  onOpen,
}: {
  board: SpaceBoard
  isDefault: boolean
  cards: CardSummary[]
  onOpen: () => void
}) {
  const decide = cards.filter((c) => c.openBlockers > 0).length
  return (
    <li>
      <button
        type="button"
        className="grid h-full w-full content-start gap-2 p-3 text-left glass-panel transition-colors hover:border-primary/60 focus-visible:outline-2 focus-visible:outline-primary"
        onClick={onOpen}
      >
        <span className="flex items-center gap-2">
          <span className="min-w-0 truncate text-sm font-medium text-ink">
            {plainLine(x.name) || "(unnamed)"}
          </span>
          {x.tag && (
            <span className="font-mono text-xs text-muted-foreground">
              {plainLine(x.tag)}#
            </span>
          )}
          {isDefault && (
            <span className="ml-auto rounded bg-muted px-1 text-[11px] text-muted-foreground">
              default
            </span>
          )}
        </span>
        <StateBar segs={boardSegs(cards, 0)} />
        <RecentCards cards={recent(cards)} />
        {decide > 0 && (
          <span className="text-xs text-destructive">
            {decide === 1 ? "1 card needs" : `${decide} cards need`} a decision
          </span>
        )}
      </button>
    </li>
  )
}

// AddBoard is + Add board and its small form.
function AddBoard({
  onAdd,
}: {
  onAdd: (name: string, tag: string) => Promise<unknown>
}) {
  const [adding, setAdding] = useState(false)
  const [name, setName] = useState("")
  const [tag, setTag] = useState("")
  const [error, setError] = useState<string | null>(null)
  const add = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await onAdd(name, tag)
      setAdding(false)
      setName("")
      setTag("")
    } catch (err) {
      setError(errorText(err))
    }
  }
  if (!adding)
    return (
      <Button
        size="sm"
        variant="ghost"
        className="h-7 px-2.5 text-[12.5px]"
        onClick={() => setAdding(true)}
      >
        + Add board
      </Button>
    )
  return (
    <form className="flex flex-wrap items-center gap-1.5" onSubmit={add}>
      <Input
        value={name}
        onChange={(e) => setName(e.target.value)}
        maxLength={MaxBoardName}
        placeholder="Board name"
        aria-label="New board's name"
        className="h-8 w-48"
        autoFocus
      />
      <Input
        value={tag}
        onChange={(e) => setTag(e.target.value.toLowerCase())}
        maxLength={16}
        placeholder="tag"
        aria-label="New board's tag (its card refs: tag#12)"
        className="h-8 w-24 font-mono"
      />
      <Button
        type="submit"
        size="sm"
        className="h-8"
        disabled={!name.trim() || !tag.trim()}
      >
        Add board
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="h-8"
        onClick={() => setAdding(false)}
      >
        Cancel
      </Button>
      {error && (
        <p className="basis-full text-xs text-destructive" role="alert">
          {plainLine(error)}
        </p>
      )}
    </form>
  )
}

const tiles =
  "grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))]"

// SpaceBoards is what a space opens on: every board as a tile.
export function SpaceBoards({
  space: b,
  cards,
}: {
  space: BoardSummary
  cards: CardSummary[]
}) {
  const list = boardsOf(b)
  return (
    <section className="grid gap-2" aria-label="Boards in this space">
      <div className="flex items-center gap-2">
        <h2 className="px-1 text-base font-semibold">Boards</h2>
        {b.ready && (
          <AddBoard
            onAdd={async (name, tag) => {
              await actions.addToSpace(b.boardId, name, tag)
              await reloadBoards()
            }}
          />
        )}
      </div>
      <ul className={tiles}>
        {list.map((x, i) => (
          <BoardTile
            key={x.boardId}
            board={x}
            isDefault={i === 0}
            cards={cardsOnBoard(b, cards, x.boardId)}
            onOpen={() =>
              go({ kind: "board", id: b.boardId, board: x.boardId })
            }
          />
        ))}
      </ul>
    </section>
  )
}

// PersonalBoards is Personal's page ("#board/personal", card #689): its
// boards, private to you. Nobody can be invited, so it has no people or
// settings of its own.
export function PersonalBoards({ cards }: { cards: CardSummary[] }) {
  const { boards: list, error, reload } = usePersonalBoards()
  return (
    <section className="grid gap-2" aria-label="Boards in Personal">
      <p className="max-w-2xl px-1 text-[13px] text-muted-foreground">
        Personal is you, your devices and your agents. Nobody else sees it.
      </p>
      <div className="flex items-center gap-2">
        <h2 className="px-1 text-base font-semibold">Boards</h2>
        <AddBoard
          onAdd={async (name, tag) => {
            await actions.addToPersonal(name, tag)
            await reload()
          }}
        />
      </div>
      {error && <p className="text-xs text-destructive">{plainLine(error)}</p>}
      <ul className={tiles}>
        {list.map((x, i) => (
          <BoardTile
            key={x.boardId}
            board={x}
            isDefault={i === 0}
            cards={cardsOnPersonal(cards, x.boardId)}
            onOpen={() => go({ kind: "board", id: Personal, board: x.boardId })}
          />
        ))}
      </ul>
    </section>
  )
}

// PersonalList is one of Personal's boards open: the same list a space's
// board has, over the person's own cards on it.
export function PersonalList({
  list,
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
}: {
  list: string
  cards: CardSummary[]
  done: DoneState
  filter: Filter
  onFilter: (f: Filter) => void
  view: View
  onView: (v: View) => void
  query: string
  onQuery: (q: string) => void
  selected: string
  onOpen: (id: string) => void
}) {
  return (
    <BoardView
      scope={ScopeOwn}
      cards={cardsOnPersonal(cards, list)}
      done={done}
      filter={filter}
      onFilter={onFilter}
      view={view}
      onView={onView}
      query={query}
      onQuery={onQuery}
      selected={selected}
      onOpen={onOpen}

      empty={
        <p className="px-1 text-[12.5px] text-muted-foreground">
          No cards on this board yet. Make one with New card.
        </p>
      }
    />
  )
}

// PersonalHeaderBar is ListHeader over one of Personal's boards.
export function PersonalHeaderBar({ list }: { list: string }) {
  const { boards: all, reload } = usePersonalBoards()
  const x = all.find((b) => b.boardId === list)
  // Deleted on another device (card #563): its cards are on the default
  // board now, so that is where the page goes.
  const gone = all.length > 0 && !x
  useEffect(() => {
    if (gone) go({ kind: "board", id: Personal, board: Personal })
  }, [gone])
  if (!x) return <h1 className="text-lg font-semibold text-ink">Personal</h1>
  return (
    <ListHeader
      space="Personal"
      onBack={() => go({ kind: "board", id: Personal })}
      list={x}
      save={async (name) => {
        await actions.renameInPersonal(x.boardId, name)
        await reload()
      }}
      remove={
        x.boardId === Personal
          ? undefined
          : {
              into: all[0]?.name ?? "Personal",
              run: async () => {
                await actions.deleteFromPersonal(x.boardId)
                await reload()
                go({ kind: "board", id: Personal })
              },
            }
      }
    />
  )
}

// ListHeaderBar is ListHeader over one board in a shared space.
export function ListHeaderBar({
  space: b,
  list: x,
}: {
  space: BoardSummary
  list: SpaceBoard
}) {
  const first = boardsOf(b)[0]
  return (
    <ListHeader
      space={plainLine(b.name) || "(unnamed)"}
      onBack={() => go({ kind: "board", id: b.boardId })}
      list={x}
      save={async (name) => {
        await actions.renameInSpace(b.boardId, x.boardId, name)
        await reloadBoards()
      }}
      remove={
        !first || first.boardId === x.boardId
          ? undefined
          : {
              into: first.name,
              run: async () => {
                await actions.deleteFromSpace(b.boardId, x.boardId)
                await reloadBoards()
                go({ kind: "board", id: b.boardId })
              },
            }
      }
    />
  )
}

// ListHeader is the header over one board's list: the space it's in (back
// to the space's page), its name and tag, Rename, which any member may use
// (card #558), and Delete (card #563). The space keeps its own name, and
// the default board can't be deleted, so it has no remove.
function ListHeader({
  space,
  onBack,
  list: x,
  save,
  remove,
}: {
  space: string
  onBack: () => void
  list: SpaceBoard
  save: (name: string) => Promise<void>
  // remove deletes this board; its cards go to the board named into.
  remove?: { into: string; run: () => Promise<void> }
}) {
  const [asking, setAsking] = useState(false)
  const [renaming, setRenaming] = useState(false)
  const [name, setName] = useState("")
  const [error, setError] = useState<string | null>(null)
  const start = () => {
    setError(null)
    setName(x.name)
    setRenaming(true)
  }
  const rename = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await save(name)
      setRenaming(false)
    } catch (err) {
      setError(errorText(err))
    }
  }
  return (
    <div className="flex min-w-0 flex-1 items-center gap-x-1.5">
      <Button variant="ghost" size="sm" className="px-1.5" onClick={onBack}>
        {space}
      </Button>
      <ChevronRightIcon
        aria-hidden
        className="size-4 shrink-0 text-muted-foreground"
      />
      {renaming ? (
        <form className="flex gap-1.5" onSubmit={rename}>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={MaxBoardName}
            aria-label="Board name"
            className="h-8 w-40 sm:w-64"
            autoFocus
          />
          <Button
            type="submit"
            size="sm"
            className="h-8"
            disabled={!name.trim()}
          >
            Save
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8"
            onClick={() => setRenaming(false)}
          >
            Cancel
          </Button>
        </form>
      ) : (
        <h1 className="flex min-w-0 items-baseline gap-2 text-lg font-semibold text-ink">
          <span className="truncate">{plainLine(x.name) || "(unnamed)"}</span>
          {x.tag && (
            <span className="font-mono text-sm font-normal text-muted-foreground">
              {plainLine(x.tag)}#
            </span>
          )}
        </h1>
      )}
      {error && (
        <span className="truncate text-xs text-destructive">
          {plainLine(error)}
        </span>
      )}
      {!renaming && (
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          <Button
            variant="ghost"
            size="sm"
            className="hidden sm:inline-flex"
            onClick={start}
          >
            Rename
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className={remove ? undefined : "sm:hidden"}
                  aria-label="Board actions"
                  title="Board actions"
                />
              }
            >
              <EllipsisIcon aria-hidden />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-auto">
              <DropdownMenuItem className="sm:hidden" onClick={start}>
                Rename
              </DropdownMenuItem>
              {remove && (
                <DropdownMenuItem
                  variant="destructive"
                  onClick={() => setAsking(true)}
                >
                  Delete this board
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </span>
      )}
      {asking && remove && (
        <ConfirmDialog
          variant="destructive"
          confirmLabel="Delete the board"
          question={
            <>
              Delete <b>{plainLine(x.name)}</b>? Its cards move to{" "}
              <b>{plainLine(remove.into)}</b>, the default board, and take new
              numbers there. Their old numbers still find them.
            </>
          }
          onConfirm={remove.run}
          open
          onOpenChange={(open) => {
            if (!open) setAsking(false)
          }}
          onError={setError}
        />
      )}
    </div>
  )
}
