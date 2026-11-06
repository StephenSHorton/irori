// The freshness lines (card #580, no-server.md §4.3): when a space or
// Personal last caught up with another device, and which one, so old data
// doesn't look current. Muted while fresh, amber after a day.

import { RefreshCwIcon } from "lucide-react"

import {
  freshLine,
  shortLine,
  stale,
  stalest,
  useFreshness,
} from "@/lib/freshness"
import { useNow } from "@/lib/irori"
import { AllSpaces } from "@/lib/spaces"
import { cn } from "@/lib/utils"

// staleTail is what an amber line adds: why the age matters.
const staleTail = ": newer changes from other devices may not be here yet"

// FreshnessLine is one log's full line: "Last synced with your PC 3h ago".
export function FreshnessLine({
  log,
  className,
}: {
  log: string
  className?: string
}) {
  const all = useFreshness()
  const now = useNow(30_000)
  const f = all.get(log)
  const text = freshLine(f, now)
  if (!text) return null
  const old = stale(f, now)
  return (
    <p
      className={cn(
        "flex min-w-0 items-center gap-1.5 text-xs",
        old ? "text-warning" : "text-muted-foreground",
        className
      )}
    >
      <RefreshCwIcon className="size-3 shrink-0" aria-hidden />
      <span className="min-w-0">
        {text}
        {old && staleTail}
      </span>
    </p>
  )
}

// FreshnessShort is the same in a few words, for a tile or a menu row.
export function FreshnessShort({
  log,
  className,
}: {
  log: string
  className?: string
}) {
  const all = useFreshness()
  const now = useNow(30_000)
  const f = all.get(log)
  const text = shortLine(f, now)
  if (!text) return null
  return (
    <span
      className={cn(
        stale(f, now) ? "text-warning" : "text-muted-foreground",
        className
      )}
      title={freshLine(f, now)}
    >
      {text}
    </span>
  )
}

function lower(s: string): string {
  return s.charAt(0).toLowerCase() + s.slice(1)
}

// HomeFreshness is Home's line for the picked space. All spaces names only
// the space that has gone longest without catching up, once it's stale.
export function HomeFreshness({
  space,
  names,
}: {
  space: string
  names: Map<string, string>
}) {
  const all = useFreshness()
  const now = useNow(30_000)
  if (space !== AllSpaces) return <FreshnessLine log={space} />
  const f = stalest(all, now)
  if (!f) return null
  const name = names.get(f.log) ?? "Personal"
  return (
    <p className="flex min-w-0 items-center gap-1.5 text-xs text-warning">
      <RefreshCwIcon className="size-3 shrink-0" aria-hidden />
      <span className="min-w-0">
        {name}: {lower(freshLine(f, now))}
        {staleTail}
      </span>
    </p>
  )
}
