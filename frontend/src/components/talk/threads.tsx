// The pieces of a Talk thread (card #390; once the Threads panel beside
// Home's Board, card #116): its state, the cards made from it, and the
// open thread itself.

import { useContext, useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { AgentsNowToggle } from "@/components/common/agents-now-toggle"
import { RichText } from "@/components/common/rich-text"
import { CommentBox } from "@/components/cards/card-detail"
import { AgentCards } from "@/components/cards/agent-hover"
import { Face } from "@/components/cards/faces"
import { Thread } from "@/components/cards/thread"
import { spawnLine } from "@/lib/spawn-line"
import {
  displayName,
  threads as threadsApi,
  useCard,
  type ThreadCard,
  type ThreadSummary,
} from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

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
            <span className="font-mono">#{plainLine(c.short)}</span>
            <span className="truncate">{plainLine(c.title)}</span>
          </Badge>
        </button>
      ))}
    </div>
  )
}

// ThreadView is an open thread on the Talk page (card #390): what the
// person asked, the answers (the card's own thread, events left out), the
// reply box and Resolve. Only its messages scroll, so the reply box stays
// in view and nothing passes under it (card #564).
export function ThreadView({
  t,
  now,
  onResolved,
  bound,
}: {
  t: ThreadSummary
  now: Date
  onResolved: () => void
  // Classes that bound its height (a max-h) where the parent doesn't, as
  // Talk's pane does: everything but the messages keeps its height.
  bound?: string
}) {
  const { detail } = useCard(t.id)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const resolved = t.state === "resolved"
  // Events stay out, but a card made from the thread is part of the answer.
  const items = (detail?.thread ?? []).filter(
    (it) => it.kind !== "event" || it.made
  )
  const act = (f: (id: string) => Promise<void>) => async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await f(t.id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const resolve = act(async (id) => {
    await threadsApi.resolve(id)
    onResolved()
  })
  const reopen = act(threadsApi.reopen)
  const setAgentsNow = (on: boolean) =>
    void act((id) => threadsApi.setAgentsNow(id, on))()
  return (
    <div className={cn("flex min-h-0 min-w-0 flex-col gap-2", bound)}>
      <div className="grid min-w-0 shrink-0 grid-cols-[minmax(0,1fr)] gap-1 text-[12.5px]">
        {/* A long question scrolls by itself, so it leaves the messages
            and the reply box their room in a short window. */}
        <div className={cn("min-w-0 overflow-y-auto", QuestionMax)}>
          <RichText
            text={t.text}
            className="[overflow-wrap:anywhere] text-ink-2"
          />
        </div>
        {t.told.length > 0 && (
          <span className="flex flex-wrap items-center gap-1 text-[11px] text-muted-foreground">
            {t.state === "waiting" ? "Asked first:" : "Told:"}
            {t.told.map((w) => (
              <Badge
                key={w.key}
                variant="slate"
                className="h-4 gap-1 px-1 text-[10.5px]"
              >
                {/* The small face fits the 16px badge (card #386), in
                    the agent's colour as the framed face was. */}
                <Face who={w} inline className="size-3 text-(--face-tint)" />
                {displayName(w)}
              </Badge>
            ))}
          </span>
        )}
      </div>
      <Thread
        cardId={t.id}
        items={items}
        now={now}
        on={{ agents: [...t.told, ...t.answeredBy], over: resolved }}
        compact
        empty={
          t.state === "waiting" && t.agentsNow
            ? "No answer yet. An agent with Offer it cards on picks it up, or Irori starts one when spawning is on."
            : t.agentsNow
              ? "No answer yet."
              : "No answer yet. Agents answer now is off: agents in it read it at their next turn."
        }
        composer={resolved ? undefined : <CommentBox cardId={t.id} reply />}
        scroll
      />
      <div className="flex shrink-0 items-center gap-2">
        {resolved ? (
          <Button
            size="xs"
            variant="outline"
            disabled={busy}
            onClick={() => void reopen()}
            title="Bring it back: your next reply goes to the agents that answered"
          >
            Reopen
          </Button>
        ) : (
          <Button
            size="xs"
            variant="outline"
            disabled={busy}
            onClick={() => void resolve()}
            title="Done with it: the thread closes"
          >
            Resolve
          </Button>
        )}
        {!resolved && (
          <AgentsNowToggle
            on={t.agentsNow}
            onChange={setAgentsNow}
            disabled={busy}
          />
        )}
        {error && <span className="text-xs text-destructive">{error}</span>}
      </div>
    </div>
  )
}

// How tall a thread's question grows before it scrolls (card #564).
const QuestionMax = "max-h-[max(3rem,calc(50svh-10rem))]"
