import { useEffect, useState } from "react"
import { Share2Icon } from "lucide-react"

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
import { boards as actions, type BoardSummary, type SharePreview } from "@/lib/boards"
import { plainLine } from "@/lib/irori"
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
              notes and blockers stay private, and this private card closes.
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
    <div className="grid gap-2 border border-border bg-muted/30 p-3 text-[13px]">
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
  onWithdrawn,
}: {
  cardId: string
  visibleTo: string[]
  onWithdrawn: (privateId: string) => void
}) {
  return (
    <ConfirmButton
      label="Withdraw"
      confirmLabel="Withdraw it"
      question={
        <>
          Take this card off the board? {visibleTo.map(plainLine).join(", ")}'s
          devices already have what was shared; nothing can be recalled. The
          work carries on in a private card.
        </>
      }
      onConfirm={async () => onWithdrawn(await actions.withdraw(cardId))}
    />
  )
}
