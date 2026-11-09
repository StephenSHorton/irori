// The space switcher (card #393, docs/design/talk-channels.md §5): a space
// is a group of people that holds boards and channels. Personal is you and
// your agents (the private log); each shared space is a BoardSummary (its
// id is its first board's, docs/design/spaces.md §1) whose boards are
// `boards`. The pick is this device's view, not data: Talk and Boards show
// that space. The Dashboard ignores it and always shows every space (card
// #1001), so "all spaces" is no pick: the picker lists Personal and the
// shared spaces, and Personal is the default.

import { useCallback, useEffect, useState } from "react"

import type { BoardSummary, SpaceBoard } from "./boards"
import type { CardSummary } from "./cards"
import { previewFlag } from "./preview"

// Personal is the one pick that isn't a shared space. AllSpaces is no pick
// (card #1001): it is "every space", which the Dashboard and the helpers
// below take as their scope, and what an old saved pick of "All spaces"
// reads as before it falls back to Personal.
export const AllSpaces = ""
export const Personal = "personal"

export type Space = {
  // Personal, or a shared space's id.
  id: string
  name: string
  // The other people in it, for "with Blake".
  others: string[]
}

// spacesOf is the switcher's list: Personal, then each shared space by name.
export function spacesOf(boards: BoardSummary[]): Space[] {
  const shared = [...boards]
    .filter((b) => !b.leaving)
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((b) => ({
      id: b.boardId,
      name: b.name || b.tag || "(unnamed)",
      others: b.members.filter((m) => !m.me).map((m) => m.name),
    }))
  return [{ id: Personal, name: "Personal", others: [] }, ...shared]
}

// boardsOf is a space's boards, its default board first (card #558). A
// joined space that isn't ready yet can have an empty name on its first
// board, so each field falls back to the space's.
export function boardsOf(s: BoardSummary): SpaceBoard[] {
  if (!s.boards.length)
    return [{ boardId: s.boardId, name: s.name, tag: s.tag }]
  return s.boards.map((x, i) =>
    i === 0 ? { ...x, name: x.name || s.name, tag: x.tag || s.tag } : x
  )
}

// openBoards is a space's (or Personal's) boards that aren't closed, its
// first board always (card #690): what the pickers, tiles and counts
// offer. closedBoards is the rest, for the Closed boards fold. The first
// board is never closed, so it is never among them.
export function openBoards<T extends { closed?: boolean }>(list: T[]): T[] {
  return list.filter((x, i) => i === 0 || !x.closed)
}

export function closedBoards<T extends { closed?: boolean }>(list: T[]): T[] {
  return list.filter((x, i) => i > 0 && !!x.closed)
}

// withLine is "with Blake", "with Blake and Casey", "with Blake +2"; "" alone.
export function withLine(others: string[]): string {
  if (others.length === 0) return ""
  if (others.length === 1) return `with ${others[0]}`
  if (others.length === 2) return `with ${others[0]} and ${others[1]}`
  return `with ${others[0]} +${others.length - 1}`
}

// known keeps a pick that still names a space; a space the person left, a
// stale id, or the old "All spaces" pick (card #1001) falls back to Personal.
export function known(id: string, boards: BoardSummary[]): string {
  if (id === Personal) return id
  return id !== AllSpaces && boards.some((b) => b.boardId === id && !b.leaving)
    ? id
    : Personal
}

// cardsOnPersonal is Personal's cards on one of its boards (card #558). A
// card with no inner board is on the default board.
export function cardsOnPersonal(
  cards: CardSummary[],
  boardId: string
): CardSummary[] {
  return cards.filter((c) => !c.board && (c.spaceBoard || Personal) === boardId)
}

// cardInSpace is whether a card shows in the picked space: Personal's are
// the private cards, a shared space's its own cards (on any of its boards).
export function cardInSpace(
  c: { board?: string | null },
  space: string
): boolean {
  if (space === AllSpaces) return true
  if (space === Personal) return !c.board
  return c.board === space
}

// convInSpace is whether a channel, DM or thread shows in the picked space
// (card #461, #558): Personal's are your own channels and threads and every
// DM, a shared space's its own (DMs are you and your agents, so they stay
// Personal's, as private cards do). A DM with another person shows under
// every pick (card #538): it lives in no space the switcher lists.
export function convInSpace(
  c: {
    spaceId?: string
    kind?: string
    members?: readonly { kind: string; key: string }[]
  },
  space: string
): boolean {
  if (space === AllSpaces) return true
  // As dmPerson (lib/conversations) tells one: people only, one of them
  // another person by `person:<id>`.
  if (
    c.kind === "dm" &&
    c.members?.every((m) => m.kind === "person") &&
    c.members.some((m) => m.key.startsWith("person:"))
  )
    return true
  if (space === Personal) return !c.spaceId
  return c.spaceId === space
}

// boardsInSpace is the spaces whose boards the picked space shows: none under
// Personal (its boards are the private ones), else that one.
export function boardsInSpace(
  boards: BoardSummary[],
  space: string
): BoardSummary[] {
  if (space === AllSpaces) return boards
  if (space === Personal) return []
  return boards.filter((b) => b.boardId === space)
}

// threadsSpace is the board id a new thread or channel starts on: a shared
// space's own, else the private log's ("").
export function threadsSpace(space: string): string {
  return space === AllSpaces || space === Personal ? "" : space
}

const spaceKey = "irori.space"
const SpaceEvent = "irori:space"

function readSpace(): string {
  // ?space=<id|personal> starts the browser preview on a space.
  const flag = previewFlag("space")
  if (flag !== null) return flag
  try {
    // No saved pick, or an old "All spaces" one: Personal (card #1001).
    return localStorage.getItem(spaceKey) || Personal
  } catch {
    return Personal
  }
}

// forgetSpace drops the saved pick (card #1096): a Reset moves this
// device's spaces aside, and a pick of one would leave the window on a
// space it no longer has, since an empty list can't tell it apart from one
// still loading. The next start is on Personal.
export function forgetSpace(): void {
  try {
    localStorage.removeItem(spaceKey)
  } catch {
    // A blocked storage saved no pick either.
  }
}

// useSpace is the picked space on this device, kept between starts; every
// user of it follows a pick made anywhere in the window.
export function useSpace(
  boards: BoardSummary[]
): [string, (id: string) => void] {
  const [raw, setRaw] = useState(readSpace)
  useEffect(() => {
    const on = () => setRaw(readSpace())
    window.addEventListener(SpaceEvent, on)
    return () => window.removeEventListener(SpaceEvent, on)
  }, [])
  const pick = useCallback((id: string) => {
    try {
      localStorage.setItem(spaceKey, id || Personal)
    } catch {
      // A blocked storage keeps it for this run only.
    }
    setRaw(id || Personal)
    window.dispatchEvent(new Event(SpaceEvent))
  }, [])
  // Until the boards load, a shared pick can't be checked: keep it.
  return [
    boards.length === 0 && raw !== AllSpaces ? raw : known(raw, boards),
    pick,
  ]
}

// waitingSpaces is the picker rows that have something waiting on the person
// (card #1001): Talk's count covers only the picked space, so what waits in
// another one shows as a dot on its row. waits says whether a thread, a
// conversation or a card (each as the pick would list it) is waiting.
export function waitingSpaces(
  spaces: { id: string }[],
  waits: (space: string) => boolean
): Set<string> {
  return new Set(spaces.filter((s) => waits(s.id)).map((s) => s.id))
}
