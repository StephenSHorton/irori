// Live tasks on cards (card #1249, docs/design/live-tasks.md §6): an
// agent's background runs (a shell, an agent or a workflow) and its
// foreground shell calls, as the card shows them. The run facts ride the
// card (CardSummary.runs: heavy and running; CardDetail.runs: all); the
// live CPU and memory come every 2 s on their own runs:changed event, so
// a reading never rebuilds the card lists (the meters.ts pattern).
//
// The types are crates/window's (gen/bindings); runsOf and withRuns read
// and set `c.runs`. In a plain browser the sample's runs move instead (?runs= in
// PREVIEW.md).

import { useSyncExternalStore } from "react"

import type * as B from "../gen/bindings"
import { runningSince } from "./agents"
import { Events, RunsService } from "./host"
import { inWails, plainLine } from "./irori"
import { fmtGB } from "./meters"
import { previewFlag } from "./preview"

// ---- types (crates/window CardRun and RunLoad, crates/term TailInfo) --------

// CardRun is one run as a card shows it. Times are ms since the epoch.
// exit is the exit code when the run said one; a run killed or gone with
// no code has ended set and exit null. stopped: the person pressed Stop,
// and whatever end came after reads "Stopped".
export type CardRun = B.CardRun
export type RunKind = B.RunKind
// TailInfo is on a terminal that follows a run's output (§5.1): read only,
// with the run's end once it has one. exit: absent while it runs, null for
// an end with no code.
export type TailInfo = B.TailInfo
export type TermWithTail = B.TermInfo
// RunLoad is one live reading (runs:changed): the full set of this
// device's live runs each time.
export type RunLoad = B.RunLoad

export const EventRunsChanged = "runs:changed"

// ---- the seam with the generated card types ---------------------------------

type HasRuns = { runs?: CardRun[] | null }

// runsOf is a card's runs ([] with none).
export function runsOf(c: object): CardRun[] {
  return (c as HasRuns).runs ?? []
}

// withRuns puts runs on a card, leaving it out when there are none (the
// Rust side's skip_serializing_if).
export function withRuns<T extends object>(c: T, runs: CardRun[]): T {
  if (!runs.length) return c
  return Object.assign(c, { runs })
}

// ---- pure helpers --------------------------------------------------------------

export function runId(r: { who?: B.AgentRef; key?: string; run: string }) {
  return `${r.key ?? r.who?.key ?? ""}\n${r.run}`
}

// running: no end yet.
export function running(r: CardRun): boolean {
  return r.ended == null && r.exit == null && !r.stopped
}

// remote: the run is on another device; nothing about it works here.
export function remote(r: CardRun): boolean {
  return r.who.here === false
}

// withLoad is the run with its latest live reading, when there is one.
export function withLoad(
  r: CardRun,
  loads: ReadonlyMap<string, RunLoad>
): CardRun {
  const l = loads.get(runId(r))
  return l ? { ...r, cpuCores: l.cores, memBytes: l.memBytes } : r
}

// heavyRuns is what a chip shows: background runs still going, the
// heaviest first. The Core already sends only the heavy ones on a summary.
export function heavyRuns(runs: readonly CardRun[]): CardRun[] {
  return runs
    .filter((r) => r.kind !== "fg" && running(r))
    .sort((a, b) => weight(b) - weight(a) || a.started - b.started)
}

// weight orders runs by load: a core counts as much as a gigabyte.
function weight(r: CardRun): number {
  return (r.cpuCores ?? 0) + (r.memBytes ?? 0) / GiB
}

const GiB = 1024 ** 3

// fmtCores is "12 cores", "1 core", "0.4 cores".
export function fmtCores(n: number): string {
  if (n >= 9.5) return `${Math.round(n)} cores`
  const t = n.toFixed(1).replace(/\.0$/, "")
  return `${t} ${t === "1" ? "core" : "cores"}`
}

// loadText is "12 cores · 5.8 GB", or what of it is known.
export function loadText(
  cores: number | null | undefined,
  mem?: number | null
) {
  const parts: string[] = []
  if (cores != null) parts.push(fmtCores(cores))
  if (mem) parts.push(`${fmtGB(mem)} GB`)
  return parts.join(" · ")
}

// endText is how a run ended: "Stopped", "Exited with code 1", "Ended".
export function endText(r: { exit?: number | null; stopped: boolean }) {
  if (r.stopped) return "Stopped"
  if (r.exit != null) return `Exited with code ${r.exit}`
  return "Ended"
}

