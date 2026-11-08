import { describe, expect, test } from "bun:test"

import { whileShown } from "./shown"

function fakeDoc() {
  const fns = new Set<() => void>()
  const d = {
    hidden: false,
    addEventListener: (_: "visibilitychange", fn: () => void) => fns.add(fn),
    removeEventListener: (_: "visibilitychange", fn: () => void) =>
      fns.delete(fn),
    set(hidden: boolean) {
      d.hidden = hidden
      for (const fn of [...fns]) fn()
    },
    listeners: () => fns.size,
  }
  return d
}

describe("whileShown", () => {
  test("runs at once while shown", () => {
    const doc = fakeDoc()
    let n = 0
    const h = whileShown(() => n++, doc)
    h.run()
    h.run()
    expect(n).toBe(2)
  })

  test("holds every signal while hidden and runs once when shown", () => {
    const doc = fakeDoc()
    let n = 0
    const h = whileShown(() => n++, doc)
    doc.set(true)
    for (let i = 0; i < 50; i++) h.run()
    expect(n).toBe(0)
    doc.set(false)
    expect(n).toBe(1)
    // Showing again with nothing owed runs nothing.
    doc.set(true)
    doc.set(false)
    expect(n).toBe(1)
  })

  test("stop drops a held call and its listener", () => {
    const doc = fakeDoc()
    let n = 0
    const h = whileShown(() => n++, doc)
    doc.set(true)
    h.run()
    h.stop()
    doc.set(false)
    expect(n).toBe(0)
    expect(doc.listeners()).toBe(0)
  })

  test("with no document it is the plain function", () => {
    let n = 0
    const h = whileShown(() => n++, null)
    h.run()
    expect(n).toBe(1)
  })
})
