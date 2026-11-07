// Boards are projects (card #690, docs/design/board.md "Boards are
// projects"): each board says what it is for, and when its project is over
// it is finished and leaves the lists. Here: New board with its mission,
// the mission at the top of a board and its edit dialog, the quiet "Looks
// finished?" line, the Finish dialog, a finished board's Reopen, and the
// Archived boards fold under a space's tiles.

import { useEffect, useState, type FormEvent, type ReactNode } from "react"
import { ChevronRightIcon } from "lucide-react"

import { cardRef } from "@/components/cards/bits"
import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  boards as actions,
  looksFinished,
  MaxBoardName,
  MaxBoardTag,
  MaxMission,
  suggestTag,
  validTag,
  type SpaceBoard,
} from "@/lib/boards"
import { cards, useDoneCards, type CardSummary } from "@/lib/cards"
import { plainLine, relativeTime } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import { Personal } from "@/lib/spaces"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// ---- asking the board's page from its header ---------------------------------

// The header (App's bar) and the board's page are separate trees: the
// header's ⋯ menu asks the page to open its Edit mission or Finish dialog.
const AskEvent = "irori:board-ask"
type Ask = { kind: "mission" | "finish"; boardId: string }

export function askBoard(kind: Ask["kind"], boardId: string) {
  window.dispatchEvent(
    new CustomEvent<Ask>(AskEvent, { detail: { kind, boardId } })
  )
}

// ---- mission fields -----------------------------------------------------------

