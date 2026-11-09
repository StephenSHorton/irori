import { describe, expect, test } from "bun:test"

import { CardsBucket, mergeCards, readGrid, type CardsHost } from "./cards"
import { CardSummary } from "./models"
import type { Want } from "./patched"

// Card #1147: the grid patches. A full read replaces it; a patch upserts
// its rows and lays every row out in the host's order.
const card = (id: string, title = id) => CardSummary({ id, title })
const want = (o: Partial<Want> = {}): Want => ({
  all: false,
  drift: false,
  full: false,
  ids: {},
  ...o,
})
const titles = (v: CardSummary[]) => v.map((c) => `${c.id}:${c.title}`)

describe("mergeCards", () => {
  const cur = [card("a"), card("b"), card("c")]

  test("a full read replaces the grid", () => {
    const next = [card("x")]
    expect(
      mergeCards(
        cur,
        { rows: next, order: null },
        want({ all: true, full: true })
      )
    ).toEqual(next)
  })

  test("a patch upserts its rows in the host's order", () => {
    const got = mergeCards(
      cur,
      { rows: [card("b", "b2"), card("d")], order: ["d", "b", "a", "c"] },
      want({ ids: { [CardsBucket]: ["b", "d"] } })
    )
    expect(titles(got)).toEqual(["d:d", "b:b2", "a:a", "c:c"])
  })

  test("a card the order leaves out is gone", () => {
    const got = mergeCards(
      cur,
      { rows: [], order: ["c", "a"] },
      want({ ids: { [CardsBucket]: ["b"] } })
    )
    expect(titles(got)).toEqual(["c:c", "a:a"])
  })

  test("an id in the order with no row yet is skipped", () => {
    const got = mergeCards(
      cur,
      { rows: [], order: ["new", "a", "b", "c"] },
      want({ ids: { [CardsBucket]: ["a"] } })
    )
    expect(titles(got)).toEqual(["a:a", "b:b", "c:c"])
  })

  test("rows the patch doesn't name keep their object", () => {
    const got = mergeCards(
      cur,
      { rows: [card("b", "b2")], order: ["a", "b", "c"] },
      want({ ids: { [CardsBucket]: ["b"] } })
    )
    expect(got[0]).toBe(cur[0])
    expect(got[2]).toBe(cur[2])
  })
})

describe("readGrid", () => {
  const host = () => {
    const calls: string[] = []
    const h: CardsHost = {
      Cards: async () => (calls.push("cards"), [card("a")]),
      CardsCold: async () => (calls.push("cold"), [card("a")]),
      CardsPatch: async (ids) => (
        calls.push(`patch ${ids.join(",")}`),
        { rows: [card("a")], order: ["a"] }
      ),
    }
    return { h, calls }
  }

  test("a drift sample reads cold on the host", async () => {
    const { h, calls } = host()
    const got = await readGrid(want({ drift: true, full: true }), h)
    expect(calls).toEqual(["cold"])
    expect(got.order).toBeNull()
  })

  test("all reads the whole grid", async () => {
    const { h, calls } = host()
    await readGrid(want({ all: true, full: true }), h)
    expect(calls).toEqual(["cards"])
  })

  test("a notice's cards read just their rows, with the order", async () => {
    const { h, calls } = host()
    const got = await readGrid(want({ ids: { [CardsBucket]: ["a", "b"] } }), h)
    expect(calls).toEqual(["patch a,b"])
    expect(got.order).toEqual(["a"])
  })
})
