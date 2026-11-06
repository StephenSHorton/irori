import { describe, expect, test } from "bun:test"

import { mainRedText, mainReds } from "./main-red"

describe("mainReds (card #436)", () => {
  test("counts the open cards that wait on each red card", () => {
    const got = mainReds([
      { id: "r", short: "436", state: "submitted", mainRed: "attach_exe" },
      { id: "a", short: "1", state: "blocked", waitTask: "r" },
      { id: "b", short: "2", state: "blocked", waitTask: "r" },
      { id: "c", short: "3", state: "completed", waitTask: "r" },
      { id: "d", short: "4", state: "working" },
      { id: "old", short: "9", state: "completed", mainRed: "flaky" },
    ])
    expect(got).toEqual([
      { id: "r", ref: "#436", test: "attach_exe", waiting: 2 },
    ])
    expect(mainRedText(got[0])).toBe(
      "Main is red: attach_exe fails on clean main; 2 cards wait on #436."
    )
  })

  test("nothing red: no line", () => {
    expect(mainReds([{ id: "a", short: "1", state: "working" }])).toEqual([])
  })

  test("one waiter reads singular, a board's card keeps its tag", () => {
    const [r] = mainReds([
      { id: "r", short: "7", tag: "wind", state: "working", mainRed: "boards" },
      { id: "a", short: "1", state: "blocked", waitTask: "r" },
    ])
    expect(mainRedText(r)).toBe(
      "Main is red: boards fails on clean main; 1 card waits on wind#7."
    )
  })
})
