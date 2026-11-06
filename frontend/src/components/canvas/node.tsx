// Canvas showcase (card #80): the Irori bits each canvas draws a node with.
// React Flow renders these components as they are; Excalidraw and pele
// can't put React inside a node, so they write their own text from the
// same fields (stepsText) in matching colours.

import { FACES_MAX, Faces, Identicon } from "@/components/cards/faces"
import {
  ProgressBar,
  StatePill,
  ShortRef,
  type Tone,
} from "@/components/cards/bits"
import { deviceVar } from "@/lib/cards"
import { cn } from "@/lib/utils"

import type { AgentNode, BlockerNode, CardNode } from "./data"

// toneVar is a status tone as a CSS colour (the minimap, card edges).
export const toneVar: Record<Tone, string> = {
  destructive: "var(--destructive)",
  success: "var(--success)",
  slate: "var(--muted-foreground)",
  info: "var(--info)",
}

// stepsText is "2/5 steps", or "" for a card without steps.
export function stepsText(p: { done: number; total: number }): string {
  return p.total ? `${p.done}/${p.total} steps` : ""
}

// CardBody is a card node: the grid card's own state pill, ref, title,
// faces and progress bar, so the canvas card reads like the list card.
export function CardBody({ n, selected }: { n: CardNode; selected?: boolean }) {
  const working = n.card.working
  const watching = n.card.watching
  return (
    <div
      className={cn(
        "flex h-full w-full min-w-0 flex-col gap-2 overflow-hidden border-l-2 p-3 text-left glass-panel",
        selected && "ring-1 ring-primary"
      )}
      style={{ borderLeftColor: toneVar[n.tone] }}
    >
      <div className="flex min-w-0 items-center gap-2">
        <StatePill card={n.card} />
        {n.where && (
          <span
            className="min-w-0 truncate text-[10.5px] text-muted-foreground"
            title={n.where}
          >
            {n.where}
          </span>
        )}
        <span className="ml-auto shrink-0">
          <ShortRef card={n.card} />
        </span>
      </div>
      <p className="line-clamp-2 text-sm leading-snug font-medium text-foreground">
        {n.label}
      </p>
      <div className="mt-auto flex min-w-0 items-center gap-2">
        {working.length > 0 && <Faces who={working} max={FACES_MAX} />}
        {watching.length > 0 && (
          <Faces who={watching} watching max={FACES_MAX} />
        )}
        {n.subtasks > 0 && (
          <span className="text-xs text-muted-foreground">
            {n.subtasks} subtask{n.subtasks === 1 ? "" : "s"}
          </span>
        )}
        <span className="ml-auto text-xs text-muted-foreground tabular-nums">
          {stepsText(n.progress)}
        </span>
      </div>
      <ProgressBar progress={n.progress} label="Steps done" tone={n.tone} />
    </div>
  )
}

// markersOf is a card's at-a-glance flags (card #587), most urgent first:
// it waits on you, nobody live is on it, nothing happened on it a while.
export function markersOf(
  n: Pick<CardNode, "needs" | "nobody" | "stale" | "done">
): {
  id: "needs" | "nobody" | "stale"
  text: string
  title: string
}[] {
  if (n.done) return []
  const out: ReturnType<typeof markersOf> = []
  if (n.needs)
    out.push({
      id: "needs",
      text: "Waiting on you",
      title: "It needs your answer or your Done",
    })
  if (n.nobody)
    out.push({
      id: "nobody",
      text: "Nobody on it",
      title: "No agent that is running holds it",
    })
  if (n.stale)
    out.push({
      id: "stale",
      text: "Stale",
      title: "Nothing has happened on it for a while, and nobody is busy on it",
    })
  return out
}

const markerLook: Record<ReturnType<typeof markersOf>[number]["id"], string> = {
  needs: "bg-destructive text-background",
  nobody: "bg-warning text-background",
  stale: "bg-muted-foreground text-background",
}