// runChipText is the tile's line for one run: "cargo test · 12 cores ·
// 5.8 GB · 4 min", short "12 cores · 5.8 GB" (Home's rows). A run on
// another device says where instead of its load, and once over its exit
// and peaks.
export function runChipText(r: CardRun, now: number, short = false): string {
  const cmd = plainLine(r.cmd) || kindName(r.kind)
  if (remote(r)) {
    const where = `on ${plainLine(r.device) || plainLine(r.who.where)}`
    if (running(r))
      return [cmd, runningSince(r.started, now), where].join(" · ")
    const peaks = loadText(r.peakCores, r.peakMemBytes)
    return [cmd, endText(r), peaks && `peak ${peaks}`, where]
      .filter(Boolean)
      .join(" · ")
  }
  const load = loadText(r.cpuCores, r.memBytes)
  if (short) return load || cmd
  return [cmd, load, runningSince(r.started, now)].filter(Boolean).join(" · ")
}

export function kindName(k: RunKind): string {
  return k === "agent" ? "an agent" : k === "workflow" ? "a workflow" : "a run"
}

// CardLoad is one card's live load, for Busy now.
export type CardLoad = {
  id: string
  short: string
  tag: string
  cmd: string
  cores: number
  memBytes: number
}

// cardLoad sums a card's heavy runs on this device; null with none (a
// run on another device has no live reading here).
export function cardLoad(
  c: { id: string; short: string; tag?: string },
  runs: readonly CardRun[]
): CardLoad | null {
  const mine = heavyRuns(runs).filter((r) => !remote(r))
  if (!mine.length) return null
  return {
    id: c.id,
    short: c.short,
    tag: c.tag ?? "",
    cmd: plainLine(mine[0].cmd) || kindName(mine[0].kind),
    cores: mine.reduce((n, r) => n + (r.cpuCores ?? 0), 0),
    memBytes: mine.reduce((n, r) => n + (r.memBytes ?? 0), 0),
  }
}

// busyList is Busy now's cards, the busiest first.
export function busyList(
  cards: readonly { id: string; short: string; tag?: string }[],
  loads: ReadonlyMap<string, RunLoad> = new Map()
): CardLoad[] {
  return cards
    .flatMap(
      (c) =>
        cardLoad(
          c,
          runsOf(c).map((r) => withLoad(r, loads))
        ) ?? []
    )
    .sort(
      (a, b) =>
        b.cores + b.memBytes / GiB - (a.cores + a.memBytes / GiB) ||
        a.short.localeCompare(b.short, undefined, { numeric: true })
    )
}

