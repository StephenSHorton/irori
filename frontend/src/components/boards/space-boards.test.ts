import { describe, expect, test } from "bun:test"

import { BoardSummary, CardSummary } from "@/lib/models"
import { cardsOnBoard } from "./board-page"

// Card #392: a space's chips show one board's cards; the space's own board
// also keeps the cards made before spaces (they carry its tag).
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
  const card = (id: string, tag?: string) => CardSummary({ id, tag })
  const cards = [card("a", "wind"), card("b", "net"), card("c")]
  test("all boards", () => {
    expect(cardsOnBoard(b, cards, "").map((c) => c.id)).toEqual(["a", "b", "c"])
  })
  test("one board", () => {
    expect(cardsOnBoard(b, cards, "net").map((c) => c.id)).toEqual(["b"])
    expect(cardsOnBoard(b, cards, "wind").map((c) => c.id)).toEqual(["a", "c"])
  })
})
