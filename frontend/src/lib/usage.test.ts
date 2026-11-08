import { describe, expect, test } from "bun:test"

import {
  asLimits,
  hasUsage,
  NO_USAGE,
  percentLeft,
  resetLine,
  sampleUsage,
  sortWindows,
  staleLine,
  stripLine,
  usageColor,
  usageTextColor,
  windowLabel,
} from "./usage"

const MIN = 60_000
const HOUR = 60 * MIN
// Local times, so the lines read the same in any time zone.
const now = new Date(2026, 10, 5, 14, 15).getTime() // Thu 5 Nov 2026 14:15

describe("Claude usage left (card #599)", () => {
  test("percent left, all of it once a window has reset", () => {
    expect(
      percentLeft(
        { kind: "five_hour", percentUsed: 38, resetsAtMs: now + HOUR },
        now
      )
    ).toBe(62)
    expect(
      percentLeft(
        { kind: "five_hour", percentUsed: 38, resetsAtMs: now - 1 },
        now
      )
    ).toBe(100)
    expect(
      percentLeft(
        { kind: "spend_limit", percentUsed: 130, resetsAtMs: null },
        now
      )
    ).toBe(0)
    expect(
      percentLeft(
        { kind: "seven_day", percentUsed: 0.4, resetsAtMs: null },
        now
      )
    ).toBe(100)
  })
  test("reset countdown: hours and minutes within a day, else the day and time", () => {
    expect(resetLine(now + 2 * HOUR + 10 * MIN, now)).toBe("resets in 2h 10m")
    expect(resetLine(now + 45 * MIN, now)).toBe("resets in 45m")
    expect(resetLine(now + 20_000, now)).toBe("resets in under a minute")
    expect(resetLine(new Date(2026, 10, 12, 14, 0).getTime(), now)).toBe(
      "resets Thu 14:00"
    )
    expect(resetLine(now - 1, now)).toBe("reset")
    expect(resetLine(null, now)).toBeNull()
  })
  test("staleness: a reading over ten minutes old says when it was taken", () => {
    expect(staleLine(now - 2 * MIN, now)).toBeNull()
    expect(staleLine(new Date(2026, 10, 5, 14, 5).getTime(), now)).toBeNull()
    expect(staleLine(new Date(2026, 10, 5, 14, 4).getTime(), now)).toBe(
      "as of 14:04"
    )
    expect(staleLine(new Date(2026, 10, 3, 9, 30).getTime(), now)).toBe(
      "as of Tue 09:30"
    )
    expect(staleLine(0, now)).toBeNull()
  })
  test("the strip's line", () => {
    expect(stripLine(sampleUsage(now), now)).toEqual({
      text: "62% · 39% left",
      title: "5-hour 62% left, weekly 39% left",
      color: null,
    })
    // A gateway's spend limit only: the strip still says something.
    const spend = {
      observedAtMs: now,
      windows: [{ kind: "spend_limit", percentUsed: 60, resetsAtMs: null }],
    }
    expect(stripLine(spend, now)).toEqual({
      text: "40% left",
      title: "Spend limit 40% left",
      color: null,
    })
    // Weekly only.
    const wk = {
      observedAtMs: now,
      windows: [{ kind: "seven_day", percentUsed: 10, resetsAtMs: null }],
    }
    expect(stripLine(wk, now)).toEqual({
      text: "90% left",
      title: "Weekly 90% left",
      color: null,
    })
    expect(stripLine(NO_USAGE, now)).toBeNull()
    expect(stripLine(sampleUsage(now, "none"), now)).toBeNull()
  })
  test("names and order", () => {
    expect(windowLabel("five_hour")).toBe("5-hour")
    expect(windowLabel("seven_day")).toBe("Weekly")
    expect(windowLabel("spend_limit")).toBe("Spend limit")
    const ws = sortWindows([
      { kind: "spend_limit", percentUsed: 1, resetsAtMs: null },
      { kind: "seven_day", percentUsed: 1, resetsAtMs: null },
      { kind: "five_hour", percentUsed: 1, resetsAtMs: null },
    ])
    expect(ws.map((w) => w.kind)).toEqual([
      "five_hour",
      "seven_day",
      "spend_limit",
    ])
  })
  test("the host's answer, or nothing without one", () => {
    expect(asLimits(null)).toEqual(NO_USAGE)
    expect(hasUsage(asLimits(null))).toBe(false)
    const u = asLimits({
      observedAtMs: 5,
      windows: [{ kind: "five_hour", percentUsed: 1, resetsAtMs: null }],
    })
    expect(hasUsage(u)).toBe(true)
  })

  test("the bar's color blends as usage runs out (card #772)", () => {
    // Plenty left: the theme's color.
    expect(usageColor(100)).toBe("var(--primary)")
    expect(usageColor(80)).toBe("var(--primary)")
    expect(usageColor(50)).toBe("var(--primary)")
    // Toward amber.
    expect(usageColor(40)).toBe(
      "color-mix(in oklch, var(--primary), var(--warning) 33%)"
    )
    expect(usageColor(35)).toBe(
      "color-mix(in oklch, var(--primary), var(--warning) 50%)"
    )
    expect(usageColor(20)).toBe("var(--warning)")
    // Toward red.
    expect(usageColor(10)).toBe(
      "color-mix(in oklch, var(--warning), var(--destructive) 50%)"
    )
    expect(usageColor(5)).toBe(
      "color-mix(in oklch, var(--warning), var(--destructive) 75%)"
    )
    expect(usageColor(0)).toBe("var(--destructive)")
    // Out of range is clamped.
    expect(usageColor(-5)).toBe("var(--destructive)")
    expect(usageColor(150)).toBe("var(--primary)")
    // Every step down is no calmer than the one before: the mix only grows.
    const rank = (c: string) => {
      if (c === "var(--primary)") return 0
      if (c === "var(--warning)") return 100
      if (c === "var(--destructive)") return 200
      const m = /var\(--(\w+)\), var\(--\w+\) (\d+)%/.exec(c)!
      return (m[1] === "primary" ? 0 : 100) + Number(m[2])
    }
    for (let l = 100; l > 0; l--)
      expect(rank(usageColor(l - 1))).toBeGreaterThanOrEqual(
        rank(usageColor(l))
      )
  })
  test("the strip's reading is tinted once low (card #772)", () => {
    expect(usageTextColor(80)).toBeNull()
    expect(usageTextColor(26)).toBeNull()
    expect(usageTextColor(25)).toBe(usageColor(25))
    expect(usageTextColor(5)).toBe(usageColor(5))
    // Tinted by the lowest window shown.
    const low = {
      observedAtMs: now,
      windows: [
        { kind: "five_hour", percentUsed: 95, resetsAtMs: now + HOUR },
        { kind: "seven_day", percentUsed: 40, resetsAtMs: null },
      ],
    }
    expect(stripLine(low, now)?.color).toBe(usageColor(5))
    // A window that has reset counts as full.
    const reset = {
      observedAtMs: now,
      windows: [
        { kind: "five_hour", percentUsed: 95, resetsAtMs: now - MIN },
        { kind: "seven_day", percentUsed: 40, resetsAtMs: null },
      ],
    }
    expect(stripLine(reset, now)?.color).toBeNull()
    // The preview's ?usage=<n> reading has n% left in both windows.
    expect(stripLine(sampleUsage(now, "20"), now)).toEqual({
      text: "20% · 20% left",
      title: "5-hour 20% left, weekly 20% left",
      color: "var(--warning)",
    })
  })
})
