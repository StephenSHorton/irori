import { beforeEach, describe, expect, test } from "bun:test"

import {
  bytesText,
  cleanableBytes,
  infoOf,
  kindInfo,
  measuredFrom,
  resetSample,
  storage,
  total,
  type StorageItem,
} from "./storage"

// The sample runs outside the host; bun has no window.
const g = globalThis as unknown as { window?: unknown }
g.window ??= { location: { search: "" } }

beforeEach(() => resetSample())

describe("Storage sizes (card #728)", () => {
  test("sizes read in plain units", () => {
    expect(bytesText(0)).toBe("0 B")
    expect(bytesText(812 * 1024)).toBe("812 KB")
    expect(bytesText(1.5 * (1 << 30))).toBe("1.5 GB")
    expect(bytesText(101 * (1 << 30))).toBe("101 GB")
    expect(bytesText(2 * 1024 ** 4)).toBe("2.0 TB")
  })
  test("the total leaves out what isn't measured yet", () => {
    expect(
      total([
        { id: "log", bytes: 10, items: 0, measured: true, path: "" },
        { id: "builds", bytes: 99, items: 0, measured: false, path: "" },
      ])
    ).toBe(10)
  })
  test("a kind is measured from its items", () => {
    const items = [
      { bytes: 3, cleanable: true },
      { bytes: 4, cleanable: false },
    ] as StorageItem[]
    expect(
      measuredFrom(
        { id: "builds", bytes: 0, items: 0, measured: false, path: "p" },
        items
      )
    ).toEqual({ id: "builds", bytes: 7, items: 2, measured: true, path: "p" })
    expect(cleanableBytes(items)).toBe(3)
  })
})

describe("Storage says what each kind is and what deleting does", () => {
  test("every kind has words, and the ones nothing deletes say so", () => {
    for (const [id, k] of Object.entries(kindInfo)) {
      expect(k.label.length).toBeGreaterThan(0)
      expect(k.what.length).toBeGreaterThan(0)
      expect(k.clean.length).toBeGreaterThan(0)
      if (["log", "spaces", "capture", "claude"].includes(id))
        expect(k.cleanable).toBe(false)
    }
    expect(infoOf("files").clean).toContain("can't be undone")
    expect(infoOf("nothing").label).toBe("nothing")
  })
})

describe("Storage's sample changes as the person cleans", () => {
  test("a clean by id takes those; one with none, every one that can go", async () => {
    const before = await storage.items("updates")
    expect(before.length).toBe(2)
    const one = await storage.clean("updates", ["Irori_0.12.6_x64-setup.exe"])
    expect(one.removed).toBe(0)
    expect(one.kept[0].why).toContain("waiting to install")
    const all = await storage.clean("updates")
    expect(all.removed).toBe(1)
    expect((await storage.items("updates")).map((i) => i.id)).toEqual([
      "Irori_0.12.6_x64-setup.exe",
    ])
  })
  test("clearing threads lists them as cleared, and Put back takes them off", async () => {
    const n = await storage.clearThreads(0)
    expect(n).toBeGreaterThan(0)
    const cleared = await storage.cleared()
    expect(cleared.filter((c) => c.thread).length).toBe(n)
    expect(await storage.clearThreadsCount(0)).toBe(0)
    await storage.unclear(cleared.map((c) => c.id))
    expect(await storage.cleared()).toEqual([])
  })
  test("the report lists build output unmeasured", async () => {
    const r = await storage.report()
    const builds = r.kinds.find((k) => k.id === "builds")
    expect(builds?.measured).toBe(false)
    expect(r.kinds.find((k) => k.id === "files")?.measured).toBe(true)
  })
})
