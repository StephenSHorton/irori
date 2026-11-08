import { describe, expect, test } from "bun:test"

import { cards, onCardsChanged } from "@/lib/cards"

describe("Undo on what Irori did (card #786)", () => {
  // The sample reads ?empty from the address; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("the line is Irori's, keyed on the record, with Undo", async () => {
    const d = await cards.detail("sample-ask-1")
    const line = d.thread.find((it) => it.undoDid)
    expect(line?.who.status).toBe("irori")
    expect(line?.undoDid).toBe("sample-did-1")
    expect(line?.text).toContain("handed this back")
  })

  test("one click undoes it, and the Undo goes", async () => {
    let told = 0
    const off = onCardsChanged(() => told++)
    await cards.undoDid("sample-ask-1", "sample-did-1")
    off()
    expect(told).toBeGreaterThan(0)
    const d = await cards.detail("sample-ask-1")
    expect(d.thread.some((it) => it.undoDid)).toBe(false)
    expect(d.thread.some((it) => it.text.includes("(undone)"))).toBe(true)
    expect(cards.undoDid("sample-ask-1", "sample-did-1")).rejects.toThrow(
      "can't be undone"
    )
  })

  test("a duplicate link names the other card and has Undo", async () => {
    const d = await cards.detail("sample-ask-2")
    const line = d.thread.find((it) => it.undoDid)
    expect(line?.who.status).toBe("irori")
    expect(line?.text).toContain("#41")
    expect(line?.text).toContain("same work")
    await cards.undoDid("sample-ask-2", "sample-did-2")
    const after = await cards.detail("sample-ask-2")
    expect(after.thread.some((it) => it.undoDid)).toBe(false)
    expect(after.thread.some((it) => it.text.includes("(undone)"))).toBe(true)
  })
})
