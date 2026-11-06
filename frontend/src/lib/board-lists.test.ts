import { describe, expect, test } from "bun:test"

import { boards } from "./boards"
import { cards } from "./cards"
import { Personal } from "./spaces"

// Card #558: New card from a board's list goes on that board, on the
// sample as in the Core, and Personal's boards are the person's own.
describe("a board's list", () => {
  // The sample runs outside the host; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("New card on an inner board goes on it, with that board's number run", async () => {
    const made = await cards.create({
      title: "from the Netcode list",
      objective: "",
      doneWhen: "",
      repoKey: "",
      assignTo: "",
      board: "sample-board-mlg",
      spaceBoard: "sample-board-mlg-net",
      files: [],
    })
    const c = await cards.detail(made.task.id)
    expect(c.tag).toBe("net")
    expect(c.spaceBoard).toBe("sample-board-mlg-net")
    expect(c.board).toBe("sample-board-mlg")
  })

  test("New card on a Personal board stays private, with the board's tag", async () => {
    const made = await cards.create({
      title: "from the House list",
      objective: "",
      doneWhen: "",
      repoKey: "",
      assignTo: "",
      board: "",
      spaceBoard: "sample-personal-home",
      files: [],
    })
    const c = await cards.detail(made.task.id)
    expect(c.tag).toBe("home")
    expect(c.onBoard ?? null).toBeNull()
  })

  test("a board renames apart from its space", async () => {
    const before = (await boards.list()).find(
      (b) => b.boardId === "sample-board-mlg"
    )!
    const s = await boards.renameInSpace(
      "sample-board-mlg",
      "sample-board-mlg-net",
      "Sockets"
    )
    expect(s.boards.find((x) => x.tag === "net")?.name).toBe("Sockets")
    expect(s.name).toBe(before.name)
  })

  test("Personal lists, adds and renames its boards, default first", async () => {
    const list = await boards.addToPersonal("Garden", "garden")
    expect(list[0].boardId).toBe(Personal)
    expect(list.at(-1)?.tag).toBe("garden")
    const named = await boards.renameInPersonal(list.at(-1)!.boardId, "Yard")
    expect(named.at(-1)?.name).toBe("Yard")
    await expect(boards.addToPersonal("Again", "garden")).rejects.toThrow(
      "already"
    )
  })
})
