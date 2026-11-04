import { useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { parseTime, plainBlock, plainLine, relativeTime } from "@/lib/irori"
import {
  allowLifts,
  errorText,
  holdText,
  HoldReason,
  State,
  SystemKey,
  talk,
  startedLine,
  useRepoNames,
  type Delivery,
  type HeldState,
} from "@/lib/talk"

export type HeldTab = "waiting" | "history"

// HeldDialog is the Held list: every message Irori is holding back until
// the person decides (Pause all's too), each saying why in plain words
// (card #488), with Release, Dismiss and Allow sender & deliver, or Start
// an agent for their own message to a repo nobody is in; and its History, the ones
// already decided, with who decided and when, where a dismissed one can
// still be released. from, when set, shows only that sender's (a held
// notification's Open). cardLabel names the card a message is about
// ("#28"), "" when it names none; onCard opens it.
export function HeldDialog({
  state,
  tab,
  onTab,
  from,
  onFrom,
  name,
  cardLabel,
  onCard,
  now,
  onClose,
}: {
  state: HeldState
  tab: HeldTab
  onTab: (t: HeldTab) => void
  from: string
  onFrom: (key: string) => void
  name: (key: string) => string
  cardLabel: (ref: string) => string
  onCard: (id: string) => void
  now: Date
  onClose: () => void
}) {
  const [busy, setBusy] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [said, setSaid] = useState("")
  const repo = useRepoNames()

  const act = async (id: string, fn: () => Promise<void>) => {
    if (busy) return
    setBusy(id)
    setError(null)
    setSaid("")
    try {
      await fn()
      await state.reload()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy("")
    }
  }

  const sender = (d: Delivery) =>
    plainLine(d.message.from.name) || name(d.message.from.key)
  const list = (tab === "waiting" ? state.held : state.history).filter(
    (d) => !from || d.message.from.key === from
  )
  const fromName = from
    ? sender(
        [...state.held, ...state.history].find(
          (d) => d.message.from.key === from
        ) ?? ({ message: { from: { key: from, name: "" } } } as Delivery)
      )
    : ""

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="gap-3 sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Held messages</DialogTitle>
          <DialogDescription>
            Messages Irori is holding back until you say, and why. Release
            delivers one; Dismiss drops it, and History can still release it.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-wrap items-center gap-1">
          <div
            className="flex items-center gap-1"
            role="group"
            aria-label="Held list"
          >
            <Button
              size="sm"
              aria-pressed={tab === "waiting"}
              variant={tab === "waiting" ? "default" : "ghost"}
              onClick={() => onTab("waiting")}
            >
              Waiting
              {state.held.length > 0 && (
                <span
                  className={
                    tab === "waiting"
                      ? "text-xs tabular-nums opacity-80"
                      : "text-xs text-destructive tabular-nums"
                  }
                >
                  {state.held.length}
                </span>
              )}
            </Button>
            <Button
              size="sm"
              aria-pressed={tab === "history"}
              variant={tab === "history" ? "default" : "ghost"}
              onClick={() => onTab("history")}
            >
              History
            </Button>
          </div>
          {from && (
            <Button
              size="sm"
              variant="outline"
              className="ml-auto"
              title="Show every sender's"
              onClick={() => onFrom("")}
            >
              From {fromName}
              <span aria-hidden>×</span>
              <span className="sr-only">: show every sender's</span>
            </Button>
          )}
        </div>
        {list.length ? (
          <ul
            className="grid max-h-[60vh] overflow-y-auto"
            aria-label={tab === "waiting" ? "Waiting on you" : "Decided"}
          >
            {list.map((d) => (
              <HeldRow
                key={d.id}
                d={d}
                from={sender(d)}
                name={name}
                repo={repo}
                cardLabel={cardLabel}
                onCard={onCard}
                now={now}
                busy={busy === d.id}
                disabled={!!busy}
                onAct={(fn) => void act(d.id, fn)}
                onStarted={setSaid}
              />
            ))}
          </ul>
        ) : (
          <p className="py-2 text-[12.5px] text-muted-foreground">
            {tab === "waiting"
              ? from
                ? `Nothing from ${fromName} is held now.`
                : "Nothing is held now."
              : from
                ? `No past held messages from ${fromName}.`
                : "No past held messages."}
          </p>
        )}
        {said && !error && (
          <p role="status" className="text-xs text-muted-foreground">
            {said}
          </p>
        )}
        {(error || state.error) && (
          <p className="text-xs text-destructive">{error || state.error}</p>
        )}
      </DialogContent>
    </Dialog>
  )
}

