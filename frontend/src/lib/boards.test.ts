import { describe, expect, test } from "bun:test"

import {
  InviteState,
  InviteStatus,
  boards,
  boardsNow,
  inviteOpen,
  looksFinished,
  presenceLine,
  reloadBoards,
  subscribeBoards,
  suggestTag,
  validTag,
  type SummaryMember,
} from "./boards"
import { cards } from "./cards"
import { Personal } from "./spaces"

// settled waits for the reloads a change started (the sample's list is a
// resolved promise, so a few turns of the queue are enough).
async function settled() {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

describe("the shared boards list (card #233)", () => {
  // The sample runs outside the host; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("every reader sees one list, read again when a board moves", async () => {
    let a = 0
    let b = 0
    const offA = subscribeBoards(() => a++)
    const offB = subscribeBoards(() => b++)
    await settled()
    const before = boardsNow().boards.map((x) => x.tag)
    expect(before).toContain("mlg")
    // A board arriving (here the sample's New board; in the app the
    // host's boards:changed) reaches both readers without a reload call.
    const made = await boards.create("Wind", "wind")
    await settled()
    const after = boardsNow().boards
    expect(after.some((x) => x.boardId === made.boardId)).toBe(true)
    expect(a).toBeGreaterThan(1)
    expect(b).toBeGreaterThan(1)
    // A page opening later (a new reader) gets the same list, not its own.
    let c = 0
    const offC = subscribeBoards(() => c++)
    await settled()
    expect(c).toBeGreaterThan(0)
    expect(boardsNow().boards.map((x) => x.boardId)).toEqual(
      after.map((x) => x.boardId)
    )
    offA()
    offB()
    offC()
    await boards.leave(made.boardId)
  })

  test("an older read that finishes last doesn't win", async () => {
    const off = subscribeBoards(() => {})
    await settled()
    const first = reloadBoards()
    const made = await boards.create("Late", "late")
    await Promise.all([first, reloadBoards()])
    expect(boardsNow().boards.some((x) => x.boardId === made.boardId)).toBe(
      true
    )
    off()
    await boards.leave(made.boardId)
  })
})

describe("board tags (identity.ValidBoardTag)", () => {
  test("a lowercase letter, then letters, digits or '-', 8 at most (card #590)", () => {
    for (const t of ["mlg", "a", "site-2", "agility", "abcdefgh"])
      expect(validTag(t)).toBe(true)
    for (const t of [
      "",
      "MLG",
      "1mlg",
      "-x",
      "has space",
      "abcdefghi",
      "abcdefghijklmnop",
      "m#1",
    ])
      expect(validTag(t)).toBe(false)
  })
  test("New board suggests one from the name's first word", () => {
    expect(suggestTag("MLG")).toBe("mlg")
    expect(suggestTag("Website refresh")).toBe("website")
    expect(suggestTag("Infrastructure plans")).toBe("infrastr")
    expect(suggestTag("2026 plans")).toBe("")
    expect(suggestTag("")).toBe("")
  })
})

describe("invites", () => {
  test("inviteOpen is true only while an invite is in progress", () => {
    expect(inviteOpen(null)).toBe(false)
    expect(inviteOpen(InviteStatus({}))).toBe(false)
    for (const st of [
      InviteState.Waiting,
      InviteState.Words,
      InviteState.Joining,
    ])
      expect(inviteOpen(InviteStatus({ state: st }))).toBe(true)
    for (const st of [InviteState.Done, InviteState.Failed])
      expect(inviteOpen(InviteStatus({ state: st }))).toBe(false)
  })
})

