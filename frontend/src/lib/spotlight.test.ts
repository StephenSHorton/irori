import { describe, expect, test } from "bun:test"

import { linePath, SpotlightMs } from "./spotlight"

describe("the chat's line to a highlight (card #784)", () => {
  const button = { left: 700, top: 400, right: 820, bottom: 430 }
  test("from the button's left middle to the nearest point of the target", () => {
    // A card header to the left: its right edge, level with the button.
    const l = linePath(button, { left: 40, top: 300, right: 600, bottom: 500 })
    expect(l?.a).toEqual({ x: 700, y: 415 })
    expect(l?.b).toEqual({ x: 600, y: 415 })
    expect(l?.d).toBe("M 700 415 Q 600 415 600 415")
  })
  test("a target above (the top bar's Stop here): into its bottom", () => {
    const l = linePath(button, { left: 760, top: 40, right: 860, bottom: 70 })
    expect(l?.b).toEqual({ x: 760, y: 70 })
    expect(l?.d).toBe("M 700 415 Q 760 415 760 70")
  })
  test("a target running under the chat: aimed left of the panel", () => {
    const panel = { left: 704, top: 145, right: 1084, bottom: 700 }
    const inChat = { left: 721, top: 360, right: 840, bottom: 386 }
    // The card page's header, under the chat's top.
    const l = linePath(
      inChat,
      { left: 262, top: 117, right: 1078, bottom: 255 },
      panel
    )
    expect(l?.b).toEqual({ x: 698, y: 255 })
    // The Settings dialog's row, under the chat's left side.
    const s = linePath(
      inChat,
      { left: 395, top: 240, right: 885, bottom: 478 },
      panel
    )
    expect(s?.b).toEqual({ x: 698, y: 373 })
  })
  test("a DOMRect-like box (getters, not fields) still clamps", () => {
    const target = Object.create({
      get left() {
        return 262
      },
      get top() {
        return 117
      },
      get right() {
        return 1078
      },
      get bottom() {
        return 255
      },
    }) as { left: number; top: number; right: number; bottom: number }
    const l = linePath(
      { left: 721, top: 360, right: 840, bottom: 386 },
      target,
      { left: 704, top: 145, right: 1084, bottom: 700 }
    )
    expect(l?.d).not.toContain("NaN")
    expect(l?.b).toEqual({ x: 698, y: 255 })
  })
  test("no line when the two touch", () => {
    expect(
      linePath(button, { left: 690, top: 400, right: 710, bottom: 430 })
    ).toBeNull()
  })
  test("the highlight holds 8 s, as the CSS animations fit", () => {
    expect(SpotlightMs).toBe(8000)
  })
})
