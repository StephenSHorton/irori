import { describe, expect, test } from "bun:test"

import { agentStatus } from "./agent-status"

// Card #489: every status reads the same words wherever it shows.
describe("agentStatus", () => {
  const now = Date.parse("2026-11-04T12:00:00Z")
  const say = (
    status: string,
    more: Partial<Parameters<typeof agentStatus>[0]> = {}
  ) => agentStatus({ status, ...more }, undefined, now)

  test("each status", () => {
    expect(say("busy")).toEqual({ text: "working", tone: "working" })
    expect(say("working")).toEqual({ text: "working", tone: "working" })
    expect(say("shell")).toEqual({ text: "running a job", tone: "job" })
    expect(say("unresponsive")).toEqual({ text: "not answering", tone: "warn" })
    expect(say("idle")).toEqual({ text: "idle", tone: "idle" })
    expect(say("waiting")).toEqual({
      text: "waiting on a permission prompt",
      tone: "warn",
    })
    expect(say("gone")).toEqual({ text: "gone", tone: "muted" })
    expect(say("starting")).toEqual({ text: "starting", tone: "muted" })
    expect(say("person")).toEqual({ text: "", tone: "muted" })
    expect(say("irori")).toEqual({ text: "", tone: "muted" })
    expect(say("compacting")).toEqual({ text: "working", tone: "working" })
  })

  test("offline and not running come before the status", () => {
    expect(say("busy", { offline: true }).text).toBe("device offline")
    expect(say("idle", { deviceState: "offline" }).text).toBe("device offline")
    expect(say("busy", { live: false }).text).toBe("not running")
    expect(say("gone", { offline: true }).text).toBe("gone")
    expect(say("idle", { live: true, deviceState: "online" }).text).toBe("idle")
  })

  test("a job says how long, from a minute on", () => {
    const job = (since: number | string | null) =>
      agentStatus({ status: "shell" }, since, now).text
    expect(job(now - 12 * 60_000)).toBe("running a job · 12 min")
    expect(job(new Date(now - 2 * 3_600_000).toISOString())).toBe(
      "running a job · 2 h"
    )
    expect(job(now - 30_000)).toBe("running a job")
    expect(job(null)).toBe("running a job")
    expect(job("")).toBe("running a job")
    // Go's zero time is no time.
    expect(job("0001-01-01T00:00:00Z")).toBe("running a job")
    // Only a job says how long.
    expect(agentStatus({ status: "busy" }, now - 12 * 60_000, now).text).toBe(
      "working"
    )
  })
})
