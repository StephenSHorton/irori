import { describe, expect, test } from "bun:test"

import {
  asLimits,
  hasUsage,
  NO_USAGE,
  percentLeft,
  percentUsed,
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
      text: "38% · 61% used",
      title: "5-hour 38% used, weekly 61% used",
      color: null,
    })
    // A gateway's spend limit only: the strip still says something.
    const spend = {
      observedAtMs: now,
      windows: [{ kind: "spend_limit", percentUsed: 60, resetsAtMs: null }],
    }
    expect(stripLine(spend, now)).toEqual({
      text: "60% used",
      title: "Spend limit 60% used",
      color: null,
    })
    // Weekly only.
    const wk = {
      observedAtMs: now,
      windows: [{ kind: "seven_day", percentUsed: 10, resetsAtMs: null }],
    }
    expect(stripLine(wk, now)).toEqual({
      text: "10% used",
      title: "Weekly 10% used",
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

  test("percent used, none once a window has reset (card #772)", () => {
    const w = { kind: "five_hour", percentUsed: 38, resetsAtMs: now + HOUR }
    expect(percentUsed(w, now)).toBe(38)
    expect(percentUsed({ ...w, resetsAtMs: now - MIN }, now)).toBe(0)
    // A spend limit past 100 shows full, never more.
    expect(percentUsed({ ...w, percentUsed: 130 }, now)).toBe(100)
  })
  test("the bar's color blends as it fills (card #772)", () => {
    // Plenty left: the theme's color.
    expect(usageColor(0)).toBe("var(--primary)")
    expect(usageColor(20)).toBe("var(--primary)")
    expect(usageColor(50)).toBe("var(--primary)")
    // Toward amber.
    expect(usageColor(60)).toBe(
      "color-mix(in oklab, var(--primary), var(--warning) 33%)"
    )
    expect(usageColor(65)).toBe(
      "color-mix(in oklab, var(--primary), var(--warning) 50%)"
    )
    expect(usageColor(80)).toBe("var(--warning)")
    // Toward red.
    expect(usageColor(90)).toBe(
      "color-mix(in oklch, var(--warning), var(--destructive) 50%)"
    )
    expect(usageColor(95)).toBe(
      "color-mix(in oklch, var(--warning), var(--destructive) 75%)"
    )
    expect(usageColor(100)).toBe("var(--destructive)")
    // Out of range is clamped.
    expect(usageColor(130)).toBe("var(--destructive)")
    expect(usageColor(-5)).toBe("var(--primary)")
    // Every step up is no calmer than the one before: the mix only grows.
    const rank = (c: string) => {
      if (c === "var(--primary)") return 0
      if (c === "var(--warning)") return 100
      if (c === "var(--destructive)") return 200
      const m = /in \w+, var\(--(\w+)\), var\(--\w+\) (\d+)%/.exec(c)!
      return (m[1] === "primary" ? 0 : 100) + Number(m[2])
    }
    for (let u = 0; u < 100; u++)
      expect(rank(usageColor(u + 1))).toBeGreaterThanOrEqual(
        rank(usageColor(u))
      )
  })
  test("the strip's reading is tinted once nearly full (card #772)", () => {
    expect(usageTextColor(20)).toBeNull()
    expect(usageTextColor(74)).toBeNull()
    expect(usageTextColor(75)).toBe(usageColor(75))
    expect(usageTextColor(95)).toBe(usageColor(95))
    // Tinted by the fullest window shown.
    const full = {
      observedAtMs: now,
      windows: [
        { kind: "five_hour", percentUsed: 95, resetsAtMs: now + HOUR },
        { kind: "seven_day", percentUsed: 40, resetsAtMs: null },
      ],
    }
    expect(stripLine(full, now)?.color).toBe(usageColor(95))
    // A window that has reset counts as unused.
    const reset = {
      observedAtMs: now,
      windows: [
        { kind: "five_hour", percentUsed: 95, resetsAtMs: now - MIN },
        { kind: "seven_day", percentUsed: 40, resetsAtMs: null },
      ],
    }
    expect(stripLine(reset, now)?.color).toBeNull()
    // The preview's ?usage=<n> reading has n% used in both windows.
    expect(stripLine(sampleUsage(now, "80"), now)).toEqual({
      text: "80% · 80% used",
      title: "5-hour 80% used, weekly 80% used",
      color: "var(--warning)",
    })
  })
})
