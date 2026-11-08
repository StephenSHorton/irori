import { describe, expect, test } from "bun:test"

import { cards } from "@/lib/cards"

describe("Undo on Irori's name for a thread (card #782)", () => {
  // The sample reads ?empty from the address; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("Irori's rename line carries the title Undo puts back", async () => {
    const d = await cards.detail("sample-ask-1")
    expect(d.title).toBe("Mac build twice as slow as the PC")
    const line = d.thread.find((it) => it.undoTitle)
    expect(line?.who.status).toBe("irori")
    expect(line?.undoTitle).toBe(
      "Why does the Mac build take twice as long as the PC's?"
    )
  })

  test("one click gives the old name back, and the Undo goes", async () => {
    await cards.undoTitle("sample-ask-1")
    const d = await cards.detail("sample-ask-1")
    expect(d.title).toBe(
      "Why does the Mac build take twice as long as the PC's?"
    )
    expect(d.thread.some((it) => it.undoTitle)).toBe(false)
    expect(d.thread.at(-1)?.text).toContain("You renamed it")
    expect(cards.undoTitle("sample-ask-1")).rejects.toThrow("already gone")
  })
})
