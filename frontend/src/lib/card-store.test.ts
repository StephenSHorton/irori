import { describe, expect, test } from "bun:test"

import { createCardStore, type CardSlot } from "./cards"
import type { CardDetail } from "./models"

// Card #468: every useCard shares one read per change.

const tick = () => new Promise((r) => setTimeout(r, 0))

function rig() {
  const reads: string[][] = []
  let version = 1
  const gone = new Set<string>()
  let fire: (ids?: readonly string[]) => void = () => {}
  let listening = 0
  const store = createCardStore(
    async (ids) => {
      reads.push(ids)
      return new Map<string, CardSlot>(
        ids.map((id) => [
          id,
          gone.has(id)
            ? { detail: null, error: `no such card ${id}` }
            : {
                detail: { id, title: `v${version}` } as unknown as CardDetail,
                error: null,
              },
        ])
      )
    },
    (again) => {
      listening++
      fire = again
      return () => {
        listening--
      }
    },
    20
  )
  return {
    store,
    reads,
    gone,
    bump: () => version++,
    change: () => fire(),
    // A notice naming these cards.
    notice: (ids: readonly string[]) => fire(ids),
    listening: () => listening,
  }
}

describe("cardStore", () => {
  test("rows mounted together load in one read; a change reads them all once", async () => {
    const r = rig()
    let renders = 0
    const offs = ["a", "b", "c"].map((id) => r.store.watch(id, () => renders++))
    await tick()
    expect(r.reads).toEqual([["a", "b", "c"]])
    expect(r.store.slot("b")?.detail?.id).toBe("b")
    expect(renders).toBe(3)

    // Unchanged cards keep their slots: nothing renders.
    const before = r.store.slot("a")
    r.change()
    await tick()
    expect(r.reads.length).toBe(2)
    expect(r.store.slot("a")).toBe(before)
    expect(renders).toBe(3)

    // A changed card is a new slot.
    r.bump()
    r.change()
    await tick()
    expect(r.store.slot("a")).not.toBe(before)
    expect(renders).toBe(6)
    offs.forEach((off) => off())
  })

  test("two rows on one card share its slot and its read", async () => {
    const r = rig()
    const offs = [r.store.watch("a", () => {}), r.store.watch("a", () => {})]
    await tick()
    expect(r.reads).toEqual([["a"]])
    offs.forEach((off) => off())
  })

  test("a gone card has its error; the others still load", async () => {
    const r = rig()
    r.gone.add("x")
    const offs = [r.store.watch("a", () => {}), r.store.watch("x", () => {})]
    await tick()
    expect(r.store.slot("x")).toEqual({ detail: null, error: "no such card x" })
    expect(r.store.slot("a")?.error).toBeNull()
    offs.forEach((off) => off())
  })

  test("the last row gone stops listening and forgets its cards", async () => {
    const r = rig()
    const off = r.store.watch("a", () => {})
    expect(r.listening()).toBe(1)
    await tick()
    off()
    expect(r.listening()).toBe(0)
    expect(r.store.slot("a")).toBeUndefined()
    expect(r.store.watch("", () => {})).toBeInstanceOf(Function)
    expect(r.listening()).toBe(0)
  })

  test("a change during a read reads again after it", async () => {
    const r = rig()
    const off = r.store.watch("a", () => {})
    await Promise.resolve()
    // The first read has started; a change now waits for it.
    r.bump()
    r.change()
    await tick()
    await tick()
    expect(r.reads).toEqual([["a"], ["a"]])
    expect(
      (r.store.slot("a")?.detail as unknown as { title: string }).title
    ).toBe("v2")
    off()
  })

  test("a failed read keeps the card shown, with the error", async () => {
    const r = rig()
    const off = r.store.watch("a", () => {})
    await tick()
    r.gone.add("a")
    r.change()
    await tick()
    expect(r.store.slot("a")?.detail?.id).toBe("a")
    expect(r.store.slot("a")?.error).toBe("no such card a")
    off()
  })

  test("a gone card is tried again once, on its own, not polled", async () => {
    const r = rig()
    r.gone.add("x")
    const offs = [r.store.watch("a", () => {}), r.store.watch("x", () => {})]
    await tick()
    await new Promise((res) => setTimeout(res, 150))
    // The first read, then one retry of x alone; the same error again
    // isn't retried.
    expect(r.reads).toEqual([["a", "x"], ["x"]])
    offs.forEach((off) => off())
  })
})

