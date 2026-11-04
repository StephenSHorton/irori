import { describe, expect, test } from "bun:test"

import { slotNumber } from "./check-slots"

describe("slotNumber", () => {
  test("reads the slot of a check slot lease", () => {
    expect(slotNumber("check/stephens-mbp-2/slot-1")).toBe("1")
    expect(slotNumber("check/pc/slot-12")).toBe("12")
    expect(slotNumber("check/pc")).toBe("?")
  })
})
