import { describe, expect, test } from "bun:test"

import { slotLine } from "./slots"

describe("slotLine", () => {
  test("reads a held slot out of its pool's limit", () => {
    expect(slotLine({ slot: 1, limit: 2 })).toBe("1 of 2")
    expect(slotLine({ slot: 12, limit: 16 })).toBe("12 of 16")
  })
})
