import { describe, expect, test } from "bun:test"

import { CardSummary } from "../../../bindings/github.com/StephenSHorton/irori/models"
import { TaskState } from "../../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import { asFilter, filters, matches, shownCards } from "./card-grid"

const card = (state: TaskState, openBlockers = 0) =>
  new CardSummary({ id: "c", state, openBlockers, scope: "mine" })

describe("filters", () => {
  test("Done is the last tab", () => {
    expect(filters.map((f) => f.id)).toEqual(["all", "decide", "mine", "done"])
    expect(filters.at(-1)?.label).toBe("Done")
  })
  test("Done holds the closed cards and nothing open", () => {
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
    expect(asFilter("nope")).toBe("all")
    expect(asFilter(null)).toBe("all")
  })
})

function card2(id: string, extra: Partial<CardSummary> = {}): CardSummary {
  return new CardSummary({ id, short: id, state: TaskState.Working, openBlockers: 0, scope: "mine", ...extra })
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
