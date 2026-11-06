import { describe, expect, test } from "bun:test"

import { DroppedInARow, windowTally } from "./window-log"

const read = { ms: 10, dropped: false, reloads: 1 }

describe("the window's line in irori.log (card #551)", () => {
  test("a minute with nothing writes no line", () => {
    expect(windowTally().line()).toBe("")
  })

  test("signals with no reads tell a read that hasn't answered", () => {
    const t = windowTally()
    t.signal("board:changed")
    t.signal("board:changed")
    t.signal("focus")
    expect(t.line()).toBe("lists: board:changed 2, focus 1")
  })

  test("names the slowest list and counts the slow reads", () => {
    const t = windowTally()
    t.read("cards", { ...read, ms: 1200 })
    t.read("threads", { ...read, ms: 2300, reloads: 9 })
    t.read("inbox", read)
    expect(t.line()).toBe(
      "lists: 3 reads for 11 reloads, 2 over 1s, slowest threads 2.3s"
    )
  })

  test("answers dropped in a row are named, and a shown one ends the run", () => {
    const t = windowTally()
    for (let i = 0; i < DroppedInARow; i++)
      t.read("threads", { ...read, dropped: true })
    t.read("threads", read)
    t.read("threads", { ...read, dropped: true })
    expect(t.line()).toBe(
      "lists: 5 reads for 5 reloads; 4 answers dropped, 3 in a row on threads"
    )
  })

  test("a few dropped answers are counted, not named", () => {
    const t = windowTally()
    t.read("threads", { ...read, dropped: true })
    expect(t.line()).toBe("lists: 1 reads for 1 reloads; 1 answers dropped")
  })

  test("each line starts a new tally", () => {
    const t = windowTally()
    t.signal("focus")
    t.line()
    expect(t.line()).toBe("")
  })
})
