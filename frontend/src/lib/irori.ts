// Package-level gateway between the UI and the Go side. Components import
// from here, never from bindings/, so the Wails surface stays in one file and
// the app still runs in a plain browser on a built-in sample roster.

import { useCallback, useEffect, useRef, useState } from "react"
import { Events } from "@wailsio/runtime"

import { RosterService } from "../../bindings/github.com/StephenSHorton/irori"
import type { Identity } from "../../bindings/github.com/StephenSHorton/irori/internal/device/models"
import type { AgentCard } from "../../bindings/github.com/StephenSHorton/irori/internal/presence/models"
import type { Event as LogEvent } from "../../bindings/github.com/StephenSHorton/irori/internal/store/models"
import { sampleRoster } from "./sample"

export type { AgentCard, Identity, LogEvent }

// EventRosterChanged mirrors the Go constant in roster_service.go.
export const EventRosterChanged = "roster:changed"

// RecentLimit is how many log rows the activity panel asks for.
export const RecentLimit = 100

// Source says where the roster came from: the Go app, or the sample used
// when the page runs in a plain browser.
export type Source = "wails" | "sample"

export type Roster = {
  source: Source
  loading: boolean
  error: string | null
  device: Identity | null
  version: string
  agents: AgentCard[]
  recent: LogEvent[]
  recentError: string | null
}

// inWails reports whether the page runs inside a Wails webview. It checks the
// same IPC handles @wailsio/runtime uses (WebView2 on Windows, WKWebView /
// WebKitGTK elsewhere), so dev mode inside the app counts as Wails too.
export function inWails(): boolean {
  const w = window as unknown as {
    chrome?: { webview?: { postMessage?: unknown } }
    webkit?: { messageHandlers?: { external?: { postMessage?: unknown } } }
  }
  return Boolean(
    w.chrome?.webview?.postMessage ||
    w.webkit?.messageHandlers?.external?.postMessage
  )
}

function message(err: unknown): string {
  if (err instanceof Error) return err.message
  return String(err)
}

// useRoster loads the device, version, agents and recent log, then keeps them
// current from roster:changed. The event carries the full roster, so the UI
// replaces rather than merges; the log is re-read after each change, with
// bursts coalesced.
export function useRoster(): Roster {
  const [state, setState] = useState<Roster>(() => ({
    source: inWails() ? "wails" : "sample",
    loading: true,
    error: null,
    device: null,
    version: "",
    agents: [],
    recent: [],
    recentError: null,
  }))
  const recentTimer = useRef<number | undefined>(undefined)
  // mounted is false after unmount so late responses are dropped.
  const mounted = useRef(true)
  // recentReq numbers Recent() calls; only the newest one's answer is kept,
  // because responses can arrive out of order.
  const recentReq = useRef(0)

  const loadRecent = useCallback(async () => {
    const req = ++recentReq.current
    try {
      const recent = (await RosterService.Recent(RecentLimit)) ?? []
      if (!mounted.current || req !== recentReq.current) return
      setState((s) => ({ ...s, recent, recentError: null }))
    } catch (err) {
      if (!mounted.current || req !== recentReq.current) return
      setState((s) => ({ ...s, recentError: message(err) }))
    }
  }, [])

  useEffect(() => {
    mounted.current = true
    if (state.source === "sample") {
      const sample = sampleRoster(new Date())
      setState((s) => ({ ...s, ...sample, loading: false }))
      return () => {
        mounted.current = false
      }
    }

    let live = true
    // Calls go over fetch and events over a separate dispatch path, with no
    // ordering between them. Once a roster:changed has landed it is newer
    // than the Agents() snapshot, so that snapshot must not overwrite it.
    let rosterEvents = 0

    const off = Events.On(EventRosterChanged, (ev) => {
      rosterEvents++
      setState((s) => ({ ...s, agents: ev.data?.agents ?? [] }))
      window.clearTimeout(recentTimer.current)
      recentTimer.current = window.setTimeout(() => void loadRecent(), 250)
    })

    Promise.all([
      RosterService.Device(),
      RosterService.Version(),
      RosterService.Agents(),
    ])
      .then(([device, version, agents]) => {
        if (!live) return
        const stale = rosterEvents > 0
        setState((s) => ({
          ...s,
          device,
          version,
          agents: stale ? s.agents : (agents ?? []),
          loading: false,
          error: null,
        }))
      })
      .catch((err: unknown) => {
        if (!live) return
        setState((s) => ({ ...s, loading: false, error: message(err) }))
      })
    void loadRecent()

    return () => {
      live = false
      mounted.current = false
      off()
      window.clearTimeout(recentTimer.current)
    }
  }, [state.source, loadRecent])

  return state
}

