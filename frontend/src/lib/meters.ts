// Hardware meters gateway (card #599): this computer's CPU, memory, disks,
// network and GPUs, read by the host every two seconds (crates/window
// meters.rs). The window keeps the last 60 readings, two minutes, for the
// top bar's strip and its panel. Components import from here, never from
// src/gen or host.ts. In a plain browser a made-up machine moves instead;
// on the phone there is nothing (the strip is desktop only).

import { useSyncExternalStore } from "react"

import type * as B from "../gen/bindings"
import { isAndroid } from "../updater/android"
import { Events, MetersService } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

export type MetersSample = B.MetersSample
export type MetersGpu = B.MetersGpu
export type MetersDisk = B.MetersDisk

export const EventMetersChanged = "meters:changed"

// RING is how many readings the window keeps: two minutes at one per 2 s.
export const RING = 60
const TICK_MS = 2000

// metersHere: this window shows meters; the phone (and ?android) doesn't.
export function metersHere(): boolean {
  return !isAndroid()
}

// metersOpenAtStart: the preview's ?meters=open opens the panel.
export function metersOpenAtStart(): boolean {
  return previewFlag("meters") === "open"
}

// ---- pure helpers -------------------------------------------------------------

// pushRing adds one reading to the end, dropping the oldest past max. It
// makes a new array, so a reader of the old one sees no change.
export function pushRing<T>(ring: readonly T[], item: T, max = RING): T[] {
  const next =
    ring.length >= max ? ring.slice(ring.length - max + 1) : ring.slice()
  next.push(item)
  return next
}

// mergeHistory lays the host's history under readings that arrived while it
// was being read: those newer than its last stay, the rest it replaces.
export function mergeHistory(
  history: readonly MetersSample[],
  live: readonly MetersSample[],
  max = RING
): MetersSample[] {
  const last = history.length ? history[history.length - 1].atMs : -Infinity
  return [...history, ...live.filter((s) => s.atMs > last)].slice(-max)
}

const GiB = 1024 ** 3

// fmtPct is a whole percent ("23%"), or "—" with no reading.
export function fmtPct(pct: number | null | undefined): string {
  if (pct == null || !Number.isFinite(pct)) return "—"
  return `${Math.round(Math.min(Math.max(pct, 0), 100))}%`
}

// memPct is how full memory is, 0 to 100.
export function memPct(mem: B.MetersMem): number | null {
  return mem.totalBytes > 0 ? (mem.usedBytes / mem.totalBytes) * 100 : null
}

// fmtGB is bytes as gigabytes with one decimal, Task Manager's way (1024-based).
export function fmtGB(bytes: number): string {
  return (bytes / GiB).toFixed(1)
}

// fmtMemory is "26.2/61.6 GB (43%)".
export function fmtMemory(mem: B.MetersMem): string {
  if (mem.totalBytes <= 0) return "—"
  return `${fmtGB(mem.usedBytes)}/${fmtGB(mem.totalBytes)} GB (${fmtPct(memPct(mem))})`
}

// fmtGpuMemory is a GPU's dedicated memory, "2.4/16.0 GB", or null when the
// GPU doesn't say (an integrated one shares the system's).
export function fmtGpuMemory(gpu: B.MetersGpu): string | null {
  if (gpu.memTotalBytes == null || gpu.memTotalBytes <= 0) return null
  return `${fmtGB(gpu.memUsedBytes ?? 0)}/${fmtGB(gpu.memTotalBytes)} GB`
}

// fmtCpu is "23% 5.33 GHz" (no speed when it isn't read).
export function fmtCpu(cpu: B.MetersCpu): string {
  const pct = fmtPct(cpu.pct)
  return cpu.ghz != null && cpu.ghz > 0
    ? `${pct} ${cpu.ghz.toFixed(2)} GHz`
    : pct
}

// fmtGpu is "21% (30 °C)": its use, and its temperature when read.
export function fmtGpu(gpu: B.MetersGpu): string {
  const pct = fmtPct(gpu.utilPct)
  return gpu.tempC != null ? `${pct} (${Math.round(gpu.tempC)} °C)` : pct
}