describe("cardStore by id (card #1148)", () => {
  test("a notice reads only the watched ids in it", async () => {
    const r = rig()
    const offs = ["a", "b", "c"].map((id) => r.store.watch(id, () => {}))
    await tick()
    expect(r.reads).toEqual([["a", "b", "c"]])

    // Two named, one of them not watched: only the watched one is read.
    r.notice(["b", "zzz"])
    await tick()
    expect(r.reads.slice(1)).toEqual([["b"]])

    // Several notices before the read start make one read of their union.
    r.notice(["a"])
    r.notice(["c", "a"])
    await tick()
    expect(r.reads.slice(2)).toEqual([["a", "c"]])

    // Nothing watched in it, or nothing named: no read at all.
    r.notice(["zzz"])
    r.notice([])
    await tick()
    expect(r.reads.length).toBe(3)

    // `all` reads every watched id.
    r.change()
    await tick()
    expect(r.reads.slice(3)).toEqual([["a", "b", "c"]])
    offs.forEach((off) => off())
  })

  test("a notice during a read waits, then reads only its ids", async () => {
    const r = rig()
    const offs = ["a", "b"].map((id) => r.store.watch(id, () => {}))
    await Promise.resolve()
    r.notice(["b"])
    await tick()
    await tick()
    expect(r.reads).toEqual([["a", "b"], ["b"]])
    offs.forEach((off) => off())
  })

  test("a touched card that changed renders; the others stay", async () => {
    const r = rig()
    const renders: Record<string, number> = { a: 0, b: 0 }
    const offs = ["a", "b"].map((id) => r.store.watch(id, () => renders[id]++))
    await tick()
    const a = r.store.slot("a")
    r.bump()
    r.notice(["b"])
    await tick()
    expect(r.store.slot("a")).toBe(a)
    expect(renders).toEqual({ a: 1, b: 2 })
    offs.forEach((off) => off())
  })
})

describe("cardStore rev gate (card #1136)", () => {
  // A host that stamps each card and answers "same" for a stamp it holds.
  function gated() {
    const asked: Map<string, string>[] = []
    let rev = 1
    let fire: (ids?: readonly string[]) => void = () => {}
    const store = createCardStore(
      async (ids, have) => {
        asked.push(new Map(have))
        return new Map<string, CardSlot>(
          ids.map((id) => {
            const r = `${id}:${rev}`
            return [
              id,
              have.get(id) === r
                ? { detail: null, error: null, rev: r, same: true }
                : {
                    detail: { id, title: r } as unknown as CardDetail,
                    error: null,
                    rev: r,
                  },
            ]
          })
        )
      },
      (again) => {
        fire = again
        return () => {}
      },
      20
    )
    return { store, asked, bump: () => rev++, change: () => fire() }
  }

  test("a change asks with the revs it holds; an unchanged card keeps its slot", async () => {
    const g = gated()
    let renders = 0
    const off = g.store.watch("a", () => renders++)
    await tick()
    expect(g.asked[0].size).toBe(0)
    const before = g.store.slot("a")
    g.change()
    await tick()
    expect(g.asked[1].get("a")).toBe("a:1")
    expect(g.store.slot("a")).toBe(before)
    expect(renders).toBe(1)
    // Moved on: the card comes whole and renders.
    g.bump()
    g.change()
    await tick()
    expect(g.store.slot("a")).not.toBe(before)
    expect(g.store.slot("a")?.detail?.title).toBe("a:2")
    expect(renders).toBe(2)
    off()
  })

  test("a stamp for a card the store no longer holds reads it whole", async () => {
    const g = gated()
    const off = g.store.watch("a", () => {})
    await tick()
    off()
    const off2 = g.store.watch("a", () => {})
    await tick()
    expect(g.asked[1].size).toBe(0)
    expect(g.store.slot("a")?.detail?.id).toBe("a")
    off2()
  })
})
