import { describe, expect, test } from "bun:test"

import { createChanges, type Notice } from "./changes"
import {
  cards,
  RefRefreshMs,
  refEntry,
  refNotice,
  resetRefs,
  sampleCardRef,
} from "./cards"
import {
  createLiveCards,
  createStore,
  deskWakes,
  deviceStateSig,
  doneWakes,
  Follows,
  heldUpsert,
  notesWake,
  refsToDrop,
  rosterWatch,
  trailing,
  upsertRows,
  type RefEntry,
  type Timers,
} from "./small-lists"
import { mergeHeld, mergeInbox, readDeliveries, State } from "./talk"
import type { Delivery } from "./models"

// Card #1152 (docs/design/list-signals.md §6.6): each small list's trigger.

// The sample runs outside the host; bun has no window.
const g = globalThis as unknown as { window?: unknown }
g.window ??= { location: { search: "" } }

// A fake host + page like changes.test.ts, to carry a payload to the list's
// own `follow`.
function rig() {
  const heard = new Map<string, (payload: unknown) => void>()
  const c = createChanges({
    listen(name, fn) {
      heard.set(name, fn)
      return () => heard.delete(name)
    },
    page: () => ({
      hidden: false,
      addEventListener() {},
      removeEventListener() {},
    }),
    now: () => 1,
    onFocus: () => () => {},
  })
  return {
    c,
    emit: (name: string, payload: unknown) => heard.get(name)?.(payload),
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

// ask subscribes like a list does, past the first (all) notice, and says
// whether `wakes` held for the next payload.
function through(
  follow: readonly string[],
  wakes: (n: Notice) => boolean,
  payload: object
): boolean {
  const r = rig()
  let woke = false
  r.c.on(
    "board:changed",
    (n) => {
      woke = wakes(n)
    },
    { follow }
  )
  r.emit("board:changed", board(1)) // the first notice is `all`
  expect(woke).toBe(true)
  woke = false
  r.emit("board:changed", board(2, payload))
  return woke
}

describe("the desk (card #1152)", () => {
  const verdicts = new Set(["card-pr-9"])
  const wakes = (payload: object) =>
    through(Follows.desk, (n) => deskWakes(n, verdicts), payload)

  test("reads on a desk row, a verdict card, and all", () => {
    expect(wakes({ desk: true })).toBe(true)
    expect(wakes({ cards: ["x", "card-pr-9"] })).toBe(true)
    expect(wakes({ all: true })).toBe(true)
  })
  test("a change to any other card or a note reads nothing", () => {
    expect(wakes({ cards: ["x", "y"] })).toBe(false)
    expect(wakes({ notes: ["n1"] })).toBe(false)
  })
})

describe("notes (card #1152)", () => {
  const named = new Set(["card-src"])
  const wakes = (payload: object) =>
    through(Follows.notes, (n) => notesWake(n, named), payload)

  test("reads on a note id, a card a note names, and all", () => {
    expect(wakes({ notes: ["n1"] })).toBe(true)
    expect(wakes({ cards: ["card-src"] })).toBe(true)
    expect(wakes({ all: true })).toBe(true)
  })
  test("another card's change reads nothing", () => {
    expect(wakes({ cards: ["other"] })).toBe(false)
    expect(wakes({ desk: true })).toBe(false)
  })
})

function clock(): Timers & { advance(ms: number): void } {
  let t = 0
  let pending: { at: number; fn: () => void; id: number } | null = null
  let n = 0
  return {
    now: () => t,
    setTimeout(fn, ms) {
      pending = { at: t + ms, fn, id: ++n }
      return pending.id
    },
    clearTimeout(id) {
      if (pending?.id === id) pending = null
    },
    advance(ms) {
      t += ms
      if (pending && pending.at <= t) {
        const p = pending
        pending = null
        p.fn()
      }
    },
  }
}

describe("the idle line (card #1152)", () => {
  test("reads at most once in 5 s, the rest as one trailing read", () => {
    const k = clock()
    let reads = 0
    const slow = trailing(() => reads++, 5000, k)
    slow.call() // the first runs at once
    expect(reads).toBe(1)
    for (let i = 0; i < 20; i++) {
      k.advance(100)
      slow.call()
    }
    expect(reads).toBe(1)
    k.advance(2999)
    expect(reads).toBe(1)
    k.advance(1)
    expect(reads).toBe(2)
    // Quiet, then a change: at once again.
    k.advance(6000)
    slow.call()
    expect(reads).toBe(3)
  })
  test("cancel drops the trailing read", () => {
    const k = clock()
    let reads = 0
    const slow = trailing(() => reads++, 5000, k)
    slow.call()
    slow.call()
    slow.cancel()
    k.advance(6000)
    expect(reads).toBe(1)
  })
})

describe("#ref chips (card #1152)", () => {
  const e = (id: string, settled = true): RefEntry => ({ id, settled })
  const notice = (ids: string[]): Notice => ({
    signal: "board:changed",
    all: false,
    ids: { cards: ids },
  })
  const entries = new Map<string, RefEntry>([
    ["#1", e("c1")],
    ["#2", e("c2")],
    ["#9", e("")], // unknown ref
    ["#3", e("", false)], // still pending
  ])

  test("drops the entry of a named card, every null and pending one", () => {
    expect(refsToDrop(notice(["c1"]), entries).sort()).toEqual([
      "#1",
      "#3",
      "#9",
    ])
  })
  test("a notice with no cards drops nothing", () => {
    expect(refsToDrop(notice([]), entries)).toEqual([])
    expect(
      refsToDrop(
        { signal: "board:changed", all: false, ids: { cards: [] } },
        entries
      )
    ).toEqual([])
  })
  test("all clears everything", () => {
    expect(
      refsToDrop({ signal: "board:changed", all: true }, entries).sort()
    ).toEqual(["#1", "#2", "#3", "#9"])
  })

  test("a chip for an unknown ref shows the card after the notice that creates it", async () => {
    resetRefs()
    // The sample numbers new cards; the chip asks for the next one.
    const shortOf = (id: string) => {
      for (let n = 1; n < 500; n++)
        if (sampleCardRef(`#${n}`)?.id === id) return n
      throw new Error("no such sample card")
    }
    const first = await cards.create({ title: "first ref" } as never)
    const next = `#${shortOf(first.task.id) + 1}`
    expect(sampleCardRef(next)).toBeNull()
    const asked = refEntry(next)
    expect(await asked.v).toBeNull()
    const settled = refEntry("#5")
    await settled.v

    // The sample's ids are made from the clock.
    await new Promise((r) => setTimeout(r, 5))
    const made = await cards.create({
      title: "the card behind the chip",
    } as never)
    expect(`#${shortOf(made.task.id)}`).toBe(next)
    // Another card changing leaves the settled chip's answer alone and
    // drops the null one; nothing is dropped before the burst wait.
    refNotice(notice([made.task.id]))
    expect(refEntry(next)).toBe(asked)
    await new Promise((r) => setTimeout(r, RefRefreshMs + 100))
    const again = refEntry(next)
    expect(again).not.toBe(asked)
    expect((await again.v)?.id).toBe(made.task.id)
    // #5 resolved to a card the notice didn't name: its answer stands.
    expect(refEntry("#5")).toBe(settled)
  })
})

describe("the Done tab (card #1152)", () => {
  const notice = (ids: string[]): Notice => ({
    signal: "board:changed",
    all: false,
    ids: { cards: ids },
  })

  test("reads for a card on its page, and for all", () => {
    const live = createLiveCards()
    live.note(["a", "b"])
    const page = new Set(["d1", "d2"])
    expect(doneWakes(notice(["d2"]), page, live)).toBe(true)
    expect(doneWakes({ signal: "board:changed", all: true }, page, live)).toBe(
      true
    )
  })
  test("a change to a live card reads nothing", () => {
    const live = createLiveCards()
    live.note(["a", "b"])
    expect(doneWakes(notice(["a"]), new Set(["d1"]), live)).toBe(false)
  })
  test("a card that leaves the grid is told to the tab", () => {
    const live = createLiveCards()
    const left: string[][] = []
    live.onLeft((ids) => left.push(ids))
    live.note(["a", "b", "c"])
    expect(left).toEqual([]) // first sight
    live.note(["a", "c"])
    expect(left).toEqual([["b"]])
    live.note(["a", "c", "n"]) // a new card isn't a leaver
    expect(left).toEqual([["b"]])
  })
  test("before the grid has answered, any named card reads", () => {
    const live = createLiveCards()
    expect(doneWakes(notice(["x"]), new Set(), live)).toBe(true)
    expect(doneWakes(notice([]), new Set(), live)).toBe(false)
  })
})

describe("the devices store (card #1152)", () => {
  const roster = (...a: Array<[string, string]>) => ({
    change: {},
    agents: a.map(([deviceId, deviceState]) => ({ deviceId, deviceState })),
  })

  test("a roster change that leaves device state alone is ignored", () => {
    const changed = rosterWatch()
    expect(changed(roster(["pc", "connected"], ["mac", "this"]))).toBe(true) // first
    expect(changed(roster(["mac", "this"], ["pc", "connected"]))).toBe(false)
    expect(
      changed(roster(["pc", "connected"], ["pc", "connected"], ["mac", "this"]))
    ).toBe(false)
    expect(changed(roster(["pc", "offline"], ["mac", "this"]))).toBe(true)
    expect(changed(roster(["mac", "this"]))).toBe(true) // the pc's agents left
  })
  test("a payload it can't read counts as a change", () => {
    expect(deviceStateSig(null)).toBe("?")
    const changed = rosterWatch()
    changed(roster(["a", "x"]))
    expect(changed({})).toBe(true)
    expect(changed({})).toBe(true)
  })

  test("one store: later readers read nothing, the last stops it", async () => {
    let reads = 0
    let started = 0
    let stopped = 0
    let wake = () => {}
    const store = createStore<number[]>({
      name: "t",
      initial: [],
      read: async () => [++reads],
      start(reload) {
        started++
        wake = reload
        return () => stopped++
      },
      noteRead: () => {},
    })
    const seen: number[][] = []
    const offs = [1, 2, 3].map(() =>
      store.subscribe(() => seen.push(store.snapshot().value))
    )
    await store.reload()
    expect(reads).toBe(2) // the first reader's, and this explicit one
    expect(started).toBe(1)
    wake()
    await store.reload()
    expect(reads).toBeGreaterThanOrEqual(3)
    expect(store.snapshot().loading).toBe(false)
    for (const off of offs) off()
    expect(stopped).toBe(1)
    expect(store.readers()).toBe(0)
  })
})

describe("inbox and held upsert by delivery (card #1152)", () => {
  const d = (id: string, at: number, state = State.Queued): Delivery =>
    ({
      id,
      state,
      createdAt: new Date(at).toISOString(),
    }) as unknown as Delivery
  const newest = (a: Delivery, b: Delivery) =>
    Date.parse(b.createdAt) - Date.parse(a.createdAt)

  test("a named delivery replaces its row, or joins at its place", () => {
    const cur = [d("c", 3), d("b", 2), d("a", 1)]
    const next = upsertRows(
      cur,
      ["b", "n"],
      new Map([
        ["b", d("b", 2, State.Delivered)],
        ["n", d("n", 4)],
      ]),
      newest,
      200
    )
    expect(next.map((x) => x.id)).toEqual(["n", "c", "b", "a"])
    expect(next[2].state).toBe(State.Delivered)
  })
  test("rows past the limit go, but held ones stay", () => {
    const cur = [d("c", 3), d("b", 2), d("a", 1, State.Held)]
    const next = upsertRows(
      cur,
      ["n"],
      new Map([["n", d("n", 4)]]),
      newest,
      2,
      (x) => x.state === State.Held
    )
    expect(next.map((x) => x.id)).toEqual(["n", "c", "a"])
  })
  test("mergeInbox: a full read replaces, a patch upserts", () => {
    const cur = { deliveries: [d("a", 1)], policy: null }
    const full = { deliveries: [d("z", 9)], policy: null }
    expect(mergeInbox(cur, { kind: "full", value: full }, [])).toBe(full)
    const got = new Map([["b", d("b", 2)]])
    expect(
      mergeInbox(cur, { kind: "rows", got }, ["b"]).deliveries.map((x) => x.id)
    ).toEqual(["b", "a"])
  })
  test("held drops a row no longer held, or whose read failed", () => {
    const cur = [
      d("c", 3, State.Held),
      d("b", 2, State.Held),
      d("a", 1, State.Held),
    ]
    const next = heldUpsert(
      cur,
      ["c", "b", "n"],
      new Map<string, Delivery | null>([
        ["c", d("c", 3, State.Delivered)], // released
        ["b", null], // Get failed
        ["n", d("n", 4, State.Held)], // newly held
      ]),
      (x) => x.state === State.Held,
      newest
    )
    expect(next.map((x) => x.id)).toEqual(["n", "a"])
    const held = mergeHeld(
      { held: cur, history: [] },
      { kind: "rows", got: new Map([["a", null]]) },
      ["a"]
    )
    expect(held.held.map((x) => x.id)).toEqual(["c", "b"])
  })
  test("readDeliveries is null when one can't be read (a full read follows)", async () => {
    const got = await readDeliveries(["a", "b"], async (id) => d(id, 1))
    expect([...got!.keys()]).toEqual(["a", "b"])
    expect(
      await readDeliveries(["a", "b"], async (id) => {
        if (id === "b") throw new Error("gone")
        return d(id, 1)
      })
    ).toBeNull()
  })

  test("the inbox follows inbox:changed.deliveries; a policy change is all", () => {
    const r = rig()
    const got: Notice[] = []
    r.c.on("inbox:changed", (n) => got.push(n), { follow: Follows.deliveries })
    const counts = { unread: 0, held: 0, paused: 0 }
    const sig = (seq: number, extra: object) => ({
      ...counts,
      seq,
      all: false,
      deliveries: [],
      cards: [],
      ...extra,
    })
    r.emit("inbox:changed", sig(1, {}))
    r.emit("inbox:changed", sig(2, { deliveries: ["d1"] }))
    r.emit("inbox:changed", sig(3, { all: true }))
    expect(got[1]).toEqual({
      signal: "inbox:changed",
      all: false,
      ids: { deliveries: ["d1"] },
    })
    expect(got[2]).toEqual({ signal: "inbox:changed", all: true })
  })
})
