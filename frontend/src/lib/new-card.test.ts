import { describe, expect, test } from "bun:test"

import { BoardSummary } from "./models"
import { boardTarget } from "./new-card"
import { Personal } from "./spaces"

// Card #558: New card puts a card on any board of its space.
describe("boardTarget", () => {
  const space = BoardSummary({
    boardId: "s",
    name: "MLG",
    tag: "mlg",
    boards: [
      { boardId: "s", name: "", tag: "" },
      { boardId: "n", name: "Netcode", tag: "net" },
    ],
  })
  const personal = [
    { boardId: Personal, name: "Personal", tag: "" },
    { boardId: "p-home", name: "House", tag: "home" },
  ]

  test("from a board's list: that board", () => {
    const t = boardTarget(space, personal, space.boards[1], null)
    expect(t.choices.map((x) => x.boardId)).toEqual(["s", "n"])
    expect(t.on?.boardId).toBe("n")
    expect([t.board, t.spaceBoard]).toEqual(["s", "n"])
  })

  test("from the space's page or Home: its first board, named as the space", () => {
    const t = boardTarget(space, personal, undefined, null)
    expect(t.on).toEqual({ boardId: "s", name: "MLG", tag: "mlg" })
    expect([t.board, t.spaceBoard]).toEqual(["s", "s"])
  })

  test("a pick wins; a pick from elsewhere is ignored", () => {
    expect(boardTarget(space, personal, undefined, "n").spaceBoard).toBe("n")
    expect(
      boardTarget(space, personal, space.boards[1], "p-home").spaceBoard
    ).toBe("n")
  })

  test("a space with one board has one choice", () => {
    const one = BoardSummary({ boardId: "o", name: "Solo", tag: "solo" })
    const t = boardTarget(one, personal, undefined, null)
    expect(t.choices).toHaveLength(1)
    expect([t.board, t.spaceBoard]).toEqual(["o", "o"])
  })

  test("no space: Personal's boards, its default first, the card private", () => {
    const t = boardTarget(undefined, personal, undefined, null)
    expect(t.choices.map((x) => x.boardId)).toEqual([Personal, "p-home"])
    expect([t.board, t.spaceBoard]).toEqual([undefined, Personal])
    expect(boardTarget(undefined, personal, personal[1], null).spaceBoard).toBe(
      "p-home"
    )
    expect(
      boardTarget(undefined, personal, undefined, "p-home").spaceBoard
    ).toBe("p-home")
  })

  test("Personal's boards not loaded yet: the open list, else nothing", () => {
    expect(boardTarget(undefined, [], personal[1], null).spaceBoard).toBe(
      "p-home"
    )
    expect(
      boardTarget(undefined, [], undefined, null).spaceBoard
    ).toBeUndefined()
  })
})
