import { describe, expect, test } from "bun:test"

import { AgentRef, CardAsk } from "../../bindings/github.com/StephenSHorton/irori/models"
import { TaskState } from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import { askLine, busy, cardFromHash, cardHash, deviceChips, deviceVar, holderLineShows, Holding, nobodyOn, openAsk, overLabel, workLabel } from "./cards"

const agent = (status: string, more: Partial<AgentRef> = {}) =>
  new AgentRef({ key: "d/s", name: "a", status, live: status !== "gone", where: "This PC", ...more })

const card = (state: TaskState, working: AgentRef[], openBlockers = 0) => ({
  state,
  openBlockers,
  working,
})

describe("busy", () => {
  test("a busy holder on a working card spins", () => {
    expect(busy(card(TaskState.Working, [agent("busy")]))).toBe(true)
  })
  test("idle, gone, offline or only assigned holders don't", () => {
    expect(busy(card(TaskState.Working, [agent("idle")]))).toBe(false)
    expect(busy(card(TaskState.Working, [agent("gone")]))).toBe(false)
    expect(busy(card(TaskState.Working, [agent("busy", { offline: true })]))).toBe(false)
    expect(busy(card(TaskState.Working, [agent("busy", { assigned: true })]))).toBe(false)
    expect(busy(card(TaskState.Working, []))).toBe(false)
  })
  test("waiting on the person or done doesn't", () => {
    expect(busy(card(TaskState.InputRequired, [agent("busy")]))).toBe(false)
    expect(busy(card(TaskState.Blocked, [agent("busy")], 1))).toBe(false)
    expect(busy(card(TaskState.Completed, [agent("busy")]))).toBe(false)
    expect(busy(card(TaskState.Canceled, [agent("busy")]))).toBe(false)
  })
  test("one busy holder among idle ones is enough", () => {
    expect(busy(card(TaskState.Working, [agent("idle"), agent("busy")]))).toBe(true)
  })
})

describe("workLabel", () => {
  test("says working only when the card spins", () => {
    expect(workLabel(card(TaskState.Working, [agent("busy")]))).toBe("working")
    expect(workLabel(card(TaskState.Working, [agent("idle")]))).toBe("on it")
    expect(workLabel(card(TaskState.Blocked, [agent("busy")], 1))).toBe("on it")
    expect(workLabel(card(TaskState.InputRequired, [agent("busy")]))).toBe("on it")
  })
  test("a finished card says nothing", () => {
    expect(workLabel(card(TaskState.Completed, [agent("busy")]))).toBe("")
  })
  test("assigned holders say so", () => {
    expect(workLabel(card(TaskState.Working, [agent("idle", { assigned: true })]))).toBe("assigned")
    expect(
      workLabel(card(TaskState.Working, [agent("idle", { assigned: true, waiting: true })]))
    ).toBe("not picked up")
    expect(workLabel(card(TaskState.Working, []))).toBe("")
  })
})

describe("card hash", () => {
  test("round-trips an id", () => {
    expect(cardFromHash(cardHash("01M62QZZYANGS87A9MNTP2M54J"))).toBe("01M62QZZYANGS87A9MNTP2M54J")
    expect(cardFromHash(cardHash("a b/c"))).toBe("a b/c")
  })
  test("anything else is the grid", () => {
    expect(cardFromHash("")).toBe("")
    expect(cardFromHash("#")).toBe("")
    expect(cardFromHash("#card/")).toBe("")
    expect(cardFromHash("#card/%E0%A4%A")).toBe("")
  })
})