// busyLine is Busy now as plain text ("" when nothing is heavy).
export function busyLine(list: readonly CardLoad[]): string {
  if (!list.length) return ""
  return `Busy now: ${list
    .map((c) => `#${c.short} ${c.cmd} ${loadText(c.cores, c.memBytes)}`)
    .join(", ")}`
}

// fgText is a foreground call's row: "cargo check · 40s · 2.1 cores".
export function fgText(r: CardRun, now: number): string {
  const end = r.ended ?? now
  const s = Math.max(0, Math.round((end - r.started) / 1000))
  const took = s < 60 ? `${s}s` : runningSince(r.started, end)
  return [plainLine(r.cmd) || "a command", took, loadText(r.cpuCores)]
    .filter(Boolean)
    .join(" · ")
}

// ---- Stop ------------------------------------------------------------------------

// stopRun ends a run's process tree (§5.3): the person's button only.
export function stopRun(r: CardRun): Promise<void> {
  if (!inWails()) {
    sampleStop(r)
    return Promise.resolve()
  }
  return RunsService.Stop(r.who.key, r.run)
}

// ---- the live readings -----------------------------------------------------------

const state = {
  value: new Map<string, RunLoad>() as ReadonlyMap<string, RunLoad>,
  readers: new Set<() => void>(),
  off: null as (() => void) | null,
}

function set(list: readonly RunLoad[]) {
  state.value = new Map(list.map((l) => [runId(l), l]))
  for (const fn of state.readers) fn()
}

function follow(): () => void {
  if (!inWails()) return startSample(set)
  return Events.On<RunLoad[]>(EventRunsChanged, (ev) => {
    if (Array.isArray(ev.data)) set(ev.data)
  })
}

// subscribeRuns adds a reader; the first starts following, the last stops.
export function subscribeRuns(reader: () => void): () => void {
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

export function runLoadsNow(): ReadonlyMap<string, RunLoad> {
  return state.value
}

// useRunLoads is the latest reading of every live run on this device.
export function useRunLoads(): ReadonlyMap<string, RunLoad> {
  return useSyncExternalStore(subscribeRuns, runLoadsNow, runLoadsNow)
}

// useCardRuns is a card's runs with their latest readings.
export function useCardRuns(c: object): CardRun[] {
  const loads = useRunLoads()
  return runsOf(c).map((r) => withLoad(r, loads))
}

// ---- the sample (a plain browser) -----------------------------------------------

// The preview's ?runs=: none (no runs), exited or stopped (sample-card-job's
// run has ended that way); anything else, or no flag, runs them.
export function sampleRunsMode(): "on" | "none" | "exited" | "stopped" {
  const f = previewFlag("runs")
  return f === "none" || f === "exited" || f === "stopped" ? f : "on"
}

// The runs the sample cards carry (sample-cards.ts makes them), kept so
// Stop and the moving readings can reach them.
const sampleRuns: CardRun[] = []
// sampleTails maps a sample run to its stand-in terminal (lib/term.ts).
const sampleTails = new Map<string, TailInfo>()
const sampleExit = new Set<(run: string) => void>()

const sampleByCard = new Map<string, CardRun[]>()

// sampleRun registers one of the sample's runs on its card. A run made
// again (the sample is rebuilt) is the one already there, so a Stop holds.
export function sampleRun(card: string, r: CardRun): CardRun {
  const had = sampleRuns.find((x) => x.run === r.run)
  if (had) return had
  sampleRuns.push(r)
  sampleByCard.set(card, [...(sampleByCard.get(card) ?? []), r])
  return r
}

// sampleRunsOn is the runs registered on a sample card.
export function sampleRunsOn(card: string): CardRun[] {
  return sampleByCard.get(card) ?? []
}

export function sampleTail(r: CardRun): TailInfo {
  const t: TailInfo = { label: r.cmd, run: r.run, stopped: r.stopped }
  if (!running(r)) t.exit = r.exit
  sampleTails.set(r.run, t)
  return t
}

// onSampleExit hears a sample run end (the stand-in terminal's exit).
export function onSampleExit(fn: (run: string) => void): () => void {
  sampleExit.add(fn)
  return () => sampleExit.delete(fn)
}

function sampleStop(r: CardRun) {
  for (const held of new Set([
    r,
    ...sampleRuns.filter((x) => x.run === r.run),
  ])) {
    held.stopped = true
    held.ended = Date.now()
  }
  const t = sampleTails.get(r.run)
  if (t) {
    t.stopped = true
    t.exit = null
  }
  for (const fn of sampleExit) fn(r.run)
  set([...state.value.values()].filter((l) => l.run !== r.run))
}

// sampleSummaryRuns is what the Core would put on the sample's summary:
// running background runs over the threshold (1 core or 1 GB), and those
// on another device.
export function sampleSummaryRuns(runs: readonly CardRun[]): CardRun[] {
  return heavyRuns(runs).filter(
    (r) => remote(r) || (r.cpuCores ?? 0) >= 1 || (r.memBytes ?? 0) >= GiB
  )
}

// sampleLoadAt is a sample run's reading at time t: its own load, moving.
export function sampleLoadAt(r: CardRun, t: number): RunLoad {
  const s = t / 1000
  const wave = (period: number) =>
    (Math.sin((s / period) * 2 * Math.PI) + 1) / 2
  return {
    key: r.who.key,
    run: r.run,
    // No reading stays none (an agent run is claude's own process).
    cores: r.cpuCores == null ? null : r.cpuCores * (0.85 + 0.25 * wave(13)),
    memBytes: (r.memBytes ?? 0) * (0.95 + 0.1 * wave(31)),
    at: t,
    device: r.device,
  }
}

function startSample(push: (l: RunLoad[]) => void): () => void {
  const tick = () =>
    push(
      sampleRuns
        .filter((r) => running(r) && !remote(r) && r.kind !== "fg")
        .map((r) => sampleLoadAt(r, Date.now()))
    )
  tick()
  const timer = setInterval(tick, 2000)
  return () => clearInterval(timer)
}
