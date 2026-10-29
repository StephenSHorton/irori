import { Badge } from "@/components/ui/badge"
import {
  displayName,
  Holding,
  percent,
  TaskState,
  type CardHolder,
  type CardSummary,
} from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { cn } from "@/lib/utils"

// ProgressBar is a card's done steps over all steps; amber while the card
// waits on a decision. Cards without steps show no bar.
export function ProgressBar({
  progress,
  blocked,
  label,
}: {
  progress: { done: number; total: number }
  blocked?: boolean
  label: string
}) {
  const p = percent(progress)
  if (p === null) return null
  return (
    <span
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={p}
      className="block h-1.5 overflow-hidden rounded-full bg-muted ring-1 ring-border ring-inset"
    >
      <span
        className={cn(
          "block h-full rounded-full",
          blocked ? "bg-warning" : "bg-primary"
        )}
        style={{ width: `${p}%` }}
      />
    </span>
  )
}

// StatePill is the card's first badge: waiting on the person's input, its
// blockers, else in progress.
export function StatePill({ card }: { card: CardSummary }) {
  if (card.state === TaskState.InputRequired)
    return (
      <Badge variant="destructive" className="rounded-full">
        Needs your input
      </Badge>
    )
  if (card.openBlockers > 0)
    return (
      <Badge variant="destructive" className="rounded-full">
        {card.openBlockers} {card.openBlockers === 1 ? "blocker" : "blockers"}
      </Badge>
    )
  if (card.state === TaskState.Completed)
    return (
      <Badge variant="success" className="rounded-full">
        Done
      </Badge>
    )
  if (card.state === TaskState.Canceled || card.state === TaskState.Failed)
    return (
      <Badge variant="slate" className="rounded-full">
        {card.state === TaskState.Failed ? "Failed" : "Canceled"}
      </Badge>
    )
  if (card.working.length === 0)
    return (
      <Badge variant="slate" className="rounded-full">
        Open
      </Badge>
    )
  if (card.holder.state === Holding.HoldWaiting)
    return (
      <Badge variant="warning" className="rounded-full">
        Not picked up
      </Badge>
    )
  if (card.holder.state === Holding.HoldAssigned)
    return (
      <Badge variant="slate" className="rounded-full">
        Assigned
      </Badge>
    )
  return (
    <Badge variant="info" className="rounded-full">
      In progress
    </Badge>
  )
}

// holderText is the holder line's words: who holds the card and what that
// agent is up to, or who it waits for.
export function holderText(h: CardHolder): string {
  const who = h.who.key ? displayName(h.who) : ""
  const more = h.others > 0 ? ` (+${h.others} more)` : ""
  switch (h.state) {
    case Holding.HoldNobody:
    case Holding.$zero:
      return "Nobody holds it yet"
    case Holding.HoldPerson:
      return `You hold it${more}`
    case Holding.HoldWaiting:
      return `Waiting for ${who}${more}: the hand-over hasn't reached it yet`
    case Holding.HoldAssigned:
      return `Assigned to ${who}${more}, not taken up yet`
    case Holding.HoldActive:
      return `${who}${more} · active`
    case Holding.HoldIdle:
      return `${who}${more} · idle`
    case Holding.HoldOffline:
      return `${who}${more} · its device is offline`
    case Holding.HoldGone:
      return `${who}${more} · gone`
  }
  return who
}

// HolderLine stands in for the progress bar on a card without steps: who
// holds it and whether that agent is active, idle or gone, or "Waiting
// for" an agent the assignment hasn't reached, so a card nobody is
// working doesn't look like work in progress.
export function HolderLine({
  holder,
  className,
}: {
  holder: CardHolder
  className?: string
}) {
  const warn =
    holder.state === Holding.HoldWaiting ||
    holder.state === Holding.HoldGone ||
    holder.state === Holding.HoldOffline
  return (
    <span
      className={cn(
        "flex min-w-0 items-center gap-1.5 text-[12.5px]",
        warn ? "text-warning" : "text-muted-foreground",
        className
      )}
    >
      {holder.state === Holding.HoldActive && (
        <span
          className="inline-block size-[7px] shrink-0 rounded-full bg-success"
          aria-hidden
        />
      )}
      <span className="truncate">{holderText(holder)}</span>
    </span>
  )
}

// ScopeBadge says where the card lives: "My devices" until Shared Boards.
export function ScopeBadge({ card }: { card: CardSummary }) {
  return (
    <Badge
      variant="outline"
      className="rounded-full bg-muted/60 font-normal text-muted-foreground"
    >
      {plainLine(card.scopeLabel) || "My devices"}
    </Badge>
  )
}

// ShortRef is the card's short ref, "#12": what the person says or types to
// an agent about it. A card renumbered when two devices that were apart
// both used its number says what it was.
export function ShortRef({ card }: { card: CardSummary }) {
  const short = plainLine(card.short)
  if (!short) return null
  const was = plainLine(card.shortWas)
  return (
    <span
      className="font-mono text-xs text-muted-foreground tabular-nums"
      title={
        was
          ? `Was #${was}: a card made earlier on another device had #${was} too`
          : `Agents know this card as #${short}`
      }
    >
      #{short}
      {was && <span className="ml-1 font-sans">(was #{was})</span>}
    </span>
  )
}
