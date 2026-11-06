import { describe, expect, test } from "bun:test"

import {
  ordinal,
  queued,
  spawnLine,
  startPastLimit,
  usageResets,
} from "./spawn-line"

describe("spawnLine (card #448)", () => {
  test("nothing to say", () => {
    expect(spawnLine({})).toBeNull()
  })
  test("starting wins over the rest", () => {
    // Card #489: this device by its own name.
    expect(
      spawnLine({
        starting: "Macbook Pro (this one)",
        atCap: "Macbook Pro (this one)",
      })?.text
    ).toBe("starting an agent on Macbook Pro (this one)")
    expect(spawnLine({ starting: "MacBook" })?.text).toBe(
      "starting an agent on MacBook"
    )
  })
  test("can't start: trust is named plainly, other reasons as given", () => {
    expect(spawnLine({ cantStart: "not trusted: /x on pc" })).toEqual({
      text: "can't start: its repo isn't trusted here",
      tone: "warn",
    })
    expect(
      spawnLine({ cantStart: "couldn't start: claude not found" })?.text
    ).toBe("can't start: claude not found")
    expect(
      spawnLine({ cantStart: "no copy of irori on Desktop Computer" })?.text
    ).toBe("can't start: no copy of irori on Desktop Computer")
  })
  test("its place in the queue (card #450)", () => {
    expect(spawnLine({ atCap: "the PC (this one)", queuePlace: 1 })).toEqual({
      text: "queued: 1st for the PC (this one)",
      tone: "quiet",
    })
    expect(spawnLine({ atCap: "the PC", queuePlace: 4 })?.text).toBe(
      "queued: 4th for the PC"
    )
  })
  test("a usage limit holds the queue (card #450)", () => {
    expect(
      spawnLine({
        atCap: "the PC",
        queuePlace: 2,
        usageLimit: "2026-11-04T23:30:00Z",
      })?.text
    ).toBe("queued: usage limit")
    expect(usageResets("2026-11-04T23:30:00Z", "en-US")).toMatch(
      /^resets \d{1,2}:30/
    )
    expect(usageResets("nope")).toBe("")
  })
  test("the daily limit waits for the person (card #450)", () => {
    expect(spawnLine({ dailyLimit: 20 })?.text).toBe(
      "waiting: today's 20 agents are started; start this one yourself"
    )
  })
  test("ordinals", () => {
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111].map(ordinal)).toEqual([
      "1st",
      "2nd",
      "3rd",
      "4th",
      "11th",
      "12th",
      "13th",
      "21st",
      "22nd",
      "101st",
      "111th",
    ])
  })
  test("queued is a place, a hold or the limit, never a failure", () => {
    expect(queued({ atCap: "pc", queuePlace: 1 })).toBe(true)
    expect(queued({ usageLimit: "2026-11-04T23:30:00Z" })).toBe(true)
    expect(queued({ dailyLimit: 3 })).toBe(true)
    expect(queued({ starting: "pc", atCap: "pc" })).toBe(false)
    expect(queued({ cantStart: "x", dailyLimit: 3 })).toBe(false)
    expect(queued({})).toBe(false)
  })
  test("at its cap", () => {
    expect(spawnLine({ atCap: "Macbook Pro (this one)" })).toEqual({
      text: "waiting: Macbook Pro (this one) is at its cap",
      tone: "quiet",
    })
  })
})

describe("startPastLimit (card #576)", () => {
  test("only a limit offers it", () => {
    expect(startPastLimit({})).toBe("")
    expect(startPastLimit({ atCap: "the PC", queuePlace: 2 })).toBe("")
    expect(startPastLimit({ dailyLimit: 20 })).toContain("past today's limit")
    expect(startPastLimit({ usageLimit: "2026-11-05T16:30:00Z" })).toContain(
      "Tries now"
    )
  })
  test("not while one is starting or can't", () => {
    expect(startPastLimit({ dailyLimit: 20, starting: "the PC" })).toBe("")
    expect(startPastLimit({ dailyLimit: 20, cantStart: "no copy" })).toBe("")
  })
})
