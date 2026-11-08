// Claude usage gateway (card #599): how much of the plan's 5-hour and weekly
// windows is left, as Claude Code last reported it on this computer. The
// host reads it from a reply's rate limits and sends usage:changed; the
// window asks once with usage_limits. Components import from here, never
// from src/gen or host.ts. In a plain browser a sample reading shows
// (?usage=none: no reading yet, ?usage=stale: one from 40 minutes ago,
// ?usage=20: both windows with 20% left, to see the bars' colors).

import { useSyncExternalStore } from "react"

import type * as B from "../gen/bindings"
import { isAndroid } from "../updater/android"
import { Events, UsageService } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

// One plan window: kind is "five_hour", "seven_day", or another the
// gateway reports ("spend_limit"); percentUsed is 0 to 100 (a spend limit
// can go past 100); resetsAtMs is null when not said.
export type UsageWindow = B.UsageWindow
// observedAtMs is when the newest reading was taken, 0 before the first.
export type UsageLimits = B.UsageLimits

export const EventUsageChanged = "usage:changed"

export const NO_USAGE: UsageLimits = { observedAtMs: 0, windows: [] }

const MIN = 60_000
const HOUR = 60 * MIN
// STALE_MS: a reading older than this says when it was taken.
export const STALE_MS = 10 * MIN

// ---- pure helpers -------------------------------------------------------------

// hasUsage: there is a reading with at least one window.
export function hasUsage(u: UsageLimits): boolean {
  return u.observedAtMs > 0 && u.windows.length > 0
}

// isReset: the window has started over since the reading.
export function isReset(w: UsageWindow, now: number): boolean {
  return w.resetsAtMs != null && w.resetsAtMs <= now
}

// percentLeft is what is left of a window, a whole 0 to 100: all of it once
// the window has started over.
export function percentLeft(w: UsageWindow, now: number): number {
  if (isReset(w, now)) return 100
  return Math.round(Math.min(Math.max(100 - w.percentUsed, 0), 100))
}

// windowLabel is a window's name: "5-hour", "Weekly", "Spend limit".
export function windowLabel(kind: string): string {
  switch (kind) {
    case "five_hour":
      return "5-hour"
    case "seven_day":
      return "Weekly"
    case "seven_day_opus":
      return "Weekly (Opus)"
    case "seven_day_sonnet":
      return "Weekly (Sonnet)"
    default: {
      const words = kind.replace(/[_-]+/g, " ").trim()
      return words ? words[0].toUpperCase() + words.slice(1) : kind
    }
  }
}

const ORDER = ["five_hour", "seven_day"]

// sortWindows puts the 5-hour and weekly windows first, the rest after.
export function sortWindows(ws: readonly UsageWindow[]): UsageWindow[] {
  const rank = (k: string) => {
    const i = ORDER.indexOf(k)
    return i < 0 ? ORDER.length : i
  }
  return [...ws].sort((a, b) => rank(a.kind) - rank(b.kind))
}

const pad = (n: number) => String(n).padStart(2, "0")
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]

