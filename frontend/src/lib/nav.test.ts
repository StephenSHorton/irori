import { describe, expect, test } from "bun:test"

import { sameSection, sectionFromHash, sectionHash } from "./nav"

describe("Talk lives in the address (card #390)", () => {
  test("#talk is the list, #talk/<id> a conversation open", () => {
    expect(sectionFromHash("#talk")).toEqual({ kind: "talk", id: "" })
    expect(sectionFromHash("#talk/01ABC")).toEqual({ kind: "talk", id: "01ABC" })
  })
  test("the hash round-trips", () => {
    for (const s of [
      { kind: "talk", id: "" },
      { kind: "talk", id: "01M6 x/y" },
      { kind: "board", id: "b1" },
      { kind: "boards" },
      { kind: "home" },
    ] as const)
      expect(sectionFromHash(sectionHash(s))).toEqual(s)
  })
  test("another conversation is another place, so it adds history", () => {
    expect(sameSection({ kind: "talk", id: "a" }, { kind: "talk", id: "a" })).toBe(true)
    expect(sameSection({ kind: "talk", id: "a" }, { kind: "talk", id: "b" })).toBe(false)
    expect(sameSection({ kind: "talk", id: "" }, { kind: "home" })).toBe(false)
  })
})
