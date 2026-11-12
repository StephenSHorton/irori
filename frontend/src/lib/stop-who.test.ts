import { afterEach, describe, expect, test } from "bun:test"

import { stopLabels } from "./stop-labels"
import {
  BOTH,
  parseStopWho,
  readStopWho,
  stopButton,
  stopCounts,
  stopHereTitle,
  stopWhoKey,
  writeStopWho,
} from "./stop-who"

const cards = { cards: true, yours: false }
const yours = { cards: false, yours: true }

describe("Stop's who choice (decision 71)", () => {
  test("nothing kept, unreadable or nothing ticked reads as both", () => {
    expect(parseStopWho(null)).toEqual(BOTH)
    expect(parseStopWho("")).toEqual(BOTH)
    expect(parseStopWho("{oops")).toEqual(BOTH)
    expect(parseStopWho("null")).toEqual(BOTH)
    expect(parseStopWho('{"cards":false,"yours":false}')).toEqual(BOTH)
    expect(parseStopWho(JSON.stringify(yours))).toEqual(yours)
    expect(parseStopWho(JSON.stringify(cards))).toEqual(cards)
  })

  describe("kept per computer", () => {
    const g = globalThis as { localStorage?: unknown }
    const had = g.localStorage
    afterEach(() => {
      g.localStorage = had
    })

    test("no storage at all (it throws): both, and writing doesn't throw", () => {
      g.localStorage = {
        getItem: () => {
          throw new Error("blocked")
        },
        setItem: () => {
          throw new Error("blocked")
        },
      }
      expect(readStopWho()).toEqual(BOTH)
      expect(() => writeStopWho(yours)).not.toThrow()
    })

    test("a kept choice comes back", () => {
      const m = new Map<string, string>()
      g.localStorage = {
        getItem: (k: string) => m.get(k) ?? null,
        setItem: (k: string, v: string) => void m.set(k, v),
      }
      expect(readStopWho()).toEqual(BOTH)
      writeStopWho(cards)
      expect(m.has(stopWhoKey)).toBe(true)
      expect(readStopWho()).toEqual(cards)
    })
  })

  test("counts come from the roster's spawned mark; Stop here counts this machine", () => {
    const agents = [
      { spawned: true, here: true },
      { spawned: true, here: false },
      { spawned: false, here: true },
      { spawned: false, here: false },
      { spawned: false, here: false },
    ]
    expect(stopCounts(agents)).toEqual({ cards: 2, yours: 3 })
    expect(stopCounts(agents, true)).toEqual({ cards: 1, yours: 1 })
  })

  test("the dialog's button says what it does, and nothing ticked has none", () => {
    const n = { cards: 3, yours: 2 }
    expect(stopButton(BOTH, n)).toBe("Stop 5 agents")
    expect(stopButton(cards, n)).toBe("Stop 3 agents")
    expect(stopButton(yours, n)).toBe("Ask 2 of yours to hand off")
    expect(stopButton({ cards: false, yours: false }, n)).toBe("")
    expect(stopButton(cards, { cards: 1, yours: 0 })).toBe("Stop 1 agent")
  })

  test("Stop here's hover names the group", () => {
    expect(stopHereTitle(BOTH)).toBe(stopLabels.here.title)
    expect(stopHereTitle(cards)).toContain("Irori started")
    expect(stopHereTitle(yours)).toContain("your own sessions")
  })

  test("no word says Pause", () => {
    const words: string[] = []
    const walk = (v: unknown) => {
      if (typeof v === "string") words.push(v)
      else if (typeof v === "function") {
        // Numbers for counts, strings for times and names.
        for (const args of [
          [2, 1],
          ["9:00", "the person"],
        ]) {
          try {
            const out = (v as (...a: unknown[]) => unknown)(...args)
            if (typeof out === "string") words.push(out)
          } catch {
            // Takes another shape (a list of machines).
          }
        }
      } else if (v && typeof v === "object") Object.values(v).forEach(walk)
    }
    walk(stopLabels)
    expect(words.length).toBeGreaterThan(20)
    for (const w of words) expect(w).not.toMatch(/pause/i)
  })
})