describe("asks", () => {
  const ask = (more: Partial<CardAsk>) =>
    new CardAsk({ state: "open", told: 3, owner: "MacBook", wake: false, taker: null, outcome: null, ...more })
  test("the tile's line says where an ask stands", () => {
    expect(askLine(ask({}))).toBe("3 agents heard it")
    expect(askLine(ask({ told: 1 }))).toBe("1 agent heard it")
    expect(askLine(ask({ told: 0 }))).toBe("Nobody was free to hear it")
    expect(askLine(ask({ state: "waiting" }))).toBe("Waiting for MacBook to hand it out")
    expect(askLine(ask({ state: "unclaimed" }))).toBe("Nobody took this yet")
    expect(askLine(ask({ state: "taken", taker: agent("idle", { name: "sample-heron" }) }))).toBe("sample-heron took it")
  })
  test("only an ask without an outcome is open", () => {
    expect(openAsk({ ask: ask({}) })).toBe(true)
    expect(openAsk({ ask: ask({ state: "closed", outcome: { kind: "none", note: "x" } }) })).toBe(false)
    expect(openAsk({ ask: null })).toBe(false)
    expect(openAsk({})).toBe(false)
  })
  test("a closed ask reads by its outcome (card #28)", () => {
    const closed = (kind: string) => ask({ state: "closed", outcome: { kind, note: "x" } })
    expect(overLabel({ state: TaskState.Completed, ask: closed("answered") })).toBe("Answered")
    expect(overLabel({ state: TaskState.Canceled, ask: closed("none") })).toBe("Closed")
    expect(overLabel({ state: TaskState.Completed, ask: closed("edit") })).toBe("Done")
    expect(overLabel({ state: TaskState.Completed })).toBe("Done")
    expect(overLabel({ state: TaskState.Canceled })).toBe("Canceled")
    expect(overLabel({ state: TaskState.Failed })).toBe("Failed")
    expect(askLine(closed("answered"))).toBe("Answered: x")
    expect(askLine(closed("none"))).toBe("Closed: x")
  })
})

describe("gone holders (card #16)", () => {
  test("a card with steps shows its holder line unless someone is busy on it", () => {
    expect(holderLineShows({ holder: { state: Holding.HoldActive } })).toBe(false)
    expect(holderLineShows({ holder: { state: Holding.HoldPerson } })).toBe(false)
    for (const s of [Holding.HoldIdle, Holding.HoldGone, Holding.HoldOffline, Holding.HoldNobody, Holding.HoldWaiting])
      expect(holderLineShows({ holder: { state: s } })).toBe(true)
  })
  test("nobody is on a card whose holder went away or was taken off", () => {
    expect(nobodyOn({ holder: { state: Holding.HoldGone } })).toBe(true)
    expect(nobodyOn({ holder: { state: Holding.HoldNobody }, returned: agent("gone") })).toBe(true)
    expect(nobodyOn({ holder: { state: Holding.HoldNobody } })).toBe(false)
    expect(nobodyOn({ holder: { state: Holding.HoldIdle } })).toBe(false)
  })
})

describe("device colours (card #22)", () => {
  const pc = agent("busy", { deviceId: "pc", where: "This PC", color: "teal" })
  const mac = agent("idle", { key: "mac/s", deviceId: "mac", where: "MacBook", color: "orange" })
  test("a chip per device the cards' agents run on, once there are two", () => {
    expect(deviceChips([{ working: [pc] }, { working: [pc] }])).toEqual([])
    expect(deviceChips([{ working: [pc] }, { working: [mac, pc] }])).toEqual([
      { id: "mac", name: "MacBook", color: "orange" },
      { id: "pc", name: "This PC", color: "teal" },
    ])
  })
  test("faces with no colour (one device) give no chips", () => {
    expect(deviceChips([{ working: [agent("busy", { deviceId: "pc" })] }, { working: [agent("idle", { deviceId: "mac" })] }])).toEqual([])
  })
  test("a colour is a CSS variable", () => {
    expect(deviceVar("teal")).toBe("var(--device-teal)")
    expect(deviceVar("")).toBe("")
  })
})

describe("closed cards (Done tab)", () => {
  test("no worker label or spinner on a closed card, whatever its holders", () => {
    for (const st of [TaskState.Completed, TaskState.Canceled, TaskState.Failed]) {
      expect(workLabel(card(st, [agent("busy")]))).toBe("")
      expect(workLabel(card(st, [agent("idle", { assigned: true })]))).toBe("")
      expect(workLabel(card(st, [agent("idle", { assigned: true, waiting: true })]))).toBe("")
      expect(busy(card(st, [agent("busy")]))).toBe(false)
    }
  })
})
