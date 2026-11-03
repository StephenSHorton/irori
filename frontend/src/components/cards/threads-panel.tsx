import { useContext, useEffect, useRef, useState } from "react"
import { ChevronDownIcon, ChevronRightIcon, XIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { RichText } from "@/components/common/rich-text"
import { CommentBox } from "./card-detail"
import { AgentCards } from "./agent-hover"
import { Face } from "./faces"
import { Thread } from "./thread"
import {
  displayName,
  threads as threadsApi,
  useCard,
  type ThreadCard,
  type ThreadSummary,
} from "@/lib/cards"
import { parseTime, plainLine, relativeTime } from "@/lib/irori"
import { errorText } from "@/lib/talk"

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

// ThreadsPanel is the Threads panel beside the Board (card #116, layout A):
// the person's conversations with the room, open ones on top (the one
// they're in unfolded), resolved ones folded under "Resolved". A thread
// isn't a card: agents on the cards it names answer it, or after 45s an
// idle agent does; the person replies here and resolves it when done.
export function ThreadsPanel({
  threads,
  open,
  onOpen,
  onHide,
  now,
  why = "",
}: {
  threads: ThreadSummary[]
  // Why nobody is free to take a waiting thread (card #172), "" when
  // someone is.
  why?: string
  // The thread unfolded ("" none).
  open: string
  onOpen: (id: string) => void
  onHide: () => void
  now: Date
}) {
  const [showResolvedPick, setShowResolved] = useState(false)
  // A resolved thread opened from a #ref (card #144) unfolds its list.
  const showResolved = resolvedShown(showResolvedPick, open, threads)
  const live = threads.filter((t) => t.state !== "resolved")
  const resolved = threads.filter((t) => t.state === "resolved")
  return (
    <aside
      className="grid min-w-0 grid-cols-[minmax(0,1fr)] content-start gap-3 overflow-clip p-3 glass-panel"
      aria-label="Threads"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Threads{live.length > 0 && ` · ${live.length} open`}
        </h2>
        <Button
          size="icon-xs"
          variant="ghost"
          aria-label="Hide threads"
          title="Hide threads"
          onClick={onHide}
        >
          <XIcon />
        </Button>
      </div>
      {live.length === 0 && (
        <p className="text-[12.5px] text-muted-foreground">
          No open threads. Ask about anything in the box above: an agent on the
          cards you name answers, or an idle one does.
        </p>
      )}
      {live.map((t) => (
        <ThreadRow
          key={t.id}
          t={t}
          open={open === t.id}
          onToggle={() => onOpen(open === t.id ? "" : t.id)}
          onResolved={() => onOpen(openAfterResolve(open, t.id))}
          now={now}
          why={why}
        />
      ))}
      {resolved.length > 0 && (
        <div className="grid gap-2 border-t border-border pt-3">
          <button
            type="button"
            className="flex items-center gap-1 justify-self-start text-xs text-muted-foreground hover:text-ink"
            onClick={() => {
              const t = resolvedToggle(showResolvedPick, open, threads)
              setShowResolved(t.pick)
              if (t.closeOpen) onOpen("")
            }}
            aria-expanded={showResolved}
          >
            {showResolved ? (
              <ChevronDownIcon className="size-3.5" />
            ) : (
              <ChevronRightIcon className="size-3.5" />
            )}
            Resolved ({resolved.length})
          </button>
          {showResolved &&
            resolved.map((t) => (
              <ThreadRow
                key={t.id}
                t={t}
                open={open === t.id}
                onToggle={() => onOpen(open === t.id ? "" : t.id)}
                onResolved={() => onOpen(openAfterResolve(open, t.id))}
                now={now}
                why={why}
              />
            ))}
        </div>
      )}
    </aside>
  )
}

// stateBadges label a thread's state on its title line (#128).
const stateBadges: Record<
  string,
  { label: string; variant: "info" | "success" | "warning" | "slate" }
> = {
  waiting: { label: "Waiting", variant: "warning" },
  handed: { label: "Answering", variant: "info" },
  answered: { label: "Answered", variant: "success" },
  resolved: { label: "Resolved", variant: "slate" },
}

// stateLine says where a folded thread stands.
function stateLine(t: ThreadSummary, why = ""): string {
  switch (t.state) {
    case "answered":
      return t.lastBy && t.lastText
        ? `${displayName(t.lastBy)}: ${plainLine(t.lastText)}`
        : "Answered"
    case "handed":
      return t.told.length
        ? `${displayName(t.told[t.told.length - 1])} is answering`
        : "An idle agent is answering"
    case "resolved":
      return "Resolved"
    default:
      return t.told.length
        ? `Waiting for ${t.told.map(displayName).join(", ")}; an idle agent answers after 45s`
        : why || "Waiting for a free agent"
  }
}

function ThreadRow({
  t,
  open,
  onToggle,
  onResolved,
  now,
  why,
}: {
  t: ThreadSummary
  open: boolean
  onToggle: () => void
  onResolved: () => void
  now: Date
  why: string
}) {
  // Opened from elsewhere (a #ref, card #144), it scrolls into view.
  const box = useRef<HTMLElement>(null)
  useEffect(() => {
    if (open) box.current?.scrollIntoView({ block: "nearest" })
  }, [open])
  const at = parseTime(t.updatedAt)
  const waiting = t.state === "waiting" || t.state === "handed"
  const badge = stateBadges[t.state]
  return (
    <section
      ref={box}
      className={
        open
          ? "grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2 rounded-lg border border-primary bg-background/40 px-3 py-2.5"
          : "grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1 rounded-lg border border-border bg-background/40 px-3 py-2.5"
      }
    >
      <button
        type="button"
        className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-0.5 text-left"
        onClick={onToggle}
        aria-expanded={open}
      >
        <span className="flex min-w-0 items-center gap-2">
          {t.short && (
            <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
              #{plainLine(t.short)}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-ink">
            {plainLine(t.title)}
          </span>
          {t.private && (
            <Badge
              variant="slate"
              className="h-4 shrink-0 px-1.5 text-[10.5px]"
              title="Only you and your own agents see this thread; it never reaches other members' devices"
            >
              Private
            </Badge>
          )}
          {badge && (
            <Badge
              variant={badge.variant}
              className="h-4 shrink-0 px-1.5 text-[10.5px]"
            >
              {badge.label}
            </Badge>
          )}
          <time
            className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
            dateTime={at?.toISOString()}
          >
            {relativeTime(at, now)}
          </time>
        </span>
        {!open && (
          <span
            className={
              waiting
                ? "line-clamp-2 text-xs text-warning"
                : "line-clamp-2 text-xs text-muted-foreground"
            }
          >
            {stateLine(t, why)}
          </span>
        )}
      </button>
      {t.cards.length > 0 && <ThreadCards cards={t.cards} />}
      {open && <ThreadView t={t} now={now} onResolved={onResolved} />}
    </section>
  )
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
    <div className="flex min-w-0 flex-wrap items-center gap-1 text-[11px] text-muted-foreground">
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

// ThreadView is an unfolded thread: what the person asked, the answers
// (the card's own thread, events left out), the reply box and Resolve.
function ThreadView({
  t,
  now,
  onResolved,
}: {
  t: ThreadSummary
  now: Date
  onResolved: () => void
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
  return (
    <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-2">
      <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-1 text-[12.5px]">
        <RichText text={t.text} className="text-ink-2 [overflow-wrap:anywhere]" />
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
          t.state === "waiting"
            ? "No answer yet. If nobody answers in 45s, an idle agent picks it up."
            : "No answer yet."
        }
        composer={resolved ? undefined : <CommentBox cardId={t.id} reply />}
        // Its messages scroll and the reply box stays in view (card #170):
        // the cap leaves room in the column for the panel's header, the
        // thread's title and question, the box and Resolve.
        scroll={OpenThreadMax}
      />
      <div className="flex items-center gap-2">
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
        {error && <span className="text-xs text-destructive">{error}</span>}
      </div>
    </div>
  )
}

// How tall an open thread's messages grow before they scroll (card #170):
// the window less the header, the ask box and the thread's own lines.
const OpenThreadMax = "max-h-[max(6rem,calc(100svh-30rem))]"
