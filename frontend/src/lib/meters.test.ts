import { describe, expect, test } from "bun:test"

import {
  diskName,
  fmtBits,
  fmtByteRate,
  fmtCpu,
  fmtGpu,
  fmtGpuMemory,
  fmtMemory,
  fmtNet,
  fmtPct,
  mergeHistory,
  pickDisk,
  pickGpu,
  pushRing,
  RING,
  sampleAt,
  type MetersGpu,
  type MetersSample,
} from "./meters"

const GiB = 1024 ** 3

describe("the ring (card #599)", () => {
  test("keeps the newest max, oldest first, in a new array", () => {
    const a = [1, 2, 3]
    const b = pushRing(a, 4, 3)
    expect(b).toEqual([2, 3, 4])
    expect(a).toEqual([1, 2, 3])
    expect(pushRing([], 1, 3)).toEqual([1])
  })
  test("a full ring stays at 60", () => {
    let r: number[] = []
    for (let i = 0; i < 100; i++) r = pushRing(r, i)
    expect(r.length).toBe(RING)
    expect(r[0]).toBe(40)
    expect(r[RING - 1]).toBe(99)
  })
  test("history goes under readings that came while it was read", () => {
    const at = (atMs: number) => sampleAt(atMs, false, () => 0.5)
    const merged = mergeHistory([at(1), at(2), at(3)], [at(3), at(4)])
    expect(merged.map((s: MetersSample) => s.atMs)).toEqual([1, 2, 3, 4])
  })
})

describe("Task Manager's numbers (card #599)", () => {
  test("percent", () => {
    expect(fmtPct(23.4)).toBe("23%")
    expect(fmtPct(null)).toBe("—")
    expect(fmtPct(104)).toBe("100%")
  })
  test("CPU with its speed", () => {
    expect(fmtCpu({ pct: 23.2, ghz: 5.33, cores: 16 })).toBe("23% 5.33 GHz")
    expect(fmtCpu({ pct: null, ghz: null, cores: 16 })).toBe("—")
  })
  test("memory used of total", () => {
    expect(fmtMemory({ usedBytes: 26.2 * GiB, totalBytes: 61.6 * GiB })).toBe("26.2/61.6 GB (43%)")
    expect(fmtMemory({ usedBytes: 0, totalBytes: 0 })).toBe("—")
  })
  test("network: sent and received in one unit, by the larger", () => {
    expect(fmtNet({ txBitsPerS: 280e3, rxBitsPerS: 72e3 })).toBe("S: 280 R: 72.0 Kbps")
    expect(fmtNet({ txBitsPerS: 0, rxBitsPerS: 0 })).toBe("S: 0 R: 0 Kbps")
    expect(fmtNet({ txBitsPerS: 400e3, rxBitsPerS: 12.5e6 })).toBe("S: 0.4 R: 12.5 Mbps")
    expect(fmtBits(1.2e9)).toBe("1.2 Gbps")
    expect(fmtBits(500)).toBe("0.5 Kbps")
    expect(fmtBits(72e3)).toBe("72.0 Kbps")
  })
  test("disk rates and names", () => {
    expect(fmtByteRate(0)).toBe("0 KB/s")
    expect(fmtByteRate(312 * 1024)).toBe("312 KB/s")
    expect(fmtByteRate(1.5 * 1024 * 1024)).toBe("1.5 MB/s")
    expect(diskName("0 C:")).toBe("Disk 0 (C:)")
    expect(diskName("disk0")).toBe("disk0")
  })
  test("GPU", () => {
    const gpu: MetersGpu = {
      name: "NVIDIA GeForce RTX 5080",
      integrated: false,
      utilPct: 21,
      tempC: 30.4,
      memUsedBytes: 2.4 * GiB,
      memTotalBytes: 16 * GiB,
    }
    expect(fmtGpu(gpu)).toBe("21% (30 °C)")
    expect(fmtGpuMemory(gpu)).toBe("2.4/16.0 GB")
    expect(fmtGpuMemory({ ...gpu, memUsedBytes: null, memTotalBytes: null })).toBeNull()
    expect(fmtGpu({ ...gpu, utilPct: null, tempC: null })).toBe("—")
  })
})

describe("what the strip picks (card #599)", () => {
  const g = (integrated: boolean, utilPct: number | null): MetersGpu => ({
    name: "",
    integrated,
    utilPct,
    tempC: null,
    memUsedBytes: null,
    memTotalBytes: null,
  })
  test("the busiest discrete GPU, else the first", () => {
    expect(pickGpu([g(true, 90), g(false, 10), g(false, 30)])).toBe(2)
    expect(pickGpu([g(true, 90), g(true, 10)])).toBe(0)
    expect(pickGpu([])).toBe(-1)
  })
  test("the busiest disk", () => {
    const d = (activePct: number | null) => ({ name: "", activePct, readBytesPerS: 0, writeBytesPerS: 0 })
    expect(pickDisk([d(2), d(40), d(null)])).toBe(1)
    expect(pickDisk([])).toBe(-1)
  })
  test("the made-up machine has two GPUs, or none with ?gpu=none", () => {
    const s = sampleAt(1_000_000)
    expect(s.gpus.length).toBe(2)
    expect(s.gpus[1].integrated).toBe(true)
    expect(sampleAt(1_000_000, true).gpus).toEqual([])
    expect(s.cpu.pct).toBeGreaterThanOrEqual(0)
    expect(s.cpu.pct).toBeLessThanOrEqual(100)
  })
})
