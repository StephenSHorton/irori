import { useState, type FormEvent } from "react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import {
  boards as actions,
  MaxBoardName,
  suggestTag,
  useBoards,
  validTag,
  type BoardSummary,
} from "@/lib/boards"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// BoardsDialog is Settings → Boards (design/v0.5-shared-boards.md §8, step
// 4): each board with its name, tag and one status dot; Rename, Leave
// (every one of your devices) and Forget (this device only); and New
// board. Boards are between your own devices until invites come; Invite,
// Join with an invite and the receive and autonomy settings come with them.
export function BoardsDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { boards, error, reload } = useBoards()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="gap-3 sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Boards</DialogTitle>
          <DialogDescription>
            Each board is its own set of cards, apart from My devices. For
            now a board is between your own devices.
          </DialogDescription>
        </DialogHeader>
        {boards.length > 0 ? (
          <ul className="grid gap-2" aria-label="Your boards">
            {boards.map((b) => (
              <BoardRow key={b.boardId} board={b} onChanged={reload} />
            ))}
          </ul>
        ) : (
          <p className="text-[13px] text-muted-foreground">No boards yet.</p>
        )}
        {error && <p className="text-xs text-destructive">{error}</p>}
        <NewBoard taken={boards.map((b) => b.tag)} onMade={reload} />
      </DialogContent>
    </Dialog>
  )
}

function BoardRow({
  board: b,
  onChanged,
}: {
  board: BoardSummary
  onChanged: () => Promise<void>
}) {
  const [renaming, setRenaming] = useState(false)
  const [name, setName] = useState(b.name)
  const [error, setError] = useState<string | null>(null)
  const rename = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    try {
      await actions.rename(b.boardId, name)
      setRenaming(false)
      await onChanged()
    } catch (err) {
      setError(errorText(err))
    }
  }
  const cards = b.cards === 1 ? "1 card" : `${b.cards} cards`
  const devices = b.devices === 1 ? "1 device" : `${b.devices} devices`
  return (
    <li className="grid grid-cols-[8px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-1 text-[13px]">
      <span
        className={cn("size-2 rounded-full", b.ready ? "bg-success" : "bg-warning")}
        aria-hidden
        title={b.ready ? "Ready" : "Its cards are still arriving from your other device"}
      />
      {renaming ? (
        <form className="flex min-w-0 gap-1.5" onSubmit={rename}>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={MaxBoardName}
            aria-label="Board name"
            className="h-7 text-[13px]"
            autoFocus
          />
          <Button type="submit" size="sm" disabled={!name.trim()}>
            Save
          </Button>
        </form>
      ) : (
        <span className="min-w-0 truncate">
          <span className="text-ink">{plainLine(b.name) || "(unnamed)"}</span>{" "}
          <span className="font-mono text-xs text-muted-foreground">{plainLine(b.tag)}#</span>{" "}
          <small className="text-xs text-muted-foreground">
            {b.ready ? `${cards} · ${devices}` : "arriving from your other device"}
          </small>
          {b.tagClash && (
            <small className="block text-xs text-destructive">
              Another board is also {plainLine(b.tag)}#: its refs need the card's id.
            </small>
          )}
        </span>
      )}
      <span className="flex items-center gap-1.5">
        {renaming ? (
          <Button variant="ghost" size="sm" onClick={() => setRenaming(false)}>
            Cancel
          </Button>
        ) : (
          b.owner && (
            <Button variant="ghost" size="sm" onClick={() => setRenaming(true)}>
              Rename
            </Button>
          )
        )}
        <ConfirmButton
          label="Leave"
          variant="destructive"
          confirmLabel="Leave the board"
          question={
            <>
              Leave <b>{plainLine(b.name)}</b>? It goes from every one of your
              devices, with its cards. Coming back needs a new board.
            </>
          }
          onConfirm={async () => {
            await actions.leave(b.boardId)
            await onChanged()
          }}
        />
      </span>
      <span className="col-start-2 col-end-4 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="min-w-0 flex-1">
          Visible to: {b.people.map(plainLine).join(", ")}
        </span>
        <ConfirmButton
          label="Forget here"
          confirmLabel="Forget on this device"
          question={
            <>
              Forget <b>{plainLine(b.name)}</b> on this device only? Its file
              goes from here; your other devices keep it.
            </>
          }
          onConfirm={async () => {
            await actions.forget(b.boardId)
            await onChanged()
          }}
        />
      </span>
      {error && <span className="col-start-2 col-end-4 text-xs text-destructive">{error}</span>}
    </li>
  )
}

// NewBoard makes a board: a name, and the short tag its cards' refs carry
// ("mlg#12"), suggested from the name until it is typed.
function NewBoard({ taken, onMade }: { taken: string[]; onMade: () => Promise<void> }) {
  const [name, setName] = useState("")
  const [tag, setTag] = useState("")
  const [typedTag, setTypedTag] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const t = (typedTag ? tag : suggestTag(name)).trim().toLowerCase()
  const clash = taken.includes(t)
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      await actions.create(name, t)
      setName("")
      setTag("")
      setTypedTag(false)
      await onMade()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="grid gap-2 border-t border-border pt-3" onSubmit={submit}>
      <span className="text-[13px] font-semibold text-ink">New board</span>
      <div className="flex gap-1.5">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          maxLength={MaxBoardName}
          placeholder="Name (MLG)"
          aria-label="New board's name"
          className="h-8 min-w-0 flex-1 text-[13px]"
        />
        <Input
          value={typedTag ? tag : t}
          onChange={(e) => {
            setTypedTag(true)
            setTag(e.target.value.toLowerCase())
          }}
          maxLength={16}
          placeholder="tag"
          aria-label="New board's tag, for refs like mlg#12"
          className="h-8 w-24 font-mono text-[13px]"
        />
        <Button type="submit" size="sm" className="h-8" disabled={busy || !name.trim() || !validTag(t) || clash}>
          Create
        </Button>
      </div>
      <p className={cn("text-xs", clash || (t && !validTag(t)) ? "text-warning" : "text-muted-foreground")}>
        {clash
          ? `Another board is tagged ${t} already.`
          : t && !validTag(t)
            ? "A tag is a lowercase letter, then letters, digits or '-' (16 at most)."
            : `Its cards are ${t || "tag"}#1, ${t || "tag"}#2, …`}
      </p>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}
