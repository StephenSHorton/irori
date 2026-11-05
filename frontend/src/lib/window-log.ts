// What the window's lists read and which signals reached it, for irori.log
// (card #551). A list that doesn't update has two explanations the Core
// can't tell apart: reads that are slow or dropped, or no signal reaching
// the window so nothing reloads until a focus. One line a minute says which:
// a slow device logging each read would rotate the 4 MiB log away.
//
// A minute with nothing to say writes no line: no signal came and no read
// ended, or the page itself wasn't running. Signals with no reads after
// them mean a read that hasn't answered.

import { WindowLog, Events } from "./host"
import { inWails } from "./irori"

// SlowReadMs is when a list's read counts as slow.
export const SlowReadMs = 1000
// WindowLogMs is how often the tally goes to the log.
export const WindowLogMs = 60_000
// DroppedInARow is how many answers dropped in a row get named.
export const DroppedInARow = 3

// The signals that reload lists, as useFresh and subscribe listen for them.
const Signals = ["board:changed", "devices:changed", "inbox:changed"]

// ReadReport is one finished read: how long it took, whether its answer
// was dropped, and how many reloads it answered.
export type ReadReport = { ms: number; dropped: boolean; reloads: number }

export function windowTally() {
  let signals = new Map<string, number>()
  let reads = 0
  let reloads = 0
  let slow = 0
  let slowest = { name: "", ms: 0 }
  let dropped = 0
  let worst = { name: "", n: 0 }
  const inARow = new Map<string, number>()

  return {
    signal(name: string) {
      signals.set(name, (signals.get(name) ?? 0) + 1)
    },
    read(name: string, r: ReadReport) {
      reads++
      reloads += r.reloads
      if (r.ms >= SlowReadMs) slow++
      if (r.ms > slowest.ms) slowest = { name, ms: r.ms }
      const n = r.dropped ? (inARow.get(name) ?? 0) + 1 : 0
      inARow.set(name, n)
      if (r.dropped) dropped++
      if (n > worst.n) worst = { name, n }
    },
    // line is the tally so far, "" with nothing to say, and starts a new one.
    line(): string {
      const parts: string[] = []
      if (signals.size)
        parts.push(
          [...signals].map(([name, n]) => `${name} ${n}`).join(", ")
        )
      if (reads) {
        let s = `${reads} reads for ${reloads} reloads`
        if (slow)
          s += `, ${slow} over ${SlowReadMs / 1000}s, slowest ${slowest.name} ${(slowest.ms / 1000).toFixed(1)}s`
        parts.push(s)
      }
      if (dropped) {
        let s = `${dropped} answers dropped`
        if (worst.n >= DroppedInARow) s += `, ${worst.n} in a row on ${worst.name}`
        parts.push(s)
      }
      signals = new Map()
      reads = reloads = slow = dropped = 0
      slowest = { name: "", ms: 0 }
      worst = { name: "", n: 0 }
      return parts.length ? `lists: ${parts.join("; ")}` : ""
    },
  }
}

const tally = windowTally()

// noteRead counts a finished read of the list called name.
export function noteRead(name: string, r: ReadReport) {
  tally.read(name, r)
}

// startWindowLog counts the signals reaching this window and writes the
// tally each minute, until the function it returns is called.
export function startWindowLog(): () => void {
  if (!inWails()) return () => {}
  const offs = Signals.map((name) =>
    Events.On(name, () => tally.signal(name))
  )
  const onFocus = () => tally.signal("focus")
  window.addEventListener("focus", onFocus)
  const timer = window.setInterval(() => {
    const line = tally.line()
    if (line) void WindowLog(line).catch(() => {})
  }, WindowLogMs)
  return () => {
    window.clearInterval(timer)
    window.removeEventListener("focus", onFocus)
    for (const off of offs) off()
  }
}
