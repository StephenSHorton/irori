import { describe, expect, test } from "bun:test"

import { CardSummary, TaskState } from "@/lib/models"
import { boardSegs } from "./board-tiles"

const who = { key: "a", name: "a", status: "active", live: true, where: "" }

describe("boardSegs (card #126)", () => {
  test("counts a board's cards the way their chips colour them", () => {
    const cs = [
      CardSummary({ id: "1", state: TaskState.Working, working: [who] }),
      CardSummary({
        id: "2",
        state: TaskState.Working,
        working: [who],
        openBlockers: 1,
      }),
      CardSummary({ id: "3", state: TaskState.Submitted }),
      CardSummary({ id: "4", state: TaskState.Submitted }),
    ]
    const n = Object.fromEntries(boardSegs(cs, 2).map((s) => [s.key, s.n]))
    expect(n).toEqual({ working: 1, open: 2, blocked: 1, done: 2 })
  })
  test("a card ready for your Done counts as done", () => {
    const cs = [
      CardSummary({
        id: "1",
        state: TaskState.Working,
        working: [who],
        openBlockers: 1,
        ready: true,
      }),
    ]
    const n = Object.fromEntries(boardSegs(cs, 0).map((s) => [s.key, s.n]))
    expect(n).toEqual({ working: 0, open: 0, blocked: 0, done: 1 })
  })
})
