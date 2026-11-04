import { describe, expect, test } from "bun:test"

import { spawnLine } from "./spawn-line"

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
      spawnLine({ cantStart: "no checkout of remote:a/b on pc" })?.text
    ).toBe("can't start: no checkout of remote:a/b on pc")
  })
  test("at its cap", () => {
    expect(spawnLine({ atCap: "Macbook Pro (this one)" })).toEqual({
      text: "waiting: Macbook Pro (this one) is at its cap",
      tone: "quiet",
    })
  })
})
