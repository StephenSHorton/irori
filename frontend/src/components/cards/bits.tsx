import { laterOf } from "@/lib/later"
import { useContext } from "react"
import { ExternalLinkIcon, UserIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { useBoards } from "@/lib/boards"
import { cn } from "@/lib/utils"
import {
  askLine,
  AskState,
  cards as cardsApi,
  displayName,
  Holding,
  idleUnnamed,
  openAsk,
  over,
  overLabel,
  percent,
  TaskState,
  type AgentRef,
  type CardAsk,
  type CardHolder,
  type CardSummary,
} from "@/lib/cards"
import { agentStatus } from "@/lib/agent-status"
import { plainLine } from "@/lib/irori"
import { waitLine } from "@/lib/wait-name"
import { LiveRoster } from "@/lib/thread"
import { prChip } from "@/lib/needs"
import { cardOwner, type OwnerCard } from "@/lib/owners"

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
  ready?: boolean
  working: readonly unknown[]
}): Tone {
  // Ready for you (card #112): done but for your word, green.
  if (card.ready && card.openBlockers === 1) return "success"
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
  // Parked (card #570): not for now, so it reads calm.
  if (laterOf(card))
    return (
      <Badge variant="slate" className="rounded-full">
        Not yet
      </Badge>
    )
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
    return <Badge variant="destructive">Needs your input</Badge>
  // An agent says it's done and only that waits on you (card #112).
  if (card.ready && card.openBlockers === 1)
    return (
      <Badge
        variant="success"
        title="The agent says it's done: mark it Done, or say why it isn't"
      >
        Ready for you
      </Badge>
    )
  if (card.openBlockers > 0)
    return (
      <Badge variant="destructive">
        {card.openBlockers} {card.openBlockers === 1 ? "blocker" : "blockers"}
      </Badge>
    )
  // The work is done and waits on its review (card #191). Yellow, as it
  // can't move on until the review comes (card #217).
  if (card.state === TaskState.InReview)
    return (
      <Badge
        variant="warning"
        title="The work is done and waits on its pull request's review; its agent is free for other work"
      >
        In review
      </Badge>
    )
  // A parent whose subtask waits on a decision: the subtask has the blocker.
  if (card.subtaskBlocked && !over(card))
    return (
      <Badge variant="warning" title={plainLine(card.subtaskBlocked)}>
        Subtask blocked
      </Badge>
    )
  // It waits on another card, a lease, an agent or something outside
  // (card #158): that ends by itself, so it's calm, not red.
  // The reason can be long ("external ww/dolphin-slot-1..4 leases
  // (reed until ...)"): the pill truncates to its card's width and the
  // tooltip has it all (card #222); the card's body says it in full.
  if (card.waitingOn) {
    const label = waitLabel(card)
    return (
      <Badge
        variant="slate"
        className="max-w-full min-w-0 shrink justify-start rounded-full"
        title={waitTip(card)}
      >
        <span className="truncate">{label}</span>
      </Badge>
    )
  }
  // Every open blocker is answered: it waits on the agents, not on you.
  if (card.waiting > 0)
    return (
      <Badge variant="slate" className="rounded-full">
        Waiting on agent
      </Badge>
    )
  if (over(card))
    return (
      <Badge
        variant={
          card.state === TaskState.Completed
            ? "success"
            : card.state === TaskState.Failed
              ? "warning"
              : "slate"
        }
      >
        {overLabel(card)}
      </Badge>
    )
  if (card.working.length === 0) return <Badge variant="slate">Open</Badge>
  if (card.holder.state === Holding.HoldWaiting)
    return (
      <Badge variant="warning" className="rounded-full">
        Not picked up
      </Badge>
    )
  if (card.holder.state === Holding.HoldAssigned)
    return (
      <Badge variant="slate" className="rounded-full">
        Offered
      </Badge>
    )
  // Held, its holders idle a while, and nothing named that it waits on
  // (card #305): not calm Waiting, so the person can tell.
  if (idleUnnamed(card))
    return (
      <Badge
        variant="warning"
        className="rounded-full"
        title="Its holder ended its turn without saying what the card waits on. Irori asked it to name the wait (a blocker) or keep going."
      >
        Idle, no reason given
      </Badge>
    )
  return <Badge variant="info">In progress</Badge>
}

