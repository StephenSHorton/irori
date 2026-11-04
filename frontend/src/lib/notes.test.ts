import { beforeEach, describe, expect, test } from "bun:test"

import {
  MineScope,
  NoteMaxChars,
  cleanTags,
  kindCounts,
  needsLook,
  noteProblem,
  notes,
  parseTag,
  resetSampleNotes,
  tagLabel,
} from "./notes"

const irori = "remote:github.com/StephenSHorton/irori"

// The sample runs outside the host; bun has no window.
const g = globalThis as unknown as { window?: unknown }
g.window ??= { location: { search: "" } }

describe("tags", () => {
  test("known shapes parse; repo keys and paths keep their case", () => {
    expect(parseTag("Everywhere")?.raw).toBe("everywhere")
    expect(parseTag(`repo:${irori}`)?.raw).toBe(`repo:${irori}`)
    expect(parseTag(`path:${irori}:frontend/src`)?.raw).toBe(
      `path:${irori}:frontend/src`
    )
    expect(parseTag("Board:MLG")?.raw).toBe("board:mlg")
    expect(parseTag("machine:The PC")?.raw).toBe("machine:the pc")
    expect(parseTag("topic:Lobby")?.raw).toBe("topic:lobby")
  })
  test("unknown shapes don't", () => {
    for (const t of [
      "",
      "irori",
      "repo:",
      "path:nopath",
      `path:${irori}:`,
      "colour:red",
    ])
      expect(parseTag(t)).toBeNull()
  })
  test("labels read as the mockup's", () => {
    expect(tagLabel(`repo:${irori}`)).toBe("repo irori")
    expect(tagLabel(`path:${irori}:frontend/`)).toBe("irori frontend/")
    expect(tagLabel("machine:the pc")).toBe("on the pc")
    expect(tagLabel("board:mlg", () => "MLG")).toBe("MLG")
    expect(tagLabel("topic:lobby")).toBe("#lobby")
  })
  test("cleanTags drops repeats and reports bad ones", () => {
    expect(cleanTags(["everywhere", "EVERYWHERE", "nope", " "])).toEqual({
      tags: ["everywhere"],
      bad: ["nope"],
    })
  })
})

describe("the form's checks match the write's (§11)", () => {
  const ok = { kind: "rule", text: "A rule.", tags: ["everywhere"] }
  test("a good note passes", () => expect(noteProblem(ok)).toBe(""))
  test("each refusal", () => {
    expect(noteProblem({ ...ok, kind: "fact" })).toContain("kind")
    expect(noteProblem({ ...ok, text: "  " })).toContain("Write")
    expect(
      noteProblem({ ...ok, text: "x".repeat(NoteMaxChars + 1) })
    ).toContain(`${NoteMaxChars}`)
    expect(noteProblem({ ...ok, tags: [] })).toContain("at least one tag")
    expect(noteProblem({ ...ok, tags: ["nope"] })).toContain("nope")
  })
})

describe("sample notes (the browser preview)", () => {
  beforeEach(() => resetSampleNotes())

  test("Mine lists its own; a board's tab its own", async () => {
    const mine = await notes.list(MineScope)
    const mlg = await notes.list("mlg")
    expect(mine.every((n) => n.scope === MineScope)).toBe(true)
    expect(mlg.length).toBeGreaterThan(0)
    expect(mlg.every((n) => n.scope === "mlg")).toBe(true)
  })

  test("Keep makes a proposal live; Remove turns it down", async () => {
    const waiting = (await notes.list(MineScope)).filter(
      (n) => n.state === "proposed"
    )
    expect(waiting.length).toBe(2)
    await notes.accept(waiting[0].id)
    await notes.remove(waiting[1].id)
    const after = await notes.list(MineScope)
    expect(after.find((n) => n.id === waiting[0].id)?.state).toBe("accepted")
    expect(after.find((n) => n.id === waiting[1].id)).toBeUndefined()
  })

  test("accepting an agent's edit applies it; New from agents empties", async () => {
    const edit = (await notes.list(MineScope)).find((n) => n.pending)!
    expect(needsLook(edit)).toBe(true)
    const proposed = edit.pending!.text!
    await notes.accept(edit.id)
    const n = (await notes.list(MineScope)).find((x) => x.id === edit.id)!
    expect(n.text).toBe(proposed)
    expect(n.pending).toBeUndefined()
    expect(needsLook(n)).toBe(false)
  })

  test("a person's new note is live at once and counted", async () => {
    const before = kindCounts(await notes.list(MineScope))[0].count
    await notes.create({
      scope: MineScope,
      kind: "decision",
      text: "Ship on Fridays.",
      tags: ["Everywhere"],
    })
    const list = await notes.list(MineScope)
    expect(kindCounts(list)[0].count).toBe(before + 1)
    expect(list[0].tags).toEqual(["everywhere"])
    expect(list[0].state).toBe("accepted")
  })

  test("a bad note is refused before it is written", async () => {
    await expect(
      notes.create({ scope: MineScope, kind: "rule", text: "x", tags: [] })
    ).rejects.toThrow()
  })
})
