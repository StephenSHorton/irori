// Board descriptions and closing (card #690, docs/design/board.md "Board
// descriptions and closing"): a board may say what it is for, and when it
// is over it can be closed, its open cards moved or canceled. Here: New
// board with its optional description, the description at the top of a
// board and its edit dialog, the Close board dialog, a closed board's
// Reopen, and the Closed boards fold under a space's tiles.

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
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Textarea } from "@/components/ui/textarea"
import {
  boards as actions,
  MaxBoardDescription,
  MaxBoardName,
  MaxBoardTag,
  suggestTag,
  validTag,
  type SpaceBoard,
} from "@/lib/boards"
import type { CardSummary } from "@/lib/cards"
import { plainBlock, plainLine, relativeTime } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import { Personal } from "@/lib/spaces"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// ---- asking the board's page from its header ---------------------------------

// The header (App's bar) and the board's page are separate trees: the
// header's ⋯ menu asks the page to open its Edit description or Close
// board dialog.
const AskEvent = "irori:board-ask"
type Ask = { kind: "description" | "close"; boardId: string }

export function askBoard(kind: Ask["kind"], boardId: string) {
  window.dispatchEvent(
    new CustomEvent<Ask>(AskEvent, { detail: { kind, boardId } })
  )
}

// ---- the description field ----------------------------------------------------

function DescriptionField({
  id,
  value,
  onChange,
  autoFocus,
}: {
  id: string
  value: string
  onChange: (s: string) => void
  autoFocus?: boolean
}) {
  const n = [...value].length
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>
        Description{" "}
        <span className="font-normal text-muted-foreground">(optional)</span>
      </Label>
      <Textarea
        id={id}
        value={value}
        autoSize
        autoFocus={autoFocus}
        maxLength={MaxBoardDescription}
        className="max-h-60"
        placeholder="What this board is for, or what it should end with (optional)"
        onChange={(e) => onChange(e.target.value)}
      />
      <p className="text-xs text-muted-foreground">
        It shows on the board and its tile, and agents working here read it.
        {n > MaxBoardDescription - 200 && ` ${n} of ${MaxBoardDescription}.`}
      </p>
    </div>
  )
}

// ---- New board ----------------------------------------------------------------

