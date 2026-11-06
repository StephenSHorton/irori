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
  diskByName,
  gpuByName,
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
    expect(fmtMemory({ usedBytes: 26.2 * GiB, totalBytes: 61.6 * GiB })).toBe(
      "26.2/61.6 GB (43%)"
    )
    expect(fmtMemory({ usedBytes: 0, totalBytes: 0 })).toBe("—")
  })
  test("network: sent and received in one unit, by the larger", () => {
    expect(fmtNet({ txBitsPerS: 280e3, rxBitsPerS: 72e3 })).toBe(
      "S: 280 R: 72.0 Kbps"
    )
    expect(fmtNet({ txBitsPerS: 0, rxBitsPerS: 0 })).toBe("S: 0 R: 0 Kbps")
    expect(fmtNet({ txBitsPerS: 400e3, rxBitsPerS: 12.5e6 })).toBe(
      "S: 0.4 R: 12.5 Mbps"
    )
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
    expect(
      fmtGpuMemory({ ...gpu, memUsedBytes: null, memTotalBytes: null })
    ).toBeNull()
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
  // at is a reading with just these GPUs and disks.
  const at = (
    gpus: MetersGpu[],
    disks: { name: string; activePct: number | null }[] = []
  ) => ({
    ...sampleAt(0),
    gpus,
    disks: disks.map((d) => ({ ...d, readBytesPerS: 0, writeBytesPerS: 0 })),
  })
  test("the discrete GPU busiest over the window, else the first", () => {
    const a = (u: number) => ({ ...g(false, u), name: "A" })
    const b = (u: number) => ({ ...g(false, u), name: "B" })
    const igpu = { ...g(true, 90), name: "iGPU" }
    // B is busier on average though A is busier now.
    const window = [
      at([igpu, a(5), b(60)]),
      at([igpu, a(5), b(60)]),
      at([igpu, a(40), b(30)]),
    ]
    expect(pickGpu(window)).toBe("B")
    // The list changing order doesn't change the pick.
    expect(pickGpu([...window, at([b(50), igpu, a(10)])])).toBe("B")
    expect(pickGpu([at([igpu, { ...g(true, 10), name: "i2" }])])).toBe("iGPU")
    expect(pickGpu([at([])])).toBeNull()
    expect(pickGpu([])).toBeNull()
  })
  test("the system disk, else the first", () => {
    const disks = [
      { name: "1 D:", activePct: 90 },
      { name: "0 C:", activePct: 2 },
    ]
    expect(pickDisk([at([], disks)])).toBe("0 C:")
    const noC = [
      { name: "1 D:", activePct: 9 },
      { name: "2 E:", activePct: 50 },
    ]
    expect(pickDisk([at([], noC)])).toBe("1 D:")
    expect(pickDisk([at([], [])])).toBeNull()
    expect(pickDisk([])).toBeNull()
  })
  test("a device is found by name", () => {
    const s = at(
      [{ ...g(false, 7), name: "X" }],
      [{ name: "0 C:", activePct: 3 }]
    )
    expect(gpuByName(s, "X")?.utilPct).toBe(7)
    expect(gpuByName(s, "Y")).toBeUndefined()
    expect(diskByName(s, "0 C:")?.activePct).toBe(3)
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
