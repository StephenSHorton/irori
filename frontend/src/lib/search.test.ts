import { describe, expect, test } from "bun:test"

import { AgentRef, CardAsk, CardHolder, CardSummary } from "./models"
import { cardMatches, matching, queryWords } from "./search"

const heron = AgentRef({
  key: "dev/heron-key",
  name: "sample-heron",
  status: "busy",
  live: true,
  where: "Macbook Pro (this one)",
  here: true,
})
const wren = AgentRef({
  key: "dev/wren-key",
  name: "sample-wren",
  status: "idle",
  live: true,
  where: "Macbook Pro (this one)",
  here: true,
})

const card = CardSummary({
  id: "a",
  short: "12",
  title: "Fix the boat clipping through the dock",
  blockerQuestion: "Clamp the roll at 30°?",
  searchText:
    "Boats stop sinking into the pier\nNo clipping in the test scene\nTune the hull collider\nYou",
  working: [heron],
  watching: [wren],
  holder: CardHolder({ who: heron }),
})

describe("queryWords", () => {
  test("splits on blanks, lower-cases, drops a lone #", () => {
    expect(queryWords("  Boat   DOCK # ")).toEqual(["boat", "dock"])
    expect(queryWords("")).toEqual([])
  })
})

describe("cardMatches", () => {
  test("an empty query matches every card", () => {
    expect(cardMatches(card, "")).toBe(true)
    expect(cardMatches(card, "   ")).toBe(true)
  })
  test("title words, any case, in any order", () => {
    expect(cardMatches(card, "DOCK boat")).toBe(true)
    expect(cardMatches(card, "clip")).toBe(true)
  })
  test("every word must match", () => {
    expect(cardMatches(card, "boat release")).toBe(false)
  })
  test("objective, done-when, step text and creator", () => {
    expect(cardMatches(card, "pier")).toBe(true)
    expect(cardMatches(card, "test scene")).toBe(true)
    expect(cardMatches(card, "collider")).toBe(true)
    expect(cardMatches(card, "you")).toBe(true)
  })
  test("the open question", () => {
    expect(cardMatches(card, "clamp roll")).toBe(true)
  })
  test("who holds or watches it", () => {
    expect(cardMatches(card, "heron")).toBe(true)
    expect(cardMatches(card, "wren")).toBe(true)
    expect(cardMatches(card, "willow")).toBe(false)
  })
  test("a card that is an ask matches its taker", () => {
    const ask = CardSummary({
      ...card,
      working: [],
      watching: [],
      holder: CardHolder(),
      ask: CardAsk({ taker: AgentRef({ ...wren, name: "willow" }) }),
    })
    expect(cardMatches(ask, "willow")).toBe(true)
  })
  test("a # word is the card's ref, exactly", () => {
    expect(cardMatches(card, "#12")).toBe(true)
    expect(cardMatches(card, "#1")).toBe(false)
    expect(cardMatches(card, "#120")).toBe(false)
    expect(cardMatches(card, "#12 dock")).toBe(true)
    expect(cardMatches(card, "#12 release")).toBe(false)
  })
  test("a bare number finds the ref too", () => {
    expect(cardMatches(card, "12")).toBe(true)
  })
  test("a shared card matches tag#short and the short it had", () => {
    const shared = CardSummary({
      ...card,
      tag: "mlg",
      short: "3",
      shortWas: "12",
    })
    expect(cardMatches(shared, "mlg#3")).toBe(true)
    expect(cardMatches(shared, "MLG#3")).toBe(true)
    expect(cardMatches(shared, "#3")).toBe(true)
    expect(cardMatches(shared, "#12")).toBe(true)
    expect(cardMatches(shared, "abc#3")).toBe(false)
  })
  test("a card without searchText still matches its title", () => {
    const bare = CardSummary({ id: "b", short: "7", title: "Ship the release" })
    expect(cardMatches(bare, "release")).toBe(true)
    expect(cardMatches(bare, "pier")).toBe(false)
  })
})

describe("matching", () => {
  test("keeps order, and every card with no query", () => {
    const other = CardSummary({
      id: "b",
      short: "7",
      title: "Boat physics pass",
    })
    expect(matching([card, other], "boat").map((c) => c.id)).toEqual(["a", "b"])
    expect(matching([card, other], "dock").map((c) => c.id)).toEqual(["a"])
    expect(matching([card, other], "").length).toBe(2)
    expect(matching([card, other], "nothing-like-it")).toEqual([])
  })
})
