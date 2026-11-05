import { useEffect, useState } from "react"
import { ArrowRightLeftIcon, Share2Icon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { boards as actions, type BoardSummary, type SharePreview, type SpaceBoard } from "@/lib/boards"
import { cards } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import type { Owner, OwnerMember } from "@/lib/owners"
import { errorText } from "@/lib/talk"

// ShareButton is Share to board on a private card's page (v0.5 §5.2, §8):
// pick a board, see exactly what will be written there after scrub and the
// stricter pass, and who will see it, then Share. The private card closes
// and its page moves to the shared one.
export function ShareButton({
  cardId,
  boards,
  onShared,
  initialOpen = false,
}: {
  cardId: string
  boards: BoardSummary[]
  onShared: (sharedId: string) => void
  initialOpen?: boolean
}) {
  const ready = boards.filter((b) => b.ready)
  const [open, setOpen] = useState(initialOpen)
  const [board, setBoard] = useState(ready[0]?.boardId ?? "")
  // The preview and its error are kept with the board they are for, so a
  // new pick shows nothing stale while its own preview loads.
  const [got, setGot] = useState<{ board: string; preview?: SharePreview; error?: string } | null>(null)
  const [failed, setFailed] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // again bumps after a refused share, so the preview is made afresh and
  // the person confirms what the card says now.
  const [again, setAgain] = useState(0)
  const pick = board || ready[0]?.boardId || ""
  const preview = got?.board === pick ? (got.preview ?? null) : null
  const error = failed ?? (got?.board === pick ? (got.error ?? null) : null)

  useEffect(() => {
    if (!open || !pick) return
    let live = true
    actions
      .preview(cardId, pick)
      .then((p) => live && setGot({ board: pick, preview: p }))
      .catch((err) => live && setGot({ board: pick, error: errorText(err) }))
    return () => {
      live = false
    }
  }, [open, pick, cardId, again])

  if (!ready.length) return null
  const share = async () => {
    if (!preview) return
    setBusy(true)
    setFailed(null)
    try {
      const id = await actions.share(cardId, pick, preview.hash)
      setOpen(false)
      onShared(id)
    } catch (err) {
      setFailed(errorText(err))
      setAgain((n) => n + 1)
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="h-7 gap-1.5 text-xs"
        title="Share to board: put a scrubbed copy on one of your boards"
        onClick={() => setOpen(true)}
      >
        <Share2Icon className="size-3.5" aria-hidden />
        Share to board
      </Button>
      <Dialog open={open} onOpenChange={(o) => !busy && setOpen(o)}>
        <DialogContent className="gap-3 sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Share to board</DialogTitle>
            <DialogDescription>
              This is exactly what goes on the board. The card's comments,
              notes and blockers stay private, and this private card
              {preview?.subtasks.length ? " and its subtasks close" : " closes"}.
            </DialogDescription>
          </DialogHeader>
          {ready.length > 1 && (
            <ChoiceSelect
              label="Board"
              showLabel
              size="sm"
              value={pick}
              onValueChange={setBoard}
              choices={ready.map((b) => ({ value: b.boardId, label: plainLine(b.name) }))}
            />
          )}
          {preview ? <Preview p={preview} /> : !error && (
            <p className="text-xs text-muted-foreground">Preparing the preview…</p>
          )}
          {error && <p className="text-xs text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="ghost" size="sm" disabled={busy} onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button size="sm" disabled={busy || !preview} onClick={() => void share()}>
              {busy ? "Sharing…" : `Share to ${plainLine(preview?.boardName ?? "")}`}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

// Preview is the scrubbed card as the board will hold it.
function Preview({ p }: { p: SharePreview }) {
  return (
    <div className="grid gap-2 rounded-lg border border-border bg-muted/30 p-3 text-[13px]">
      <p className="font-semibold text-ink">{plainLine(p.title)}</p>
      {p.goal && <p className="whitespace-pre-wrap text-ink-2">{p.goal}</p>}
      {p.doneWhen && (
        <p className="text-xs text-muted-foreground">
          <span className="font-medium text-ink-2">Done when:</span> {plainLine(p.doneWhen)}
        </p>
      )}
      {p.steps.length > 0 && (
        <ol className="grid list-decimal gap-0.5 pl-5 text-xs text-ink-2">
          {p.steps.map((s, i) => (
            <li key={i}>{plainLine(s)}</li>
          ))}
        </ol>
      )}
      {p.links.length > 0 && (
        <p className="truncate font-mono text-xs text-muted-foreground" title={p.links.join("\n")}>
          {p.links.length === 1 ? plainLine(p.links[0]) : `${p.links.length} links`}
        </p>
      )}
      {p.subtasks.length > 0 && (
        <div className="grid gap-1 border-t border-border pt-2">
          <p className="text-xs font-medium text-ink-2">
            {p.subtasks.length === 1 ? "Its subtask goes with it" : `Its ${p.subtasks.length} subtasks go with it`}
          </p>
          <ul className="grid gap-1 pl-3" aria-label="Subtasks that go with it">
            {p.subtasks.map((s) => (
              <li key={s.taskId} className="grid gap-0.5 border-l-2 border-border pl-2">
                <span className="text-ink">
                  <span className="mr-1.5 font-mono text-xs text-muted-foreground">{plainLine(s.ref)}</span>
                  {plainLine(s.title)}
                </span>
                {s.steps.length > 0 && (
                  <span className="text-xs text-muted-foreground">
                    {s.steps.length === 1 ? "1 step" : `${s.steps.length} steps`}
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {p.keptPrivate.length > 0 && (
        <p className="text-xs text-muted-foreground">
          <span className="font-medium text-ink-2">Not shared (ended):</span>{" "}
          {p.keptPrivate.map(plainLine).join("; ")}
        </p>
      )}
      <p className="border-t border-border pt-2 text-xs text-ink-2">
        <span className="font-medium">Visible to:</span> {p.visibleTo.map(plainLine).join(", ")}
      </p>
      {p.dropped.length > 0 && (
        <p className="text-xs text-warning">
          Taken out: {p.dropped.map(plainLine).join("; ")}.
        </p>
      )}
    </div>
  )
}

// WithdrawButton takes a shared card back off its board (§5.3). Nothing
// can be recalled from devices that already have it.
export function WithdrawButton({
  cardId,
  visibleTo,
  family,
  onWithdrawn,
}: {
  cardId: string
  visibleTo: string[]
  // A parent: its subtasks come off the board with it (card #180).
  family?: boolean
  onWithdrawn: (privateId: string) => void
}) {
  return (
    <ConfirmButton
      label="Withdraw"
      confirmLabel="Withdraw it"
      question={
        <>
          Take this card{family && " and its subtasks"} off the board?{" "}
          {visibleTo.map(plainLine).join(", ")}'s devices already have what was
          shared; nothing can be recalled. The work carries on in private
          {family ? " cards" : " a private card"}.
        </>
      }
      onConfirm={async () => onWithdrawn(await actions.withdraw(cardId))}
    />
  )
}

// TransferButton changes whose a shared card is (card #432): its owner
// offers it to another member, and anyone else asks the owner for it. The
// other side gets an Accept/Decline question; the card moves only on
// Accept. The owner decides Done and answers the questions its agents
// raise.
export function TransferButton({
  cardId,
  owner,
  members,
  onAsked,
  initialOpen = false,
}: {
  cardId: string
  // The card's owner now, null when this device can't say.
  owner: Owner | null
  // The board's members.
  members: OwnerMember[]
  onAsked?: () => void
  initialOpen?: boolean
}) {
  const others = members.filter((m) => m.personId !== owner?.personId)
  const [open, setOpen] = useState(initialOpen)
  const [to, setTo] = useState(others[0]?.personId ?? "")
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  if (!owner || !others.length) return null
  const offering = owner.me
  const pick = to || others[0]?.personId || ""
  const pickName = plainLine(others.find((m) => m.personId === pick)?.name ?? "")
  const ownerName = plainLine(owner.name)
  const send = async () => {
    setBusy(true)
    setFailed(null)
    try {
      await actions.transferOwner(cardId, offering ? pick : "")
      setOpen(false)
      onAsked?.()
    } catch (err) {
      setFailed(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="h-7 gap-1.5 text-xs"
        title={
          offering
            ? "Transfer ownership: hand this card to another member"
            : `Ask ${ownerName} to hand this card over to you`
        }
        onClick={() => setOpen(true)}
      >
        <ArrowRightLeftIcon className="size-3.5" aria-hidden />
        {offering ? "Transfer ownership…" : "Ask to own it…"}
      </Button>
      <Dialog open={open} onOpenChange={(o) => !busy && setOpen(o)}>
        <DialogContent className="gap-3 sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{offering ? "Transfer ownership" : "Ask to own this card"}</DialogTitle>
            <DialogDescription>
              {offering
                ? "The card's owner decides Done and answers the questions its agents raise. They get the question, and the card is theirs only once they accept."
                : `${ownerName} owns it: they decide Done and answer its questions. They get the question, and the card is yours only once they accept.`}
            </DialogDescription>
          </DialogHeader>
          {offering && others.length > 1 && (
            <ChoiceSelect
              label="Hand it to"
              showLabel
              size="sm"
              value={pick}
              onValueChange={setTo}
              choices={others.map((m) => ({ value: m.personId, label: plainLine(m.name) }))}
            />
          )}
          {failed && <p className="text-xs text-destructive">{failed}</p>}
          <DialogFooter>
            <Button variant="ghost" size="sm" disabled={busy} onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button size="sm" disabled={busy || (offering && !pick)} onClick={() => void send()}>
              {busy ? "Asking…" : offering ? `Offer it to ${pickName}` : `Ask ${ownerName}`}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

// MoveBoardButton puts a card on another board in its space (card #563).
// It takes the next number there; its old ref still finds it. A parent
// takes its subtasks along.
export function MoveBoardButton({
  id,
  boards,
  tag,
  family,
  initialOpen = false,
}: {
  id: string
  boards: SpaceBoard[]
  // The card's board now, by tag.
  tag: string
  family: boolean
  initialOpen?: boolean
}) {
  const others = boards.filter((b) => b.tag !== tag)
  const [open, setOpen] = useState(initialOpen)
  const [to, setTo] = useState(others[0]?.boardId ?? "")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const move = async () => {
    setBusy(true)
    setError(null)
    try {
      await cards.move(id, to)
      setOpen(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <Button variant="ghost" size="sm" onClick={() => setOpen(true)}>
        Move
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Move to another board</DialogTitle>
            <DialogDescription>
              It takes the next number on that board. Links to its old number
              still find it.{family ? " Its subtasks move with it." : ""}
            </DialogDescription>
          </DialogHeader>
          <ChoiceSelect
            label="Board"
            showLabel
            size="sm"
            value={to}
            onValueChange={setTo}
            choices={others.map((b) => ({ value: b.boardId, label: plainLine(b.name) }))}
          />
          {error && (
            <p className="text-xs text-destructive" role="alert">
              {plainLine(error)}
            </p>
          )}
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button size="sm" disabled={busy || !to} onClick={() => void move()}>
              Move
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