// num3 is a number with about three digits: "280", "72.0", "8.0", "0".
function num3(n: number): string {
  if (n === 0) return "0"
  return n >= 100 ? n.toFixed(0) : n.toFixed(1)
}

const BIT_UNITS = ["Kbps", "Mbps", "Gbps"] as const

// bitUnit picks one unit for a set of bit rates, by the largest: Kbps at the
// least, as Task Manager shows an idle link.
function bitUnit(max: number): { div: number; unit: string } {
  let div = 1000
  let i = 0
  while (i < BIT_UNITS.length - 1 && max >= div * 1000) {
    div *= 1000
    i++
  }
  return { div, unit: BIT_UNITS[i] }
}

// fmtBits is one bit rate: "72.0 Kbps", "1.2 Mbps".
export function fmtBits(bitsPerS: number): string {
  const { div, unit } = bitUnit(bitsPerS)
  return `${num3(bitsPerS / div)} ${unit}`
}

// fmtNet is sent and received in one unit: "S: 280 R: 72.0 Kbps".
export function fmtNet(net: B.MetersNet): string {
  const { div, unit } = bitUnit(Math.max(net.txBitsPerS, net.rxBitsPerS))
  return `S: ${num3(net.txBitsPerS / div)} R: ${num3(net.rxBitsPerS / div)} ${unit}`
}

const BYTE_UNITS = ["B/s", "KB/s", "MB/s", "GB/s"] as const

// fmtByteRate is a disk's rate: "0 KB/s", "312 KB/s", "1.2 MB/s" (1024-based).
export function fmtByteRate(bytesPerS: number): string {
  let n = bytesPerS / 1024
  let i = 1
  while (i < BYTE_UNITS.length - 1 && n >= 1024) {
    n /= 1024
    i++
  }
  return `${num3(n)} ${BYTE_UNITS[i]}`
}

// diskName is "Disk 0 (C:)" for the host's "0 C:"; another name stays.
export function diskName(name: string): string {
  const m = /^(\d+)\s+(.+)$/.exec(name.trim())
  return m ? `Disk ${m[1]} (${m[2]})` : name
}

// pickGpu is the GPU the strip shows, by name: the discrete one busiest
// on average over the window (so the pick holds still), else the first.
// null with no GPU.
export function pickGpu(samples: readonly MetersSample[]): string | null {
  const last = samples.at(-1)
  if (!last?.gpus.length) return null
  let best: string | null = null
  let bestAvg = -1
  for (const g of last.gpus) {
    if (g.integrated) continue
    const avg = average(
      samples.map((s) => gpuByName(s, g.name)?.utilPct ?? null)
    )
    if (best == null || avg > bestAvg) {
      best = g.name
      bestAvg = avg
    }
  }
  return best ?? last.gpus[0].name
}

// pickDisk is the disk the strip shows, by name: the system disk (C:), else
// the first. null with no disk.
export function pickDisk(samples: readonly MetersSample[]): string | null {
  const disks = samples.at(-1)?.disks ?? []
  if (!disks.length) return null
  return (disks.find((d) => /(^|\s)C:/i.test(d.name)) ?? disks[0]).name
}

// gpuByName and diskByName find a device in one reading, so a graph follows
// the device even when the list changes order.
export function gpuByName(
  s: MetersSample,
  name: string
): B.MetersGpu | undefined {
  return s.gpus.find((g) => g.name === name)
}
export function diskByName(
  s: MetersSample,
  name: string
): B.MetersDisk | undefined {
  return s.disks.find((d) => d.name === name)
}

// average of the readings there are; -1 with none.
function average(values: readonly (number | null)[]): number {
  let sum = 0
  let n = 0
  for (const v of values) {
    if (v == null || Number.isNaN(v)) continue
    sum += v
    n++
  }
  return n ? sum / n : -1
}

// ---- the made-up machine (a plain browser) ---------------------------------------

