import { describe, expect, test } from "bun:test"

import { rank, score, wordScore } from "./fuzzy"
import { activeCommands, register, shortcutKeys } from "./commands"

describe("wordScore", () => {
  test("prefix beats a word start beats inside beats letters in order", () => {
    const prefix = wordScore("new", "new card")
    const wordStart = wordScore("card", "new card")
    const inside = wordScore("ard", "new card")
    const letters = wordScore("nwcd", "new card")
    expect(prefix).toBeGreaterThan(wordStart)
    expect(wordStart).toBeGreaterThan(inside)
    expect(inside).toBeGreaterThan(letters)
    expect(letters).toBeGreaterThan(0)
  })
  test("no match is 0", () => {
    expect(wordScore("xyz", "new card")).toBe(0)
    expect(wordScore("dc", "new card")).toBe(0)
  })
})

describe("score", () => {
  test("every word has to match", () => {
    expect(score("new card", "New card")).toBeGreaterThan(0)
    expect(score("new zebra", "New card")).toBe(0)
  })
  test("keywords count, below the title", () => {
    expect(score("preferences", "Open settings", ["preferences"])).toBeGreaterThan(0)
    expect(score("settings", "Settings")).toBeGreaterThan(
      score("settings", "Devices", ["settings"])
    )
  })
  test("a card ref is found by its ref text", () => {
    expect(score("#212", "Command palette", ["#212", "irori#212"])).toBeGreaterThan(0)
    expect(score("wind#51", "Boss relay", ["#51", "wind#51"])).toBeGreaterThan(0)
  })
})

describe("rank", () => {
  test("best first, misses dropped", () => {
    const items = ["Pin this board", "Open settings", "Settings: Devices", "Home"]
    const got = rank(items, "settings", (t) => ({ title: t })).map((s) => s.item)
    expect(got).toEqual(["Settings: Devices", "Open settings"])
  })
})

describe("commands", () => {
  test("register, when() and unregister", () => {
    let ran = 0
    const off = register([
      { id: "t.a", title: "A", run: () => ran++ },
      { id: "t.b", title: "B", when: () => false, run: () => {} },
    ])
    const active = activeCommands().map((c) => c.id)
    expect(active).toContain("t.a")
    expect(active).not.toContain("t.b")
    activeCommands().find((c) => c.id === "t.a")?.run()
    expect(ran).toBe(1)
    off()
    expect(activeCommands().map((c) => c.id)).not.toContain("t.a")
  })
  test("shortcut keys per platform", () => {
    expect(shortcutKeys("Mod+K", true)).toEqual(["⌘", "K"])
    expect(shortcutKeys("Mod+K", false)).toEqual(["Ctrl", "K"])
  })
})
