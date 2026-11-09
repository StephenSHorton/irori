import { describe, expect, test } from "bun:test"

import type { Notice } from "./changes"
import {
  createDriftSchedule,
  DriftEveryMs,
  DriftSlots,
  patchedReads,
  rowsDiffer,
  type Want,
} from "./patched"
import { StuckReadMs } from "./talk"

// Card #1146: the patch hook's core with reads finished by hand. A read is
// a promise the test settles; the list is rows {id, v}.
type Row = { id: string; v: number }
type Read = {
  want: Want
  resolve: (rows: Row[]) => void
  reject: (e: unknown) => void
}

const settled = async () => {
  for (let i = 0; i < 8; i++) await Promise.resolve()
}

function rig(drift = true) {
  let value: Row[] = []
  const reads: Read[] = []
  const errors: unknown[] = []
  const drifts: number[] = []
  const dropped = { n: 0 }
  const clock = { now: 0 }
  const core = patchedReads<Row[], Row[]>({
    name: "test",
    read: (want) =>
      new Promise<Row[]>((resolve, reject) =>
        reads.push({ want, resolve, reject })
      ),
    merge(cur, patch, want) {
      if (want.full) return patch
      const byId = new Map(cur.map((r) => [r.id, r]))
      for (const r of patch) byId.set(r.id, r)
      return [...byId.values()]
    },
    current: () => value,
    set: (next) => {
      value = next
    },
    fail: (e) => errors.push(e),
    rows: drift ? (v) => v.map((r) => [r.id, r]) : undefined,
    noteDrift: (n) => drifts.push(n),
    noteDropped: () => dropped.n++,
    noteRead: () => {},
    now: () => clock.now,
  })
  const notice = (...cards: string[]): Notice => ({
    signal: "board:changed",
    all: false,
    ids: { cards },
  })
  const all: Notice = { signal: "board:changed", all: true }
  return {
    core,
    reads,
    errors,
    drifts,
    dropped,
    clock,
    notice,
    all,
    value: () => value,
    // finish answers read n with rows and lets the queue move on.
    async finish(n: number, rows: Row[]) {
      reads[n].resolve(rows)
      await settled()
    },
    // start puts the list through its first full read.
    async start(rows: Row[] = []) {
      void core.reload()
      await settled()
      await this.finish(reads.length - 1, rows)
    },
  }
}

const cardsOf = (w: Want) => w.ids["board:changed.cards"] ?? []

describe("patched: one stable load", () => {
  test("the first read and a hook's own reload are full reads", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    expect(r.reads[0].want.full).toBe(true)
    void r.core.reload()
    await settled()
    expect(r.reads.length).toBe(2)
    expect(r.reads[1].want.full).toBe(true)
    expect(r.reads[1].want.all).toBe(true)
  })

  test("a notice with ids reads those ids only", async () => {
    const r = rig()
    await r.start([
      { id: "a", v: 1 },
      { id: "b", v: 1 },
    ])
    r.core.notice(r.notice("b"))
    await settled()
    expect(r.reads.length).toBe(2)
    expect(r.reads[1].want.full).toBe(false)
    expect(cardsOf(r.reads[1].want)).toEqual(["b"])
    await r.finish(1, [{ id: "b", v: 2 }])
    expect(r.value()).toEqual([
      { id: "a", v: 1 },
      { id: "b", v: 2 },
    ])
  })

  test("a notice for none of its buckets reads nothing", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.notice())
    await settled()
    expect(r.reads.length).toBe(1)
  })

  test("an all notice reads everything", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.all)
    await settled()
    expect(r.reads[1].want).toMatchObject({ all: true, full: true })
  })

  test("notices during a read go into one follow-up, none beside it", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.notice("a"))
    await settled()
    expect(r.reads.length).toBe(2)
    for (const id of ["b", "c", "b", "d"]) r.core.notice(r.notice(id))
    await settled()
    // Still one read in flight.
    expect(r.reads.length).toBe(2)
    await r.finish(1, [])
    expect(r.reads.length).toBe(3)
    expect(cardsOf(r.reads[2].want).sort()).toEqual(["b", "c", "d"])
    // And a notice during the follow-up waits for it too.
    r.core.notice(r.notice("e"))
    await settled()
    expect(r.reads.length).toBe(3)
    await r.finish(2, [])
    expect(cardsOf(r.reads[3].want)).toEqual(["e"])
    await r.finish(3, [])
    expect(r.reads.length).toBe(4)
  })

  test("an all among ids makes the follow-up one full read", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.notice("a"))
    await settled()
    r.core.notice(r.notice("b"))
    r.core.notice(r.all)
    await r.finish(1, [])
    expect(r.reads.length).toBe(3)
    expect(r.reads[2].want.full).toBe(true)
  })

  test("nothing pending when the follow-up starts reads nothing", async () => {
    const r = rig()
    await r.start()
    void r.core.reload()
    await settled()
    // Two own reloads while one reads: one follow-up, which drains all.
    void r.core.reload()
    void r.core.reload()
    await r.finish(1, [])
    expect(r.reads.length).toBe(3)
    await r.finish(2, [])
    expect(r.reads.length).toBe(3)
  })
})

