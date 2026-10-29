import { describe, expect, test } from "bun:test"

import { AgentRef } from "../../bindings/github.com/StephenSHorton/irori/models"
import { TaskState } from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import { busy, cardFromHash, cardHash, workLabel } from "./cards"

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
