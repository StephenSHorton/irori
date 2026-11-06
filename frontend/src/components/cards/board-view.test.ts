import { describe, expect, test } from "bun:test"

import { CardSummary, TaskState } from "@/lib/models"
import { asFilter, matches } from "./card-grid"
import {
  ScopeOwn,
  activeFilter,
  hiddenByFilter,
  homeBoards,
  inScope,
  rememberedFilter,
  savedHomeFilter,
} from "./board-view"

const priv = CardSummary({ id: "p", scope: "mine" })
const ours = CardSummary({
  id: "o",
  scope: "board",
  board: "b-wind",
  scopeLabel: "Wind Waker Multiplayer",
  tag: "wind",
  ours: true,
})
const theirs = CardSummary({
  id: "t",
  scope: "board",
  board: "b-wind",
  scopeLabel: "Wind Waker Multiplayer",
  tag: "wind",
})
const horton = CardSummary({
  id: "h",
  scope: "board",
  board: "b-horton",
  scopeLabel: "horton",
  tag: "horton",
  ours: true,
})

describe("Home lists your agents' board cards (card #199)", () => {
  test("Home: private cards and board cards one of your agents is on", () => {
    expect(inScope(priv, ScopeOwn)).toBe(true)
    expect(inScope(ours, ScopeOwn)).toBe(true)
    expect(inScope(theirs, ScopeOwn)).toBe(false)
  })
  test("a board's page is unchanged: every card of that board, nothing else", () => {
    expect(inScope(ours, "b-wind")).toBe(true)
    expect(inScope(theirs, "b-wind")).toBe(true)
    expect(inScope(horton, "b-wind")).toBe(false)
    expect(inScope(priv, "b-wind")).toBe(false)
  })
  test("the board row: each board with a card on Home, once, in order", () => {
    const spaces = [
      { boardId: "b-wind", name: "Wind Waker Multiplayer", tag: "wind" },
      { boardId: "b-horton", name: "horton", tag: "horton" },
    ]
    expect(homeBoards([priv, ours, horton, theirs], spaces)).toEqual(spaces)
    expect(homeBoards([priv])).toEqual([])
  })
  test("the board row tags a space as the space, not by its first card (card #558)", () => {
    const net = CardSummary({ ...ours, id: "n", tag: "net", scopeLabel: "Wind · Netcode" })
    const spaces = [{ boardId: "b-wind", name: "Wind Waker Multiplayer", tag: "wind" }]
    expect(homeBoards([net, ours], spaces)).toEqual(spaces)
    // A space not heard of yet: the card's label, no tag.
    expect(homeBoards([net])).toEqual([{ boardId: "b-wind", name: "Wind · Netcode", tag: "" }])
  })
})

describe("Home's board filter (card #199)", () => {
  const row = homeBoards([priv, ours, horton])
  test("Private narrows to the cards on no board", () => {
    expect(matches(priv, "private")).toBe(true)
    expect(matches(ours, "private")).toBe(false)
    expect(asFilter("private")).toBe("private")
  })
  test("a board chip narrows to that board", () => {
    expect(matches(ours, "board:b-wind")).toBe(true)
    expect(matches(horton, "board:b-wind")).toBe(false)
  })
  test("on Home, Private and a board with cards there stay; a board with none falls back", () => {
    expect(activeFilter("private", [], row)).toBe("private")
    expect(activeFilter("board:b-horton", [], row)).toBe("board:b-horton")
    expect(activeFilter("board:b-gone", [], row)).toBe("all")
  })
  test("on a board page (no row) they fall back to All, as before", () => {
    expect(activeFilter("private", [])).toBe("all")
    expect(activeFilter("board:b-wind", [])).toBe("all")
    expect(activeFilter("mine", [], row)).toBe("all")
  })
  test("the pick Home remembers: All, Private or a board; not the other tabs", () => {
    expect(rememberedFilter("all")).toBe(true)
    expect(rememberedFilter("private")).toBe(true)
    expect(rememberedFilter("board:b-wind")).toBe(true)
    expect(rememberedFilter("decide")).toBe(false)
    expect(rememberedFilter("done")).toBe(false)
    expect(savedHomeFilter(null)).toBe("all")
    expect(savedHomeFilter("board:b-wind")).toBe("board:b-wind")
    expect(savedHomeFilter("private")).toBe("private")
    expect(savedHomeFilter("done")).toBe("all")
    expect(savedHomeFilter("nonsense")).toBe("all")
  })
})

describe("hiddenByFilter (card #237)", () => {
  const waiting = CardSummary({ id: "w", scope: "mine", openBlockers: 1 })
  const review = CardSummary({
    id: "r",
    scope: "mine",
    state: TaskState.InReview,
  })
  const plain = CardSummary({ id: "x", scope: "mine", title: "plain card" })
  const cards = [waiting, review, plain]

  test("nothing on All with no search, or on Closed", () => {
    expect(hiddenByFilter(cards, "all", "")).toBeNull()
    expect(hiddenByFilter(cards, "done", "")).toBeNull()
  })

  test("a filter that hides cards says how many, and how many need a decision", () => {
    expect(hiddenByFilter(cards, "review", "")).toEqual({
      hidden: 2,
      decide: 1,
    })
  })

  test("a search hides too", () => {
    expect(hiddenByFilter(cards, "all", "plain")).toEqual({
      hidden: 2,
      decide: 1,
    })
  })

  test("nothing hidden when the filter keeps everything open", () => {
    expect(hiddenByFilter([waiting], "decide", "")).toBeNull()
  })
})

describe("activeFilter's member chips (card #268)", () => {
  const members = [
    { personId: "p-me", name: "Stephen", me: true },
    { personId: "p-blake", name: "Blake", me: false },
  ]
  test("a member chip holds on a board that member is on", () => {
    expect(activeFilter("owner:p-blake", [], undefined, members)).toBe("owner:p-blake")
  })
  test("elsewhere it falls back to All", () => {
    expect(activeFilter("owner:p-gone", [], undefined, members)).toBe("all")
    expect(activeFilter("owner:p-blake", [], undefined, [members[0]])).toBe("all")
    expect(activeFilter("owner:p-blake", [])).toBe("all")
  })
})

