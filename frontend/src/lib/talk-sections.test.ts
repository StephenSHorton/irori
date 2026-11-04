import { describe, expect, test } from "bun:test"

import { TalkConversation } from "@/lib/models"
import {
  archived,
  dmSections,
  foldedByDefault,
  needsLook,
  sectionRows,
  shownAtOnce,
  unreadIn,
} from "./talk-sections"

const dm = (
  id: string,
  p: Partial<Parameters<typeof TalkConversation>[0]> = {}
) => TalkConversation({ convId: id, kind: "dm", name: id, members: [], ...p })

describe("Talk list sections (card #463)", () => {
  test("DMs split into yours, with gone agents, and agents talking", () => {
    const s = dmSections(
      [dm("a"), dm("b", { readOnly: true }), dm("c")],
      (c) => c.convId === "c"
    )
    expect(s.yours.map((c) => c.convId)).toEqual(["a"])
    expect(s.gone.map((c) => c.convId)).toEqual(["c"])
    expect(s.agents.map((c) => c.convId)).toEqual(["b"])
  })

  test("gone agents' DMs archive once read; unread or mentions keep them up top (card #473)", () => {
    const gone = () => true
    const s = dmSections(
      [
        dm("read"),
        dm("unread", { unread: 1 }),
        dm("named", { mentions: 1 }),
        dm("kept"),
        dm("watched", { readOnly: true }),
      ],
      gone,
      "kept"
    )
    expect(s.yours.map((c) => c.convId)).toEqual(["unread", "named", "kept"])
    expect(s.gone.map((c) => c.convId)).toEqual(["read"])
    expect(s.agents.map((c) => c.convId)).toEqual(["watched"])
  })

  test("a DM whose agent came back (reattached) leaves the Archive", () => {
    const c = dm("back")
    expect(archived(c, () => true)).toBe(true)
    expect(archived(c, () => false)).toBe(false)
    expect(dmSections([c], () => false).yours.map((x) => x.convId)).toEqual(["back"])
  })

  test("the Archive folds by default and still finds its open DM", () => {
    expect(foldedByDefault.gone).toBe(true)
    const { rows, hidden } = sectionRows([dm("a"), dm("b")], true, "b", false)
    expect(rows.map((c) => c.convId)).toEqual(["b"])
    expect(hidden).toBe(1)
  })

  test("only Threads and Channels start unfolded", () => {
    expect(foldedByDefault).toEqual({
      threads: false,
      channels: false,
      dms: true,
      gone: true,
      agents: true,
    })
  })

  test("a folded section still shows the open one and unread DMs, never an agent↔agent DM's unread", () => {
    const items = [
      dm("quiet"),
      dm("new", { unread: 2 }),
      dm("named", { mentions: 1 }),
      dm("open"),
      dm("watched", { readOnly: true, unread: 9 }),
    ]
    const { rows, hidden } = sectionRows(items, true, "open", false)
    expect(rows.map((c) => c.convId)).toEqual(["new", "named", "open"])
    expect(hidden).toBe(2)
    expect(needsLook(dm("watched", { readOnly: true }), "watched")).toBe(true)
  })

  test("an unfolded section lists the newest few, keeps the open one, and Show more lists all", () => {
    const items = Array.from({ length: 20 }, (_, i) => dm(`c${i}`))
    const some = sectionRows(items, false, "c15", false)
    expect(some.rows.length).toBe(shownAtOnce + 1)
    expect(some.rows.at(-1)?.convId).toBe("c15")
    expect(some.hidden).toBe(20 - shownAtOnce - 1)
    const all = sectionRows(items, false, "", true)
    expect(all.rows.length).toBe(20)
    expect(all.hidden).toBe(0)
  })

  test("the unread count leaves out read-only agent↔agent DMs", () => {
    expect(
      unreadIn([
        dm("a", { unread: 1 }),
        dm("b", { readOnly: true, unread: 3 }),
        dm("c"),
      ])
    ).toBe(1)
  })
})
