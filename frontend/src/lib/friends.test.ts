import { describe, expect, test } from "bun:test"

import { friendLine, friendsOf } from "./friends"
import { BoardSummary } from "./models"

const member = (personId: string, name: string, me = false) => ({
  personId,
  name,
  me,
  presence: "online",
  working: 0,
  free: 0,
})

const mlg = BoardSummary({
  boardId: "b-mlg",
  name: "MLG",
  tag: "mlg",
  members: [member("me", "Stephen", true), member("p1", "Blake"), member("p2", "Casey")],
})
// Blake's friend space: hidden, named after whoever invited.
const withBlake = BoardSummary({
  boardId: "b-fr1",
  name: "Stephen",
  tag: "fr-0a1b2c3d",
  friend: true,
  members: [member("me", "Stephen", true), member("p1", "Blake")],
})
// Dana is only a friend-invited friend.
const withDana = BoardSummary({
  boardId: "b-fr2",
  name: "Dana",
  tag: "fr-4e5f6a7b",
  friend: true,
  members: [member("me", "Stephen", true), member("p3", "Dana")],
})
const solo = BoardSummary({
  boardId: "b-solo",
  name: "Notes",
  tag: "notes",
  members: [member("me", "Stephen", true)],
})

describe("friends (card #517)", () => {
  test("a friend is anyone you share a space with, friend spaces included, once each", () => {
    const fs = friendsOf([mlg, withBlake, withDana, solo])
    expect(fs.map((f) => f.name)).toEqual(["Blake", "Casey", "Dana"])
  })

  test("friend spaces mark the friend and stay out of the space names", () => {
    const [blake, casey, dana] = friendsOf([mlg, withBlake, withDana])
    expect(blake.invited).toBe(true)
    expect(blake.spaces).toEqual(["MLG"])
    expect(friendLine(blake)).toBe("in MLG")
    expect(casey.invited).toBe(false)
    expect(dana.spaces).toEqual([])
    expect(friendLine(dana)).toBe("friend")
  })

  test("a space you're leaving doesn't count", () => {
    const leaving = BoardSummary({ ...mlg, leaving: true })
    expect(friendsOf([leaving])).toEqual([])
  })
})
