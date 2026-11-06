import { describe, expect, test } from "bun:test"

import { BoardSummary, CardSummary } from "@/lib/models"
import { cardsOnBoard } from "./space-boards"

// Card #392, #558: a board's list shows its cards, by the board's id; the
// space's first board also keeps the cards with no inner board (those
// made before spaces).
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
  const card = (id: string, spaceBoard?: string, tag?: string) =>
    CardSummary({ id, tag, spaceBoard, board: "s" })
  const cards = [card("a", "s", "wind"), card("b", "n", "net"), card("c")]
  test("one board", () => {
    expect(cardsOnBoard(b, cards, "n").map((c) => c.id)).toEqual(["b"])
    expect(cardsOnBoard(b, cards, "s").map((c) => c.id)).toEqual(["a", "c"])
  })
  test("another space's cards aren't on it", () => {
    expect(cardsOnBoard(b, [CardSummary({ id: "x", board: "t", spaceBoard: "n" })], "n")).toEqual([])
  })
  test("two boards with the same tag keep their own cards", () => {
    const twin = BoardSummary({
      ...b,
      boards: [...b.boards, { boardId: "m", name: "Netcode (old)", tag: "net" }],
    })
    const both = [card("b", "n", "net"), card("d", "m", "net")]
    expect(cardsOnBoard(twin, both, "n").map((c) => c.id)).toEqual(["b"])
    expect(cardsOnBoard(twin, both, "m").map((c) => c.id)).toEqual(["d"])
    expect(cardsOnBoard(twin, both, "s")).toEqual([])
  })
})
