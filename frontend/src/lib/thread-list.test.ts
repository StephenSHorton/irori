import { describe, expect, test } from "bun:test"

import type { ThreadSummary } from "./models"
import type { Want } from "./patched"
import {
  byNewest,
  mergeThreads,
  readThreads,
  ResolvedShown,
  threadFollow,
  type ReadDeps,
  type ThreadList,
} from "./thread-list"

// Card #1149: the thread lists read by id.
const row = (id: string, o: Partial<ThreadSummary> = {}): ThreadSummary =>
  ({
    id,
    state: "waiting",
    updatedAt: "2026-11-09T10:00:00Z",
    unread: false,
    private: false,
    ...o,
  }) as ThreadSummary

const want = (ids: Record<string, string[]>, full = false): Want => ({
  all: full,
  drift: false,
  full,
  ids,
})
const cards = (...ids: string[]) => want({ "board:changed.cards": ids })
const convs = (...ids: string[]) => want({ "talk:changed.convs": ids })

const deps = (o: {
  list?: ThreadSummary[]
  byId?: { rows: ThreadSummary[]; gone: string[] }
  current?: ThreadSummary[]
}): ReadDeps & { calls: string[] } => {
  const calls: string[] = []
  return {
    calls,
    startedSeq: () => 3,
    full: async () => {
      calls.push("full")
      return o.list ?? []
    },
    byId: async (ids) => {
      calls.push(`byId:${ids.join(",")}`)
      return o.byId ?? { rows: [], gone: [] }
    },
    current: () => o.current ?? [],
  }
}

describe("threadFollow", () => {
  test("no list wakes on inbox:changed; talk's read cursors are in", () => {
    const signals = threadFollow.map((f) => f.signal)
    expect(signals).not.toContain("inbox:changed")
    expect(signals).toContain("talk:changed")
    expect(signals).toContain("devices:changed")
    expect(
      threadFollow.find((f) => f.signal === "talk:changed")?.buckets
    ).toEqual(["convs"])
  })
})

describe("order and cap", () => {
  test("open first, newest first, equal times by id, to the nanosecond", () => {
    const rows = [
      row("b", { updatedAt: "2026-11-09T10:00:00.000000002Z" }),
      row("a", { updatedAt: "2026-11-09T10:00:00.000000002Z" }),
      row("c", { updatedAt: "2026-11-09T10:00:00.000000003Z" }),
      row("d", { state: "resolved", updatedAt: "2026-11-09T11:00:00Z" }),
      row("e", { updatedAt: "2026-11-09T09:00:00Z" }),
    ]
    expect(rows.sort(byNewest).map((r) => r.id)).toEqual([
      "c",
      "a",
      "b",
      "e",
      "d",
    ])
  })

  test("a patch keeps the newest 20 resolved per bucket", () => {
    const old = Array.from({ length: ResolvedShown }, (_, i) =>
      row(`r${i}`, {
        state: "resolved",
        updatedAt: `2026-11-08T10:00:${String(i).padStart(2, "0")}Z`,
      })
    )
    const left = row("left", {
      state: "resolved",
      private: true,
      updatedAt: "2026-11-01T10:00:00Z",
    })
    const cur: ThreadList = { rows: [...old.reverse(), left], asOf: 1 }
    const newer = row("new", {
      state: "resolved",
      updatedAt: "2026-11-09T10:00:00Z",
    })
    const out = mergeThreads(
      cur,
      { full: false, rows: [newer], gone: [], asOf: 5 },
      null
    )
    const ids = out.rows.map((r) => r.id)
    expect(ids[0]).toBe("new")
    expect(ids).not.toContain("r0")
    // The left-space bucket is its own: its one resolved thread stays.
    expect(ids).toContain("left")
    expect(out.rows.length).toBe(ResolvedShown + 1)
    // Only a full read moves asOf.
    expect(out.asOf).toBe(1)
  })
})

describe("merge", () => {
  test("upserts rows and drops gone", () => {
    const cur: ThreadList = { rows: [row("a"), row("b")], asOf: 0 }
    const out = mergeThreads(
      cur,
      {
        full: false,
        rows: [row("a", { title: "new" } as Partial<ThreadSummary>), row("c")],
        gone: ["b"],
        asOf: 0,
      },
      null
    )
    expect(out.rows.map((r) => r.id).sort()).toEqual(["a", "c"])
    expect(
      (out.rows.find((r) => r.id === "a") as { title?: string }).title
    ).toBe("new")
  })

  test("a board's list takes only its bucket", () => {
    const cur: ThreadList = { rows: [row("a", { spaceId: "x" })], asOf: 0 }
    const out = mergeThreads(
      cur,
      {
        full: false,
        rows: [row("b", { spaceId: "y" }), row("a", { spaceId: "y" })],
        gone: [],
        asOf: 0,
      },
      "x"
    )
    expect(out.rows).toEqual([])
  })
})

describe("reads", () => {
  test("a card change reads just its ids; a full read reads the list", async () => {
    const d = deps({ byId: { rows: [row("a")], gone: ["nope"] } })
    const p = await readThreads(cards("a", "nope"), d)
    expect(p).toMatchObject({ full: false, gone: ["nope"], asOf: 3 })
    expect(d.calls).toEqual(["byId:a,nope"])
    const f = deps({ list: [row("z")] })
    expect(await readThreads(want({}, true), f)).toMatchObject({ full: true })
    expect(f.calls).toEqual(["full"])
  })

  test("marking a thread read patches its unread", async () => {
    const cur = [row("a", { unread: true })]
    const d = deps({
      current: cur,
      byId: { rows: [row("a", { unread: false })], gone: [] },
    })
    const p = await readThreads(convs("a"), d)
    expect(d.calls).toEqual(["byId:a"])
    const out = mergeThreads({ rows: cur, asOf: 0 }, p, null)
    expect(out.rows[0].unread).toBe(false)
  })

  test("a reopened (or withdrawn) resolved thread reads the whole list", async () => {
    const resolved = [row("a", { state: "resolved" })]
    const reopened = deps({
      current: resolved,
      list: [row("a"), row("z")],
      byId: { rows: [row("a", { state: "waiting" })], gone: [] },
    })
    expect(await readThreads(cards("a"), reopened)).toMatchObject({
      full: true,
    })
    expect(reopened.calls).toEqual(["byId:a", "full"])
    const withdrawn = deps({
      current: resolved,
      byId: { rows: [], gone: ["a"] },
    })
    expect(await readThreads(cards("a"), withdrawn)).toMatchObject({
      full: true,
    })
  })

  test("a notice naming no thread bucket reads nothing", async () => {
    const d = deps({})
    const p = await readThreads(want({ "board:changed.notes": ["n"] }), d)
    expect(p).toMatchObject({ full: false, rows: [], gone: [] })
    expect(d.calls).toEqual([])
  })
})
