// BoardPage is one board as its own page (card #96): the board's cards
// (List or Canvas, with Needs a decision and Closed) on the left, and on
// the right who is on it and Invite someone (owners). Rename, the board's
// settings (a cog opening a dialog, card #312), Leave and Forget sit by
// its name. The box over the
// cards starts a thread on this board (card #134), listed in the board's
// own Threads panel above who is on it.

import { useEffect, useState, type FormEvent } from "react"
import { ArrowLeftIcon, EllipsisIcon, PinIcon, PinOffIcon, SettingsIcon } from "lucide-react"

import { AskBox } from "@/components/cards/ask-box"
import { useCommands } from "@/lib/commands"
import { BoardView } from "@/components/cards/board-view"
import type { Filter } from "@/components/cards/card-grid"
import type { BoardView as View } from "@/components/canvas/switch"
import { ConfirmDialog, type ConfirmAsk } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
  PresenceReload,
  reloadBoards,
  useBoards,
  useInvite,
  type BoardSummary,
} from "@/lib/boards"
import { useThreads, type CardSummary, type DoneState } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { go } from "@/lib/nav"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

import { BoardSettingsPanel, InviteHow, InvitePanel, useInviteReach } from "./parts"

export function BoardPage({
  boardId,
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
}: {
  boardId: string
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
  now: Date
}) {
  const { boards } = useBoards()
  const { threads } = useThreads(boardId)
  // Presence on the member chips (card #371) moves without a
  // boards:changed: read the list again while the page is open.
  useEffect(() => {
    const t = setInterval(() => void reloadBoards(), PresenceReload)
    return () => clearInterval(t)
  }, [])
  // The thread unfolded in the panel, and whether the person hid it.
  const [thread, setThread] = useState("")
  const [threadsHidden, setThreadsHidden] = useState(false)
  const b = boards.find((x) => x.boardId === boardId)
  if (!b)
    return (
      <div className="grid gap-2">
        <BackToBoards />
        <p className="text-sm text-muted-foreground">
          This board isn't here any more: you may have left or forgotten it.
        </p>
      </div>
    )
  // The same board view as Home (card #161), over this board's cards,
  // with who is on it beside them.
  return (
    <>
      <BoardNotes board={b} />
      <BoardView
        scope={boardId}
        cards={cards}
        done={done}
        filter={filter}
        onFilter={onFilter}
        view={view}
        onView={onView}
        query={query}
        onQuery={onQuery}
        selected={selected}
        onOpen={onOpen}
        now={now}
        header={
          <AskBox
            board={boardId}
            onThread={(id) => {
              setThread(id)
              setThreadsHidden(false)
            }}
          />
        }
        empty={
          <p className="px-1 text-[12.5px] text-muted-foreground">
            No cards on this board yet. Make one with New card, or share a card
            to it from its page.
          </p>
        }
        nothingDecide="Nothing on this board waits on you."
        threads={{
          list: threads,
          open: thread,
          onOpen: setThread,
          hidden: threadsHidden,
          onHide: setThreadsHidden,
        }}
      />
    </>
  )
}

function BackToBoards() {
  return (
    <Button variant="ghost" size="sm" className="w-fit px-1.5" onClick={() => go({ kind: "boards" })}>
      <ArrowLeftIcon aria-hidden />
      Boards
    </Button>
  )
}

