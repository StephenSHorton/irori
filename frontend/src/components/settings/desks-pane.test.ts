import { describe, expect, test } from "bun:test"

import { keptPollDevice } from "./desks-pane"

const mine = [
  { value: "pc", label: "PC" },
  { value: "mac", label: "Mac" },
]

describe("keptPollDevice (card #1240)", () => {
  test("one of your devices is kept", () => {
    expect(keptPollDevice("mac", mine, true)).toBe("mac")
  })
  test("a removed device is blank: the desk follows where it was seen", () => {
    expect(keptPollDevice("old-phone", mine, true)).toBe("")
  })
  test("blank stays blank", () => {
    expect(keptPollDevice("", mine, true)).toBe("")
  })
  test("before the devices are read, every id is kept", () => {
    expect(keptPollDevice("old-phone", [], false)).toBe("old-phone")
  })
})