// AddBoardButton is + Add board over a space's (or Personal's) tiles, and
// the New board dialog it opens: a name, the tag its cards' refs carry
// (suggested from the name until typed), and an optional description.
export function AddBoardButton({
  onAdd,
}: {
  onAdd: (name: string, tag: string, description: string) => Promise<unknown>
}) {
  const [open, setOpen] = useState(() => previewFlag("addboard") !== null)
  const [name, setName] = useState("")
  const [tag, setTag] = useState("")
  const [typedTag, setTypedTag] = useState(false)
  const [description, setDescription] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const t = (typedTag ? tag : suggestTag(name)).trim().toLowerCase()
  const reset = () => {
    setName("")
    setTag("")
    setTypedTag(false)
    setDescription("")
    setError(null)
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await onAdd(name, t, description)
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
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
          <form className="grid gap-3" onSubmit={(e) => void submit(e)}>
            <DialogHeader>
              <DialogTitle>New board</DialogTitle>
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
            <DescriptionField
              id="new-board-description"
              value={description}
              onChange={setDescription}
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

// ---- Edit description ---------------------------------------------------------

function DescriptionDialog({
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
  save: (description: string) => Promise<void>
}) {
  const [description, setDescription] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // Each opening starts from the board as it is now.
  const [was, setWas] = useState(false)
  if (open !== was) {
    setWas(open)
    if (open) {
      setDescription(x.description ?? "")
      setError(null)
    }
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await save(description)
      onOpenChange(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Dialog open={open} onOpenChange={(o) => !busy && onOpenChange(o)}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <form className="grid gap-3" onSubmit={(e) => void submit(e)}>
          <DialogHeader>
            <DialogTitle>
              Describe {plainLine(x.name) || "this board"}
            </DialogTitle>
            <DialogDescription>
              Leave it empty for no description.
              {personal ? "" : " Anyone in the space can change it."}
            </DialogDescription>
          </DialogHeader>
          <DescriptionField
            id="edit-board-description"
            value={description}
            onChange={setDescription}
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

// longDescription is whether a description shows folded to a few lines
// with More: past a few lines or a few hundred characters.
export function longDescription(text: string): boolean {
  return text.length > 240 || text.split("\n").length > 3
}

// BoardDescription is a board's description over its cards, only when it
// has one: a long one shows a few lines, run together, and More shows it
// as written.
function BoardDescription({ text }: { text: string }) {
  const [more, setMore] = useState(false)
  const long = longDescription(text)
  const folded = long && !more
  return (
    <div className="grid max-w-3xl min-w-0 justify-items-start gap-0.5">
      <p
        className={cn(
          "text-[13px] break-words text-ink-2",
          folded ? "line-clamp-3" : "whitespace-pre-line"
        )}
      >
        {folded ? plainLine(text) : text}
      </p>
      {long && (
        <button
          type="button"
          aria-expanded={more}
          className="text-xs text-muted-foreground underline-offset-2 hover:text-ink hover:underline"
          onClick={() => setMore(!more)}
        >
          {more ? "Less" : "More"}
        </button>
      )}
    </div>
  )
}

// BoardTop sits over a board's cards: its description when it has one,
// and on a closed board when it closed and Reopen. It holds the board's
// Edit description and Close board dialogs, which its header's ⋯ menu
// opens.
export function BoardTop({
  space,
  board: x,
  boards,
  open,
  now,
  onOpen,
}: {
  // The space's id, or Personal.
  space: string
  board: SpaceBoard
  // The space's (or Personal's) open boards, where open cards can move.
  boards: SpaceBoard[]
  // The board's open cards.
  open: CardSummary[]
  now: Date
  onOpen: (id: string) => void
}) {
  const [editing, setEditing] = useState(
    () => previewFlag("description") === x.boardId && !x.closed
  )
  const [closing, setClosing] = useState(
    () => previewFlag("close") === x.boardId && !x.closed
  )
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    const on = (e: Event) => {
      const a = (e as CustomEvent<Ask>).detail
      if (a.boardId !== x.boardId) return
      if (a.kind === "description") setEditing(true)
      else setClosing(true)
    }
    window.addEventListener(AskEvent, on)
    return () => window.removeEventListener(AskEvent, on)
  }, [x.boardId])
  const description = plainBlock(x.description ?? "")
  const closedAt = x.closedAt ? new Date(x.closedAt) : null
  const reopen = async () => {
    setError(null)
    try {
      await actions.reopen(space, x.boardId)
    } catch (err) {
      setError(errorText(err))
    }
  }
  const shows = x.closed || description || error
  return (
    <>
      {shows && (
        <div className="grid min-w-0 gap-1.5 px-1">
          {x.closed && (
            <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px]">
              <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
                Closed
                {closedAt && !Number.isNaN(closedAt.getTime())
                  ? ` ${relativeTime(closedAt, now)}`
                  : ""}
              </span>
              <span className="text-muted-foreground">
                Read-only: reopen it to add or move cards here.
              </span>
              {x.mayClose && (
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
          {description && <BoardDescription text={description} />}
          {error && (
            <p className="text-xs text-destructive" role="alert">
              {plainLine(error)}
            </p>
          )}
        </div>
      )}
      {!x.closed && (
        <DescriptionDialog
          board={x}
          personal={space === Personal}
          open={editing}
          onOpenChange={setEditing}
          save={(d) => actions.setDescription(space, x.boardId, d)}
        />
      )}
      {x.mayClose && !x.closed && (
        <CloseDialog
          space={space}
          board={x}
          boards={boards}
          open={open}
          shown={closing}
          onShown={setClosing}
          onOpenCard={onOpen}
        />
      )}
    </>
  )
}

// ---- Close board --------------------------------------------------------------

// CloseDialog closes a board (card #690). With none of its cards open it
// is a plain confirm; otherwise it lists the open ones, each to open, and
// asks whether to move them to another open board (a subtask goes with its
// parent) or cancel them. One close does it all.
function CloseDialog({
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
  const targets = boards.filter((b) => b.boardId !== x.boardId && !b.closed)
  const [how, setHow] = useState<"move" | "cancel">("move")
  const [to, setTo] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const target = targets.find((b) => b.boardId === to) ?? targets[0]
  const n = open.length
  // With nowhere to move them, the only way is to cancel them.
  const choice = n === 0 ? "keep" : target ? how : "cancel"
  const close = async () => {
    setBusy(true)
    setError(null)
    try {
      await actions.close(
        space,
        x.boardId,
        choice,
        choice === "move" ? (target?.boardId ?? "") : ""
      )
      onShown(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const name = plainLine(x.name) || "this board"
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
          <DialogTitle>Close {name}?</DialogTitle>
          <DialogDescription>
            A closed board is read-only and goes under Closed boards with its
            cards. You can reopen it any time.
          </DialogDescription>
        </DialogHeader>
        {n > 0 ? (
          <div className="grid min-w-0 gap-3">
            <div className="grid min-w-0 gap-1.5">
              <p className="text-[13px]">
                {n === 1
                  ? "1 card on it is still open:"
                  : `${n} cards on it are still open:`}
              </p>
              <ul className="grid max-h-48 min-w-0 gap-1 overflow-y-auto">
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
            </div>
            <RadioGroup
              aria-label="What happens to the open cards"
              value={choice}
              onValueChange={(v) => setHow(v === "cancel" ? "cancel" : "move")}
              disabled={busy}
              className="gap-2.5"
            >
              <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5">
                <label className="flex items-center gap-2 text-[13px]">
                  <RadioGroupItem value="move" disabled={!target} />
                  {n === 1 ? "Move it to" : "Move them to"}
                </label>
                {target ? (
                  <ChoiceSelect
                    label="Board to move them to"
                    size="sm"
                    className="min-w-40"
                    value={target.boardId}
                    disabled={busy || choice !== "move"}
                    onValueChange={setTo}
                    choices={targets.map((b) => ({
                      value: b.boardId,
                      label: plainLine(b.name) || "(unnamed)",
                    }))}
                  />
                ) : (
                  <span className="text-xs text-muted-foreground">
                    (there is no other open board)
                  </span>
                )}
              </div>
              <label className="flex items-center gap-2 text-[13px]">
                <RadioGroupItem value="cancel" />
                {n === 1 ? "Cancel it" : "Cancel them"}
              </label>
            </RadioGroup>
          </div>
        ) : (
          <p className="text-[13px]">
            None of its cards is open. Its cards and threads stay as they are,
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
          <Button disabled={busy} onClick={() => void close()}>
            {busy ? "Closing…" : "Close board"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// ---- Closed boards ------------------------------------------------------------

// ClosedFold is the closed boards under a space's tiles (card #690): one
// quiet line, "Closed boards · N", until it is opened (?closed opens it in
// the preview).
export function ClosedFold({
  count,
  children,
}: {
  count: number
  children: ReactNode
}) {
  const [open, setOpen] = useState(() => previewFlag("closed") !== null)
  if (count === 0) return null
  return (
    <section className="grid min-w-0 gap-2" aria-label="Closed boards">
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
        Closed boards · {count}
      </button>
      {open && children}
    </section>
  )
}
