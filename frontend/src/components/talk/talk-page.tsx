// TalkPage is Talk (card #390, design docs/design/talk-channels.md §1 b1,
// mockups talk-1 and talk-2b): one list beside the conversation. The list
// holds Threads (the person's asks to the room, card #116, once a panel
// beside Home's Board); Channels and Direct messages join it once Talk 1
// (#389) stores them. Resolved threads leave the list but stay one click
// away, folded at its foot.

import { useState, type ReactNode } from "react"
import {
  ChevronLeftIcon,
  MessageSquareTextIcon,
  SearchIcon,
  SquarePenIcon,
} from "lucide-react"

import { AskBox } from "@/components/cards/ask-box"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { type ThreadSummary } from "@/lib/cards"
import { parseTime, plainLine, relativeTime } from "@/lib/irori"
import { useNarrow } from "@/lib/media"
import { cn } from "@/lib/utils"
import {
  ThreadCards,
  ThreadView,
  openAfterResolve,
  resolvedShown,
  resolvedToggle,
  stateBadges,
  stateLine,
} from "./threads"

// talkMatches is whether a thread fits the list's search: its title, what
// was asked, its #ref, or the newest reply.
export function talkMatches(t: ThreadSummary, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [t.title, t.text, t.short && `#${t.short}`, t.lastText ?? ""].some(
    (s) => !!s && s.toLowerCase().includes(q)
  )
}

// needsYou is whether a thread waits on the person: an agent answered and
// it isn't resolved. Talk's count in the sidebar counts these.
export function needsYou(t: Pick<ThreadSummary, "state">): boolean {
  return t.state === "answered"
}

export function TalkPage({
  threads,
  open,
  onOpen,
  now,
  why = "",
}: {
  threads: ThreadSummary[]
  // The conversation open ("" none: the new-thread box).
  open: string
  onOpen: (id: string) => void
  now: Date
  // Why nobody is free to take a waiting thread (card #172).
  why?: string
}) {
  const narrow = useNarrow()
  const [query, setQuery] = useState("")
  const [showResolvedPick, setShowResolved] = useState(false)
  // On a narrow window the pen opens the new-thread box over the list.
  const [composing, setComposing] = useState(false)
  const shown = threads.filter((t) => talkMatches(t, query))
  const live = shown.filter((t) => t.state !== "resolved")
  const resolved = shown.filter((t) => t.state === "resolved")
  const showResolved = resolvedShown(showResolvedPick, open, threads) || !!query.trim()
  const current = threads.find((t) => t.id === open)

  const list = (
    <nav
      aria-label="Talk"
      className={cn(
        "flex min-h-0 shrink-0 flex-col gap-1 overflow-y-auto py-3",
        narrow ? "w-full px-1" : "w-[260px] border-r border-border px-2"
      )}
    >
      <div className="mx-1 flex items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-1.5 text-[12.5px]">
        <SearchIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search talk"
          aria-label="Search talk"
          className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-muted-foreground"
        />
        <button
          type="button"
          onClick={() => (narrow ? setComposing(true) : onOpen(""))}
          aria-label="New thread"
          title="Start a thread: ask the room about anything"
          className="shrink-0 rounded p-0.5 text-ink-2 hover:bg-secondary hover:text-ink"
        >
          <SquarePenIcon className="size-4" aria-hidden />
        </button>
      </div>
      <Group label={`Threads${live.length > 0 ? ` · ${live.length} open` : ""}`}>
        {live.length === 0 && (
          <p className="px-2 py-1 text-[12px] text-muted-foreground">
            {query.trim()
              ? "No open thread matches."
              : "No open threads. Start one with the pen above."}
          </p>
        )}
        {live.map((t) => (
          <ThreadItem
            key={t.id}
            t={t}
            current={t.id === open}
            why={why}
            onClick={() => onOpen(t.id)}
          />
        ))}
      </Group>
      {resolved.length > 0 && (
        <div className="mt-auto grid gap-0.5 pt-3">
          <button
            type="button"
            className="px-2 text-left text-[11px] text-muted-foreground hover:text-ink"
            aria-expanded={showResolved}
            onClick={() => {
              const t = resolvedToggle(showResolvedPick, open, threads)
              setShowResolved(t.pick)
              if (t.closeOpen) onOpen("")
            }}
          >
            Resolved threads: {resolved.length}
            {showResolved ? " · hide" : " · show"}
          </button>
          {showResolved &&
            resolved.map((t) => (
              <ThreadItem
                key={t.id}
                t={t}
                current={t.id === open}
                why={why}
                onClick={() => onOpen(t.id)}
              />
            ))}
        </div>
      )}
    </nav>
  )

  const conversation = current ? (
    <Conversation
      key={current.id}
      t={current}
      now={now}
      narrow={narrow}
      onBack={() => onOpen("")}
      onResolved={() => onOpen(openAfterResolve(open, current.id))}
    />
  ) : (
    <div className="grid min-w-0 flex-1 content-start gap-3 p-4">
      <h2 className="flex items-center gap-2 text-[13px] font-medium text-ink">
        {narrow && threads.length > 0 && (
          <Button size="icon-xs" variant="ghost" aria-label="Back to Talk" onClick={() => setComposing(false)}>
            <ChevronLeftIcon />
          </Button>
        )}
        Start a thread
      </h2>
      <p className="text-[12.5px] text-muted-foreground">
        Ask the room about anything. An agent on the cards you name answers,
        or an idle one does. It stays in Threads until you resolve it.
      </p>
      <AskBox
        onThread={(id) => {
          setComposing(false)
          onOpen(id)
        }}
      />
    </div>
  )

  // A phone or a narrow window shows one at a time: the list, or the
  // conversation with Back.
  if (narrow)
    return (
      <div className="flex min-h-0 w-full min-w-0 flex-1 flex-col overflow-x-hidden">
        {current || composing || !threads.length ? conversation : list}
      </div>
    )
  return (
    <div className="glass-panel flex min-h-0 flex-1 overflow-hidden mb-4">
      {list}
      {conversation}
    </div>
  )
}

