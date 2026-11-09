import { describe, expect, test } from "bun:test"

import { TalkConversation } from "@/lib/models"
import {
  archived,
  dmSections,
  foldedByDefault,
  sectionHeader,
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

  test("gone agents' DMs archive read or unread; only the kept open one stays up top (card #473)", () => {
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
    expect(s.yours.map((c) => c.convId)).toEqual(["kept"])
    expect(s.gone.map((c) => c.convId)).toEqual([
      "read",
      "unread",
      "named",
      "watched",
    ])
    expect(s.agents).toEqual([])
  })

  test("agent↔agent DMs archive once every agent in them has gone (card #1037)", () => {
    const s = dmSections(
      [
        dm("live", { readOnly: true }),
        dm("dead", { readOnly: true }),
        dm("yours"),
      ],
      (c) => c.convId === "dead"
    )
    expect(s.agents.map((c) => c.convId)).toEqual(["live"])
    expect(s.gone.map((c) => c.convId)).toEqual(["dead"])
    expect(s.yours.map((c) => c.convId)).toEqual(["yours"])
    // An agent reattaching brings it back.
    const back = dmSections([dm("dead", { readOnly: true })], () => false)
    expect(back.agents.map((c) => c.convId)).toEqual(["dead"])
  })

  test("a DM whose agent came back (reattached) leaves the Archive", () => {
    const c = dm("back")
    expect(archived(c, () => true)).toBe(true)
    expect(archived(c, () => false)).toBe(false)
    expect(dmSections([c], () => false).yours.map((x) => x.convId)).toEqual([
      "back",
    ])
  })

  test("the Archive and Agents talking start folded; your DMs, Threads and Channels don't", () => {
    expect(foldedByDefault).toEqual({
      threads: false,
      huddles: false,
      channels: false,
      dms: false,
      gone: true,
      agents: true,
    })
  })

  test("a folded section lists no rows at all: not its unread, mentions or the open one (card #484)", () => {
    const items = [
      dm("quiet"),
      dm("new", { unread: 2 }),
      dm("named", { mentions: 1, unread: 1 }),
      dm("open"),
      dm("watched", { readOnly: true, unread: 9 }),
    ]
    const { rows, hidden } = sectionRows(items, true, "open", false)
    expect(rows).toEqual([])
    expect(hidden).toBe(5)
    // What it hides still counts beside its name.
    expect(unreadIn(items)).toBe(2)
    expect(sectionRows(items, true, "", true).rows).toEqual([])
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

  test("the folded Archive shows only its count: no new pill, no Mark all read (card #514)", () => {
    const items = [dm("a", { unread: 1 }), dm("b", { unread: 4 }), dm("c")]
    expect(sectionHeader("gone", true, items)).toEqual({
      unread: 0,
      markAll: false,
    })
    // Opened, its rows keep their marks and Mark all read is there.
    expect(sectionHeader("gone", false, items)).toEqual({
      unread: 2,
      markAll: true,
    })
    // Nothing new, nothing to mark.
    expect(sectionHeader("gone", false, [dm("c")])).toEqual({
      unread: 0,
      markAll: false,
    })
  })

  test("other sections keep their new pill folded and never offer Mark all read", () => {
    const items = [dm("a", { unread: 1 }), dm("b", { unread: 4 }), dm("c")]
    for (const k of ["threads", "channels", "dms", "agents"] as const) {
      expect(sectionHeader(k, true, items)).toEqual({
        unread: 2,
        markAll: false,
      })
      expect(sectionHeader(k, false, items)).toEqual({
        unread: 2,
        markAll: false,
      })
    }
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
