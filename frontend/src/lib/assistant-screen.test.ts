import { describe, expect, test } from "bun:test"

import {
  lookNow,
  openOn,
  pageName,
  screenLabel,
  snapshotText,
} from "./assistant-screen"
import { sectionFromHash } from "./nav"

describe("the screen snapshot (card #1009)", () => {
  test("names the page and what is open on it", async () => {
    const l = await lookNow("#board/personal", async () => "")
    expect(l).toEqual({
      page: "Boards",
      open: "the boards of space personal",
      view: "",
    })
    expect(pageName(sectionFromHash(""))).toBe("Home")
    expect(openOn(sectionFromHash("#storage/files"))).toBe("the files tab")
  })

  test("puts the card in view on its own line", async () => {
    const l = await lookNow("#card/abc", async () => "card #12 “Fix it”")
    const text = snapshotText(l)
    expect(text).toBe(
      "Page: Home\nOpen: your own cards\nIn view: card #12 “Fix it”"
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
})
