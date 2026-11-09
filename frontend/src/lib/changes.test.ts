import { describe, expect, test } from "bun:test"

import { createChanges, FocusEveryMs, type Notice } from "./changes"

// Card #1146: the listener module with a fake host, page and clock.
function rig() {
  const heard = new Map<string, (payload: unknown) => void>()
  const listening = new Map<string, number>()
  const visible = new Set<() => void>()
  const focuses = new Set<() => void>()
  const page = {
    hidden: false,
    addEventListener: (_: "visibilitychange", fn: () => void) =>
      visible.add(fn),
    removeEventListener: (_: "visibilitychange", fn: () => void) =>
      visible.delete(fn),
  }
  const clock = { now: 1_000_000 }
  const c = createChanges({
    listen(name, fn) {
      heard.set(name, fn)
      listening.set(name, (listening.get(name) ?? 0) + 1)
      return () => {
        heard.delete(name)
      }
    },
    page: () => page,
    now: () => clock.now,
    onFocus(fn) {
      focuses.add(fn)
      return () => focuses.delete(fn)
    },
  })
  return {
    c,
    clock,
    page,
    listening,
    emit: (name: string, payload: unknown) => heard.get(name)?.(payload),
    live: (name: string) => heard.has(name),
    hide(hidden: boolean) {
      page.hidden = hidden
      for (const fn of [...visible]) fn()
    },
    focus() {
      for (const fn of [...focuses]) fn()
    },
    focusListeners: () => focuses.size,
    visibleListeners: () => visible.size,
  }
}

const board = (seq: number, extra: object = {}) => ({
  seq,
  all: false,
  cards: [],
  notes: [],
  desk: false,
  ...extra,
})

const follow = ["cards", "notes", "desk"]

function collect(r: ReturnType<typeof rig>, opts = {}) {
  const got: Notice[] = []
  const off = r.c.on("board:changed", (n) => got.push(n), {
    follow,
    ...opts,
  })
  return { got, off }
}

describe("changes: the notice a list hears", () => {
  test("the first notice after mount is all, then ids", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(5, { cards: ["a"] }))
    r.emit("board:changed", board(6, { cards: ["b"], notes: ["n"] }))
    expect(got).toEqual([
      { signal: "board:changed", all: true },
      {
        signal: "board:changed",
        all: false,
        ids: { cards: ["b"], notes: ["n"], desk: [] },
      },
    ])
  })

  test("a list hears only the buckets it follows", () => {
    const r = rig()
    const got: Notice[] = []
    r.c.on("board:changed", (n) => got.push(n), { follow: ["cards"] })
    r.emit("board:changed", board(1))
    r.emit(
      "board:changed",
      board(2, { cards: ["a"], notes: ["n"], desk: true })
    )
    expect(got[1]).toEqual({
      signal: "board:changed",
      all: false,
      ids: { cards: ["a"] },
    })
  })

  test("a later list's first notice is all though the signal is live", () => {
    const r = rig()
    const a = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { cards: ["x"] }))
    const b = collect(r)
    r.emit("board:changed", board(3, { cards: ["y"] }))
    expect(a.got[2]).toMatchObject({ all: false })
    expect(b.got).toEqual([{ signal: "board:changed", all: true }])
    // One listener for the window.
    expect(r.listening.get("board:changed")).toBe(1)
  })

  test("a gap gives all", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { cards: ["a"] }))
    r.emit("board:changed", board(4, { cards: ["b"] }))
    r.emit("board:changed", board(5, { cards: ["c"] }))
    expect(got.map((n) => n.all)).toEqual([true, false, true, false])
  })

  test("out of order gives all, and the next in order is not a gap", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { cards: ["a"] }))
    r.emit("board:changed", board(3, { cards: ["a"] }))
    r.emit("board:changed", board(2, { cards: ["a"] }))
    r.emit("board:changed", board(4, { cards: ["a"] }))
    expect(got.map((n) => n.all)).toEqual([true, false, false, true, false])
  })

  test("an event with no seq gives all", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", {})
    expect(got[1]).toEqual({ signal: "board:changed", all: true })
  })

  test("all in the payload gives all", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { all: true, cards: ["a"] }))
    expect(got[1]).toEqual({ signal: "board:changed", all: true })
  })

  test("signals that name no rows are always all", () => {
    const r = rig()
    const got: Notice[] = []
    r.c.on("devices:changed", (n) => got.push(n))
    r.c.on("boards:changed", (n) => got.push(n))
    r.emit("devices:changed", { seq: 1, all: false })
    r.emit("devices:changed", { seq: 2, all: false })
    r.emit("boards:changed", { seq: 1 })
    r.emit("boards:changed", { seq: 2 })
    expect(got.every((n) => n.all)).toBe(true)
    expect(got.length).toBe(4)
  })

  test("rekey makes the next notice all", () => {
    const r = rig()
    const { got, off } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { cards: ["a"] }))
    off.rekey()
    r.emit("board:changed", board(3, { cards: ["a"] }))
    r.emit("board:changed", board(4, { cards: ["a"] }))
    expect(got.map((n) => n.all)).toEqual([true, false, true, false])
  })

  test("the desk flag is the bucket desk", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { desk: true }))
    expect(got[1]).toMatchObject({ ids: { desk: ["desk"] } })
  })

  test("the last listener leaving stops the signal and starts it afresh", () => {
    const r = rig()
    const a = collect(r)
    r.emit("board:changed", board(1))
    a.off()
    expect(r.live("board:changed")).toBe(false)
    expect(r.visibleListeners()).toBe(0)
    const b = collect(r)
    // seq 9 is not a gap against the old life: it is the start.
    r.emit("board:changed", board(9))
    r.emit("board:changed", board(10, { cards: ["a"] }))
    expect(b.got.map((n) => n.all)).toEqual([true, false])
  })
})

