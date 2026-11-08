import { afterEach, beforeEach, describe, expect, test } from "bun:test"

import {
  clampHeight,
  defaultHeight,
  heightKey,
  maxHeight,
  minHeight,
  pageRoom,
  readHeight,
  saveHeight,
} from "./term-height"

describe("maxHeight", () => {
  test("the window less the page's room", () => {
    expect(maxHeight(900)).toBe(900 - pageRoom)
  })
  test("never under the minimum, even in a short window", () => {
    expect(maxHeight(150)).toBe(minHeight)
  })
})

describe("clampHeight", () => {
  test("keeps it between the ends", () => {
    expect(clampHeight(50, 900)).toBe(minHeight)
    expect(clampHeight(5000, 900)).toBe(900 - pageRoom)
    expect(clampHeight(412.6, 900)).toBe(413)
  })
  test("a bad value is the default", () => {
    expect(clampHeight(Number("abc"), 900)).toBe(defaultHeight)
  })
})

describe("readHeight and saveHeight", () => {
  const store = new Map<string, string>()
  const real = (globalThis as { localStorage?: Storage }).localStorage
  beforeEach(() => {
    store.clear()
    ;(globalThis as { localStorage?: unknown }).localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    }
  })
  afterEach(() => {
    ;(globalThis as { localStorage?: unknown }).localStorage = real
  })

  test("nothing saved is the default", () => {
    expect(readHeight()).toBe(defaultHeight)
  })
  test("a saved height comes back", () => {
    saveHeight(520.4)
    expect(store.get(heightKey)).toBe("520")
    expect(readHeight()).toBe(520)
  })
  test("null forgets it", () => {
    saveHeight(520)
    saveHeight(null)
    expect(readHeight()).toBe(defaultHeight)
  })
  test("junk or a height under the minimum is the default", () => {
    store.set(heightKey, "tall")
    expect(readHeight()).toBe(defaultHeight)
    store.set(heightKey, "40")
    expect(readHeight()).toBe(defaultHeight)
  })
  test("shut storage is the default and saving doesn't throw", () => {
    ;(globalThis as { localStorage?: unknown }).localStorage = {
      getItem: () => {
        throw new Error("denied")
      },
      setItem: () => {
        throw new Error("denied")
      },
      removeItem: () => {
        throw new Error("denied")
      },
    }
    expect(readHeight()).toBe(defaultHeight)
    expect(() => saveHeight(400)).not.toThrow()
  })
})
