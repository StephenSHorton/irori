// A board tile's parts (card #126): its bar of cards by state and its few
// most recently active cards. A space's page (space-boards.tsx) shows them
// on each of its boards; the Spaces page that first had them is gone (card
// #689: the sidebar's space picker is where spaces are picked and made).

import { cardTone, type Tone } from "@/components/cards/bits"
import { Face } from "@/components/cards/faces"
import { TaskState, type CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { cn } from "@/lib/utils"

// The state bar's segments (card #126), in the colours the cards' state
// chips use: blocked or waiting on someone, being worked, open (nobody on
// it), done this week.
const segTone: Record<Tone, string> = {
  destructive: "bg-destructive",
  info: "bg-info",
  slate: "bg-muted-foreground/45",
  success: "bg-success",
}

type Seg = { key: string; label: string; n: number; tone: Tone }

// boardSegs counts a board's live cards by state, as the cards' own chips
// colour them, plus the board's done this week.
export function boardSegs(mine: CardSummary[], doneWeek: number): Seg[] {
  let blocked = 0
  let working = 0
  let open = 0
  for (const c of mine) {
    const t = cardTone(c)
    if (t === "destructive") blocked++
    else if (t === "info") working++
    else if (t === "success") doneWeek++
    else if (c.state !== TaskState.Canceled && c.state !== TaskState.Failed)
      open++
  }
  return [
    { key: "working", label: "working", n: working, tone: "info" },
    { key: "open", label: "open", n: open, tone: "slate" },
    { key: "blocked", label: "blocked", n: blocked, tone: "destructive" },
    { key: "done", label: "done this week", n: doneWeek, tone: "success" },
  ]
}

// StateBar is the tile's thin bar of card counts by state, with the
// counts spelled out under it.
export function StateBar({ segs }: { segs: Seg[] }) {
  const total = segs.reduce((n, s) => n + s.n, 0)
  const shown = segs.filter((s) => s.n > 0)
  const text = shown.map((s) => `${s.n} ${s.label}`).join(" · ")
  return (
    <span className="grid gap-1">
      <span
        className="flex h-1.5 w-full overflow-hidden rounded-full bg-muted"
        role="img"
        aria-label={text || "No cards"}
      >
        {total > 0 &&
          shown.map((s) => (
            <span
              key={s.key}
              className={cn("h-full", segTone[s.tone])}
              style={{ width: `${(s.n / total) * 100}%` }}
            />
          ))}
      </span>
      <span className="text-[11px] text-muted-foreground tabular-nums">
        {text || "No cards yet"}
      </span>
    </span>
  )
}

// RecentCards is a tile's few most recently active cards, with their
// holders' faces.
export function RecentCards({ cards }: { cards: CardSummary[] }) {
  if (cards.length === 0) return null
  return (
    <ul className="grid gap-1" aria-label="Recently active cards">
      {cards.map((c) => (
        <li
          key={c.id}
          className="flex min-w-0 items-center gap-2 text-[12.5px]"
        >
          {c.holder.who.key ? (
            <Face who={c.holder.who} noHover noDot className="size-5" />
          ) : (
            <span
              className="size-5 shrink-0 border border-dashed border-muted-foreground/40"
              title="Nobody holds it"
              aria-label="Nobody holds it"
            />
          )}
          <span className="font-mono text-[11px] text-muted-foreground">
            {c.tag ?? ""}#{c.short}
          </span>
          <span className="min-w-0 truncate text-ink-2">
            {plainLine(c.title)}
          </span>
        </li>
      ))}
    </ul>
  )
}
