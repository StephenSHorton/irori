import { describe, expect, test } from "bun:test"

import type { WindDownRow, WindDownStatus } from "@/gen/bindings"

import { windDownLine } from "./wind-down"

const row = (state: string): WindDownRow => ({
  key: "k",
  name: "n",
  deviceId: "",
  device: "",
  deliveryId: "d",
  state,
  reply: "",
  error: "",
})

const run = (states: string[], more: Partial<WindDownStatus> = {}): WindDownStatus => ({
  id: "wd_1",
  scope: "this",
  pauseAfter: true,
  paused: false,
  finished: true,
  rows: states.map(row),
  ...more,
})

describe("Home's Wind down line (card #484)", () => {
  test("one line for the whole run: how many it reached and how many answered", () => {
    const states = [...Array(23).fill("done"), ...Array(20).fill("no_answer")]
    expect(windDownLine(run(states))).toBe("Wound down 43 agents, 23 confirmed")
    expect(windDownLine(run(states, { paused: true }))).toBe(
      "Wound down 43 agents, 23 confirmed · Pause all is on"
    )
  })

  test("while it runs it says so", () => {
    expect(windDownLine(run(["done", "winding", "waiting"], { finished: false }))).toBe(
      "Winding down 3 agents, 1 confirmed so far"
    )
    expect(windDownLine(run(["done"]))).toBe("Wound down 1 agent, 1 confirmed")
  })

  test("a run none of whose sends went out says so, not 'Winding down' for ever", () => {
    const failed = { ...row("no_answer"), deliveryId: "", error: "no route" }
    expect(windDownLine({ ...run([]), finished: false, rows: [failed, failed] })).toBe(
      "Wind down reached none of 2 agents"
    )
  })

  test("no run, no line", () => {
    expect(windDownLine(run([], { id: "" }))).toBe("")
    expect(windDownLine(run([]))).toBe("")
  })
})
