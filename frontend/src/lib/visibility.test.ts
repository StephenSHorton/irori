import { describe, expect, test } from "bun:test"

import { whileShown } from "./shown"
import { combinedPage, type WindowChange, type WindowState } from "./visibility"

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
  }
  return d
}

function fakeWindow() {
  let fire: (change: WindowChange) => void = () => {}
  const w = {
    minimised: false,
    visible: true,
    fail: false,
    state: {
      hidden: async () => {
        if (w.fail) throw new Error("no window")
        return w.minimised || !w.visible
      },
      onChange: (fn: (change: WindowChange) => void) => {
        fire = fn
        return () => {}
      },
    } satisfies WindowState,
    event: (change: WindowChange) => fire(change),
  }
  return w
}

function fakeTimers() {
  const live = new Map<number, () => void>()
  let next = 1
  return {
    set: (fn: () => void) => {
      live.set(next, fn)
      return next++
    },
    clear: (id: unknown) => live.delete(id as number),
    tick: () => [...live.values()].forEach((fn) => fn()),
    count: () => live.size,
  }
}

const settle = () => new Promise((r) => setTimeout(r, 0))

describe("combinedPage", () => {
  test("a minimised window counts as hidden though the page says visible", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const page = combinedPage(doc, win.state, fakeTimers())
    await settle()
    expect(page.hidden).toBe(false)
    win.minimised = true
    win.event("resize")
    await settle()
    expect(page.hidden).toBe(true)
    expect(doc.hidden).toBe(false)
  })

  test("a window closed to the tray counts as hidden, and showing it tells listeners", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const page = combinedPage(doc, win.state, fakeTimers())
    await settle()
    let changes = 0
    page.addEventListener("visibilitychange", () => changes++)
    win.visible = false
    win.event("blur")
    await settle()
    expect(page.hidden).toBe(true)
    win.visible = true
    win.event("focus")
    await settle()
    expect(page.hidden).toBe(false)
    expect(changes).toBe(2)
  })

  test("an unfocused window re-checks on a timer until it is focused", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const timers = fakeTimers()
    const page = combinedPage(doc, win.state, timers)
    await settle()
    win.event("blur")
    expect(timers.count()).toBe(1)
    // Hidden with no event at all: the timer finds it.
    win.visible = false
    timers.tick()
    await settle()
    expect(page.hidden).toBe(true)
    win.visible = true
    win.event("focus")
    expect(timers.count()).toBe(0)
  })

  test("the page's own hidden still counts", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const page = combinedPage(doc, win.state, fakeTimers())
    await settle()
    doc.set(true)
    expect(page.hidden).toBe(true)
  })

  test("a window that can't be asked counts as shown", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    win.fail = true
    const page = combinedPage(doc, win.state, fakeTimers())
    await settle()
    expect(page.hidden).toBe(false)
  })

  test("whileShown holds reloads while the window is in the tray and runs one on show", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const page = combinedPage(doc, win.state, fakeTimers())
    await settle()
    let reads = 0
    const h = whileShown(() => reads++, page)
    win.visible = false
    win.event("blur")
    await settle()
    h.run()
    h.run()
    h.run()
    expect(reads).toBe(0)
    win.visible = true
    win.event("focus")
    await settle()
    expect(reads).toBe(1)
  })

  test("closing to the tray counts as hidden at once, before the window has hidden", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const timers = fakeTimers()
    const page = combinedPage(doc, win.state, timers)
    await settle()
    win.event("close") // the shell hides it right after; still visible now
    expect(page.hidden).toBe(true)
    win.visible = false
    timers.tick()
    await settle()
    expect(page.hidden).toBe(true)
  })

  test("a show from the tray that gets no focus is found by the timer", async () => {
    const doc = fakeDoc()
    const win = fakeWindow()
    const timers = fakeTimers()
    const page = combinedPage(doc, win.state, timers)
    await settle()
    let reads = 0
    const h = whileShown(() => reads++, page)
    win.event("close")
    win.visible = false
    h.run()
    expect(reads).toBe(0)
    win.visible = true // shown, but Windows gave it no focus: no event
    timers.tick()
    await settle()
    expect(page.hidden).toBe(false)
    expect(reads).toBe(1)
  })
})
