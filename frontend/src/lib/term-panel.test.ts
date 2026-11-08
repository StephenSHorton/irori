import { describe, expect, test } from "bun:test"

import {
  MinHeight,
  PageRoom,
  clampHeight,
  isTerminalToggle,
} from "./term-panel"

const k = (
  code: string,
  mods: Partial<{
    ctrlKey: boolean
    metaKey: boolean
    altKey: boolean
    shiftKey: boolean
  }> = {}
) => ({
  code,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
})

describe("isTerminalToggle", () => {
  test("Ctrl+` on both platforms", () => {
    expect(isTerminalToggle(k("Backquote", { ctrlKey: true }))).toBe(true)
  })
  test("by the key's place, whatever it types", () => {
    // A layout where that key types ^ or ° still has code Backquote.
    expect(isTerminalToggle(k("Backquote", { ctrlKey: true }))).toBe(true)
    expect(isTerminalToggle(k("Quote", { ctrlKey: true }))).toBe(false)
  })
  test("not plain `, Cmd+` (switches windows), or with Shift or Alt", () => {
    expect(isTerminalToggle(k("Backquote"))).toBe(false)
    expect(isTerminalToggle(k("Backquote", { metaKey: true }))).toBe(false)
    expect(
      isTerminalToggle(k("Backquote", { ctrlKey: true, shiftKey: true }))
    ).toBe(false)
    expect(
      isTerminalToggle(k("Backquote", { ctrlKey: true, altKey: true }))
    ).toBe(false)
    expect(
      isTerminalToggle(k("Backquote", { ctrlKey: true, metaKey: true }))
    ).toBe(false)
  })
})

describe("clampHeight", () => {
  test("keeps a height that fits", () => {
    expect(clampHeight(300, 900)).toBe(300)
  })
  test("never below the least height", () => {
    expect(clampHeight(10, 900)).toBe(MinHeight)
  })
  test("leaves the page room above it", () => {
    expect(clampHeight(5000, 900)).toBe(900 - PageRoom)
  })
  test("a tiny window still gets the least height", () => {
    expect(clampHeight(300, 200)).toBe(MinHeight)
  })
})