describe("patched: dropped and errored answers", () => {
  test("a stuck read's late answer is dropped and its ids read again", async () => {
    const r = rig()
    await r.start([
      { id: "a", v: 1 },
      { id: "b", v: 1 },
    ])
    r.core.notice(r.notice("a"))
    await settled()
    expect(r.reads.length).toBe(2)
    // Read 1 sticks past StuckReadMs; the next notice reads beside it.
    r.clock.now += StuckReadMs
    r.core.notice(r.notice("b"))
    await settled()
    expect(r.reads.length).toBe(3)
    expect(cardsOf(r.reads[2].want)).toEqual(["b"])
    await r.finish(2, [{ id: "b", v: 2 }])
    expect(r.reads.length).toBe(3)
    // The stuck read answers late: its answer is not shown, its ids go back.
    await r.finish(1, [{ id: "a", v: 9 }])
    expect(r.value().find((x) => x.id === "a")?.v).toBe(1)
    expect(r.reads.length).toBe(4)
    expect(cardsOf(r.reads[3].want)).toEqual(["a"])
    await r.finish(3, [{ id: "a", v: 2 }])
    expect(r.value()).toEqual([
      { id: "a", v: 2 },
      { id: "b", v: 2 },
    ])
  })

  test("a dropped full read leads to a full read", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.all)
    await settled()
    r.clock.now += StuckReadMs
    r.core.notice(r.notice("x"))
    await settled()
    await r.finish(2, [])
    await r.finish(1, [{ id: "late", v: 1 }])
    expect(r.value()).toEqual([])
    expect(r.reads.length).toBe(4)
    expect(r.reads[3].want.full).toBe(true)
  })

  test("an errored read's ids and all are read by the next", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    r.core.notice(r.notice("a"))
    await settled()
    r.reads[1].reject(new Error("boom"))
    await settled()
    expect(r.errors.length).toBe(1)
    // No read starts by itself; the next notice or the hook's retry does.
    expect(r.reads.length).toBe(2)
    r.core.notice(r.notice("b"))
    await settled()
    expect(r.reads.length).toBe(3)
    expect(r.reads[2].want.full).toBe(true)
    expect(cardsOf(r.reads[2].want).sort()).toEqual(["a", "b"])
    await r.finish(2, [{ id: "a", v: 2 }])
    expect(r.value()).toEqual([{ id: "a", v: 2 }])
  })

  test("after an error the hook's retry (its own reload) is full", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.notice("a"))
    await settled()
    r.reads[1].reject(new Error("boom"))
    await settled()
    void r.core.reload()
    await settled()
    expect(r.reads[2].want.full).toBe(true)
  })

  test("an errored stale read is dropped and read again", async () => {
    const r = rig()
    await r.start()
    r.core.notice(r.notice("a"))
    await settled()
    r.clock.now += StuckReadMs
    r.core.notice(r.notice("b"))
    await settled()
    await r.finish(2, [{ id: "b", v: 1 }])
    r.reads[1].reject(new Error("late failure"))
    await settled()
    // Its answer isn't shown (no error on screen), and its ids are read.
    expect(r.errors.length).toBe(0)
    expect(r.reads.length).toBe(4)
    expect(r.reads[3].want.full).toBe(true)
    expect(cardsOf(r.reads[3].want)).toEqual(["a"])
  })
})