function HeldRow({
  d,
  from,
  name,
  repo,
  cardLabel,
  onCard,
  now,
  busy,
  disabled,
  onAct,
  onStarted,
}: {
  d: Delivery
  from: string
  name: (key: string) => string
  repo: (key: string) => string
  cardLabel: (ref: string) => string
  onCard: (id: string) => void
  now: Date
  busy: boolean
  disabled: boolean
  onAct: (fn: () => Promise<void>) => void
  onStarted: (said: string) => void
}) {
  const held = d.state === State.Held
  const to = d.to.startsWith("user:")
    ? "you"
    : d.to
      ? name(d.to)
      : plainLine(repo(d.repoKey ?? "")) || "a repo"
  const unowned = d.hold === HoldReason.HoldUnowned && !d.to
  const mine = d.message.from.key.startsWith("user:")
  const where = plainLine(repo(d.repoKey ?? "")) || "its repo"
  const sent = parseTime(d.message.sentAt)
  const ref = d.message.ref ?? ""
  const card = ref.startsWith("task:") ? cardLabel(ref) : ""
  const at = parseTime(d.decidedAt || d.updatedAt)
  // Moved straight out of Held (to another agent): its last row is the
  // decision, so nothing was released.
  const movedHeld =
    d.state === State.Moved &&
    at?.getTime() === parseTime(d.updatedAt)?.getTime()
  const by =
    d.decidedBy === SystemKey ? "Irori" : d.decidedBy ? name(d.decidedBy) : ""

  return (
    <li className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1 border-b border-border py-2.5 text-[13px] last:border-b-0">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5">
        <span className="font-medium text-ink">{from}</span>
        <span className="text-muted-foreground" aria-hidden>
          {"->"}
        </span>
        <span className="sr-only">to</span>
        <span className="min-w-0 font-medium break-all text-ink">{to}</span>
        {card && (
          <Button
            size="xs"
            variant="ghost"
            className="text-muted-foreground"
            title="Open the card"
            onClick={() => onCard(ref.slice("task:".length))}
          >
            {card}
          </Button>
        )}
        <time
          className="ml-auto text-xs text-muted-foreground tabular-nums"
          dateTime={sent?.toISOString()}
          title={sent?.toLocaleString()}
        >
          {relativeTime(sent, now)}
        </time>
      </div>
      <p className="text-xs text-muted-foreground">
        {plainLine(holdText(d, { name, repo }))}
      </p>
      <p className="line-clamp-4 text-[12.5px] whitespace-pre-wrap text-ink-2">
        {plainBlock(d.message.text)}
      </p>
      {held ? (
        <div className="flex flex-wrap gap-1.5 pt-0.5">
          {unowned ? (
            // Card #488: Release can't deliver it (nobody is there); the
            // person's own message can start an agent there instead, and
            // an agent's goes on by itself once one arrives.
            mine && (
              <Button
                size="xs"
                disabled={disabled}
                onClick={() =>
                  onAct(async () => {
                    onStarted(startedLine(await talk.startAgent(d.id)))
                  })
                }
              >
                {busy ? "Working…" : `Start an agent in ${where}`}
              </Button>
            )
          ) : (
            <Button
              size="xs"
              disabled={disabled}
              onClick={() => onAct(() => talk.release(d.id))}
            >
              {busy ? "Working…" : "Release"}
            </Button>
          )}
          <Button
            size="xs"
            variant="outline"
            disabled={disabled}
            onClick={() => onAct(() => talk.dismiss(d.id))}
          >
            Dismiss
          </Button>
          {allowLifts(d) && (
            <Button
              size="xs"
              variant="ghost"
              disabled={disabled}
              title={`Adds ${from} to your allow list, then delivers this`}
              onClick={() => onAct(() => talk.allowAndRelease(d.id))}
            >
              Allow sender &amp; deliver
            </Button>
          )}
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-1.5 pt-0.5 text-xs text-muted-foreground">
          <Badge
            variant={d.state === State.Dismissed ? "slate" : "success"}
            className="h-4 px-1.5 text-[10.5px]"
          >
            {d.state === State.Dismissed
              ? "Dismissed"
              : movedHeld
                ? "Moved"
                : d.decidedBy === SystemKey
                  ? "Let through"
                  : "Released"}
          </Badge>
          <span>
            {by && `by ${by} `}
            <time dateTime={at?.toISOString()} title={at?.toLocaleString()}>
              {relativeTime(at, now)}
            </time>
            {movedHeld
              ? d.movedTo && ` · to ${name(d.movedTo)}`
              : d.state !== State.Dismissed && ` · now ${d.state}`}
          </span>
          {d.state === State.Dismissed && (
            <Button
              size="xs"
              variant="outline"
              className="ml-auto"
              disabled={disabled}
              onClick={() => onAct(() => talk.release(d.id))}
            >
              {busy ? "Working…" : "Release"}
            </Button>
          )}
        </div>
      )}
    </li>
  )
}
