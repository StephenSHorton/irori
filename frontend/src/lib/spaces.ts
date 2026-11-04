// The space switcher (card #393, docs/design/talk-channels.md §5): a space
// is a group of people that holds boards and channels. Personal is you and
// your agents (the private log); each shared space is a BoardSummary (its
// id is its first board's, docs/design/spaces.md §1) whose boards are
// `boards`. The pick is this device's view, not data: Home, Talk and
// Boards then show that space, and All spaces (the default) shows
// everything, Needs you across all of it.

import { useCallback, useEffect, useState } from "react"

import type { BoardSummary } from "./boards"
import type { CardSummary } from "./cards"
import { previewFlag } from "./preview"

// AllSpaces and Personal are the two picks that aren't a shared space.
export const AllSpaces = ""
export const Personal = "personal"

export type Space = {
  // AllSpaces, Personal, or a shared space's id.
  id: string
  name: string
  // The other people in it, for "with Blake".
  others: string[]
}

// spacesOf is the switcher's list: All spaces, Personal, then each shared
// space by name.
export function spacesOf(boards: BoardSummary[]): Space[] {
  const shared = [...boards]
    .filter((b) => !b.leaving)
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((b) => ({
      id: b.boardId,
      name: b.name || b.tag || "(unnamed)",
      others: b.members.filter((m) => !m.me).map((m) => m.name),
    }))
  return [
    { id: AllSpaces, name: "All spaces", others: [] },
    { id: Personal, name: "Personal", others: [] },
    ...shared,
  ]
}

// withLine is "with Blake", "with Blake and Casey", "with Blake +2"; "" alone.
export function withLine(others: string[]): string {
  if (others.length === 0) return ""
  if (others.length === 1) return `with ${others[0]}`
  if (others.length === 2) return `with ${others[0]} and ${others[1]}`
  return `with ${others[0]} +${others.length - 1}`
}

// known keeps a pick that still names a space; a space the person left
// (or a stale id) falls back to All spaces.
export function known(id: string, boards: BoardSummary[]): string {
  if (id === AllSpaces || id === Personal) return id
  return boards.some((b) => b.boardId === id && !b.leaving) ? id : AllSpaces
}

// cardInSpace is whether a card shows in the picked space: Personal's are
// the private cards, a shared space's its own cards (on any of its boards).
export function cardInSpace(c: CardSummary, space: string): boolean {
  if (space === AllSpaces) return true
  if (space === Personal) return !c.board
  return c.board === space
}

// convInSpace is whether a channel or DM shows in the picked space (card
// #461): Personal's are your own channels and every DM, a shared space's
// its own channels (DMs are you and your agents, so they stay Personal's,
// as private cards do).
export function convInSpace(c: { spaceId?: string }, space: string): boolean {
  if (space === AllSpaces) return true
  if (space === Personal) return !c.spaceId
  return c.spaceId === space
}

// boardsInSpace is the Boards page in the picked space: every shared space
// under All spaces, none under Personal, else that one.
export function boardsInSpace(boards: BoardSummary[], space: string): BoardSummary[] {
  if (space === AllSpaces) return boards
  if (space === Personal) return []
  return boards.filter((b) => b.boardId === space)
}

// threadsSpace is the board id Talk's threads come from: a shared space's
// own, else the private log's ("").
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
    return localStorage.getItem(spaceKey) ?? AllSpaces
  } catch {
    return AllSpaces
  }
}

// useSpace is the picked space on this device, kept between starts; every
// user of it follows a pick made anywhere in the window.
export function useSpace(boards: BoardSummary[]): [string, (id: string) => void] {
  const [raw, setRaw] = useState(readSpace)
  useEffect(() => {
    const on = () => setRaw(readSpace())
    window.addEventListener(SpaceEvent, on)
    return () => window.removeEventListener(SpaceEvent, on)
  }, [])
  const pick = useCallback((id: string) => {
    try {
      if (id === AllSpaces) localStorage.removeItem(spaceKey)
      else localStorage.setItem(spaceKey, id)
    } catch {
      // A blocked storage keeps it for this run only.
    }
    setRaw(id)
    window.dispatchEvent(new Event(SpaceEvent))
  }, [])
  // Until the boards load, a shared pick can't be checked: keep it.
  return [boards.length === 0 && raw !== Personal ? raw : known(raw, boards), pick]
}
