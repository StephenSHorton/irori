import { describe, expect, test } from "bun:test"

import { createCardStore, type CardSlot } from "./cards"
import type { CardDetail } from "./models"

// Card #468: every useCard shares one read per change.

const tick = () => new Promise((r) => setTimeout(r, 0))

function rig() {
  const reads: string[][] = []
  let version = 1
  const gone = new Set<string>()
  let fire = () => {}
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
    }
  )
  return {
    store,
    reads,
    gone,
    bump: () => version++,
    change: () => fire(),
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
})
