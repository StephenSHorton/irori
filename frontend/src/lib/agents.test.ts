import { describe, expect, test } from "bun:test"

import { effortLabel, idleLabel, modelLabel, runningSince } from "./agents"

describe("Settings › Agents labels", () => {
  test("the default is marked, the rest named", () => {
    expect(modelLabel("sonnet", "sonnet")).toBe("Sonnet (default)")
    expect(modelLabel("opus", "sonnet")).toBe("Opus")
    expect(effortLabel("xhigh", "medium")).toBe("Extra high")
    expect(effortLabel("medium", "medium")).toBe("Medium (default)")
  })
  test("idle limits in minutes or hours, 30 marked", () => {
    expect(idleLabel(10)).toBe("10 minutes")
    expect(idleLabel(30)).toBe("30 minutes (default)")
    expect(idleLabel(60)).toBe("1 hour")
    expect(idleLabel(240)).toBe("4 hours")
  })
  test("an unknown value shows as itself", () => {
    expect(modelLabel("fable", "sonnet")).toBe("fable")
  })
})

describe("runningSince", () => {
  test("minutes, then hours", () => {
    expect(runningSince(0, 30_000)).toBe("just now")
    expect(runningSince(0, 6 * 60_000)).toBe("6 min")
    expect(runningSince(0, 130 * 60_000)).toBe("2 h")
  })
})
