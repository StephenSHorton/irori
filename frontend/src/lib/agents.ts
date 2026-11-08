// Settings › Agents (card #448, docs/design/spawned-agents.md §10, sa-6):
// a board's spawned-agent switch and the model and effort its agents start
// with. The app only writes them; no tool can. Outside the app the preview
// keeps a sample in memory so the pane can be looked at.

import { useCallback, useEffect, useState } from "react"

import type {
  CardWeight,
  AgentsChange,
  AgentStartsView,
  AgentsLimits,
  AgentsPauseView,
  AgentsSettings,
  RunningAgent,
  TermInfo,
  TrustedRepo,
} from "../gen/bindings"
import { AgentsService, Events } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

export type {
  AgentsChange,
  AgentsLimits,
  AgentsPauseView,
  AgentStartsView,
  AgentsSettings,
  RunningAgent,
  TrustedRepo,
}

// sampleAgents is the preview's private board: chosen and on, or with
// ?spawn=first on by default and not yet chosen, so Home shows the
// first-run notice. A function, so tests (no window) can import this file.
export function sampleAgents(): AgentsSettings {
  return {
    boardId: "",
    on: previewFlag("spawn") !== "off",
    chosen: previewFlag("spawn") !== "first",
    model: "sonnet",
    effort: "medium",
    models: ["sonnet", "opus"],
    efforts: ["low", "medium", "high", "xhigh", "max"],
    defaultModel: "sonnet",
    defaultEffort: "medium",
    lightModel: "sonnet",
    lightEffort: "medium",
    heavyModel: "opus",
    heavyEffort: "high",
    idleMins: 30,
    idleChoices: [10, 30, 60, 120, 240],
    crashLimit: 2,
    startTries: 3,
    defaultCrashLimit: 2,
    defaultStartTries: 3,
  }
}

const modelNames: Record<string, string> = {
  sonnet: "Sonnet",
  opus: "Opus",
  haiku: "Haiku",
}

const effortNames: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
}

// choiceLabel names a model or effort, marking the default.
export function choiceLabel(
  v: string,
  names: Record<string, string>,
  def: string
): string {
  const n = names[v] ?? v
  return v === def ? `${n} (default)` : n
}

export const modelLabel = (v: string, def: string) =>
  choiceLabel(v, modelNames, def)
export const effortLabel = (v: string, def: string) =>
  choiceLabel(v, effortNames, def)

// A card's weight (card #963): light, normal or heavy, each mapped to a
// model and effort per board in Settings › Agents. No weight runs as normal.
export const WEIGHT_LEVELS = ["light", "normal", "heavy"] as const

// shortModel is a model and effort in brief, as a card's label wants it:
// "Opus high", "Sonnet extra high".
export function shortModel(model: string, effort: string): string {
  const m = modelNames[model] ?? model
  const e = (effortNames[effort] ?? effort).toLowerCase()
  return [m, e].filter(Boolean).join(" ")
}

// weightLabel is a card's weight as it shows: "heavy · Opus high: planning
// the split", the reason only when there is one.
export function weightLabel(w: CardWeight): string {
  const run = shortModel(w.model, w.effort)
  const head = run ? `${w.level} · ${run}` : w.level
  const why = w.reason?.trim()
  return why ? `${head}: ${why}` : head
}

// weightBy says who set a weight, for its tooltip: the person's own is
// theirs, and agents can't change it.
export function weightBy(w: CardWeight): string {
  if (w.setBy === "assistant") return "Irori's assistant set it"
  if (w.setBy === "person")
    return w.byName && w.byName !== "you"
      ? `${w.byName} set it; agents can't change it`
      : "You set it; agents can't change it"
  return `${w.byName || "An agent"} set it`
}

// idleLabel names an idle limit in minutes, marking the default.
export function idleLabel(m: number, def = 30): string {
  const n =
    m % 60 === 0 ? `${m / 60} hour${m === 60 ? "" : "s"}` : `${m} minutes`
  return m === def ? `${n} (default)` : n
}

