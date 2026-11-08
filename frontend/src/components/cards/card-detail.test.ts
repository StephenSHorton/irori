import { describe, expect, test } from "bun:test"

import { AgentRef, CardAsk, CardDetail, Holding, TaskState } from "@/lib/models"
import { askedTitle, startable, threadAgents } from "./card-detail"

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

// Card #780: a Talk thread opens in its ask card view.
describe("a thread in the ask card view", () => {
  const heron = AgentRef({ key: "heron", name: "sample-heron" })
  const otter = AgentRef({ key: "otter", name: "sample-otter" })
  const t = { openedBy: null, told: [heron], answeredBy: [heron, otter] }
  test("its question is what you asked, or the agent's that opened it", () => {
    expect(askedTitle({ ask: null }, t)).toBe("You asked")
    expect(askedTitle({ ask: null }, { openedBy: otter })).toBe("Asked")
  })
  test("a card keeps its objective, an open ask its question", () => {
    expect(askedTitle({ ask: null })).toBe("Objective")
    expect(askedTitle({ ask: CardAsk({ state: "taken" }) })).toBe("You asked")
    expect(askedTitle({ ask: CardAsk({ state: "closed" }) })).toBe("Objective")
  })
  test("its agents are the holders and the thread's, once each", () => {
    const keys = threadAgents({ working: [heron] }, t).map((a) => a.key)
    expect(keys).toEqual(["heron", "otter"])
  })
  test("a card's agents are its holders", () => {
    expect(threadAgents({ working: [otter] })).toEqual([otter])
  })
})
