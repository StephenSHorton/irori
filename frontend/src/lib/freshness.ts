// How fresh each log is (card #580, docs/design/no-server.md §4.3): when
// this device last caught up with another device, and which one, for
// Personal and for each shared space. Irori has no server, so a space is
// only as fresh as the last time a device that holds it was on at the same
// time as this one. Saying so keeps old data from looking current.
//
// Components import from here, never from src/gen. In a plain browser it
// runs on samples.

import { useSyncExternalStore } from "react"

import { EventBoardsChanged } from "./boards"
import { Events, SyncService } from "./host"
import { timedRead } from "./window-log"
import { inWails, parseTime, relativeTime } from "./irori"
import { previewFlag } from "./preview"
import { Personal } from "./spaces"

// Freshness is one log's: `log` is "personal" or a shared space's id.
export type Freshness = {
  log: string
  // When this device was last in step with another device on it; absent
  // when it hasn't been since it started and holds nothing from one.
  at?: string
  // That device, as the person would say it: "your PC", "Blake's laptop".
  with?: string
  // Another device holds it at all. False for a log only this device has
  // written (a fresh Personal with no other devices): nothing to say.
  shared: boolean
}

// StaleAfterMs is when the line turns amber: a day without catching up.
export const StaleAfterMs = 24 * 3600_000

// stale is whether f is old enough to warn about.
export function stale(f: Freshness | undefined, now: Date): boolean {
  if (!f?.shared) return false
  const at = parseTime(f.at)
  return !at || now.getTime() - at.getTime() > StaleAfterMs
}

// freshLine is the line a page shows: "Last synced with your PC 3h ago",
// "Synced with your PC just now", or "Not synced with another device since
// Irori started". "" when there's nothing to say.
export function freshLine(f: Freshness | undefined, now: Date): string {
  if (!f?.shared) return ""
  const at = parseTime(f.at)
  if (!at) return "Not synced with another device since Irori started here"
  const ago = relativeTime(at, now)
  const who = f.with ? ` with ${f.with}` : ""
  if (ago === "just now") return `Synced${who} just now`
  return `Last synced${who} ${ago}`
}

// shortLine is the same in a few words, for a tile or a menu row:
// "Synced 3h ago", "Not synced yet".
export function shortLine(f: Freshness | undefined, now: Date): string {
  if (!f?.shared) return ""
  const at = parseTime(f.at)
  if (!at) return "Not synced yet"
  return `Synced ${relativeTime(at, now)}`
}

// stalest is the log that has gone longest without catching up, among
// those old enough to warn about: All spaces names only that one.
export function stalest(
  all: Map<string, Freshness>,
  now: Date
): Freshness | undefined {
  let worst: Freshness | undefined
  let worstAt = Infinity
  for (const f of all.values()) {
    if (!stale(f, now)) continue
    const at = parseTime(f.at)?.getTime() ?? 0
    if (at < worstAt) {
      worst = f
      worstAt = at
    }
  }
  return worst
}

// ---- reading it -------------------------------------------------------------

// Refresh is how often the lines re-read while shown. A catch-up that
// brings nothing new fires no event, so a quiet log is polled.
const RefreshMs = 30_000

function sampleFreshness(): Freshness[] {
  // ?fresh=stale shows every log a few days behind (the amber lines).
  const old = previewFlag("fresh") === "stale"
  const ago = (ms: number) =>
    new Date(Date.now() - (old ? 3 * 86400_000 : ms)).toISOString()
  return [
    { log: Personal, at: ago(4 * 60_000), with: "your PC", shared: true },
    {
      log: "sample-board-mlg",
      at: ago(3 * 3600_000),
      with: "Blake",
      shared: true,
    },
    {
      log: "sample-board-site",
      at: ago(26 * 3600_000),
      with: "your phone",
      shared: true,
    },
  ]
}

async function load(): Promise<Freshness[]> {
  if (!inWails()) return sampleFreshness()
  return (await timedRead("freshness", () => SyncService.Freshness())).map(
    (f) => ({
      log: f.log,
      at: f.at,
      with: f.with || undefined,
      shared: f.shared,
    })
  )
}

// One reader for the whole window, polling while anything shows a line.
let current = new Map<string, Freshness>()
const listeners = new Set<() => void>()
let stop: (() => void) | null = null

function read() {
  void load()
    .then((fs) => {
      current = new Map(fs.map((f) => [f.log, f]))
      for (const l of listeners) l()
    })
    .catch(() => {
      // Keep the last answer: a failed read isn't news about the log.
    })
}

function subscribe(l: () => void): () => void {
  listeners.add(l)
  if (!stop) {
    read()
    const id = window.setInterval(read, RefreshMs)
    const off = inWails() ? Events.OnShown(EventBoardsChanged, read) : () => {}
    stop = () => {
      window.clearInterval(id)
      off()
    }
  }
  return () => {
    listeners.delete(l)
    if (listeners.size === 0 && stop) {
      stop()
      stop = null
    }
  }
}

// useFreshness is every log's freshness by log id, kept current while shown.
export function useFreshness(): Map<string, Freshness> {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => current
  )
}
