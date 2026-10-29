import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"
import {
  askLine,
  AskState,
  displayName,
  Holding,
  openAsk,
  percent,
  TaskState,
  type AgentRef,
  type CardAsk,
  type CardHolder,
  type CardSummary,
} from "@/lib/cards"
import { plainLine } from "@/lib/irori"

// Tone is a card's status colour, shared by its state chip and its progress
// bar so the two always agree.
export type Tone = "destructive" | "success" | "slate" | "info"

const barTone: Record<Tone, string> = {
  destructive: "bg-destructive",
  success: "bg-success",
  slate: "bg-muted-foreground",
  info: "bg-info",
}

// cardTone: waiting on the person or blocked is red, done green, canceled,
// failed or unheld grey, else in progress blue.
export function cardTone(card: {
  state: string
  openBlockers: number
  working: readonly unknown[]
}): Tone {
  if (card.state === TaskState.InputRequired || card.openBlockers > 0)
    return "destructive"
  if (card.state === TaskState.Completed) return "success"
  if (card.state === TaskState.Canceled || card.state === TaskState.Failed)
    return "slate"
  if (card.working.length === 0) return "slate"
  return "info"
}

// ProgressBar is a card's done steps over all steps, in the card's status
// colour. Cards without steps show no bar.
export function ProgressBar({
  progress,
  label,
  tone = "info",
}: {
  progress: { done: number; total: number }
  label: string
  tone?: Tone
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
      className="block h-1 overflow-hidden bg-muted"
    >
      <span
        className={`block h-full ${barTone[tone]} transition-[width] duration-300`}
        style={{ width: `${p}%` }}
      />
    </span>
  )
}

// StatePill is the card's first badge: waiting on the person's input, its
// blockers, waiting on an agent to act on the person's answers, else in
// progress.
export function StatePill({ card }: { card: CardSummary }) {
  // An ask: the person's prompt to the room, before it is a card.
  if (openAsk(card))
    return (
      <Badge
        variant={card.ask?.state === AskState.Unclaimed ? "warning" : "info"}
        className="rounded-full"
        title="You asked the room; one agent takes it"
      >
        Ask
      </Badge>
    )
  if (card.state === TaskState.InputRequired)
    return (
      <Badge variant="destructive">
        Needs your input
      </Badge>
    )
  if (card.openBlockers > 0)
    return (
      <Badge variant="destructive">
        {card.openBlockers} {card.openBlockers === 1 ? "blocker" : "blockers"}
      </Badge>
    )
  // Every open blocker is answered: it waits on the agents, not on you.
  if (card.waiting > 0)
    return (
      <Badge variant="slate" className="rounded-full">
        Waiting on agent
      </Badge>
    )
  if (card.state === TaskState.Completed)
    return (
      <Badge variant="success">
        Done
      </Badge>
    )
  if (card.state === TaskState.Canceled || card.state === TaskState.Failed)
    return (
      <Badge variant="slate">
        {card.state === TaskState.Failed ? "Failed" : "Canceled"}
      </Badge>
    )
  if (card.working.length === 0)
    return (
      <Badge variant="slate">
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
    <Badge variant="info">
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
  returned,
  className,
}: {
  holder: CardHolder
  // returned: the agent that went away when Irori took it off (card #16).
  returned?: AgentRef | null
  className?: string
}) {
  const warn =
    !!returned ||
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
      <span className="truncate">
        {returned && holder.state === Holding.HoldNobody
          ? `Nobody on it: ${displayName(returned)} went away`
          : holderText(holder)}
      </span>
    </span>
  )
}

// AskLine stands in for the holder line on an open ask: "3 agents heard
// it", "Waiting for the MacBook to hand it out", "Nobody took this yet",
// or who took it.
export function AskLine({
  ask,
  className,
}: {
  ask: CardAsk
  className?: string
}) {
  const warn =
    ask.state === AskState.Unclaimed ||
    ask.state === AskState.Waiting ||
    (ask.state === AskState.Open && ask.told === 0)
  return (
    <span
      className={cn(
        "flex min-w-0 items-center gap-1.5 text-[12.5px]",
        warn ? "text-warning" : "text-muted-foreground",
        className
      )}
    >
      {ask.state === AskState.Taken && (
        <span
          className="inline-block size-[7px] shrink-0 rounded-full bg-success"
          aria-hidden
        />
      )}
      <span className="truncate">{askLine(ask)}</span>
      {ask.taker?.device && (
        <span className="shrink-0 text-[11px] text-muted-foreground">
          on {plainLine(ask.taker.device)}
        </span>
      )}
    </span>
  )
}

// ScopeBadge says where the card lives: "My devices" until Shared Boards.
export function ScopeBadge({ card }: { card: CardSummary }) {
  return (
    <Badge
      variant="outline"
      className="font-normal text-muted-foreground"
    >
      {plainLine(card.scopeLabel) || "My devices"}
    </Badge>
  )
}

// ShortRef is the card's short ref, "#12": what the person says or types to
// an agent about it. A card renumbered when two devices that were apart
// both used its number (both gain a letter) says what it was.
export function ShortRef({ card }: { card: CardSummary }) {
  const short = plainLine(card.short)
  if (!short) return null
  const was = plainLine(card.shortWas)
  return (
    <span
      className="font-mono text-xs text-muted-foreground tabular-nums"
      title={
        was
          ? `Was #${was}: a card made on another device while they were apart had #${was} too, so each now has a letter`
          : `Agents know this card as #${short}`
      }
    >
      #{short}
      {was && <span className="ml-1 font-sans">(was #{was})</span>}
    </span>
  )
}

// Spinner says an agent is working on the card right now (busy in
// lib/cards). With reduced motion it is a still dot instead.
export function Spinner({
  label = "An agent is working on it",
  className,
}: {
  label?: string
  className?: string
}) {
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={cn(
        "inline-grid size-3.5 shrink-0 place-items-center text-primary",
        className
      )}
    >
      <span className="col-start-1 row-start-1 block size-full rounded-full border-[1.5px] border-current border-t-transparent motion-safe:animate-spin motion-reduce:hidden" />
      <span className="col-start-1 row-start-1 hidden size-[7px] rounded-full bg-current motion-reduce:block" />
    </span>
  )
}