// sampleAt is the preview's reading at time t: moving waves near Stephen's
// PC (16 cores, 61.6 GB, an RTX 5080 and the CPU's Radeon), with no GPUs
// for ?gpu=none.
export function sampleAt(
  t: number,
  noGpu = false,
  jitter: () => number = Math.random
): MetersSample {
  const s = t / 1000
  const wave = (period: number, phase = 0) =>
    (Math.sin((s / period) * 2 * Math.PI + phase) + 1) / 2
  const clamp = (n: number) => Math.min(Math.max(n, 0), 100)
  const total = 61.6 * GiB
  const burst = wave(23, 1) > 0.85 ? 1 : 0
  return {
    atMs: t,
    cpu: {
      pct: clamp(14 + 18 * wave(17) + 30 * burst + 8 * jitter()),
      ghz: 4.6 + 0.8 * wave(11) + 0.1 * jitter(),
      cores: 16,
    },
    mem: { usedBytes: total * (0.41 + 0.05 * wave(60)), totalBytes: total },
    disks: [
      {
        name: "0 C:",
        activePct: clamp(2 + 40 * burst * jitter() + 6 * wave(9) * jitter()),
        readBytesPerS: (burst ? 18e6 : 0.3e6) * jitter(),
        writeBytesPerS: (0.2e6 + 2e6 * wave(13)) * jitter(),
      },
      {
        name: "1 D:",
        activePct: clamp(3 * jitter()),
        readBytesPerS: 0,
        writeBytesPerS: 40e3 * jitter(),
      },
    ],
    net: {
      txBitsPerS: 60e3 + 400e3 * wave(7, 2) * jitter(),
      rxBitsPerS: 120e3 + 2.4e6 * wave(19) * jitter() + 30e6 * burst * jitter(),
    },
    gpus: noGpu
      ? []
      : [
          {
            name: "NVIDIA GeForce RTX 5080",
            integrated: false,
            utilPct: clamp(6 + 30 * wave(29, 3) + 10 * jitter()),
            tempC: 30 + 12 * wave(29, 3),
            memUsedBytes: (2.2 + 0.6 * wave(40)) * GiB,
            memTotalBytes: 16 * GiB,
          },
          {
            name: "AMD Radeon(TM) Graphics",
            integrated: true,
            utilPct: clamp(1 + 4 * jitter()),
            tempC: 47 + 2 * wave(31),
            memUsedBytes: null,
            memTotalBytes: null,
          },
        ],
  }
}

// startSample runs the made-up machine: two minutes of history at once,
// then a reading every 2 s.
function startSample(
  push: (s: MetersSample) => void,
  seed: (h: MetersSample[]) => void
): () => void {
  const noGpu = previewFlag("gpu") === "none"
  const now = Date.now()
  const history: MetersSample[] = []
  for (let i = RING - 1; i >= 0; i--)
    history.push(sampleAt(now - i * TICK_MS, noGpu))
  seed(history)
  const timer = setInterval(() => push(sampleAt(Date.now(), noGpu)), TICK_MS)
  return () => clearInterval(timer)
}

// ---- the store -------------------------------------------------------------------

const state = {
  value: [] as MetersSample[],
  readers: new Set<() => void>(),
  off: null as (() => void) | null,
}

function set(next: MetersSample[]) {
  state.value = next
  for (const fn of state.readers) fn()
}

// follow starts reading: the host's history, then each meters:changed.
function follow(): () => void {
  const push = (s: MetersSample) => set(pushRing(state.value, s))
  if (isAndroid()) return () => {}
  if (!inWails()) return startSample(push, set)
  let live = true
  // The history is asked for once the listener is live, so no reading
  // falls between the two.
  const off = Events.On<MetersSample>(
    EventMetersChanged,
    (ev) => {
      if (ev.data) push(ev.data)
    },
    () => {
      MetersService.History()
        .then((history) => {
          if (live) set(mergeHistory(history, state.value))
        })
        .catch(() => {})
    }
  )
  return () => {
    live = false
    off()
  }
}

// subscribeMeters adds a reader; the first starts following the host and
// the last stops it. The readings so far stay for the next reader.
export function subscribeMeters(reader: () => void): () => void {
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

export function metersNow(): MetersSample[] {
  return state.value
}

// useMeters is the last two minutes of readings, oldest first ([] until
// the first; always [] on the phone).
export function useMeters(): MetersSample[] {
  return useSyncExternalStore(subscribeMeters, metersNow, metersNow)
}