// clock is a local "14:05".
export function clock(ms: number): string {
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function sameDay(a: number, b: number): boolean {
  return new Date(a).toDateString() === new Date(b).toDateString()
}

// resetLine says when a window starts over: "resets in 2h 10m" within a
// day, "resets Thu 14:00" further off, "reset" once it has.
export function resetLine(
  resetsAtMs: number | null,
  now: number
): string | null {
  if (resetsAtMs == null) return null
  const left = resetsAtMs - now
  if (left <= 0) return "reset"
  if (left < MIN) return "resets in under a minute"
  if (left < 24 * HOUR) {
    const h = Math.floor(left / HOUR)
    const m = Math.floor((left % HOUR) / MIN)
    return h ? `resets in ${h}h ${m}m` : `resets in ${m}m`
  }
  return `resets ${DAYS[new Date(resetsAtMs).getDay()]} ${clock(resetsAtMs)}`
}

// staleLine says when an old reading was taken: "as of 14:05", "as of Tue
// 14:05" on another day; null when it is recent.
export function staleLine(observedAtMs: number, now: number): string | null {
  if (observedAtMs <= 0 || now - observedAtMs <= STALE_MS) return null
  const day = sameDay(observedAtMs, now)
    ? ""
    : `${DAYS[new Date(observedAtMs).getDay()]} `
  return `as of ${day}${clock(observedAtMs)}`
}

// Usage color (card #772): a bar is the theme color while plenty is left,
// turns toward amber as it runs low, then toward red near none. The stops
// are % left: at or above CALM it is all primary, at AMBER all warning, at 0
// all destructive; between two stops the color is mixed from the theme's own
// tokens, so it holds in every theme, light and dark. Primary to amber mixes
// in oklab: a straight line, where oklch would swing round the hue wheel
// (blue to amber goes by cyan in one theme and purple in another). Amber to
// red is close in hue, so it mixes in oklch and keeps its strength.
export const USAGE_CALM = 50
export const USAGE_AMBER = 20
// USAGE_LOW: at or under this % left the header strip's reading is tinted.
export const USAGE_LOW = 25

// mix is CSS for `a` with `pct`% of `b` mixed in `space`, or just the end
// token when pct is 0 or 100.
function mix(space: string, a: string, b: string, pct: number): string {
  const p = Math.round(pct)
  if (p <= 0) return `var(${a})`
  if (p >= 100) return `var(${b})`
  return `color-mix(in ${space}, var(${a}), var(${b}) ${p}%)`
}

// usageColor is a CSS color for a reading with `left`% left.
export function usageColor(left: number): string {
  const l = Math.min(Math.max(left, 0), 100)
  if (l >= USAGE_CALM) return "var(--primary)"
  if (l >= USAGE_AMBER)
    return mix(
      "oklab",
      "--primary",
      "--warning",
      ((USAGE_CALM - l) / (USAGE_CALM - USAGE_AMBER)) * 100
    )
  return mix(
    "oklch",
    "--warning",
    "--destructive",
    ((USAGE_AMBER - l) / USAGE_AMBER) * 100
  )
}

// usageTextColor is the header strip's tint for a reading with `left`% left:
// none (the strip's own muted text) until it is low, then the bar's color.
export function usageTextColor(left: number): string | null {
  return left <= USAGE_LOW ? usageColor(left) : null
}

// StripLine is the top bar's reading of what is left: text is "62% · 39%
// left" (5-hour, then weekly), title spells it out.
// color tints it by the lowest window shown, null while plenty is left.
export type StripLine = { text: string; title: string; color: string | null }

// stripLine is the top bar's Claude reading, or null with no reading. With
// no 5-hour or weekly window (a gateway's spend limit only) it shows the
// first window there is.
export function stripLine(u: UsageLimits, now: number): StripLine | null {
  if (!hasUsage(u)) return null
  let shown = u.windows.filter((w) => ORDER.includes(w.kind))
  shown = shown.length ? sortWindows(shown) : sortWindows(u.windows).slice(0, 1)
  const lefts = shown.map((w) => percentLeft(w, now))
  const pcts = lefts.map((l) => `${l}%`)
  const title = shown
    .map((w, i) => {
      const label = windowLabel(w.kind)
      return `${i ? label.toLowerCase() : label} ${pcts[i]} left`
    })
    .join(", ")
  return {
    text: `${pcts.join(" · ")} left`,
    title,
    color: usageTextColor(Math.min(...lefts)),
  }
}

// asLimits reads the host's answer: no answer is no reading.
export function asLimits(v: UsageLimits | null | undefined): UsageLimits {
  return v ?? NO_USAGE
}

// sampleUsage is the preview's reading, taken two minutes before now.
export function sampleUsage(
  now: number,
  kind: string | null = null
): UsageLimits {
  if (kind === "none") return NO_USAGE
  const observed = kind === "stale" ? now - 40 * MIN : now - 2 * MIN
  const left = kind != null && /^\d+$/.test(kind) ? Number(kind) : null
  return {
    observedAtMs: observed,
    windows: [
      {
        kind: "five_hour",
        percentUsed: left == null ? 38 : 100 - left,
        resetsAtMs: now + 2 * HOUR + 10 * MIN,
      },
      {
        kind: "seven_day",
        percentUsed: left == null ? 61 : 100 - left,
        resetsAtMs: now + 3 * 24 * HOUR,
      },
    ],
  }
}

// ---- the store -------------------------------------------------------------------

const state = {
  value: NO_USAGE,
  readers: new Set<() => void>(),
  off: null as (() => void) | null,
}

function set(next: UsageLimits) {
  state.value = next
  for (const fn of state.readers) fn()
}

function follow(): () => void {
  if (isAndroid()) return () => {}
  if (!inWails()) {
    set(sampleUsage(Date.now(), previewFlag("usage")))
    return () => {}
  }
  let live = true
  let heard = false
  // The reading so far is asked for once the listener is live, so a
  // usage:changed can't fall between the two.
  const off = Events.On<UsageLimits>(
    EventUsageChanged,
    (ev) => {
      heard = true
      set(asLimits(ev.data))
    },
    () => {
      UsageService.Limits()
        .then((v) => {
          if (live && !heard) set(asLimits(v))
        })
        .catch(() => {})
    }
  )
  return () => {
    live = false
    off()
  }
}

export function subscribeUsage(reader: () => void): () => void {
  state.readers.add(reader)
  state.off ??= follow()
  return () => {
    state.readers.delete(reader)
    if (state.readers.size === 0) {
      state.off?.()
      state.off = null
    }
  }
}

export function usageNow(): UsageLimits {
  return state.value
}

// useUsage is the newest reading of Claude's plan windows (NO_USAGE before
// one; always on the phone).
export function useUsage(): UsageLimits {
  return useSyncExternalStore(subscribeUsage, usageNow, usageNow)
}

// ---- a clock for countdowns ---------------------------------------------------------

const clockState = {
  now: Date.now(),
  readers: new Set<() => void>(),
  timer: 0 as ReturnType<typeof setInterval> | 0,
}

function subscribeClock(reader: () => void): () => void {
  clockState.readers.add(reader)
  if (!clockState.timer) {
    clockState.now = Date.now()
    clockState.timer = setInterval(() => {
      clockState.now = Date.now()
      for (const fn of clockState.readers) fn()
    }, 15_000)
  }
  return () => {
    clockState.readers.delete(reader)
    if (clockState.readers.size === 0 && clockState.timer) {
      clearInterval(clockState.timer)
      clockState.timer = 0
    }
  }
}

const clockNow = () => clockState.now

// useNow is the time, kept within 15 s, for "resets in" and "as of".
export function useNow(): number {
  return useSyncExternalStore(subscribeClock, clockNow, clockNow)
}
