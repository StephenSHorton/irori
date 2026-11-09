import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { CardSummary } from "@/lib/models"
import type { GivenGroup } from "@/lib/needs"
import { GivenBatch, givenHeader, startAllText } from "./needs-given"

const spencer = { key: "person:spencer", name: "Spencer", status: "person" }
const cardOf = (n: number) =>
  CardSummary({
    id: `c${n}`,
    short: String(n),
    title: `Task ${n}`,
    givenQuestion: `q${n}`,
  })
const group = (n: number, note = ""): GivenGroup => ({
  id: "person:spencer|b1",
  giver: spencer as GivenGroup["giver"],
  batch: "b1",
  viaAssistant: false,
  note,
  cards: Array.from({ length: n }, (_, i) => cardOf(i + 1)),
})
const render = (g: GivenGroup) =>
  renderToStaticMarkup(
    createElement(GivenBatch, { group: g, onOpen: () => {} })
  )

describe("GivenBatch (decision 69)", () => {
  test("the header reads Spencer gave you 4 tasks", () => {
    expect(givenHeader(group(4))).toBe("Spencer gave you 4 tasks")
    expect(givenHeader(group(1))).toBe("Spencer gave you a task")
    expect(render(group(4))).toContain("Spencer gave you 4 tasks")
  })
  test("the note is shown as the giver's words", () => {
    const html = render(group(2, "Start with the retry."))
    expect(html).toContain("<blockquote")
    expect(html).toContain("Start with the retry.")
    expect(html).toContain("Spencer&#x27;s note")
  })
  test("Start all and the confirm name the count", () => {
    expect(startAllText(4)).toBe(
      "That's 4 yeses, one per card: 4 agents start."
    )
    expect(startAllText(1)).toBe("That's 1 yes, one per card: 1 agent starts.")
    expect(render(group(4))).toContain("Start all 4")
  })
  test("each card has Start an agent and Not now, and the foot says who tells", () => {
    const html = render(group(2))
    expect(html.match(/Start an agent/g)?.length).toBe(2)
    expect(html).toContain("Give back…")
    expect(html).toContain("Spencer&#x27;s assistant will tell them.")
  })
})
