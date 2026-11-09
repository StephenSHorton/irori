// <Ago at={time}/> is "5m ago" that keeps itself current (card #1137). Only
// this text node re-renders as the clock turns over (every second under a
// minute, then once a minute, an hour, a day), so the page around it, and
// its parent, need no `now`.

import { relativeTime } from "@/lib/irori"
import { useClockValue } from "@/lib/clock"

export function Ago({ at }: { at: Date | null }) {
  return <>{useClockValue((now) => relativeTime(at, now))}</>
}

/**
 * Tick is any text built from the time: `of(now)` is re-read every second and
 * the text re-renders only when it changes.
 */
export function Tick({ of }: { of: (now: Date) => string }) {
  return <>{useClockValue(of)}</>
}
