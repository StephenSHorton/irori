// A space's boards (card #558): a space is a group of people, a board is a
// list of tasks. SpaceBoards is what a space opens on: a tile per board,
// each opening that board's list, and Add board. ListHeaderBar is the
// header over one open board: which space it's in, and renaming or
// deleting the board. Leave, Invite and Space settings stay on the space.

import { useState, type FormEvent } from "react"
import { ChevronRightIcon, EllipsisIcon } from "lucide-react"

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
  type BoardSummary,
  type SpaceBoard,
} from "@/lib/boards"
import type { CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { go } from "@/lib/nav"
import { boardsOf } from "@/lib/spaces"
import { errorText } from "@/lib/talk"

import { boardSegs, RecentCards, StateBar } from "./boards-page"

// cardsOnBoard is the space's cards on board `tag`: the space's own board
// also has the cards from before spaces, which carry its tag.
export function cardsOnBoard(b: BoardSummary, cards: CardSummary[], tag: string): CardSummary[] {
  return cards.filter((c) => c.board === b.boardId && (c.tag || b.tag) === tag)
}

// recent is a board's newest few cards.
function recent(cards: CardSummary[]): CardSummary[] {
  return [...cards]
    .sort((x, y) => (y.updatedAt > x.updatedAt ? 1 : y.updatedAt < x.updatedAt ? -1 : 0))
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
        className="glass-panel grid h-full w-full content-start gap-2 p-3 text-left transition-colors hover:border-primary/60 focus-visible:outline-2 focus-visible:outline-primary"
        onClick={onOpen}
      >
        <span className="flex items-center gap-2">
          <span className="min-w-0 truncate text-sm font-medium text-ink">
            {plainLine(x.name) || "(unnamed)"}
          </span>
          {x.tag && <span className="font-mono text-xs text-muted-foreground">{plainLine(x.tag)}#</span>}
          {isDefault && (
            <span className="ml-auto rounded bg-muted px-1 text-[11px] text-muted-foreground">default</span>
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
function AddBoard({ spaceId }: { spaceId: string }) {
  const [adding, setAdding] = useState(false)
  const [name, setName] = useState("")
  const [tag, setTag] = useState("")
  const [error, setError] = useState<string | null>(null)
  const add = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await actions.addToSpace(spaceId, name, tag)
      setAdding(false)
      setName("")
      setTag("")
      await reloadBoards()
    } catch (err) {
      setError(errorText(err))
    }
  }
  if (!adding)
    return (
      <Button size="sm" variant="ghost" className="h-7 px-2.5 text-[12.5px]" onClick={() => setAdding(true)}>
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
      <Button type="submit" size="sm" className="h-8" disabled={!name.trim() || !tag.trim()}>
        Add board
      </Button>
      <Button type="button" variant="ghost" size="sm" className="h-8" onClick={() => setAdding(false)}>
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

const tiles = "grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))]"

// SpaceBoards is what a space opens on: every board as a tile.
export function SpaceBoards({
  space: b,
  cards,
}: {
  space: BoardSummary
  cards: CardSummary[]
  now: Date
}) {
  const list = boardsOf(b)
  return (
    <section className="grid gap-2" aria-label="Boards in this space">
      <div className="flex items-center gap-2">
        <h2 className="px-1 text-base font-semibold">Boards</h2>
        {b.ready && <AddBoard spaceId={b.boardId} />}
      </div>
      <ul className={tiles}>
        {list.map((x, i) => (
          <BoardTile
            key={x.boardId}
            board={x}
            isDefault={i === 0}
            cards={cardsOnBoard(b, cards, x.tag)}
            onOpen={() => go({ kind: "board", id: b.boardId, board: x.boardId })}
          />
        ))}
      </ul>
    </section>
  )
}

// PersonalBoards is Personal's page: its boards, private to you. Nobody can
// be invited, so it has no people or settings of its own.
export function PersonalBoards({ cards }: { cards: CardSummary[]; now: Date }) {
  const own = cards.filter((c) => !c.board)
  const list: SpaceBoard[] = [
    { boardId: "", name: "My cards", tag: "" },
    { boardId: "sample-personal-home", name: "House", tag: "home" },
  ]
  return (
    <section className="grid gap-2" aria-label="Boards in Personal">
      <p className="max-w-2xl px-1 text-[13px] text-muted-foreground">
        Personal is you, your devices and your agents. Nobody else sees it.
      </p>
      <div className="flex items-center gap-2">
        <h2 className="px-1 text-base font-semibold">Boards</h2>
        <Button size="sm" variant="ghost" className="h-7 px-2.5 text-[12.5px]">
          + Add board
        </Button>
      </div>
      <ul className={tiles}>
        {list.map((x, i) => (
          <BoardTile
            key={x.boardId}
            board={x}
            isDefault={i === 0}
            cards={i === 0 ? own : []}
            onOpen={() => go({ kind: "home" })}
          />
        ))}
      </ul>
    </section>
  )
}

// ListHeaderBar is the header over one board's list: the space it's in
// (back to the space's page), its name and tag, and Rename and Delete. The
// default board can't be deleted.
export function ListHeaderBar({ space: b, list: x }: { space: BoardSummary; list: SpaceBoard }) {
  const [asking, setAsking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const isDefault = boardsOf(b)[0]?.boardId === x.boardId
  return (
    <div className="flex min-w-0 flex-1 items-center gap-x-1.5">
      <Button variant="ghost" size="sm" className="px-1.5" onClick={() => go({ kind: "board", id: b.boardId })}>
        {plainLine(b.name) || "(unnamed)"}
      </Button>
      <ChevronRightIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <h1 className="flex min-w-0 items-baseline gap-2 text-lg font-semibold text-ink">
        <span className="truncate">{plainLine(x.name) || "(unnamed)"}</span>
        <span className="font-mono text-sm font-normal text-muted-foreground">{plainLine(x.tag)}#</span>
      </h1>
      {error && <span className="truncate text-xs text-destructive">{plainLine(error)}</span>}
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        <Button variant="ghost" size="sm" className="hidden sm:inline-flex">
          Rename
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                className={isDefault ? "sm:hidden" : undefined}
                aria-label="Board actions"
                title="Board actions"
              />
            }
          >
            <EllipsisIcon aria-hidden />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-auto">
            <DropdownMenuItem className="sm:hidden">Rename</DropdownMenuItem>
            {!isDefault && (
              <DropdownMenuItem variant="destructive" onClick={() => setAsking(true)}>
                Delete this board
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </span>
      {asking && (
        <ConfirmDialog
          variant="destructive"
          confirmLabel="Delete the board"
          question={
            <>
              Delete <b>{plainLine(x.name)}</b>? Its cards move to <b>{plainLine(boardsOf(b)[0]?.name ?? "")}</b>,
              the default board, and take new numbers there.
            </>
          }
          onConfirm={async () => {
            await actions.deleteFromSpace(b.boardId, x.boardId)
            await reloadBoards()
            go({ kind: "board", id: b.boardId })
          }}
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
