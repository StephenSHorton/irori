import { describe, expect, test } from "bun:test"

import { BoardSummary, CardSummary } from "@/lib/models"
import { cardsOnBoard } from "./space-boards"

// Card #392: a board's list shows its cards; the space's own board also
// keeps the cards made before spaces (they carry its tag).
describe("cardsOnBoard", () => {
  const b = BoardSummary({
    boardId: "s",
    name: "Wind Waker Multiplayer",
    tag: "wind",
    boards: [
      { boardId: "s", name: "Wind Waker Multiplayer", tag: "wind" },
      { boardId: "n", name: "Netcode", tag: "net" },
    ],
  })
  const card = (id: string, tag?: string) => CardSummary({ id, tag, board: "s" })
  const cards = [card("a", "wind"), card("b", "net"), card("c")]
  test("one board", () => {
    expect(cardsOnBoard(b, cards, "net").map((c) => c.id)).toEqual(["b"])
    expect(cardsOnBoard(b, cards, "wind").map((c) => c.id)).toEqual(["a", "c"])
  })
  test("another space's cards aren't on it", () => {
    expect(cardsOnBoard(b, [CardSummary({ id: "x", board: "t", tag: "net" })], "net")).toEqual([])
  })
})
