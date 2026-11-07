import { describe, expect, test } from "bun:test"

import {
  InviteState,
  InviteStatus,
  boards,
  boardsNow,
  inviteOpen,
  MaxBoardDescription,
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

describe("closing a board on the sample (card #690)", () => {
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }
  const space = "sample-board-mlg"
  const find = async (id: string) =>
    (await boards.list())
      .find((b) => b.boardId === space)
      ?.boards.find((x) => x.boardId === id)
  const mine = async (id: string) =>
    (await boards.personal()).find((x) => x.boardId === id)

  test("the first board and Personal's default are never closed", async () => {
    await expect(boards.close(space, space, "keep")).rejects.toThrow(
      "first board"
    )
    await expect(boards.close(Personal, Personal, "keep")).rejects.toThrow(
      "default board"
    )
  })

  test("keep refuses while a card is open, naming it; with none open it closes and reopens", async () => {
    // A board of its own, so other tests' sample changes don't reach it.
    await boards.addToSpace(space, "Keep me", "keep", "Try closing")
    const id = `${space}-keep`
    const made = await cards.create({
      title: "Still open",
      board: space,
      spaceBoard: id,
    })
    try {
      await expect(boards.close(space, id, "keep")).rejects.toThrow(
        "1 open card (keep#"
      )
      expect((await find(id))?.closed).toBeFalsy()
      await cards.move(made.task.id, space)
      await boards.close(space, id, "keep")
      const x = await find(id)
      expect(x?.closed).toBe(true)
      expect(x?.closedAt).toBeTruthy()
      // Read-only: nothing moves onto it, and its description stays.
      await expect(cards.move(made.task.id, id)).rejects.toThrow("is closed")
      await expect(
        boards.setDescription(space, id, "Something else")
      ).rejects.toThrow("is closed")
      await boards.reopen(space, id)
      expect((await find(id))?.closed).toBe(false)
      expect((await find(id))?.description).toBe("Try closing")
      await expect(boards.reopen(space, id)).rejects.toThrow("already open")
    } finally {
      await cards.remove(made.task.id)
      await boards.deleteFromSpace(space, id)
    }
  })

  test("move takes the open cards to another open board, then closes", async () => {
    await boards.addToSpace(space, "Move me", "mv", "")
    const id = `${space}-mv`
    const made = await cards.create({
      title: "Goes elsewhere",
      board: space,
      spaceBoard: id,
    })
    try {
      // Not onto itself, nor a closed board, nor with nowhere named.
      await expect(boards.close(space, id, "move", id)).rejects.toThrow(
        "another board"
      )
      await expect(
        boards.close(space, id, "move", "sample-board-mlg-proto")
      ).rejects.toThrow("is closed")
      await expect(boards.close(space, id, "move", "")).rejects.toThrow(
        "pick a board"
      )
      expect((await find(id))?.closed).toBeFalsy()
      await boards.close(space, id, "move", "sample-board-mlg-net")
      expect((await find(id))?.closed).toBe(true)
      const d = await cards.detail(made.task.id)
      expect(d.spaceBoard).toBe("sample-board-mlg-net")
      expect(d.tag).toBe("net")
      expect(d.state).not.toBe("canceled")
    } finally {
      await cards.remove(made.task.id)
      await boards.reopen(space, id)
      await boards.deleteFromSpace(space, id)
    }
  })

  test("cancel ends the open cards with a note, then closes", async () => {
    const list = await boards.addToPersonal("Cancel me", "cx", "")
    const id = list.find((x) => x.tag === "cx")!.boardId
    const made = await cards.create({ title: "Never mind", spaceBoard: id })
    try {
      await boards.close(Personal, id, "cancel")
      expect((await mine(id))?.closed).toBe(true)
      const d = await cards.detail(made.task.id)
      expect(d.state).toBe("canceled")
      expect(d.note).toBe("its board was closed")
      await boards.reopen(Personal, id)
      expect((await mine(id))?.closed).toBe(false)
    } finally {
      await cards.remove(made.task.id)
      await boards.deleteFromPersonal(id)
    }
  })

  test("Personal's move goes to its default board by its id", async () => {
    const list = await boards.addToPersonal("Move home", "mh", "")
    const id = list.find((x) => x.tag === "mh")!.boardId
    const made = await cards.create({ title: "Back home", spaceBoard: id })
    try {
      await boards.close(Personal, id, "move", Personal)
      expect((await mine(id))?.closed).toBe(true)
      expect((await cards.detail(made.task.id)).spaceBoard).toBe(Personal)
    } finally {
      await cards.remove(made.task.id)
      await boards.deleteFromPersonal(id)
    }
  })

  test("a description is written and cleared, 2000 characters at most", async () => {
    const art = "sample-board-mlg-art"
    expect(MaxBoardDescription).toBe(2000)
    await boards.setDescription(space, art, " Paint the boats ")
    expect((await find(art))?.description).toBe("Paint the boats")
    await boards.setDescription(space, art, "x".repeat(2000))
    await expect(
      boards.setDescription(space, art, "x".repeat(2001))
    ).rejects.toThrow("2000")
    await boards.setDescription(space, art, "")
    expect((await find(art))?.description).toBeFalsy()
    await expect(
      boards.addToSpace(space, "Too long", "toolong", "y".repeat(2001))
    ).rejects.toThrow("2000")
  })
})
