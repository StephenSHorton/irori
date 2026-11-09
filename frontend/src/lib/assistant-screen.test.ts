import { describe, expect, test } from "bun:test"

import {
  lookNow,
  openOn,
  pageName,
  screenLabel,
  snapshotText,
} from "./assistant-screen"
import type { BoardSummary } from "./boards"
import { sectionFromHash } from "./nav"

describe("the screen snapshot (card #1009)", () => {
  test("names the page and what is open on it", async () => {
    const l = await lookNow("#board/personal", async () => "")
    expect(l).toEqual({
      page: "Boards",
      open: "Personal (private)",
      view: "",
    })
    expect(pageName(sectionFromHash(""))).toBe("Home")
    expect(openOn(sectionFromHash("#storage/files"))).toBe("the files tab")
  })

  test("puts the card in view on its own line", async () => {
    const l = await lookNow("#card/abc", async () => "card #12 “Fix it”")
    const text = snapshotText(l)
    expect(text).toBe(
      "Page: Home\nOpen: cards from every space\nIn view: card #12 “Fix it”"
    )
    expect(screenLabel(text)).toBe("Home, card #12 “Fix it”")
  })

  test("a name that can't be read leaves the card out, not the ask", async () => {
    const l = await lookNow("#card/abc", async () => {
      throw new Error("no")
    })
    expect(l.view).toBe("")
    expect(screenLabel(snapshotText(l))).toBe("Home")
  })

  test("names a shared space and its boards' tags (card #1198)", () => {
    const space = {
      boardId: "sp1",
      name: "mlg farming",
      tag: "mlg",
      boards: [
        { boardId: "sp1", name: "", tag: "mlg" },
        { boardId: "b2", name: "Art", tag: "art" },
      ],
    } as unknown as BoardSummary
    expect(openOn(sectionFromHash("#board/sp1"), [space])).toBe(
      "the boards of shared space “mlg farming” (tags mlg, art)"
    )
    expect(openOn(sectionFromHash("#board/sp1/b2"), [space])).toBe(
      "board art “Art” in shared space “mlg farming”"
    )
    // A space it doesn't know yet is said by its id.
    expect(openOn(sectionFromHash("#board/zz"), [])).toBe(
      "the boards of space zz"
    )
  })
})
