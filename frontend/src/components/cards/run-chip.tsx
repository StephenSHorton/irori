// RunChips is a card's heavy runs as the tile and Home's rows show them
// (card #1249, live-tasks.md §1, §6): one line per run, "cargo test · 12
// cores · 5.8 GB · 4 min", or short "12 cores · 5.8 GB" with the whole
// line on hover. Plain text: the tile is a button, so Stop is on the
// card's page. Only cards with a run mount it, so only they follow the
// live readings.

import { ActivityIcon } from "lucide-react"

import { Tick } from "@/components/common/ago"
import {
  heavyRuns,
  runChipText,
  runId,
  runsOf,
  useCardRuns,
  type CardRun,
} from "@/lib/runs"
import { cn } from "@/lib/utils"

export function RunChips({
  card,
  short = false,
  className,
}: {
  card: object
  short?: boolean
  className?: string
}) {
  if (!runsOf(card).length) return null
  return <Chips card={card} short={short} className={className} />
}

function Chips({
  card,
  short,
  className,
}: {
  card: object
  short: boolean
  className?: string
}) {
  const runs = heavyRuns(useCardRuns(card))
  if (!runs.length) return null
  return (
    <span className={cn("grid min-w-0 gap-0.5", className)}>
      {runs.map((r) => (
        <Chip key={runId(r)} run={r} short={short} />
      ))}
    </span>
  )
}

function Chip({ run, short }: { run: CardRun; short: boolean }) {
  return (
    <span
      className="flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground tabular-nums"
      // CPU and memory are summed over the run's processes, read every
      // 2 s: about, not exact.
      title={`About: ${runChipText(run, Date.now())}`}
    >
      <ActivityIcon className="size-3 shrink-0" aria-hidden />
      <span className="truncate">
        <Tick of={(now) => runChipText(run, now.getTime(), short)} />
      </span>
    </span>
  )
}
