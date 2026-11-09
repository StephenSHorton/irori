// What the window's lists read and which signals reached it, for irori.log
// (card #551). A list that doesn't update has two explanations the Core
// can't tell apart: reads that are slow or dropped, or no signal reaching
// the window so nothing reloads until a focus. One line a minute says which:
// a slow device logging each read would rotate the 4 MiB log away.
//
// A minute with nothing to say writes no line: no signal came and no read
// ended, or the page itself wasn't running. A window started hidden on a
// Mac wrote no line in 3.5 minutes, so no lines there say nothing about
// signals. Signals with no reads after them mean a read that hasn't
// answered.

import { WindowLog, Events } from "./host"
import { onIpc } from "./ipc-count"
import { inWails } from "./irori"

// SlowReadMs is when a list's read counts as slow.
export const SlowReadMs = 1000
// WindowLogMs is how often the tally goes to the log.
export const WindowLogMs = 60_000
// DroppedInARow is how many answers dropped in a row get named.
export const DroppedInARow = 3

// The signals that reload lists, as useFresh and subscribe listen for them.
export const Signals = [
  "board:changed",
  "devices:changed",
  "inbox:changed",
  "talk:changed",
  "notifications:changed",
  "boards:changed",
  "roster:changed",
  "assistant:changed",
]

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
  let ipc = new Map<string, number>()
  // The drift check (card #1146, changes.ts/patched.ts): per list, rows
  // that differed in the compared samples and how many were compared, and
  // the samples dropped because a notice came during them.
  let drifts = new Map<string, { rows: number; samples: number }>()
  let driftDropped = 0

  return {
    // drift notes one compared sample of the list called name.
    drift(name: string, rows: number) {
      const d = drifts.get(name) ?? { rows: 0, samples: 0 }
      drifts.set(name, { rows: d.rows + rows, samples: d.samples + 1 })
    },
    // driftDropped notes a sample dropped without comparing.
    driftDropped() {
      driftDropped++
    },
    signal(name: string) {
      signals.set(name, (signals.get(name) ?? 0) + 1)
    },
    // call counts one IPC call to the Rust side, by command name.
    call(command: string) {
      ipc.set(command, (ipc.get(command) ?? 0) + 1)
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
        parts.push([...signals].map(([name, n]) => `${name} ${n}`).join(", "))
      if (reads) {
        let s = `${reads} reads for ${reloads} reloads`
        if (slow)
          s += `, ${slow} over ${SlowReadMs / 1000}s, slowest ${slowest.name} ${(slowest.ms / 1000).toFixed(1)}s`
        parts.push(s)
      }
      if (dropped) {
        let s = `${dropped} answers dropped`
        if (worst.n >= DroppedInARow)
          s += `, ${worst.n} in a row on ${worst.name}`
        parts.push(s)
      }
      if (drifts.size || driftDropped) {
        const names = [...drifts]
          .sort((a, b) => (a[0] < b[0] ? -1 : 1))
          .map(([name, d]) => `${name} ${d.rows} of ${d.samples}`)
        if (driftDropped) names.push(`dropped ${driftDropped}`)
        parts.push(`drift ${names.join(", ")}`)
      }
      if (ipc.size)
        parts.push(
          `ipc: ${[...ipc]
            .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1))
            .map(([name, n]) => `${name} ${n}`)
            .join(", ")}`
        )
      signals = new Map()
      drifts = new Map()
      driftDropped = 0
      ipc = new Map()
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

// noteDrift counts one compared drift sample of the list called name: how
// many rows differed. noteDriftDropped counts one dropped without comparing.
export function noteDrift(name: string, rows: number) {
  tally.drift(name, rows)
}
export function noteDriftDropped() {
  tally.driftDropped()
}

// timedRead runs a read of the list called name and counts it, one reload
// per call, the way useLatest does for its lists. A path that has no
// latestReads of its own (the card store, the follower, the closed tab…)
// goes through here so the line counts what it reads too.
export async function timedRead<T>(
  name: string,
  read: () => Promise<T>
): Promise<T> {
  const since = Date.now()
  try {
    return await read()
  } finally {
    noteRead(name, { ms: Date.now() - since, dropped: false, reloads: 1 })
  }
}

// startWindowLog counts the signals reaching this window and writes the
// tally each minute, until the function it returns is called.
export function startWindowLog(): () => void {
  if (!inWails()) return () => {}
  const offs = Signals.map((name) => Events.On(name, () => tally.signal(name)))
  const offIpc = onIpc((command) => {
    // The line itself would keep every quiet minute from being quiet.
    if (command !== "window_log") tally.call(command)
  })
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
    offIpc()
  }
}
