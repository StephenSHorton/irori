// Which board New card puts a card on (card #558). A space offers its
// boards; with no space the card is private, so Personal's boards are
// offered. The board whose list is open is the default, else the first.

import type { BoardSummary, SpaceBoard } from "./boards"
import { boardsOf, openBoards } from "./spaces"

export type BoardTarget = {
  // The boards to pick from, in the space's order.
  choices: SpaceBoard[]
  // The board the card goes on.
  on: SpaceBoard | undefined
  // What cards.create gets: the space ("" or none for Personal) and the
  // board inside it.
  board?: string
  spaceBoard?: string
}

// boardTarget is the boards on offer and the one the card goes on: the
// one picked, else the open list's, else the first. A pick that isn't
// among the choices (another space's) is ignored. A closed board (card
// #690) is never on offer: from one's list, the card goes on the first.
export function boardTarget(
  space: BoardSummary | undefined,
  personal: SpaceBoard[],
  list: SpaceBoard | undefined,
  picked: string | null
): BoardTarget {
  const choices = openBoards(space ? boardsOf(space) : personal)
  const find = (id: string | null | undefined) =>
    id ? choices.find((x) => x.boardId === id) : undefined
  const on =
    find(picked) ??
    find(list?.boardId) ??
    choices[0] ??
    (list?.closed ? undefined : list)
  return { choices, on, board: space?.boardId, spaceBoard: on?.boardId }
}