// BoardHeaderBar is a board's page header (card #135): its state dot,
// name and tag, its cards and people, and the actions on the board itself:
// Pin to the sidebar and Rename (owners) in the open; Leave and Forget here
// in its ⋯ menu (card #211).
export function BoardHeaderBar({ boardId }: { boardId: string }) {
  const { boards, reload } = useBoards()
  const b = boards.find((x) => x.boardId === boardId)
  const [renaming, setRenaming] = useState(false)
  const [name, setName] = useState("")
  const [error, setError] = useState<string | null>(null)
  // The palette's board commands (card #213), while this board is open.
  useCommands(
    b
      ? [
          {
            id: "board.pin",
            title: b.pinned ? "Unpin this board" : "Pin this board",
            keywords: ["sidebar", "favourite", "favorite"],
            run: () => {
              setError(null)
              actions
                .pin(b.boardId, !b.pinned)
                .then(reload)
                .catch((err) => setError(errorText(err)))
            },
          },
          ...(b.owner
            ? [
                {
                  id: "board.rename",
                  title: "Rename this board",
                  run: () => {
                    setName(b.name)
                    setRenaming(true)
                  },
                },
              ]
            : []),
        ]
      : []
  )
  if (!b)
    return <h1 className="text-lg font-semibold text-ink">Board</h1>
  const rename = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await actions.rename(b.boardId, name)
      setRenaming(false)
      await reload()
    } catch (err) {
      setError(errorText(err))
    }
  }
  const pin = async () => {
    setError(null)
    try {
      await actions.pin(b.boardId, !b.pinned)
      await reload()
    } catch (err) {
      setError(errorText(err))
    }
  }
  const ok = b.ready && !b.warnings?.length
  return (
    <div className="flex min-w-0 flex-1 items-center gap-x-3">
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
          <Button type="submit" size="sm" className="h-8" disabled={!name.trim()}>
            Save
          </Button>
          <Button type="button" variant="ghost" size="sm" className="h-8" onClick={() => setRenaming(false)}>
            Cancel
          </Button>
        </form>
      ) : (
        <h1 className="flex min-w-0 items-baseline gap-2 text-lg font-semibold text-ink">
          <span className={cn("size-2 shrink-0 self-center rounded-full", ok ? "bg-success" : "bg-warning")} aria-hidden />
          <span className="truncate">{plainLine(b.name) || "(unnamed)"}</span>
          <span className="font-mono text-sm font-normal text-muted-foreground">{plainLine(b.tag)}#</span>
        </h1>
      )}
      <span className="hidden min-w-0 truncate text-xs text-muted-foreground lg:inline">
        {b.ready
          ? `${b.cards === 1 ? "1 card" : `${b.cards} cards`} · visible to ${b.people.map(plainLine).join(", ")}`
          : "Its cards are still arriving from your other device"}
      </span>
      {error && <span className="truncate text-xs text-destructive">{error}</span>}
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        <Button
          variant="ghost"
          size="sm"
          aria-pressed={!!b.pinned}
          aria-label={b.pinned ? "Unpin" : "Pin"}
          onClick={() => void pin()}
          title={b.pinned ? "Take it out of the sidebar" : "Keep it in the sidebar, on all your devices"}
        >
          {b.pinned ? <PinOffIcon aria-hidden /> : <PinIcon aria-hidden />}
          <span className="hidden sm:inline">{b.pinned ? "Unpin" : "Pin"}</span>
        </Button>
        {b.ready && <BoardSettingsButton board={b} reload={reload} />}
        {b.owner && !renaming && (
          <Button
            variant="ghost"
            size="sm"
            className="hidden sm:inline-flex"
            onClick={() => {
              setName(b.name)
              setRenaming(true)
            }}
          >
            Rename
          </Button>
        )}
        <BoardActionsMenu
          board={b}
          reload={reload}
          onError={setError}
          onRename={
            b.owner && !renaming
              ? () => {
                  setName(b.name)
                  setRenaming(true)
                }
              : undefined
          }
        />
      </span>
    </div>
  )
}

// BoardSettingsButton is the header's cog (card #312): the board's
// settings (members, what reaches your agents, what they may do, threads)
// in a dialog rather than always open beside the cards. Closing it reloads
// the board so a changed member list shows.
function BoardSettingsButton({ board: b, reload }: { board: BoardSummary; reload: () => Promise<unknown> }) {
  const [open, setOpen] = useState(false)
  const changed = async () => {
    await reload()
  }
  return (
    <>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Board settings"
        title="Board settings"
        onClick={() => setOpen(true)}
      >
        <SettingsIcon aria-hidden />
      </Button>
      <Dialog
        open={open}
        onOpenChange={(o) => {
          setOpen(o)
          if (!o) void changed()
        }}
      >
        <DialogContent className="max-h-[85vh] gap-3 overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Board settings</DialogTitle>
            <DialogDescription>
              {plainLine(b.name) || "(unnamed)"}: who is on it, what reaches your agents and what they may do here.
            </DialogDescription>
          </DialogHeader>
          {open && <BoardInvite board={b} onChanged={changed} />}
          {open && <BoardSettingsPanel boardId={b.boardId} onChanged={changed} />}
        </DialogContent>
      </Dialog>
    </>
  )
}

