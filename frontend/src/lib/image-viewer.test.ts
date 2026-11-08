import { describe, expect, test } from "bun:test"

import { navDirection, stepIndex } from "./image-viewer"

describe("stepIndex", () => {
  test("moves by one", () => {
    expect(stepIndex(1, 1, 4)).toBe(2)
    expect(stepIndex(2, -1, 4)).toBe(1)
  })
  test("wraps at both ends", () => {
    expect(stepIndex(3, 1, 4)).toBe(0)
    expect(stepIndex(0, -1, 4)).toBe(3)
  })
  test("a lone picture stays put", () => {
    expect(stepIndex(0, 1, 1)).toBe(0)
    expect(stepIndex(0, -1, 1)).toBe(0)
    expect(stepIndex(0, 1, 0)).toBe(0)
  })
})

describe("navDirection", () => {
  test("arrows move, other keys don't", () => {
    expect(navDirection({ key: "ArrowRight" })).toBe(1)
    expect(navDirection({ key: "ArrowLeft" })).toBe(-1)
    expect(navDirection({ key: "ArrowUp" })).toBeNull()
    expect(navDirection({ key: "Escape" })).toBeNull()
  })
  test("chords and text fields are left alone", () => {
    expect(navDirection({ key: "ArrowRight", metaKey: true })).toBeNull()
    expect(navDirection({ key: "ArrowLeft", altKey: true })).toBeNull()
    expect(
      navDirection({ key: "ArrowLeft", target: { tagName: "INPUT" } })
    ).toBeNull()
    expect(
      navDirection({ key: "ArrowLeft", target: { tagName: "BUTTON" } })
    ).toBe(-1)
  })
})
