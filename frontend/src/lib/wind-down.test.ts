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

const run = (
  states: string[],
  more: Partial<WindDownStatus> = {}
): WindDownStatus => ({
  on: false,
  since: 0,
  devices: [],
  id: "wd_1",
  scope: "this",
  finished: true,
  off: false,
  rows: states.map(row),
  ...more,
})

describe("Home's Stop line (cards #484, #773)", () => {
  test("one line for the whole run: how many it reached and how many answered", () => {
    const states = [...Array(23).fill("done"), ...Array(20).fill("no_answer")]
    expect(windDownLine(run(states))).toBe("Stopped 43 agents, 23 handed off")
  })

  test("while it runs it says so", () => {
    expect(
      windDownLine(run(["done", "winding", "waiting"], { finished: false }))
    ).toBe("Stopping 3 agents, 1 handed off so far")
    expect(windDownLine(run(["done"]))).toBe("Stopped 1 agent, 1 handed off")
  })

  test("a run none of whose sends went out says so, not 'Stopping' for ever", () => {
    const failed = { ...row("no_answer"), deliveryId: "", error: "no route" }
    expect(
      windDownLine({ ...run([]), finished: false, rows: [failed, failed] })
    ).toBe("Stopping reached none of 2 agents")
  })

  test("while Stop everywhere is on, a finished run says so (cards #773, #787)", () => {
    const states = [...Array(23).fill("done"), ...Array(20).fill("no_answer")]
    const on = { on: true, since: 1, devices: ["this Mac"] }
    expect(windDownLine(run(states, on))).toBe(
      "Stopped 43 agents, 23 handed off · stopped everywhere"
    )
    // Mid-run it is on by definition: no tail.
    expect(
      windDownLine(run(["done", "waiting"], { ...on, finished: false }))
    ).toBe("Stopping 2 agents, 1 handed off so far")
  })

  test("started again mid-run: the rows it never reached are canceled (card #787)", () => {
    // Start everywhere closes the wind-downs not reached yet; every row has an end.
    expect(
      windDownLine(
        run(["done", "no_answer", "canceled", "canceled"], { off: true })
      )
    ).toBe("Stopped 4 agents, 1 handed off · started again")
    // An agent already handing off may still answer after starting again.
    expect(
      windDownLine(
        run(["done", "winding", "canceled"], { off: true, finished: false })
      )
    ).toBe("Stopping 3 agents, 1 handed off so far · started again")
  })

  test("Stop everywhere on with no run here (another machine turned it on): no line", () => {
    expect(
      windDownLine(run([], { id: "", on: true, devices: ["Stephen's PC"] }))
    ).toBe("")
  })

  test("this machine's Stop here shows its run the same way (card #773)", () => {
    expect(
      windDownLine(
        run(["done", "winding"], { scope: "device", finished: false })
      )
    ).toBe("Stopping 2 agents, 1 handed off so far")
  })

  test("no run, no line", () => {
    expect(windDownLine(run([], { id: "" }))).toBe("")
    expect(windDownLine(run([]))).toBe("")
  })
})
