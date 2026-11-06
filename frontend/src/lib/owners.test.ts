import { describe, expect, test } from "bun:test"

import {
  cardOwner,
  FORMER_MEMBER,
  memberOf,
  ownedBy,
  sharedBoard,
} from "./owners"

const me = { personId: "p-stephen", name: "Stephen", me: true }
const blake = { personId: "p-blake", name: "Blake", me: false }
const shared = { boardId: "b-mlg", members: [me, blake] }
const solo = { boardId: "b-solo", members: [me] }
const boards = [shared, solo]

const card = (owner: string, board = "b-mlg", scope = "board") => ({
  scope,
  board,
  owner,
})

describe("cardOwner (card #268)", () => {
  test("a card made by the other member, or by one of their agents, is theirs", () => {
    expect(cardOwner(card("p-blake"), boards)).toEqual({
      personId: "p-blake",
      name: "Blake",
      me: false,
      known: true,
    })
  })

  test("the person's own card names them and says it is theirs", () => {
    expect(cardOwner(card("p-stephen"), boards)).toMatchObject({
      name: "Stephen",
      me: true,
    })
  })

  test("a member who left, or one not known here yet, is a former member", () => {
    expect(cardOwner(card("p-gone"), boards)).toEqual({
      personId: "p-gone",
      name: FORMER_MEMBER,
      me: false,
      known: false,
    })
  })

  test("no owner from the window: no chip", () => {
    expect(cardOwner(card(""), boards)).toBeNull()
  })

  test("private cards and boards that are only the person's show no owner", () => {
    expect(cardOwner(card("p-stephen", "", "mine"), boards)).toBeNull()
    expect(cardOwner(card("p-stephen", "b-solo"), boards)).toBeNull()
  })

  test("a board not in the list (not loaded yet) shows no owner", () => {
    expect(cardOwner(card("p-blake", "b-other"), boards)).toBeNull()
  })

  test("a member with no name is still named", () => {
    const b = {
      boardId: "b-x",
      members: [me, { personId: "p-x", name: " ", me: false }],
    }
    expect(memberOf("p-x", b.members)?.name).toBe("Someone")
  })
})

describe("sharedBoard and ownedBy", () => {
  test("only a board with two or more members counts as shared", () => {
    expect(sharedBoard(card(""), boards)?.boardId).toBe("b-mlg")
    expect(sharedBoard(card("", "b-solo"), boards)).toBeNull()
  })

  test("ownedBy matches the owner filter value", () => {
    expect(ownedBy(card("p-blake"), "owner:p-blake")).toBe(true)
    expect(ownedBy(card("p-blake"), "owner:p-stephen")).toBe(false)
    expect(ownedBy(card(""), "owner:")).toBe(false)
  })
})