function Group({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid gap-0.5">
      <h2 className="px-2 pt-3 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {label}
      </h2>
      {children}
    </div>
  )
}

function ThreadItem({
  t,
  current,
  why,
  onClick,
}: {
  t: ThreadSummary
  current: boolean
  why: string
  onClick: () => void
}) {
  const waiting = t.state === "waiting" || t.state === "handed"
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={current ? "page" : undefined}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-ink-2 hover:bg-secondary/60",
        current && "bg-secondary"
      )}
    >
      <MessageSquareTextIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
      <span className="grid min-w-0 flex-1">
        <span className={cn("truncate", needsYou(t) && "font-medium text-ink")}>
          {plainLine(t.title)}
        </span>
        <span
          className={cn(
            "truncate text-[11px]",
            waiting ? "text-warning" : "text-muted-foreground"
          )}
        >
          {stateLine(t, why)}
        </span>
      </span>
      {needsYou(t) && (
        <span
          className="size-2 shrink-0 rounded-full bg-primary"
          title="Answered: waiting on you"
        />
      )}
    </button>
  )
}

function Conversation({
  t,
  now,
  narrow,
  onBack,
  onResolved,
}: {
  t: ThreadSummary
  now: Date
  narrow: boolean
  onBack: () => void
  onResolved: () => void
}) {
  const badge = stateBadges[t.state]
  const at = parseTime(t.updatedAt)
  return (
    <section
      aria-label={plainLine(t.title)}
      className={cn(
        "grid min-h-0 min-w-0 flex-1 grid-cols-[minmax(0,1fr)] content-start gap-3 overflow-y-auto",
        narrow ? "px-1 py-2" : "p-4"
      )}
    >
      <header className="flex min-w-0 items-center gap-2">
        {narrow && (
          <Button size="icon-xs" variant="ghost" aria-label="Back to Talk" onClick={onBack}>
            <ChevronLeftIcon />
          </Button>
        )}
        <MessageSquareTextIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        {t.short && (
          <span className="shrink-0 text-[12px] text-muted-foreground tabular-nums">
            #{plainLine(t.short)}
          </span>
        )}
        <h2 className="min-w-0 flex-1 truncate text-[15px] font-semibold text-ink">
          {plainLine(t.title)}
        </h2>
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
          <Badge variant={badge.variant} className="h-4 shrink-0 px-1.5 text-[10.5px]">
            {badge.label}
          </Badge>
        )}
        <time
          className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
          dateTime={at?.toISOString()}
        >
          {relativeTime(at, now)}
        </time>
      </header>
      {t.cards.length > 0 && <ThreadCards cards={t.cards} />}
      <ThreadView t={t} now={now} onResolved={onResolved} />
    </section>
  )
}
