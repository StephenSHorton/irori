import { describe, expect, test } from "bun:test"
import { readFileSync } from "node:fs"
import { join } from "node:path"

import { splitDraft, splitReply } from "./assistant"
import {
  goesByItself,
  goLabel,
  parseGo,
  settingSpots,
  splitGo,
  type GoTarget,
} from "./goto"

describe("Ask Irori's go lines (card #784)", () => {
  test("each kind it knows", () => {
    expect(parseGo("card #779")).toEqual({ kind: "card", ref: "#779" })
    expect(parseGo("card Suzuri#12")).toEqual({
      kind: "card",
      ref: "suzuri#12",
    })
    expect(parseGo("thread #5")).toEqual({ kind: "card", ref: "#5" })
    expect(parseGo("setting stop-here")).toEqual({
      kind: "setting",
      key: "stop-here",
    })
    expect(parseGo("setting agents.model")).toEqual({
      kind: "setting",
      key: "agents.model",
    })
    expect(parseGo("page Talk")).toEqual({ kind: "page", page: "talk" })
    expect(
      parseGo("pr https://github.com/StephenSHorton/irori/pull/184")
    ).toEqual({
      kind: "pr",
      url: "https://github.com/StephenSHorton/irori/pull/184",
      number: "184",
    })
  })
  test("anything else names nothing", () => {
    for (const body of [
      "",
      "card",
      "card 779",
      "card #779 and #780",
      "setting agents.nope",
      "setting KEY",
      "page settings",
      "page NAME",
      "pr https://example.com/pull/1",
      "pr javascript:alert(1)",
      "agent sample-otter",
    ])
      expect(parseGo(body)).toBeNull()
  })
  test("the line comes out of the text; the first that names something wins", () => {
    const s = splitGo(
      "It's in the top bar.\n\n<<go setting nope>>\n<<go setting stop-here>>\n<<go card #1>>"
    )
    expect(s.text).toBe("It's in the top bar.")
    expect(s.go).toEqual({ kind: "setting", key: "stop-here" })
  })
  test("a line mid-reply comes out too, and the paragraphs close up", () => {
    const s = splitGo("That's #779.\n<<go card #779>>\n\nIt's working.")
    expect(s.text).toBe("That's #779.\n\nIt's working.")
    expect(s.go).toEqual({ kind: "card", ref: "#779" })
  })
  test("a go line being written never flashes", () => {
    for (const tail of ["<<", "<<g", "<<go", "<<go ", "<<go card #7"])
      expect(splitGo(`That's #779.\n\n${tail}`, true)).toEqual({
        text: "That's #779.",
        go: null,
      })
    // A lone "<" may be the reply's own.
    expect(splitGo("a < b", true).text).toBe("a < b")
    // A draft being written is left to splitDraft.
    expect(
      splitDraft(splitGo("Drafted.\n\n<<thread>>\nAdd it", true).text)
    ).toEqual({
      text: "Drafted.",
      draft: "Add it",
      partial: true,
      check: false,
    })
  })
  test("in a finished reply a malformed go line stays, and so does what follows", () => {
    const raw = "It's on the card. <<go card #779> Then tap Start."
    expect(splitGo(raw)).toEqual({ text: raw, go: null })
  })
  test("a go line inside a draft stays in the draft: it posts as shown", () => {
    const r = splitReply(
      "Drafted.\n\n<<thread>>\nLook at #779.\n<<go card #779>>\n<</thread>>"
    )
    expect(r.go).toBeNull()
    expect(r.text).toBe("Drafted.")
    expect(r.draft).toBe("Look at #779.\n<<go card #779>>")
  })
  test("with a drafted thread, both come out", () => {
    const raw =
      "Drafted one.\n\n<<thread>>\nFix #3.\n<</thread>>\n<<go card #3>>"
    const g = splitGo(raw)
    expect(g.go).toEqual({ kind: "card", ref: "#3" })
    expect(splitDraft(g.text)).toEqual({
      text: "Drafted one.",
      draft: "Fix #3.",
      partial: false,
      check: false,
    })
  })
  test("button words", () => {
    const l = (t: GoTarget) => goLabel(t)
    expect(l({ kind: "card", ref: "#779" })).toBe("Open #779")
    expect(l({ kind: "page", page: "held" })).toBe("Open Held messages")
    expect(l({ kind: "setting", key: "stop-here" })).toBe("Show Stop here")
    expect(l({ kind: "setting", key: "agents" })).toBe("Open Settings › Agents")
    expect(l({ kind: "setting", key: "agents.model" })).toBe(
      "Show Settings › Agents › What a new agent starts with"
    )
    expect(
      l({ kind: "pr", url: "https://github.com/a/b/pull/9", number: "9" })
    ).toBe("Open PR #9 on GitHub")
  })
  test("a PR waits for a tap; the rest go by themselves", () => {
    expect(
      goesByItself({
        kind: "pr",
        url: "https://github.com/a/b/pull/9",
        number: "9",
      })
    ).toBe(false)
    expect(goesByItself({ kind: "card", ref: "#1" })).toBe(true)
    expect(goesByItself({ kind: "setting", key: "app" })).toBe(true)
  })
})

describe("the go lines' settings match the assistant's prompt (card #784)", () => {
  const rust = readFileSync(
    join(import.meta.dir, "../../../crates/window/src/assistant.rs"),
    "utf8"
  )
  const system = rust.slice(
    rust.indexOf("pub const SYSTEM"),
    rust.indexOf("pub const GO_OPEN")
  )
  const prompted = [...system.matchAll(/^- ([a-z][a-z.-]*): /gm)].map(
    (m) => m[1]
  )
  test("every key the prompt lists, the chat knows, and back", () => {
    expect(prompted.length).toBeGreaterThan(10)
    expect([...prompted].sort()).toEqual(settingSpots.map((s) => s.key).sort())
  })
  test("every page the prompt lists, the chat knows", () => {
    const line =
      /<<go page NAME>> a page: ([^\n]*)/
        .exec(system.replace(/\\\r?\n/g, ""))?.[1]
        ?.replace(/\([^)]*\)/g, "") ?? ""
    const pages = line
      .split(/,|\bor\b/)
      .map((p) => p.trim())
      .filter(Boolean)
    expect(pages.length).toBeGreaterThan(5)
    for (const p of pages) expect(parseGo(`page ${p}`)).not.toBeNull()
  })
})
