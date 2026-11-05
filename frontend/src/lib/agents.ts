// Settings › Agents (card #448, docs/design/spawned-agents.md §10, sa-6):
// a board's spawned-agent switch and the model and effort its agents start
// with. The app only writes them; no tool can. Outside the app the preview
// keeps a sample in memory so the pane can be looked at.

import { useCallback, useEffect, useState } from "react"

import type {
  AgentsChange,
  AgentStartsView,
  AgentsSettings,
  RunningAgent,
  TermInfo,
  TrustedRepo,
} from "../gen/bindings"
import { AgentsService } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

export type { AgentStartsView, AgentsSettings, RunningAgent, TrustedRepo }

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
    models: ["sonnet", "opus", "haiku"],
    efforts: ["low", "medium", "high", "xhigh", "max"],
    defaultModel: "sonnet",
    defaultEffort: "medium",
    idleMins: 30,
    idleChoices: [10, 30, 60, 120, 240],
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

// idleLabel names an idle limit in minutes, marking the default.
export function idleLabel(m: number, def = 30): string {
  const n =
    m % 60 === 0 ? `${m / 60} hour${m === 60 ? "" : "s"}` : `${m} minutes`
  return m === def ? `${n} (default)` : n
}

// "My agents may start agents on my cards" (card #575): the person's own
// limit, on all their devices. Only this pane changes it; no agent's tool can.
const sampleStarts: AgentStartsView = {
  on: previewFlag("agentstarts") !== "off",
  max: 100,
  used: previewFlag("agentstarts") === "full" ? 100 : 3,
  ceiling: 1000,
}

export function useAgentStarts(open: boolean) {
  const [s, setS] = useState<AgentStartsView | null>(
    inWails() ? null : sampleStarts
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
  const set = (on: boolean, max: number) => {
    if (!inWails()) {
      setS((p) => p && { ...p, on, max })
      return
    }
    AgentsService.SetStarts(on, max).then(
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
        idleMins: c.idleMins ?? null,
      }
      if (!inWails()) {
        const x = sampleFor(boardId)
        const next = {
          ...x,
          on: c.on ?? x.on,
          chosen: x.chosen || c.on != null,
          model: c.model ?? x.model,
          effort: c.effort ?? x.effort,
          idleMins: c.idleMins ?? x.idleMins,
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
  return { settings: s, error, set }
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
  const [error, setError] = useState("")
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    AgentsService.Trusted().then(
      (v) => live && (setRows(v), setError("")),
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
  return { rows, error, trust }
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
  const [error, setError] = useState("")
  const [tick, setTick] = useState(0)
  useEffect(() => {
    if (!open || !inWails()) return
    let live = true
    const load = () =>
      AgentsService.Running().then(
        (v) => live && (setRows(v), setError("")),
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
  return { rows, error, refresh }
}

// runningSince says how long ago an agent was asked for: "just now",
// "6 min", "2 h".
export function runningSince(at: number, now: number): string {
  const m = Math.floor((now - at) / 60_000)
  if (m < 1) return "just now"
  if (m < 60) return `${m} min`
  return `${Math.floor(m / 60)} h`
}
