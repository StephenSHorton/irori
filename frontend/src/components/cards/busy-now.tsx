// BusyNow is Home's one line of what is heavy on this device right now
// (card #1249, live-tasks.md §1): "Busy now: #54 cargo test 12 cores ·
// 5.8 GB, #12 …", the busiest first, each ref opening its card. Nothing
// heavy, no line. Board pages don't show it. Styled like the idle line.

import { busyList, loadText, runsOf, useRunLoads } from "@/lib/runs"

type Card = { id: string; short: string; tag?: string }

export function BusyNow({
  cards,
  onOpen,
}: {
  cards: readonly Card[]
  onOpen: (id: string) => void
}) {
  const withRuns = cards.filter((c) => runsOf(c).length > 0)
  if (!withRuns.length) return null
  return <Line cards={withRuns} onOpen={onOpen} />
}

function Line({
  cards,
  onOpen,
}: {
  cards: readonly Card[]
  onOpen: (id: string) => void
}) {
  const list = busyList(cards, useRunLoads())
  if (!list.length) return null
  return (
    <p
      className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 px-1 text-[12.5px] text-muted-foreground"
      title="CPU and memory of each card's runs on this device, about"
    >
      <span>Busy now:</span>
      {list.map((c, i) => (
        <span key={c.id} className="min-w-0 break-words tabular-nums">
          <button
            type="button"
            className="font-mono text-foreground underline-offset-2 hover:underline"
            onClick={() => onOpen(c.id)}
          >
            {c.tag ? `${c.tag}#${c.short}` : `#${c.short}`}
          </button>{" "}
          {c.cmd} {loadText(c.cores, c.memBytes)}
          {i < list.length - 1 && ","}
        </span>
      ))}
    </p>
  )
}