// Card #371: the People panel's line for each member.
describe("presenceLine", () => {
  const now = new Date("2026-11-03T20:00:00Z")
  const ago = (ms: number) => new Date(now.getTime() - ms).toISOString()
  const m = (p: Partial<SummaryMember>): SummaryMember => ({
    personId: "p",
    name: "Blake",
    me: false,
    presence: "online",
    working: 0,
    free: 0,
    ...p,
  })

  test("online, with their agents on the board", () => {
    // Card #512: no "free" count.
    expect(presenceLine(m({ working: 2, free: 1 }), now)).toBe(
      "online · 2 working"
    )
    expect(presenceLine(m({ free: 1 }), now)).toBe("online")
    expect(presenceLine(m({}), now)).toBe("online · no agents")
  })

  test("recent says when they were seen", () => {
    expect(
      presenceLine(
        m({ presence: "recent", lastSeen: ago(5 * 60_000), working: 1 }),
        now
      )
    ).toBe("seen 5m ago · 1 working")
  })

  test("offline says when, and leaves out agent counts that may be stale", () => {
    expect(
      presenceLine(
        m({
          presence: "offline",
          lastSeen: ago(2 * 3600_000),
          working: 3,
          free: 1,
        }),
        now
      )
    ).toBe("offline · seen 2h ago")
    expect(presenceLine(m({ presence: "offline" }), now)).toBe("offline")
    expect(presenceLine(m({ presence: "" }), now)).toBe("offline")
  })
})

// Card #690: a board whose cards are all done may look finished.
describe("looksFinished", () => {
  const now = new Date("2026-11-07T12:00:00Z")
  const ago = (days: number) =>
    new Date(now.getTime() - days * 24 * 3600_000).toISOString()
  const done = (days: number) => ({ state: "completed", updatedAt: ago(days) })

  test("no cards, or one open: never", () => {
    expect(looksFinished([], "Shipped", now)).toBe(false)
    expect(
      looksFinished(
        [done(9), { state: "working", updatedAt: ago(9) }],
        "Shipped",
        now
      )
    ).toBe(false)
    // In review, blocked and Not yet count as open.
    for (const state of ["input-required", "blocked", "submitted"])
      expect(looksFinished([{ state, updatedAt: ago(9) }], "", now)).toBe(false)
  })

  test("all done with a done-when: at once", () => {
    expect(looksFinished([done(0)], "Shipped", now)).toBe(true)
    expect(
      looksFinished(
        [done(0), { state: "canceled", updatedAt: ago(0) }],
        "x",
        now
      )
    ).toBe(true)
  })

  test("all done without one: once nothing moved for three days", () => {
    expect(looksFinished([done(5), done(2)], "", now)).toBe(false)
    expect(looksFinished([done(5), done(3)], "  ", now)).toBe(true)
    expect(
      looksFinished(
        [done(5), { state: "failed", updatedAt: ago(4) }],
        undefined,
        now
      )
    ).toBe(true)
  })
})

describe("finishing a board on the sample (card #690)", () => {
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }
  const space = "sample-board-mlg"
  const find = async (id: string) =>
    (await boards.list())
      .find((b) => b.boardId === space)
      ?.boards.find((x) => x.boardId === id)

  test("the first board and Personal's default never finish", async () => {
    await expect(boards.finish(space, space)).rejects.toThrow("first board")
    await expect(boards.finish(Personal, Personal)).rejects.toThrow(
      "default board"
    )
  })

  test("open cards refuse a finish; once moved off, it finishes and reopens", async () => {
    // A board of its own, so other tests' sample changes don't reach it.
    await boards.addToSpace(space, "Finish me", "fin", "Try finishing", "")
    const fin = `${space}-fin`
    const made = await cards.create({
      title: "Still open",
      board: space,
      spaceBoard: fin,
    })
    try {
      await expect(boards.finish(space, fin)).rejects.toThrow("1 open card")
      expect((await find(fin))?.finished).toBeFalsy()
      await cards.move(made.task.id, space)
      await boards.finish(space, fin)
      const x = await find(fin)
      expect(x?.finished).toBe(true)
      expect(x?.finishedAt).toBeTruthy()
      // Read-only: nothing moves onto it.
      await expect(cards.move(made.task.id, fin)).rejects.toThrow("is finished")
      await boards.reopen(space, fin)
      expect((await find(fin))?.finished).toBe(false)
      expect((await find(fin))?.mission).toBe("Try finishing")
    } finally {
      await cards.remove(made.task.id)
      await boards.deleteFromSpace(space, fin)
    }
  })

  test("a mission is written and cleared, 280 characters at most", async () => {
    const art = "sample-board-mlg-art"
    await boards.setMission(space, art, " Paint the boats ", "")
    expect((await find(art))?.mission).toBe("Paint the boats")
    await expect(
      boards.setMission(space, art, "x".repeat(281), "")
    ).rejects.toThrow("280")
    await boards.setMission(space, art, "", "")
    expect((await find(art))?.mission).toBe("")
  })
})
