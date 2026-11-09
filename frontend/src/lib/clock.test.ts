import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToString } from "react-dom/server"

import { Ago, Tick } from "@/components/common/ago"

describe("Ago (card #1137)", () => {
  test("prints the age of a time against the clock", () => {
    const at = new Date(Date.now() - 5 * 60_000)
    expect(renderToString(createElement(Ago, { at }))).toBe("5m ago")
  })
  test("a missing time is never", () => {
    expect(renderToString(createElement(Ago, { at: null }))).toBe("never")
  })
  test("Tick prints what its function makes of now", () => {
    const year = String(new Date().getFullYear())
    expect(
      renderToString(
        createElement(Tick, { of: (now: Date) => `y${now.getFullYear()}` })
      )
    ).toBe(`y${year}`)
  })
})