// BoardActionsMenu is the header's ⋯ menu: the actions that take the board
// away (Leave and Forget here), each behind
// its confirm step. Each item opens its dialog once the menu has closed.
function BoardActionsMenu({
  board: b,
  reload,
  onError,
  onRename,
}: {
  board: BoardSummary
  reload: () => Promise<unknown>
  onError: (error: string) => void
  // Rename for owners: in the menu only below sm, where the header has no
  // room for its button (card #227).
  onRename?: () => void
}) {
  const [asking, setAsking] = useState<"leave" | "forgetHere" | null>(null)
  const asks: Record<"leave" | "forgetHere", ConfirmAsk> = {
    leave: {
      variant: "destructive",
      confirmLabel: "Leave the board",
      question:
        b.people.length > 1 ? (
          <>
            Leave <b>{plainLine(b.name)}</b>? It goes from every one of your devices now, and nothing
            you write there counts after it. The others hear that you left when their Irori next
            reaches one of your devices. Coming back needs a new invite.
          </>
        ) : (
          <>
            Leave <b>{plainLine(b.name)}</b>? It goes from every one of your devices, with its cards.
            Coming back needs a new board.
          </>
        ),
      onConfirm: async () => {
        // The board goes from every list at once (card #304).
        await actions.leave(b.boardId)
        await reload()
        go({ kind: "boards" })
      },
    },
    forgetHere: {
      confirmLabel: "Forget on this device",
      question: (
        <>
          Forget <b>{plainLine(b.name)}</b> on this device only? Its file goes from here; your other
          devices keep it.
        </>
      ),
      onConfirm: async () => {
        await actions.forget(b.boardId)
        go({ kind: "boards" })
      },
    },
  }
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={<Button variant="ghost" size="icon-sm" aria-label="Board actions" title="Board actions" />}
        >
          <EllipsisIcon aria-hidden />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-auto">
          {onRename && (
            <DropdownMenuItem className="sm:hidden" onClick={onRename}>
              Rename
            </DropdownMenuItem>
          )}
          <DropdownMenuItem variant="destructive" onClick={() => setAsking("leave")}>
            Leave
          </DropdownMenuItem>
          <DropdownMenuItem variant="destructive" onClick={() => setAsking("forgetHere")}>
            Forget here
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {asking && (
        <ConfirmDialog
          {...asks[asking]}
          open
          onOpenChange={(open) => {
            if (!open) setAsking(null)
          }}
          onError={onError}
        />
      )}
    </>
  )
}

// BoardNotes is what the page says about the board under its header: a tag
// that clashes with another board's, and the board's warning lines.
function BoardNotes({ board: b }: { board: BoardSummary }) {
  if (!b.tagClash && !b.warnings?.length) return null
  return (
    <div className="grid gap-1">
      {b.tagClash && (
        <p className="text-xs text-destructive">
          Another board is also {plainLine(b.tag)}#: its refs need the card's id.
        </p>
      )}
      {b.warnings?.map((w, i) => (
        <p key={i} className="text-xs text-warning">
          {plainLine(w)}
        </p>
      ))}
    </div>
  )
}

// inviteError is a failed Invite in plain words: the mesh's own errors
// ("mesh: transport closed") mean Irori isn't online to make a link.
function inviteError(err: unknown): string {
  const t = errorText(err)
  if (/^mesh:|transport closed|not connected|no relay/i.test(t))
    return "Irori couldn't make an invite link: this device isn't connected right now. Check your network (or that Irori is running), then try Invite again."
  return t
}

// BoardInvite is how to get more people on the board, in its settings
// (card #371 moved it there from a People panel beside the cards; who is
// on it and online shows on the member chips): Invite someone (an owner,
// once the board is ready) with how an invite works, else why there is no
// Invite. An open invite stands in for the Invite button until it ends
// (card #103).
function BoardInvite({ board: b, onChanged }: { board: BoardSummary; onChanged: () => Promise<void> }) {
  const { invite, setInvite } = useInvite(true)
  const reach = useInviteReach(true)
  const [error, setError] = useState<string | null>(null)
  const inviting = !!invite && invite.state !== ""
  return (
    <section className="grid gap-2 text-xs" aria-label="Invite people">
      {inviting ? (
        <InvitePanel
          s={invite}
          onClose={() => {
            setInvite(null)
            void onChanged()
          }}
        />
      ) : (
        <>
          {!b.owner ? (
            <p className="text-xs text-muted-foreground">
              Only an owner can invite people to this board: ask one of them.
            </p>
          ) : !b.ready ? (
            <p className="text-xs text-muted-foreground">
              You can invite people once its cards have arrived from your other device.
            </p>
          ) : (
            (
              <div className="grid gap-2">
                <Button
                  className="w-fit"
                  onClick={async () => {
                    setError(null)
                    try {
                      setInvite(await actions.invite(b.boardId))
                    } catch (err) {
                      setError(inviteError(err))
                    }
                  }}
                >
                  Invite someone
                </Button>
                <InviteHow reach={reach} />
              </div>
            )
          )}
          {error && <p className="text-xs text-destructive">{error}</p>}
        </>
      )}
    </section>
  )
}