describe("patched: the drift check", () => {
  test("does nothing until a list has patched", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    r.core.driftCheck()
    await settled()
    expect(r.reads.length).toBe(1)
    const none = rig(false)
    await none.start()
    none.core.notice(none.notice("a"))
    await settled()
    await none.finish(1, [])
    none.core.driftCheck()
    await settled()
    // A list with no rows() has no check.
    expect(none.reads.length).toBe(2)
  })

  test("a full read through the same queue, compared and then shown", async () => {
    const r = rig()
    await r.start([
      { id: "a", v: 1 },
      { id: "b", v: 1 },
    ])
    r.core.notice(r.notice("a"))
    await settled()
    await r.finish(1, [{ id: "a", v: 2 }])
    r.core.driftCheck()
    await settled()
    expect(r.reads.length).toBe(3)
    expect(r.reads[2].want).toMatchObject({
      full: true,
      drift: true,
      all: false,
    })
    // The host has b changed, which no patch brought.
    await r.finish(2, [
      { id: "a", v: 2 },
      { id: "b", v: 5 },
    ])
    expect(r.drifts).toEqual([1])
    expect(r.dropped.n).toBe(0)
    expect(r.value()).toEqual([
      { id: "a", v: 2 },
      { id: "b", v: 5 },
    ])
    // Replaced, so a second check waits for another patch.
    r.core.driftCheck()
    await settled()
    expect(r.reads.length).toBe(3)
  })

  test("an agreeing sample logs drift 0", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    r.core.notice(r.notice("a"))
    await settled()
    await r.finish(1, [{ id: "a", v: 1 }])
    r.core.driftCheck()
    await settled()
    await r.finish(2, [{ id: "a", v: 1 }])
    expect(r.drifts).toEqual([0])
  })

  test("a sample with a notice during it is dropped, not compared", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    r.core.notice(r.notice("a"))
    await settled()
    await r.finish(1, [{ id: "a", v: 1 }])
    r.core.driftCheck()
    await settled()
    // The notice that changes a comes while the sample reads.
    r.core.notice(r.notice("a"))
    await r.finish(2, [{ id: "a", v: 1 }])
    expect(r.drifts).toEqual([])
    expect(r.dropped.n).toBe(1)
    // Its notice is read next, as a patch.
    expect(r.reads.length).toBe(4)
    expect(r.reads[3].want.full).toBe(false)
    expect(cardsOf(r.reads[3].want)).toEqual(["a"])
  })

  test("a notice for another list during a sample does not drop it", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    r.core.notice(r.notice("a"))
    await settled()
    await r.finish(1, [{ id: "a", v: 1 }])
    r.core.driftCheck()
    await settled()
    r.core.notice(r.notice())
    await r.finish(2, [{ id: "a", v: 1 }])
    expect(r.drifts).toEqual([0])
    expect(r.dropped.n).toBe(0)
  })

  test("a sample dropped as stale is counted dropped, not compared", async () => {
    const r = rig()
    await r.start([{ id: "a", v: 1 }])
    r.core.notice(r.notice("a"))
    await settled()
    await r.finish(1, [{ id: "a", v: 1 }])
    r.core.driftCheck()
    await settled()
    r.clock.now += StuckReadMs
    r.core.notice(r.notice("a"))
    await settled()
    await r.finish(3, [{ id: "a", v: 2 }])
    await r.finish(2, [{ id: "a", v: 1 }])
    expect(r.drifts).toEqual([])
    expect(r.dropped.n).toBe(1)
  })
})

describe("rowsDiffer", () => {
  const rows = (...ids: string[]): Array<[string, unknown]> =>
    ids.map((id) => [id, { id }])
  test("counts missing, extra, changed and a different order", () => {
    expect(rowsDiffer(rows("a", "b"), rows("a", "b"))).toBe(0)
    expect(rowsDiffer(rows("a", "b"), rows("a"))).toBe(1)
    expect(rowsDiffer(rows("a"), rows("a", "b"))).toBe(1)
    expect(rowsDiffer(rows("a", "b"), rows("b", "a"))).toBe(1)
    expect(rowsDiffer([["a", 1]], [["a", 2]])).toBe(1)
    expect(rowsDiffer(rows("a", "b", "c"), rows("c", "x"))).toBe(3)
  })
})

describe("the drift schedule", () => {
  function fakeTimers() {
    const timers = new Map<number, { at: number; fn: () => void }>()
    let next = 1
    const t = { now: 0, hidden: false }
    return {
      t,
      deps: {
        setTimeout(fn: () => void, ms: number) {
          timers.set(next, { at: t.now + ms, fn })
          return next++
        },
        clearTimeout(id: unknown) {
          timers.delete(id as number)
        },
        hidden: () => t.hidden,
      },
      advance(ms: number) {
        const end = t.now + ms
        for (;;) {
          const due = [...timers].filter(([, x]) => x.at <= end)
          if (!due.length) break
          due.sort((a, b) => a[1].at - b[1].at)
          const [id, x] = due[0]
          timers.delete(id)
          t.now = x.at
          x.fn()
        }
        t.now = end
      },
    }
  }

  test("lists are staggered over the period and repeat", () => {
    const f = fakeTimers()
    const s = createDriftSchedule(f.deps)
    const ran: number[] = []
    for (let i = 0; i < 3; i++) s.register(() => ran.push(i))
    f.advance(DriftEveryMs)
    // Each ran once within the first period, at different times.
    expect([...ran].sort()).toEqual([0, 1, 2])
    f.advance(DriftEveryMs)
    expect(ran.length).toBe(6)
    expect(DriftSlots).toBeGreaterThan(1)
  })

  test("a hidden window skips the sample, and a stop ends it", () => {
    const f = fakeTimers()
    const s = createDriftSchedule(f.deps)
    let n = 0
    const stop = s.register(() => n++)
    f.t.hidden = true
    f.advance(DriftEveryMs)
    expect(n).toBe(0)
    f.t.hidden = false
    f.advance(DriftEveryMs)
    expect(n).toBe(1)
    stop()
    f.advance(DriftEveryMs * 3)
    expect(n).toBe(1)
  })
})
