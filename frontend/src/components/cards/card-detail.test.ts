import { describe, expect, test } from "bun:test"

import { AgentRef, CardDetail, Holding, TaskState } from "@/lib/models"
import { startable } from "./card-detail"

const nobody = {
  state: Holding.HoldNobody,
  who: AgentRef({}),
} as CardDetail["holder"]

// Card #558: a tag no longer means shared, since a Personal board's card
// has one.
describe("startable", () => {
  const card = (p: Partial<CardDetail>) =>
    CardDetail({ state: TaskState.Submitted, holder: nobody, ...p })
  test("a private card on a Personal board can start an agent", () => {
    expect(startable(card({ tag: "home" }))).toBe(true)
    expect(startable(card({}))).toBe(true)
  })
  test("a shared card can't", () => {
    const onBoard = { id: "s" } as CardDetail["onBoard"]
    expect(startable(card({ tag: "mlg", onBoard }))).toBe(false)
  })
})