describe("changes: a hidden window", () => {
  test("a hidden stretch gives one all, and no gap after", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    r.emit("board:changed", board(2, { cards: ["a"] }))
    got.length = 0
    r.hide(true)
    for (let seq = 3; seq <= 40; seq++)
      r.emit("board:changed", board(seq, { cards: [`c${seq}`] }))
    expect(got).toEqual([])
    r.hide(false)
    expect(got).toEqual([{ signal: "board:changed", all: true }])
    r.emit("board:changed", board(41, { cards: ["z"] }))
    expect(got[1]).toMatchObject({ all: false, ids: { cards: ["z"] } })
  })

  test("a hidden stretch with no notice reads nothing", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    got.length = 0
    r.hide(true)
    r.hide(false)
    expect(got).toEqual([])
  })

  test("a hidden stretch drops a gap's second all", () => {
    const r = rig()
    const { got } = collect(r)
    r.emit("board:changed", board(1))
    got.length = 0
    r.hide(true)
    r.emit("board:changed", board(2))
    r.emit("board:changed", board(5))
    r.hide(false)
    expect(got).toEqual([{ signal: "board:changed", all: true }])
  })

  test("a subscriber that is not held hears notices while hidden", () => {
    const r = rig()
    const { got } = collect(r, { held: false })
    r.emit("board:changed", board(1))
    r.hide(true)
    r.emit("board:changed", board(2, { cards: ["a"] }))
    expect(got.length).toBe(2)
    r.hide(false)
    expect(got.length).toBe(2)
  })

  test("the talk listener is held while hidden", () => {
    const r = rig()
    const got: Notice[] = []
    r.c.on("talk:changed", (n) => got.push(n), { follow: ["convs", "calls"] })
    r.emit("talk:changed", { seq: 1, all: false, convs: [], calls: [] })
    got.length = 0
    r.hide(true)
    r.emit("talk:changed", { seq: 2, all: false, convs: ["c"], calls: [] })
    expect(got).toEqual([])
    r.hide(false)
    expect(got).toEqual([{ signal: "talk:changed", all: true }])
  })

  test("a stopped subscriber is owed nothing", () => {
    const r = rig()
    const a = collect(r)
    const b = collect(r)
    r.emit("board:changed", board(1))
    a.got.length = b.got.length = 0
    r.hide(true)
    r.emit("board:changed", board(2))
    a.off()
    r.hide(false)
    expect(a.got).toEqual([])
    expect(b.got.length).toBe(1)
  })
})

describe("changes: focus", () => {
  test("is a full read at most once a minute", () => {
    const r = rig()
    const { got } = collect(r, { focus: true })
    r.focus()
    expect(got).toEqual([{ signal: "board:changed", all: true }])
    r.clock.now += FocusEveryMs - 1
    r.focus()
    expect(got.length).toBe(1)
    r.clock.now += 1
    r.focus()
    expect(got.length).toBe(2)
  })

  test("only a subscriber that asks hears it, and it stops with it", () => {
    const r = rig()
    const quiet = collect(r)
    expect(r.focusListeners()).toBe(0)
    r.focus()
    expect(quiet.got).toEqual([])
    const loud = collect(r, { focus: true })
    expect(r.focusListeners()).toBe(1)
    loud.off()
    expect(r.focusListeners()).toBe(0)
  })
})