// "My agents may start agents on my cards" (card #575): the person's own
// limit, on all their devices. Only this pane changes it; no agent's tool can.
// The preview's sample, read when used: tests load this file outside a page.
function sampleStarts(): AgentStartsView {
  return {
    on: previewFlag("agentstarts") !== "off",
    max: 100,
    used: previewFlag("agentstarts") === "full" ? 100 : 3,
    ceiling: 1000,
  }
}

export function useAgentStarts(open: boolean) {
  const [s, setS] = useState<AgentStartsView | null>(
    inWails() ? null : sampleStarts()
  )
  const [error, setError] = useState("")
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    AgentsService.Starts().then(
      (v) => live && (setS(v), setError("")),
      (e: Error) => live && setError(e.message)
    )
    return () => {
      live = false
    }
  }, [open])
  // set settles once the change is saved or refused (an error).
  const set = (on: boolean, max: number): Promise<void> => {
    if (!inWails()) {
      setS((p) => p && { ...p, on, max })
      return Promise.resolve()
    }
    return AgentsService.SetStarts(on, max).then(
      (v) => (setS(v), setError("")),
      (e: Error) => setError(e.message)
    )
  }
  return { starts: s, error, set }
}

// The preview's settings by board; a space's switch starts off, as it does
// in the app.
const sampleSaved = new Map<string, AgentsSettings>()
function sampleFor(boardId: string): AgentsSettings {
  const s = sampleAgents()
  return sampleSaved.get(boardId) ?? { ...s, boardId, on: !boardId && s.on }
}

// Every open copy of a board's settings hears a save (card #573): a
// space's switch and its off line sit on one page.
const saved = new Set<(s: AgentsSettings) => void>()
function publish(s: AgentsSettings) {
  for (const fn of saved) fn(s)
}

// useAgents loads a board's settings while open and saves a change,
// keeping the last good settings when a save fails.
export function useAgents(open: boolean, boardId = "") {
  const first = () => (inWails() ? null : sampleFor(boardId))
  const [s, setS] = useState<AgentsSettings | null>(first)
  const [error, setError] = useState("")
  // Another board's settings are never shown as this one's.
  const [shown, setShown] = useState(boardId)
  if (shown !== boardId) {
    setShown(boardId)
    setS(first())
  }
  useEffect(() => {
    const fn = (v: AgentsSettings) => {
      if (v.boardId === boardId) setS(v)
    }
    saved.add(fn)
    return () => {
      saved.delete(fn)
    }
  }, [boardId])
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    AgentsService.Settings(boardId).then(
      (v) => live && (setS(v), setError("")),
      (e: Error) => live && setError(e.message)
    )
    return () => {
      live = false
    }
  }, [open, boardId])
  const set = useCallback(
    (c: Omit<AgentsChange, "boardId">) => {
      const change: AgentsChange = {
        boardId,
        on: c.on ?? null,
        model: c.model ?? null,
        effort: c.effort ?? null,
        lightModel: c.lightModel ?? null,
        lightEffort: c.lightEffort ?? null,
        heavyModel: c.heavyModel ?? null,
        heavyEffort: c.heavyEffort ?? null,
        idleMins: c.idleMins ?? null,
        crashLimit: c.crashLimit ?? null,
        startTries: c.startTries ?? null,
      }
      if (!inWails()) {
        const x = sampleFor(boardId)
        const next = {
          ...x,
          on: c.on ?? x.on,
          chosen: x.chosen || c.on != null,
          model: c.model ?? x.model,
          effort: c.effort ?? x.effort,
          lightModel: c.lightModel ?? x.lightModel,
          lightEffort: c.lightEffort ?? x.lightEffort,
          heavyModel: c.heavyModel ?? x.heavyModel,
          heavyEffort: c.heavyEffort ?? x.heavyEffort,
          idleMins: c.idleMins ?? x.idleMins,
          crashLimit: c.crashLimit ?? x.crashLimit,
          startTries: c.startTries ?? x.startTries,
        }
        sampleSaved.set(boardId, next)
        publish(next)
        return
      }
      AgentsService.Set(change).then(
        (v) => (publish(v), setError("")),
        (e: Error) => setError(e.message)
      )
    },
    [boardId]
  )
  // loading is true until the first read lands, for the pane's skeleton
  // (card #598).
  return { settings: s, error, loading: s === null && !error, set }
}