// CardMarkers sits on a card's top edge, outside its clipped body, so the
// flags read even zoomed out.
export function CardMarkers({ n }: { n: CardNode }) {
  const ms = markersOf(n)
  if (!ms.length) return null
  return (
    <div className="pointer-events-none absolute -top-2.5 right-2 z-10 flex gap-1">
      {ms.map((m) => (
        <span
          key={m.id}
          data-marker={m.id}
          title={m.title}
          className={cn(
            "rounded-sm px-1.5 py-px text-[10px] leading-4 font-semibold tracking-wide uppercase shadow-sm",
            markerLook[m.id]
          )}
        >
          {m.text}
        </span>
      ))}
    </div>
  )
}

// BlockerBody is an open blocker under its card: the question, red when
// it waits on the person, amber when an answer waits on an agent.
export function BlockerBody({ n }: { n: BlockerNode }) {
  const person = n.state === "open"
  return (
    <div
      className={cn(
        "flex h-full w-full flex-col justify-center gap-1 border px-3 py-2 text-left",
        person
          ? "border-destructive/60 bg-destructive/10"
          : "border-warning/60 bg-warning/10"
      )}
    >
      <span
        className={cn(
          "text-[10px] font-semibold tracking-wide uppercase",
          person ? "text-destructive" : "text-warning"
        )}
      >
        {person ? "Needs you" : "Answered, waiting on agent"}
      </span>
      <span className="line-clamp-2 text-xs leading-snug text-foreground">
        {n.label}
      </span>
    </div>
  )
}

// statusDot is an agent status tone as a colour.
export const statusDot: Record<AgentNode["tone"], string> = {
  working: "var(--success)",
  job: "var(--success)",
  warn: "var(--warning)",
  idle: "var(--muted-foreground)",
  muted: "color-mix(in oklch, var(--muted-foreground) 50%, transparent)",
}

// AgentBody is an agent (or the person) as a pill in its device colour.
export function AgentBody({ n }: { n: AgentNode }) {
  // You stay in the person tint; another person on a shared board wears
  // their own colour (card #385), as their agents do.
  const tint =
    n.person && n.agent.deviceId
      ? "var(--person)"
      : deviceVar(n.agent.color) ||
        (n.person ? "var(--person)" : "var(--primary)")
  // Its status shows as a dot (card #587): green working (it pulses, see
  // flow.css), amber when it needs something, grey idle.
  const dot = statusDot[n.tone]
  const working = n.tone === "working" || n.tone === "job"
  return (
    <div
      className={cn(
        "relative flex h-full w-full items-center gap-2 rounded-full border px-3 text-xs font-medium",
        !n.live && !n.person && "opacity-55"
      )}
      data-working={working ? "" : undefined}
      style={{
        borderColor: `color-mix(in oklch, ${tint} 60%, transparent)`,
        background: `color-mix(in oklch, ${tint} 14%, transparent)`,
        color: tint,
        ["--pulse" as string]: dot,
      }}
      title={[n.person ? "you" : n.label, n.status, n.agent.where]
        .filter(Boolean)
        .join(" · ")}
    >
      {n.person ? (
        <span
          className="size-2 shrink-0 rounded-full"
          style={{ background: "var(--success)" }}
        />
      ) : (
        // The agent's identicon, in its colour (the node's text colour).
        <span className="-ml-0.5 flex size-4 shrink-0">
          <Identicon seed={n.agent.key} full />
        </span>
      )}
      <span className="truncate">{n.person ? "you" : n.label}</span>
      {!n.person && (
        <span
          className="ml-auto size-2 shrink-0 rounded-full"
          style={{ background: dot }}
          aria-label={n.status || undefined}
        />
      )}
    </div>
  )
}

// openTarget is the card a click on a node opens: a card itself, a
// blocker's card, nothing for an agent or group.
export function openTarget(n: {
  kind: string
  id: string
  card?: unknown
}): string | null {
  if (n.kind === "card") return n.id
  if (n.kind === "blocker") return (n as BlockerNode).card
  return null
}
