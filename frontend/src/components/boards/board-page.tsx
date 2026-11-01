// BoardPage is one board as its own page (card #96): the board's cards
// (List or Canvas, with Needs a decision and Closed) on the left, and on
// the right who is on it, Invite someone (owners), and the board's
// settings. Rename, Leave and Forget sit by its name. The box over the
// cards starts a thread on this board (card #134), listed in the board's
// own Threads panel above who is on it.

import { useState, type FormEvent } from "react"
import { ArrowLeftIcon, PinIcon, PinOffIcon } from "lucide-react"

import { AskBox } from "@/components/cards/ask-box"
import { useCommands } from "@/lib/commands"
import { BoardView } from "@/components/cards/board-view"
import type { Filter } from "@/components/cards/card-grid"
import type { BoardView as View } from "@/components/canvas/switch"
import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  boards as actions,
  MaxBoardName,
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
  const { boards, reload } = useBoards()
  const { threads } = useThreads(boardId)
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
        aside={<PeoplePanel board={b} onChanged={reload} />}
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
// Pin to the sidebar, Rename (owners), Leave, Forget here.
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
            className="h-8 w-64"
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
        {b.leaving
          ? "Leaving…"
          : b.ready
            ? `${b.cards === 1 ? "1 card" : `${b.cards} cards`} · visible to ${b.people.map(plainLine).join(", ")}`
            : "Its cards are still arriving from your other device"}
      </span>
      {error && <span className="truncate text-xs text-destructive">{error}</span>}
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        <Button
          variant="ghost"
          size="sm"
          aria-pressed={!!b.pinned}
          onClick={() => void pin()}
          title={b.pinned ? "Take it out of the sidebar" : "Keep it in the sidebar, on all your devices"}
        >
          {b.pinned ? <PinOffIcon aria-hidden /> : <PinIcon aria-hidden />}
          {b.pinned ? "Unpin" : "Pin"}
        </Button>
        {b.owner && !renaming && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setName(b.name)
              setRenaming(true)
            }}
          >
            Rename
          </Button>
        )}
        {b.leaving ? (
          <ConfirmButton
            label="Forget now"
            variant="destructive"
            confirmLabel="Forget it now"
            question={
              <>
                Forget <b>{plainLine(b.name)}</b> on every one of your devices now? The others may not hear
                that you left until one of their devices next reaches yours, which may be never.
              </>
            }
            onConfirm={async () => {
              await actions.forgetNow(b.boardId)
              go({ kind: "boards" })
            }}
          />
        ) : (
          <ConfirmButton
            label="Leave"
            variant="destructive"
            confirmLabel="Leave the board"
            question={
              b.people.length > 1 ? (
                <>
                  Leave <b>{plainLine(b.name)}</b>? It goes from every one of your devices once another
                  member's Irori has your leave, and nothing you write there counts after it. Coming back
                  needs a new invite.
                </>
              ) : (
                <>
                  Leave <b>{plainLine(b.name)}</b>? It goes from every one of your devices, with its cards.
                  Coming back needs a new board.
                </>
              )
            }
            onConfirm={async () => {
              await actions.leave(b.boardId)
              await reload()
            }}
          />
        )}
        <ConfirmButton
          label="Forget here"
          confirmLabel="Forget on this device"
          question={
            <>
              Forget <b>{plainLine(b.name)}</b> on this device only? Its file goes from here; your other
              devices keep it.
            </>
          }
          onConfirm={async () => {
            await actions.forget(b.boardId)
            go({ kind: "boards" })
          }}
        />
      </span>
    </div>
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

// PeoplePanel is who is on the board and how to get more people on it:
// Invite someone (an owner, once the board is ready) with how an invite
// works, else why there is no Invite; then the members and the board's
// settings. An open invite stands in for the Invite button until it
// ends; the settings stay (card #103).
function PeoplePanel({ board: b, onChanged }: { board: BoardSummary; onChanged: () => Promise<void> }) {
  const { invite, setInvite } = useInvite(true)
  const reach = useInviteReach(true)
  const [error, setError] = useState<string | null>(null)
  const inviting = !!invite && invite.state !== ""
  return (
    <aside className="glass-panel grid grid-cols-[minmax(0,1fr)] gap-3 p-4 text-[13px]" aria-label="People on this board">
      <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">People</h2>
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
            !b.leaving && (
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
      {b.ready && !b.leaving && (
        <div className="border-t border-border pt-3">
          <BoardSettingsPanel boardId={b.boardId} onChanged={onChanged} />
        </div>
      )}
    </aside>
  )
}