// sampleTrusted is the preview's repos on this computer, one not trusted.
export function sampleTrusted(): TrustedRepo[] {
  const row = (label: string, trusted: boolean): TrustedRepo => {
    const folder = `C:/Users/you/projects/${label}`
    return {
      repoKey: `remote:github.com/sample-org/${label}`,
      label,
      folder,
      trusted,
      what: `Claude Code will run agents in ${folder} on this computer without asking you first. It's the trust Claude Code asks for the first time you open a folder.`,
    }
  }
  return [row("app-repo", false), row("game-repo", true)]
}

// useTrusted loads this computer's repos and whether each is trusted for
// agents (card #456, sa-6) while open; trust records one after the
// person's confirm, and throws its error for the confirm to show.
export function useTrusted(open: boolean) {
  const [rows, setRows] = useState<TrustedRepo[]>(() =>
    inWails() ? [] : sampleTrusted()
  )
  // An empty list is a real answer, so the first read is its own flag.
  const [loaded, setLoaded] = useState(() => !inWails())
  const [error, setError] = useState("")
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    AgentsService.Trusted().then(
      (v) => live && (setRows(v), setLoaded(true), setError("")),
      (e: Error) => live && setError(e.message)
    )
    return () => {
      live = false
    }
  }, [open])
  const trust = useCallback(async (repoKey: string) => {
    if (!inWails()) {
      setRows((rs) =>
        rs.map((r) => (r.repoKey === repoKey ? { ...r, trusted: true } : r))
      )
      return
    }
    setRows(await AgentsService.TrustRepo(repoKey))
  }, [])
  return { rows, error, loading: !loaded && !error, trust }
}

// endAgent ends an agent Irori started (card #448): its hand-off on the
// card, then stop and rm. The preview has nothing to end.
export async function endAgent(key: string): Promise<void> {
  if (!inWails()) return
  await AgentsService.End(key)
}

// setAgentOffer turns "Offer it cards" on or off for one of the person's
// terminal agents (card #453); roster:changed brings the new value. The
// preview has no agents to change.
export async function setAgentOffer(key: string, on: boolean): Promise<void> {
  if (!inWails()) return
  await AgentsService.OfferSet(key, on)
}

// TermOpened is the event Attach raises once a card's terminal has a new
// tab (card #456): the card page's Terminal section shows it.
export const TermOpened = "irori:term-opened"
export type TermOpenedDetail = { cardId: string; term: TermInfo }

// attachAgent opens a tab in the agent's card terminal running `claude
// attach` (card #456, sa-2): typing there talks to it, and it stays
// spawned. The preview has no terminals, so it only says what would open.
export async function attachAgent(key: string, cardId: string): Promise<void> {
  if (!inWails()) return
  const term = await AgentsService.Attach(key, 100, 24)
  window.dispatchEvent(
    new CustomEvent<TermOpenedDetail>(TermOpened, { detail: { cardId, term } })
  )
}

// agentLogs is a spawned agent's recent terminal output, read only.
export async function agentLogs(key: string): Promise<string> {
  if (!inWails()) return sampleLogs
  return AgentsService.Logs(key)
}

const sampleLogs = [
  "● Read src/world/lantern.rs (212 lines)",
  "● The flicker comes from two timers: tick_light() and the dock's day/night fade both write alpha.",
  "● Update(src/world/lantern.rs)",
  "  ⎿ Updated with 9 additions and 14 removals",
  "● Bash(cargo test -p game lantern)",
  "  ⎿ test result: ok. 12 passed; 0 failed",
  "● irori - board_step (MCP)(task_id: 58, step: 3, state: done)",
  "✻ Checking the night scene for the second flicker…",
].join("\n")

