import { describe, expect, test } from "bun:test"

import { CardSummary } from "@/lib/models"
import { TaskState } from "@/lib/models"
import { asFilter, filters, matches, ofOutcome, outcomes, shownCards } from "./card-grid"

const card = (state: TaskState, openBlockers = 0) =>
  CardSummary({ id: "c", state, openBlockers, scope: "mine" })

describe("filters", () => {
  test("Closed is the last tab (card #57)", () => {
    expect(filters.map((f) => f.id)).toEqual(["all", "decide", "mine", "done"])
    expect(filters.at(-1)?.label).toBe("Closed")
  })
  test("Closed narrows by outcome", () => {
    const cs = [TaskState.Completed, TaskState.Canceled, TaskState.Failed].map((s) =>
      CardSummary({ id: s, state: s, scope: "mine" })
    )
    expect(outcomes.map((o) => o.label)).toEqual(["Any outcome", "Done", "Canceled", "Failed"])
    expect(ofOutcome(cs, "any").length).toBe(3)
    expect(ofOutcome(cs, "completed").map((c) => c.id)).toEqual([TaskState.Completed])
    expect(ofOutcome(cs, "canceled").map((c) => c.id)).toEqual([TaskState.Canceled])
    expect(ofOutcome(cs, "failed").map((c) => c.id)).toEqual([TaskState.Failed])
  })
  test("Closed holds done, canceled and failed cards and nothing open", () => {
    expect(matches(card(TaskState.Completed), "done")).toBe(true)
    expect(matches(card(TaskState.Canceled), "done")).toBe(true)
    expect(matches(card(TaskState.Failed), "done")).toBe(true)
    expect(matches(card(TaskState.Working), "done")).toBe(false)
    expect(matches(card(TaskState.Blocked, 1), "done")).toBe(false)
  })
  test("the other tabs are over open cards only", () => {
    expect(matches(card(TaskState.Working), "all")).toBe(true)
    expect(matches(card(TaskState.Completed), "all")).toBe(false)
    expect(matches(card(TaskState.Blocked, 1), "decide")).toBe(true)
    expect(matches(card(TaskState.Working), "mine")).toBe(true)
    expect(matches(card(TaskState.Canceled), "mine")).toBe(false)
  })
  test("asFilter reads a known id, else all", () => {
    expect(asFilter("done")).toBe("done")
    expect(asFilter("closed")).toBe("done")
    expect(asFilter("nope")).toBe("all")
    expect(asFilter(null)).toBe("all")
    expect(asFilter("board:b1")).toBe("board:b1")
  })
  test("a board chip holds that board's cards; My devices holds none of them (v0.5 §8)", () => {
    const shared = CardSummary({ id: "s", state: TaskState.Working, scope: "board", board: "b1", tag: "mlg" })
    expect(matches(shared, "all")).toBe(true)
    expect(matches(shared, "board:b1")).toBe(true)
    expect(matches(shared, "board:b2")).toBe(false)
    expect(matches(shared, "mine")).toBe(false)
    expect(matches(card(TaskState.Working), "board:b1")).toBe(false)
  })
})

function card2(id: string, extra: Partial<CardSummary> = {}): CardSummary {
  return CardSummary({ id, short: id, state: TaskState.Working, openBlockers: 0, scope: "mine", ...extra })
}

describe("shownCards", () => {
  const parent = card2("7")
  const blocked = card2("9", { parent: { id: "7", short: "7", title: "p" }, openBlockers: 1 })
  const quiet = card2("8", { parent: { id: "7", short: "7", title: "p" } })
  const other = card2("3")
  const all = [parent, quiet, blocked, other]

  test("all shows every card and no context", () => {
    const { shown, context } = shownCards(all, "all")
    expect(shown.map((c) => c.id)).toEqual(["7", "8", "9", "3"])
    expect(context.size).toBe(0)
  })

  test("a matching subtask brings its parent along, dimmed", () => {
    const { shown, context } = shownCards(all, "decide")
    expect(shown.map((c) => c.id)).toEqual(["7", "9"])
    expect([...context]).toEqual(["7"])
  })
})
