import { beforeEach, describe, expect, test } from "bun:test"

import {
  NoteMaxChars,
  cleanTags,
  kindCounts,
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

  test("one list: notes are the person's own (card #616)", async () => {
    const all = await notes.list()
    expect(all.length).toBeGreaterThan(0)
    // A note about a shared space is a Mine note with a board: tag.
    expect(all.some((n) => n.tags.includes("board:mlg"))).toBe(true)
    expect(all.every((n) => !("scope" in n) && !("boardId" in n))).toBe(true)
  })

  test("agents' notes are live with no accepter, and removable (card #616)", async () => {
    const list = await notes.list()
    expect(list.every((n) => n.state === "accepted")).toBe(true)
    const theirs = list.filter((n) => !n.byPerson)
    expect(theirs.length).toBeGreaterThan(0)
    expect(theirs.every((n) => !n.acceptedBy)).toBe(true)
    await notes.remove(theirs[0].id)
    expect(
      (await notes.list()).find((n) => n.id === theirs[0].id)
    ).toBeUndefined()
  })

  test("the person's edit of an agent's note applies at once", async () => {
    const n = (await notes.list()).find((x) => !x.byPerson)!
    await notes.edit(n.id, { text: "Shorter." })
    expect((await notes.list()).find((x) => x.id === n.id)!.text).toBe(
      "Shorter."
    )
  })

  test("a person's new note is live at once and counted", async () => {
    const before = kindCounts(await notes.list())[0].count
    await notes.create({
      kind: "decision",
      text: "Ship on Fridays.",
      tags: ["Everywhere"],
    })
    const list = await notes.list()
    expect(kindCounts(list)[0].count).toBe(before + 1)
    expect(list[0].tags).toEqual(["everywhere"])
    expect(list[0].state).toBe("accepted")
  })

  test("a bad note is refused before it is written", async () => {
    await expect(
      notes.create({ kind: "rule", text: "x", tags: [] })
    ).rejects.toThrow()
  })
})