// holderText is the holder line's words: who holds the card and what that
// agent is up to, or who it waits for. An active, idle or silent holder
// reads its status as everywhere else (card #489): "· running a job · 12
// min" for a shell, not "· active"; since is when that status began and
// status the live roster's, which wins over the card's (card #317).
export function holderText(
  h: CardHolder,
  since?: string,
  status?: string
): string {
  const who = h.who.key ? displayName(h.who) : ""
  const more = h.others > 0 ? ` (+${h.others} more)` : ""
  switch (h.state) {
    case Holding.HoldNobody:
      return "Nobody holds it yet"
    case Holding.HoldPerson:
      return `You hold it${more}`
    case Holding.HoldWaiting:
      return `Waiting for ${who}${more}: the hand-over hasn't reached it yet`
    case Holding.HoldAssigned:
      return `Offered to ${who}${more}, not taken up yet`
    case Holding.HoldActive:
    case Holding.HoldIdle:
    case Holding.HoldSilent:
      // The state already weighed gone and offline; say the status.
      return `${who}${more} · ${agentStatus({ status: status || h.who.status }, since).text}`
    case Holding.HoldOffline:
      return `${who}${more} · its device is offline`
    case Holding.HoldGone:
      // An agent Irori started ends when its work is done (card #456,
      // spawned agents §10): finished, not lost. One that ended before it
      // took the card up died (card #699).
      return h.who.spawned && !h.who.assigned
        ? `${who}${more} · agent finished`
        : `${who}${more} · gone`
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
  const finished =
    holder.state === Holding.HoldGone &&
    !!holder.who.spawned &&
    !holder.who.assigned
  // The live roster's status and when it began, as the hover card and
  // Who's on it read them (cards #317, #489).
  const live = useContext(LiveRoster).get(holder.who.key)
  const since = live?.statusSince
  const status = live?.status
  const tone =
    holder.state === Holding.HoldActive ||
    holder.state === Holding.HoldIdle ||
    holder.state === Holding.HoldSilent
      ? agentStatus({ status: status || holder.who.status }).tone
      : null
  const warn =
    (!!returned && !returned.spawned) ||
    holder.state === Holding.HoldWaiting ||
    (holder.state === Holding.HoldGone && !finished) ||
    tone === "warn" ||
    holder.state === Holding.HoldOffline
  return (
    <span
      className={cn(
        "flex min-w-0 items-center gap-1.5 text-[12.5px]",
        warn ? "text-warning" : "text-muted-foreground",
        className
      )}
    >
      {(tone === "working" || tone === "job") && (
        <span
          className="inline-block size-[7px] shrink-0 rounded-full bg-success"
          aria-hidden
        />
      )}
      <span className="truncate">
        {returned && holder.state === Holding.HoldNobody
          ? returned.spawned
            ? `Nobody on it: ${displayName(returned)} finished`
            : `Nobody on it: ${displayName(returned)} went away`
          : holderText(holder, since, status)}
      </span>
    </span>
  )
}

// AskLine stands in for the holder line on an open ask: "3 agents heard
// it", "Waiting for the MacBook to hand it out", "Nobody took this yet",
// who Irori handed it to, or who took it.
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
      {(ask.state === AskState.Taken || ask.state === AskState.Handed) && (
        <span
          className={cn(
            "inline-block size-[7px] shrink-0 rounded-full",
            ask.state === AskState.Taken ? "bg-success" : "bg-info"
          )}
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

// ScopeBadge says where the card lives: "My devices", or its shared
// board's name (v0.5 §8).
export function ScopeBadge({ card }: { card: CardSummary }) {
  return (
    <Badge variant="outline" className="font-normal text-muted-foreground">
      {plainLine(card.scopeLabel) || "My devices"}
    </Badge>
  )
}

// SideBadge is the work/personal side a card is on (card #494): only the
// devices that run that side take it up. None for a card on any side.
// With sideWait set (card #495) the device that picks an agent for it
// doesn't run that side, so the card waits: the badge says so.
export function SideBadge({ card }: { card: CardSummary }) {
  const side = plainLine(card.side ?? "")
  if (!side) return null
  const wait = plainLine(card.sideWait ?? "")
  return (
    <Badge
      variant={wait ? "outline" : "secondary"}
      className="font-normal"
      title={
        wait
          ? `Waits: ${wait}. It is never handed to another device.`
          : `On the ${side} side: devices that run ${side} take it up`
      }
    >
      {wait ? `${side}: waits, ${wait}` : side}
    </Badge>
  )
}

// PrChip is the small "waits on PR #N" an in-review card shows on itself
// (card #471): one click opens the PR. Tiles and rows are buttons, so it is
// a link-role span that keeps its click to itself.
export function PrChip({ card }: { card: CardSummary }) {
  const c = prChip(card)
  if (!c) return null
  const open = () => void cardsApi.openURL(c.url)
  return (
    <Badge
      variant="warning"
      role="link"
      tabIndex={0}
      data-pr={c.url}
      title={c.title}
      className="cursor-pointer gap-1 font-normal hover:underline"
      onClick={(e) => {
        e.stopPropagation()
        e.preventDefault()
        open()
      }}
      onKeyDown={(e) => {
        if (e.key !== "Enter" && e.key !== " ") return
        e.stopPropagation()
        e.preventDefault()
        open()
      }}
    >
      {c.text}
      <ExternalLinkIcon aria-hidden />
    </Badge>
  )
}

// OwnerChip is whose a shared board's card is (card #268): the member who
// made it, by themselves or through one of their agents, or whoever it was
// handed to since (card #432). Nothing shows on a private card or a board
// that is only the person's.
export function OwnerChip({ card }: { card: OwnerCard }) {
  const { boards } = useBoards()
  const o = cardOwner(card, boards)
  if (!o) return null
  const name = plainLine(o.name)
  return (
    <Badge
      variant={o.me ? "info" : "outline"}
      data-owner={o.personId}
      className="gap-1 font-normal"
      title={
        o.me
          ? `Yours: you decide Done and answer its questions`
          : o.known
            ? `${name}'s card: they decide Done and answer its questions`
            : "Made by someone who is no longer in the space"
      }
    >
      <UserIcon className="size-3" aria-hidden />
      {name}
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
  // A shared board's card carries its board's tag: "mlg#4" (v0.5 §2.5).
  const tag = plainLine(card.tag ?? "")
  return (
    <span
      className="font-mono text-xs text-muted-foreground tabular-nums"
      title={
        was
          ? `Was ${tag}#${was}: a card made on another device while they were apart had ${tag}#${was} too, so each now has a letter`
          : `Agents know this card as ${tag}#${short}`
      }
    >
      {tag}#{short}
      {was && (
        <span className="ml-1 font-sans">
          (was {tag}#{was})
        </span>
      )}
    </span>
  )
}

// cardRef is a card's short ref as text: "#12", or "mlg#12" on a shared
// board.
export function cardRef(card: { short: string; tag?: string }): string {
  return `${plainLine(card.tag ?? "")}#${plainLine(card.short)}`
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

// waitLabel: what a card waits on that isn't you (card #158), a lease in
// plain words (card #487: "Waiting for a free Dolphin slot").
export function waitLabel(card: {
  waitingOn?: number
  waitOn?: string
  waitKind?: string
}): string {
  const n = card.waitingOn ?? 0
  const on = card.waitOn ? waitLine(card) : "Waiting"
  return n > 1 ? `${on} +${n - 1}` : on
}

// waitTip: the state pill's tooltip, the wait in full (card #553: a version
// wait's devices, which the pill leaves out) and the question it was raised
// with.
export function waitTip(card: {
  waitingOn?: number
  waitOn?: string
  waitFull?: string
  waitKind?: string
  waitQuestion?: string
}): string {
  const label = waitLabel({ ...card, waitOn: card.waitFull || card.waitOn })
  return card.waitQuestion ? `${label}: ${plainLine(card.waitQuestion)}` : label
}

// Asker names who asks a blocker's question, above it (card #491): the
// person clicks its options, so "sample-otter asks:" keeps the agent's
// words from reading as their own.
export function Asker({
  who,
  className,
}: {
  who: AgentRef
  className?: string
}) {
  return (
    <p className={cn("text-[11.5px] text-muted-foreground", className)}>
      <span className="font-medium text-ink-2">{displayName(who)}</span> asks:
    </p>
  )
}