function MissionFields({
  id,
  mission,
  doneWhen,
  onMission,
  onDoneWhen,
  autoFocus,
}: {
  id: string
  mission: string
  doneWhen: string
  onMission: (s: string) => void
  onDoneWhen: (s: string) => void
  autoFocus?: boolean
}) {
  return (
    <>
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-mission`}>Mission</Label>
        <Textarea
          id={`${id}-mission`}
          value={mission}
          autoSize
          autoFocus={autoFocus}
          maxLength={MaxMission}
          className="max-h-40"
          placeholder="What this board is for, in a sentence or two"
          onChange={(e) => onMission(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          It shows on the board and its tile, and agents working here read it.
        </p>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`${id}-done`}>Done when</Label>
        <Input
          id={`${id}-done`}
          value={doneWhen}
          maxLength={MaxMission}
          placeholder="How to tell the project is over (optional)"
          onChange={(e) => onDoneWhen(e.target.value)}
        />
      </div>
    </>
  )
}

// ---- New board ----------------------------------------------------------------

// AddBoardButton is + Add board over a space's (or Personal's) tiles, and
// the New board dialog it opens: a name, the tag its cards' refs carry
// (suggested from the name until typed), the mission and done-when.
export function AddBoardButton({
  onAdd,
}: {
  onAdd: (
    name: string,
    tag: string,
    mission: string,
    doneWhen: string
  ) => Promise<unknown>
}) {
  const [open, setOpen] = useState(() => previewFlag("addboard") !== null)
  const [name, setName] = useState("")
  const [tag, setTag] = useState("")
  const [typedTag, setTypedTag] = useState(false)
  const [mission, setMission] = useState("")
  const [doneWhen, setDoneWhen] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const t = (typedTag ? tag : suggestTag(name)).trim().toLowerCase()
  const reset = () => {
    setName("")
    setTag("")
    setTypedTag(false)
    setMission("")
    setDoneWhen("")
    setError(null)
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await onAdd(name, t, mission, doneWhen)
      setOpen(false)
      reset()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <Button
        size="sm"
        variant="ghost"
        className="h-7 px-2.5 text-[12.5px]"
        onClick={() => setOpen(true)}
      >
        + Add board
      </Button>
      <Dialog open={open} onOpenChange={(o) => !busy && setOpen(o)}>
        <DialogContent className="sm:max-w-lg">
          <form className="grid gap-3" onSubmit={(e) => void submit(e)}>
            <DialogHeader>
              <DialogTitle>New board</DialogTitle>
              <DialogDescription>
                A board is a project: say what it's for. When it's over, finish
                it and it leaves the lists.
              </DialogDescription>
            </DialogHeader>
            <div className="grid grid-cols-[minmax(0,1fr)_6.5rem] gap-x-2 gap-y-1.5">
              <Label htmlFor="new-board-name">Name</Label>
              <Label htmlFor="new-board-tag">Tag</Label>
              <Input
                id="new-board-name"
                value={name}
                autoFocus
                maxLength={MaxBoardName}
                placeholder="Board name"
                onChange={(e) => setName(e.target.value)}
              />
              <Input
                id="new-board-tag"
                value={typedTag ? tag : t}
                maxLength={MaxBoardTag}
                placeholder="tag"
                className="font-mono"
                onChange={(e) => {
                  setTypedTag(true)
                  setTag(e.target.value.toLowerCase())
                }}
              />
              <p
                className={cn(
                  "col-span-2 text-xs",
                  t && !validTag(t) ? "text-warning" : "text-muted-foreground"
                )}
              >
                {t && !validTag(t)
                  ? `A tag is a lowercase letter, then letters, digits or '-' (${MaxBoardTag} at most).`
                  : `Its cards are ${t || "tag"}#1, ${t || "tag"}#2, …`}
              </p>
            </div>
            <MissionFields
              id="new-board"
              mission={mission}
              doneWhen={doneWhen}
              onMission={setMission}
              onDoneWhen={setDoneWhen}
            />
            {error && (
              <p className="text-xs text-destructive" role="alert">
                {plainLine(error)}
              </p>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                disabled={busy}
                onClick={() => setOpen(false)}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={busy || !name.trim() || !validTag(t)}
              >
                {busy ? "Making it…" : "Make the board"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}

// ---- edit mission -------------------------------------------------------------

function MissionDialog({
  board: x,
  personal,
  open,
  onOpenChange,
  save,
}: {
  board: SpaceBoard
  // One of Personal's boards: nobody else edits it.
  personal: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  save: (mission: string, doneWhen: string) => Promise<void>
}) {
  const [mission, setMission] = useState("")
  const [doneWhen, setDoneWhen] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // Each opening starts from the board as it is now.
  const [was, setWas] = useState(false)
  if (open !== was) {
    setWas(open)
    if (open) {
      setMission(x.mission ?? "")
      setDoneWhen(x.doneWhen ?? "")
      setError(null)
    }
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await save(mission, doneWhen)
      onOpenChange(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogContent className="sm:max-w-lg">
        <form className="grid gap-3" onSubmit={(e) => void submit(e)}>
          <DialogHeader>
            <DialogTitle>
              {plainLine(x.name) || "This board"}'s mission
            </DialogTitle>
            <DialogDescription>
              What the board is for, and how to tell its project is over.
              {personal ? "" : " Anyone in the space can change it."}
            </DialogDescription>
          </DialogHeader>
          <MissionFields
            id="edit-mission"
            mission={mission}
            doneWhen={doneWhen}
            onMission={setMission}
            onDoneWhen={setDoneWhen}
            autoFocus
          />
          {error && (
            <p className="text-xs text-destructive" role="alert">
              {plainLine(error)}
            </p>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              disabled={busy}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

// ---- the top of a board -------------------------------------------------------

// BoardTop sits over a board's cards: its mission and done-when (or a
// quiet Add a mission), "Looks finished?" when it does, and on a finished
// board when it finished and Reopen. It holds the board's Edit mission and
// Finish dialogs, which its header's ⋯ menu opens too.
export function BoardTop({
  space,
  board: x,
  boards,
  open,
  doneScope,
  onBoard,
  now,
  onOpen,
}: {
  // The space's id, or Personal.
  space: string
  board: SpaceBoard
  // The space's (or Personal's) live boards, the move's targets.
  boards: SpaceBoard[]
  // The board's open cards.
  open: CardSummary[]
  // useDoneCards' scope for the board's space, and which of its done cards
  // are on this board.
  doneScope: string
  onBoard: (cards: CardSummary[]) => CardSummary[]
  now: Date
  onOpen: (id: string) => void
}) {
  const [editing, setEditing] = useState(
    () => previewFlag("mission") === x.boardId
  )
  const [finishing, setFinishing] = useState(
    () => previewFlag("finish") === x.boardId && !x.finished
  )
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    const on = (e: Event) => {
      const a = (e as CustomEvent<Ask>).detail
      if (a.boardId !== x.boardId) return
      if (a.kind === "mission") setEditing(true)
      else setFinishing(true)
    }
    window.addEventListener(AskEvent, on)
    return () => window.removeEventListener(AskEvent, on)
  }, [x.boardId])
  const mission = (x.mission ?? "").trim()
  const doneWhen = (x.doneWhen ?? "").trim()
  const finishedAt = x.finishedAt ? new Date(x.finishedAt) : null
  const reopen = async () => {
    setError(null)
    try {
      await actions.reopen(space, x.boardId)
    } catch (err) {
      setError(errorText(err))
    }
  }
  return (
    <div className="grid min-w-0 gap-1 px-1">
      {x.finished && (
        <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px]">
          <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
            Finished
            {finishedAt && !Number.isNaN(finishedAt.getTime())
              ? ` ${relativeTime(finishedAt, now)}`
              : ""}
          </span>
          <span className="text-muted-foreground">
            Read-only: reopen it to add or move cards here.
          </span>
          {x.mayFinish && (
            <Button
              size="sm"
              variant="outline"
              className="h-7"
              onClick={() => void reopen()}
            >
              Reopen board
            </Button>
          )}
        </p>
      )}
      {mission ? (
        <div className="grid max-w-3xl gap-0.5">
          <p className="text-[13px] text-ink-2">
            {plainLine(mission)}
            {!x.finished && (
              <>
                {" "}
                <button
                  type="button"
                  className="text-xs text-muted-foreground underline-offset-2 hover:text-ink hover:underline"
                  onClick={() => setEditing(true)}
                >
                  Edit
                </button>
              </>
            )}
          </p>
          {doneWhen && (
            <p className="text-xs text-muted-foreground">
              Done when: {plainLine(doneWhen)}
            </p>
          )}
        </div>
      ) : (
        !x.finished && (
          <button
            type="button"
            className="justify-self-start text-xs text-muted-foreground underline-offset-2 hover:text-ink hover:underline"
            onClick={() => setEditing(true)}
          >
            Add a mission
          </button>
        )
      )}
      {x.mayFinish && !x.finished && open.length === 0 && (
        <LooksFinished
          scope={doneScope}
          onBoard={onBoard}
          doneWhen={doneWhen}
          now={now}
          onFinish={() => setFinishing(true)}
        />
      )}
      {error && (
        <p className="text-xs text-destructive" role="alert">
          {plainLine(error)}
        </p>
      )}
      {!x.finished && (
        <MissionDialog
          board={x}
          personal={space === Personal}
          open={editing}
          onOpenChange={setEditing}
          save={(m, d) => actions.setMission(space, x.boardId, m, d)}
        />
      )}
      {x.mayFinish && !x.finished && (
        <FinishDialog
          space={space}
          board={x}
          boards={boards}
          open={open}
          shown={finishing}
          onShown={setFinishing}
          onOpenCard={onOpen}
        />
      )}
    </div>
  )
}

// LooksFinished is the one quiet line a board whose cards are all done
// shows (card #690). It reads the space's newest done cards: a board whose
// done cards are all older than the space's first page never shows it,
// which only leaves a suggestion out.
function LooksFinished({
  scope,
  onBoard,
  doneWhen,
  now,
  onFinish,
}: {
  scope: string
  onBoard: (cards: CardSummary[]) => CardSummary[]
  doneWhen: string
  now: Date
  onFinish: () => void
}) {
  const done = useDoneCards(true, scope)
  if (done.loading || !looksFinished(onBoard(done.cards), doneWhen, now))
    return null
  return (
    <p className="text-xs text-muted-foreground">
      Looks finished?{" "}
      <button
        type="button"
        className="text-primary underline-offset-2 hover:underline"
        onClick={onFinish}
      >
        Finish board
      </button>
    </p>
  )
}

// ---- Finish -------------------------------------------------------------------

// FinishDialog finishes a board once none of its cards is open: it lists
// the open ones, each to open and close, and moves them all to a board the
// person picks. A subtask can't move alone, so its parent moves, with its
// family.
function FinishDialog({
  space,
  board: x,
  boards,
  open,
  shown,
  onShown,
  onOpenCard,
}: {
  space: string
  board: SpaceBoard
  boards: SpaceBoard[]
  open: CardSummary[]
  shown: boolean
  onShown: (shown: boolean) => void
  onOpenCard: (id: string) => void
}) {
  const targets = boards.filter((b) => b.boardId !== x.boardId)
  const [to, setTo] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const target = targets.find((b) => b.boardId === to) ?? targets[0]
  // One move per family: a subtask's parent carries it.
  const movers = [...new Set(open.map((c) => c.parent?.id ?? c.id))]
  const moveAll = async () => {
    if (!target) return
    setBusy(true)
    setError(null)
    try {
      for (const id of movers) await cards.move(id, target.boardId)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const finish = async () => {
    setBusy(true)
    setError(null)
    try {
      await actions.finish(space, x.boardId)
      onShown(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const n = open.length
  return (
    <Dialog
      open={shown}
      onOpenChange={(o) => {
        if (busy) return
        if (o) setError(null)
        onShown(o)
      }}
    >
      <DialogContent className="max-h-[85vh] gap-3 overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Finish {plainLine(x.name) || "this board"}?</DialogTitle>
          <DialogDescription>
            A finished board is read-only and leaves the lists. It waits under
            Archived boards with its cards, and can be reopened.
          </DialogDescription>
        </DialogHeader>
        {n > 0 ? (
          <div className="grid min-w-0 gap-2">
            <p className="text-[13px]">
              {n === 1
                ? "1 card on it is still open. Move it to another board, or open it to close it."
                : `${n} cards on it are still open. Move them to another board, or open one to close it.`}
            </p>
            <ul className="grid min-w-0 gap-1">
              {open.map((c) => (
                <li
                  key={c.id}
                  className="flex min-w-0 items-center gap-2 text-[13px]"
                >
                  <span className="shrink-0 font-mono text-xs text-muted-foreground">
                    {cardRef(c)}
                  </span>
                  <span className="min-w-0 flex-1 truncate">
                    {plainLine(c.title)}
                    {c.parent && (
                      <span className="text-xs text-muted-foreground">
                        {" "}
                        · moves with its parent
                      </span>
                    )}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 shrink-0"
                    onClick={() => {
                      onShown(false)
                      onOpenCard(c.id)
                    }}
                  >
                    Open
                  </Button>
                </li>
              ))}
            </ul>
            {target ? (
              <div className="flex flex-wrap items-end gap-2">
                <ChoiceSelect
                  label={`Move ${n === 1 ? "it" : `all ${n}`} to`}
                  showLabel
                  size="sm"
                  className="min-w-40"
                  value={target.boardId}
                  onValueChange={setTo}
                  choices={targets.map((b) => ({
                    value: b.boardId,
                    label: plainLine(b.name),
                  }))}
                />
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() => void moveAll()}
                >
                  {n === 1 ? "Move 1 open card" : `Move ${n} open cards`}
                </Button>
              </div>
            ) : (
              <p className="text-xs text-muted-foreground">
                There is no other board to move them to: close them first.
              </p>
            )}
          </div>
        ) : (
          <p className="text-[13px]">
            Every card on it is done. Its cards and threads stay as they are,
            and links to them still work.
          </p>
        )}
        {error && (
          <p className="text-xs text-destructive" role="alert">
            {plainLine(error)}
          </p>
        )}
        <DialogFooter>
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => onShown(false)}
          >
            Cancel
          </Button>
          <Button disabled={busy || n > 0} onClick={() => void finish()}>
            Finish board
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// ---- Archived boards ----------------------------------------------------------

// ArchivedFold is the finished boards under a space's tiles (card #690):
// one quiet line, "Archived boards · N", until it is opened (?archived opens
// it in the preview).
export function ArchivedFold({
  count,
  children,
}: {
  count: number
  children: ReactNode
}) {
  const [open, setOpen] = useState(() => previewFlag("archived") !== null)
  if (count === 0) return null
  return (
    <section className="grid min-w-0 gap-2" aria-label="Archived boards">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 justify-self-start px-1 text-xs text-muted-foreground outline-none hover:text-ink focus-visible:ring-1 focus-visible:ring-ring"
      >
        <ChevronRightIcon
          aria-hidden
          className={cn("size-3 transition-transform", open && "rotate-90")}
        />
        Archived boards · {count}
      </button>
      {open && children}
    </section>
  )
}
