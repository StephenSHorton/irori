import { describe, expect, test } from "bun:test"

import { cards, threads } from "@/lib/cards"
import { nextTitle } from "./title-edit"

describe("nextTitle (card #779)", () => {
  test("trims what it saves", () => {
    expect(nextTitle("  Join the lobby  ", "lobby join")).toBe("Join the lobby")
  })
  test("nothing to save when empty or the same", () => {
    expect(nextTitle("   ", "lobby join")).toBeNull()
    expect(nextTitle(" lobby join ", "lobby join")).toBeNull()
    // The field shows a title on one line: saving that is no change.
    expect(nextTitle("lobby join flow", "lobby join\nflow")).toBeNull()
  })
})

describe("renaming on the sample (card #779)", () => {
  // The sample reads ?empty from the address; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("a card's page reads its new name", async () => {
    await cards.rename("sample-card-4", "  A name the person gave  ")
    const d = await cards.detail("sample-card-4")
    expect(d.title).toBe("A name the person gave")
  })

  test("a thread's new name is in the Threads list", async () => {
    await cards.rename("sample-thread-1", "Renamed thread")
    const list = await threads.listAll()
    expect(list.find((t) => t.id === "sample-thread-1")?.title).toBe(
      "Renamed thread"
    )
  })

  test("an empty or too-long name is refused, as the host does", async () => {
    expect(cards.rename("sample-card-4", "  ")).rejects.toThrow("needs a title")
    expect(cards.rename("sample-card-4", "x".repeat(201))).rejects.toThrow(
      "longer than 200"
    )
  })
})
