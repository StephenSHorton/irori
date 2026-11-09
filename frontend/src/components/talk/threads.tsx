// The pieces of a Talk thread (card #390; once the Threads panel beside
// Home's Board, card #116): its state, the cards made from it, and the
// open thread itself.

import { cardRef } from "@/components/cards/bits"
import { useContext } from "react"

import { Badge } from "@/components/ui/badge"
import { AgentCards } from "@/components/cards/agent-hover"
import { Face } from "@/components/cards/faces"
import { spawnLine } from "@/lib/spawn-line"
import {
  displayName,
  type AgentRef,
  type ThreadCard,
  type ThreadSummary,
} from "@/lib/cards"
import { plainLine } from "@/lib/irori"

// resolvedShown is whether the "Resolved" list is unfolded: the person's
// pick, or a resolved thread open (a #ref to one unfolds it, card #144).
export function resolvedShown(
  pick: boolean,
  open: string,
  threads: Pick<ThreadSummary, "id" | "state">[]
): boolean {
  return pick || threads.some((t) => t.id === open && t.state === "resolved")
}

// resolvedToggle is a click on the "Resolved" chevron (card #194): the
// pick it sets, and whether to close the open thread. Folding while a
// resolved thread is open closes that thread too, or it would keep the
// list unfolded; an open live thread stays open.
export function resolvedToggle(
  pick: boolean,
  open: string,
  threads: Pick<ThreadSummary, "id" | "state">[]
): { pick: boolean; closeOpen: boolean } {
  if (!resolvedShown(pick, open, threads))
    return { pick: true, closeOpen: false }
  const openResolved = threads.some(
    (t) => t.id === open && t.state === "resolved"
  )
  return { pick: false, closeOpen: openResolved }
}

// openAfterResolve is the thread to keep open once the person resolves id
// (card #194, #200): resolving the open thread closes it, so it moves into
// the folded "Resolved" list instead of unfolding it; another open thread
// stays open.
export function openAfterResolve(open: string, id: string): string {
  return open === id ? "" : open
}

// stateBadges label a thread's state on its title line (#128).
export const stateBadges: Record<
  string,
  { label: string; variant: "info" | "success" | "warning" | "slate" }
> = {
  waiting: { label: "Waiting", variant: "warning" },
  handed: { label: "Answering", variant: "info" },
  answered: { label: "Answered", variant: "success" },
  resolved: { label: "Resolved", variant: "slate" },
}

// stateLine says where a thread stands, under its title in the Talk list.
export function stateLine(t: ThreadSummary, why = ""): string {
  // A follow-up no computer can start an agent for (card #541): why, even
  // on a thread an agent answered before.
  if (t.state === "answered" && t.noAgent) return t.noAgent
  switch (t.state) {
    case "answered":
      return t.lastBy && t.lastText
        ? `${displayName(t.lastBy)}: ${plainLine(t.lastText)}`
        : "Answered"
    case "handed":
      return t.told.length
        ? `${displayName(t.told[t.told.length - 1])} is answering`
        : "An agent is answering"
    case "resolved":
      return "Resolved"
    default:
      // An agent Irori starts for it, or why it can't (card #470).
      return spawnLine(t)?.text ?? (why || "Waiting for an agent")
  }
}

// cardStates colour a card chip by its state.
const cardStates: Record<string, "info" | "success" | "warning" | "slate"> = {
  working: "info",
  input_required: "warning",
  blocked: "warning",
  in_review: "warning",
  completed: "success",
}

// ThreadCards are the cards made or changed from a thread (card #155),
// each one click from its page.
export function ThreadCards({ cards }: { cards: ThreadCard[] }) {
  const ctx = useContext(AgentCards)
  return (
    <div className="flex max-h-16 min-w-0 shrink-0 flex-wrap items-center gap-1 overflow-y-auto text-[11px] text-muted-foreground">
      Cards:
      {cards.map((c) => (
        <button
          key={c.id}
          type="button"
          className="max-w-full min-w-0"
          title={`${plainLine(c.title)} (${c.state.replace("_", " ")})`}
          disabled={!ctx}
          onClick={() => ctx?.onOpen(c.id)}
        >
          <Badge
            variant={cardStates[c.state] ?? "slate"}
            className="h-4 max-w-full gap-1 px-1 text-[10.5px]"
          >
            <span className="font-mono">{cardRef(c)}</span>
            <span className="truncate">{plainLine(c.title)}</span>
          </Badge>
        </button>
      ))}
    </div>
  )
}

// OpenedBy names the agent that opened a thread itself (card #631).
export function OpenedBy({ who }: { who: AgentRef }) {
  return (
    <span className="flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground">
      <Face who={who} inline className="size-3 text-(--face-tint)" />
      <span className="truncate">{openedLine(who)}</span>
    </span>
  )
}

export function openedLine(who: AgentRef): string {
  return `${displayName(who)} opened this thread`
}

// emptyLine is what a thread with no replies yet says in its pane.
export function emptyLine(
  t: Pick<ThreadSummary, "openedBy" | "noAgent" | "state" | "agentsNow">
): string {
  if (t.openedBy) return emptyOpened(t.openedBy)
  // Why no computer can start one, and what happens next (card #541):
  // "Irori starts one when spawning is on" would give the wrong reason.
  if (t.noAgent) return t.noAgent
  if (t.state === "waiting" && t.agentsNow)
    return "No answer yet. An agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
  if (t.agentsNow) return "No answer yet."
  return "No answer yet. It was sent as Just a note: agents in it read it at their next turn."
}

// emptyOpened is an agent's own thread before anyone writes in it: its
// agent holds it, and none other starts until the person writes.
export function emptyOpened(who: AgentRef): string {
  return `No replies yet. ${displayName(who)} hears yours; no other agent is started for it until you write.`
}