// sampleRunning is the preview's Running agents list: two working, one
// running a job between turns (card #489), one on another device, one
// starting.
function sampleRunning(): RunningAgent[] {
  const now = Date.now()
  const row = (p: Partial<RunningAgent>): RunningAgent => ({
    key: "",
    name: "",
    status: "busy",
    cardId: "",
    cardShort: "",
    cardTitle: "",
    device: "Macbook Pro (this one)",
    here: true,
    model: "sonnet",
    effort: "medium",
    startedAt: now,
    ...p,
  })
  return [
    row({
      status: "starting",
      cardId: "c65",
      cardShort: "#65",
      cardTitle: "Explain the cache eviction in the README",
      startedAt: now - 20_000,
    }),
    row({
      key: "dev/heron",
      name: "heron",
      cardId: "c64",
      cardShort: "#64",
      cardTitle: "Retry the upload on a dropped connection",
      startedAt: now - 6 * 60_000,
    }),
    row({
      key: "dev/finch",
      name: "finch",
      status: "idle",
      cardId: "c61",
      cardShort: "#61",
      cardTitle: "Bump the icon set",
      model: "opus",
      effort: "high",
      startedAt: now - 25 * 60_000,
    }),
    row({
      key: "sample-device/sample05-synthetic-session",
      name: "sample-mink",
      status: "shell",
      cardId: "c63",
      cardShort: "#63",
      cardTitle: "Migrate the sample store",
      startedAt: now - 30 * 60_000,
    }),
    row({
      key: "pc/wren",
      name: "wren",
      cardId: "c58",
      cardShort: "#58",
      cardTitle: "Port the build script to Windows",
      device: "Desktop Computer",
      here: false,
      startedAt: now - 40 * 60_000,
    }),
  ]
}

// useRunning lists the agents Irori started, and the ones starting,
// refreshed every few seconds while open.
export function useRunning(open: boolean) {
  const [rows, setRows] = useState<RunningAgent[]>(() =>
    inWails() ? [] : sampleRunning()
  )
  // An empty list is a real answer, so the first read is its own flag; the
  // poll after it never shows the skeleton again (card #598).
  const [loaded, setLoaded] = useState(() => !inWails())
  const [error, setError] = useState("")
  const [tick, setTick] = useState(0)
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    const load = () =>
      AgentsService.Running().then(
        (v) => live && (setRows(v), setLoaded(true), setError("")),
        (e: Error) => live && setError(e.message)
      )
    void load()
    const id = window.setInterval(load, 4000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [open, tick])
  const refresh = useCallback(() => setTick((t) => t + 1), [])
  return { rows, error, loading: !loaded && !error, refresh }
}

// TRIES_CHOICES are the crashes or failed starts before Irori asks you
// (card #576). A number set some other way is added to the list where it's
// shown.
export const TRIES_CHOICES = [1, 2, 3, 5, 10, 20]

// withChoice is the choices with n among them, in order.
export function withChoice(choices: number[], n: number): number[] {
  return choices.includes(n) ? choices : [...choices, n].sort((a, b) => a - b)
}

// countLabel names a number with its unit, marking the default.
export function countLabel(
  n: number,
  def: number,
  one: string,
  many: string
): string {
  const s = `${n} ${n === 1 ? one : many}`
  return n === def ? `${s} (default)` : s
}

// costLabel says what today's agents used at API prices: "$1.24",
// "under $0.01", "$0".
export function costLabel(usd: number | null | undefined): string {
  const n = usd ?? 0
  if (n <= 0) return "$0"
  if (n < 0.01) return "under $0.01"
  return `$${n.toFixed(2)}`
}

// sampleLimits is the preview's caps and today (sa-6): this Mac at 6 of
// 12, the PC at its default, 14 agents today. ?spawn=limit holds the
// queue on a usage limit.
function sampleLimits(): AgentsLimits {
  return {
    devices: [
      {
        device: "mac",
        name: "Macbook Pro (this one)",
        here: true,
        cap: 12,
        chosen: false,
        running: 6,
      },
      {
        device: "pc",
        name: "Desktop Computer",
        here: false,
        cap: 6,
        chosen: true,
        running: 1,
      },
    ],
    maxCap: 200,
    today: 14,
    todayHere: 11,
    costUsd: 3.42,
    heldUntil:
      previewFlag("spawn") === "limit" ? Date.now() + 90 * 60_000 : null,
    queued: 2,
  }
}

