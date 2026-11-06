import { describe, expect, test } from "bun:test"

import {
  around,
  chainOf,
  follows,
  followsOf,
  headOf,
  laterOf,
  whenLater,
  type OrderState,
  type OrderStep,
} from "./later"
import { CardLater } from "./models"

const step = (short: string, state: OrderState): OrderStep => ({
  id: `c${short}`,
  short,
  title: `Card ${short}`,
  state,
})

// A run of four as the host sends it: every card carries the whole run.
const run = [
  step("80", "done"),
  step("81", "working"),
  step("82", "waiting"),
  step("83", "later"),
]
const inRun = (short: string) => ({
  id: `c${short}`,
  order: { steps: run },
})

describe("later", () => {
  test("laterOf reads the card's field", () => {
    expect(laterOf({ id: "a" })).toBeUndefined()
    expect(laterOf({ id: "a", later: null })).toBeUndefined()
    const l = CardLater({ why: "x", back: "word", line: "when you say" })
    expect(laterOf({ id: "a", later: l })).toBe(l)
  })

  test("whenLater is one line: what brings it back, then why", () => {
    expect(
      whenLater(
        CardLater({
          why: "Not before the release",
          back: "date",
          line: "on 1 Dec",
        })
      )
    ).toBe("Not yet: on 1 Dec. Not before the release.")
    // A why that ends its own sentence isn't stopped twice.
    expect(
      whenLater(
        CardLater({ why: "An idea.", back: "word", line: "When you say" })
      )
    ).toBe("Not yet: when you say. An idea.")
    expect(whenLater(CardLater({ back: "word", line: "when you say" }))).toBe(
      "Not yet: when you say."
    )
  })
})

describe("order", () => {
  test("chainOf is the run, and none for a card on its own", () => {
    expect(chainOf(inRun("82"))).toBe(run)
    expect(chainOf({ id: "a" })).toBeUndefined()
    expect(chainOf({ id: "a", order: null })).toBeUndefined()
    expect(
      chainOf({ id: "a", order: { steps: [step("1", "waiting")] } })
    ).toBeUndefined()
  })

  test("the head is the first card not done, and the rest hang under it", () => {
    expect(headOf(run)?.id).toBe("c81")
    expect(followsOf(inRun("81")).map((s) => s.id)).toEqual(["c82", "c83"])
    expect(followsOf(inRun("82"))).toEqual([])
    expect(followsOf({ id: "lone" })).toEqual([])
    expect(follows(inRun("81"))).toBe(false)
    expect(follows(inRun("82"))).toBe(true)
    expect(follows(inRun("83"))).toBe(true)
    expect(follows({ id: "lone" })).toBe(false)
  })

  test("around is the card before and after", () => {
    expect(around(inRun("80"))).toEqual({ before: undefined, after: run[1] })
    expect(around(inRun("82"))).toEqual({ before: run[1], after: run[3] })
    expect(around(inRun("83"))).toEqual({ before: run[2], after: undefined })
    expect(around({ id: "lone" })).toEqual({})
  })
})
