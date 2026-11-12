import { describe, expect, test } from "bun:test"

import type { WindDownRow, WindDownStatus } from "@/gen/bindings"

import { canCallOff, windDownLine, yoursOnly } from "./wind-down"

const row = (state: string): WindDownRow => ({
  key: "k",
  name: "n",
  deviceId: "",
  device: "",
  deliveryId: "d",
  state,
  reply: "",
  error: "",
  spawned: false,
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
  who: { cards: true, yours: true },
  rows: states.map(row),
  yoursWaiting: 0,
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

describe("Stop's who (decision 71)", () => {
  const yours = { who: { cards: false, yours: true } }
  test("a Yours-only run asks, it doesn't stop", () => {
    expect(
      windDownLine(run(["done", "winding"], { ...yours, finished: false }))
    ).toBe("Asking 2 of yours to hand off, 1 handed off so far")
    expect(windDownLine(run(["done", "no_answer"], yours))).toBe(
      "Asked 2 of yours to hand off, 1 handed off"
    )
  })

  test("Call off ends it: called off, never started again or stopped everywhere", () => {
    expect(
      windDownLine(run(["done", "canceled"], { ...yours, off: true }))
    ).toBe("Asked 2 of yours to hand off, 1 handed off · called off")
    // Stop everywhere lit by an earlier run: the Yours run's line has no
    // "stopped everywhere".
    expect(windDownLine(run(["done"], { ...yours, on: true, since: 1 }))).toBe(
      "Asked 1 of yours to hand off, 1 handed off"
    )
  })

  test("On cards alone reads like today's Stop", () => {
    expect(
      windDownLine(
        run(["done", "waiting"], {
          who: { cards: true, yours: false },
          finished: false,
        })
      )
    ).toBe("Stopping 2 agents, 1 handed off so far")
  })

  test("yoursOnly needs a run that reached Yours alone", () => {
    expect(yoursOnly(run(["done"], yours))).toBe(true)
    expect(yoursOnly(run(["done"]))).toBe(false)
    expect(yoursOnly(run([], { ...yours, id: "" }))).toBe(false)
    // The empty status (no run) has neither ticked.
    expect(
      yoursOnly(run([], { id: "", who: { cards: false, yours: false } }))
    ).toBe(false)
  })

  test("Call off shows for a Yours run going, or any Yours ask waiting", () => {
    const y = { who: { cards: false, yours: true } }
    expect(canCallOff(run(["waiting"], { ...y, finished: false }))).toBe(true)
    expect(
      canCallOff(run(["canceled"], { ...y, finished: true, off: true }))
    ).toBe(false)
    // No run in memory (a restart), asks still waiting.
    expect(canCallOff(run([], { id: "", yoursWaiting: 2 }))).toBe(true)
    // A Cards run replaced the Yours run; its asks still wait.
    const c = { who: { cards: true, yours: false }, on: true }
    expect(canCallOff(run(["winding"], { ...c, finished: false }))).toBe(false)
    expect(
      canCallOff(run(["winding"], { ...c, finished: false, yoursWaiting: 1 }))
    ).toBe(true)
  })
})