// useNow returns the current time, re-rendering every intervalMs so relative
// times tick.
export function useNow(intervalMs = 1000): Date {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), intervalMs)
    return () => window.clearInterval(id)
  }, [intervalMs])
  return now
}

// parseTime reads a Go time.Time from JSON. Go's zero time and anything
// unparseable come back as null.
export function parseTime(v: unknown): Date | null {
  if (v instanceof Date) return Number.isNaN(v.getTime()) ? null : v
  if (typeof v !== "string" || v === "" || v.startsWith("0001-")) return null
  const d = new Date(v)
  return Number.isNaN(d.getTime()) ? null : d
}

// hlcWall is the wall-clock part of an HLC timestamp (high 48 bits, in ms).
// The uint64 arrives as a JS number, so the logical low bits may be rounded;
// dividing by 2^16 still gives the right millisecond.
export function hlcWall(ts: number): Date | null {
  if (!ts) return null
  return new Date(Math.floor(ts / 65536))
}

// relativeTime formats then against now: "just now", "42s ago", "5m ago",
// "3h ago", "2d ago".
export function relativeTime(then: Date | null, now: Date): string {
  if (!then) return "never"
  const s = Math.round((now.getTime() - then.getTime()) / 1000)
  if (s < 5) return "just now"
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h ago`
  return `${Math.floor(h / 24)}d ago`
}

// plainLine makes untrusted agent text safe to show on one line: control,
// zero-width and bidi-override characters become spaces and whitespace runs
// collapse. React escapes it; this stops it from reordering or hiding text.
export function plainLine(s: string): string {
  let out = ""
  for (const ch of s ?? "") out += hidden(ch.codePointAt(0) ?? 0) ? " " : ch
  return out.replace(/\s+/g, " ").trim()
}

// plainBlock is plainLine for multi-line text shown with pre-wrap: the same
// characters are neutralised, but newlines and tabs stay so the message
// keeps its shape. It is still rendered as a text node, never as HTML or
// markdown.
export function plainBlock(s: string): string {
  let out = ""
  for (const ch of (s ?? "").replace(/\r\n?/g, "\n")) {
    const c = ch.codePointAt(0) ?? 0
    out += c === 0x0a || c === 0x09 || !hidden(c) ? ch : " "
  }
  return out.replace(/\n{4,}/g, "\n\n\n").trim()
}

// untilTime formats a future time against now: "in 42s", "in 5m",
// "in 3h", "in 2d", or "expired".
export function untilTime(then: Date | null, now: Date): string {
  if (!then) return "never"
  const s = Math.round((then.getTime() - now.getTime()) / 1000)
  if (s <= 0) return "expired"
  if (s < 60) return `in ${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `in ${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `in ${h}h ${m % 60}m`
  return `in ${Math.floor(h / 24)}d`
}

// shortRepo names a repo_key in a row: "owner/name" for a hosted remote,
// "local repo 1a2b3c4" for a root-commit key.
export function shortRepo(key: string): string {
  if (!key) return ""
  if (key.startsWith("root:")) return `local repo ${key.slice(5, 12)}`
  const parts = key.split("/").filter(Boolean)
  return parts.length > 2 ? parts.slice(-2).join("/") : key
}

// sessionShort is the first 8 characters of a key's session part.
export function sessionShort(key: string): string {
  return (key.split("/").pop() ?? key).slice(0, 8)
}

// hidden is true for C0/C1 controls, zero-width marks, bidi embeddings,
// overrides and isolates, and the BOM.
function hidden(c: number): boolean {
  return (
    c < 0x20 ||
    (c >= 0x7f && c <= 0x9f) ||
    (c >= 0x200b && c <= 0x200f) ||
    (c >= 0x202a && c <= 0x202e) ||
    (c >= 0x2066 && c <= 0x2069) ||
    c === 0xfeff
  )
}

const homePrefix =
  /^(?:[A-Za-z]:[\\/]Users[\\/][^\\/]+|\/Users\/[^/]+|\/home\/[^/]+)(?=[\\/]|$)/

// shortPath turns a cwd into something that fits a row: the home directory
// becomes "~" and only the last two segments are kept
// (C:\Users\me\projects\a\b -> ~\…\a\b).
export function shortPath(p: string): string {
  if (!p) return "(no directory)"
  const sep = p.includes("\\") ? "\\" : "/"
  const home = p.match(homePrefix)
  const head = home ? "~" : (p.match(/^[A-Za-z]:/)?.[0] ?? "")
  const parts = p
    .slice(home ? home[0].length : head.length)
    .split(/[\\/]/)
    .filter(Boolean)
  if (parts.length > 2) return [head, "…", ...parts.slice(-2)].join(sep)
  if (home) return [head, ...parts].join(sep)
  return p
}