// useLimits loads Settings › Agents' caps and today's count and cost,
// refreshed every few seconds while open, sets a device's cap, and ends a
// usage hold here early.
export function useLimits(open: boolean) {
  const [l, setL] = useState<AgentsLimits | null>(() =>
    inWails() ? null : sampleLimits()
  )
  const [error, setError] = useState("")
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    const load = () =>
      AgentsService.Limits().then(
        (v) => live && (setL(v), setError("")),
        (e: Error) => live && setError(e.message)
      )
    void load()
    const id = window.setInterval(load, 4000)
    return () => {
      live = false
      window.clearInterval(id)
    }
  }, [open])
  // setCap settles once the cap is saved or refused (an error).
  const setCap = useCallback((device: string, cap: number): Promise<void> => {
    if (!inWails()) {
      setL((x) =>
        x
          ? {
              ...x,
              devices: x.devices.map((d) =>
                d.device === device ? { ...d, cap, chosen: true } : d
              ),
            }
          : x
      )
      return Promise.resolve()
    }
    return AgentsService.CapSet(device, cap).then(
      (v) => (setL(v), setError("")),
      (e: Error) => setError(e.message)
    )
  }, [])
  const releaseHold = useCallback(() => {
    if (!inWails()) {
      setL((x) => (x ? { ...x, heldUntil: undefined } : x))
      return
    }
    AgentsService.UsageRelease().then(
      (v) => (setL(v), setError("")),
      (e: Error) => setError(e.message)
    )
  }, [])
  return { limits: l, error, setCap, releaseHold }
}

// runningSince says how long ago an agent was asked for: "just now",
// "6 min", "2 h".
export function runningSince(at: number, now: number): string {
  const m = Math.floor((now - at) / 60_000)
  if (m < 1) return "just now"
  if (m < 60) return `${m} min`
  return `${Math.floor(m / 60)} h`
}

// Pause agents (card #665): this device's switch that stops Irori starting
// or restarting any agent here. Running agents keep running. Its agents may
// set it too (the agents_pause tool), so it is read again on the event, on
// focus and every few seconds. The preview's sample: `?agentspause=on`.
function samplePause(): AgentsPauseView {
  return previewFlag("agentspause") === "on"
    ? {
        on: true,
        since: Date.now() - 42 * 60_000,
        by: "the person on MacBook Pro",
      }
    : { on: false, since: 0, by: "" }
}

export const EVENT_AGENTS_PAUSED = "agents:paused"

export function useAgentsPause() {
  const [p, setP] = useState<AgentsPauseView | null>(() =>
    inWails() ? null : samplePause()
  )
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (!inWails()) return
    let live = true
    const load = () =>
      AgentsService.Pause().then(
        (v) => live && (setP(v), setError("")),
        (e: Error) => live && setError(e.message)
      )
    void load()
    const id = window.setInterval(load, 10_000)
    const off = Events.On<AgentsPauseView>(EVENT_AGENTS_PAUSED, (ev) => {
      if (live && ev.data) setP(ev.data)
    })
    window.addEventListener("focus", load)
    return () => {
      live = false
      window.clearInterval(id)
      window.removeEventListener("focus", load)
      off()
    }
  }, [])
  const set = useCallback((on: boolean) => {
    if (!inWails()) {
      setP(
        on
          ? { on: true, since: Date.now(), by: "the person on MacBook Pro" }
          : { on: false, since: 0, by: "the person on MacBook Pro" }
      )
      return
    }
    setBusy(true)
    AgentsService.PauseSet(on)
      .then(
        (v) => (setP(v), setError("")),
        (e: Error) => setError(e.message)
      )
      .finally(() => setBusy(false))
  }, [])
  return { pause: p, error, busy, set }
}
