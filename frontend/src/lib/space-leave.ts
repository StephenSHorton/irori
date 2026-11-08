// Switching spaces moves you off what the space you left was showing (card
// #994): a thread, channel or DM goes to Talk's landing, a card closes to
// the section behind it. It runs on a change of the pick, wherever that
// came from, and never at render: a notification or a #ref link to another
// space's thread still opens (card #558).
//
// A pick can change before the lists that say what is in it have loaded, so
// the decision waits for them: nothing moves while membership is unknown.

import { boardsFor, onBoards, type Section } from "./nav"
import { cardInSpace, convInSpace } from "./spaces"

type Conv = Parameters<typeof convInSpace>[0]

// SpaceView is what the window shows and what it knows, at one render.
export type SpaceView = {
  space: string
  // The pick is one the boards list holds (or Personal). useSpace keeps a
  // shared id it can't check while the list is empty, even a space since
  // left, so a loaded list isn't enough to trust it.
  spaceKnown: boolean
  section: Section
  // The open card's id, "" for none.
  open: string
  // The open card from the cards list, else its detail (a closed card);
  // null until one has it.
  card: { board?: string | null } | null
  // The open card could not be read, so it will never be known.
  cardFailed: boolean
  threads: (Conv & { id: string })[]
  conversations: (Conv & { convId: string })[]
  threadsLoading: boolean
  conversationsLoading: boolean
}

// LeaveState is what the effect keeps between renders: the last pick, and a
// switch still waiting on its lists (from: the pick it left; shown: what it
// would move off, so going somewhere else meanwhile calls it off).
export type LeaveState = {
  space: string
  pending: { from: string; shown: string } | null
}

export function startLeave(space: string): LeaveState {
  return { space, pending: null }
}

const Wait = Symbol("wait")

// shownKey names what a switch could move off; "" when nothing can be.
function shownKey(v: SpaceView): string {
  if (v.open) return `card:${v.open}`
  return v.section.kind === "talk" && v.section.id ? `talk:${v.section.id}` : ""
}

// talkInSpace says whether conversation id is in the pick. One neither list
// has, once both have loaded, stays: nothing says it belongs elsewhere.
function talkInSpace(v: SpaceView, id: string): boolean | typeof Wait {
  const c =
    v.threads.find((t) => t.id === id) ??
    v.conversations.find((x) => x.convId === id)
  if (c) return convInSpace(c, v.space)
  return v.threadsLoading || v.conversationsLoading ? Wait : true
}

// leaveTarget is where the window goes: a section, null to stay, or Wait.
function leaveTarget(v: SpaceView): Section | null | typeof Wait {
  const { section } = v
  const talk = section.kind === "talk" && section.id ? section.id : ""
  if (!v.open) {
    if (!talk) return null
    const here = talkInSpace(v, talk)
    if (here === Wait) return Wait
    return here ? null : { kind: "talk", id: "" }
  }
  // The Dashboard shows every space (card #1001): a card opened over it
  // belongs there whatever the pick.
  if (section.kind === "home") return null
  if (!v.card) return v.cardFailed ? null : Wait
  if (cardInSpace(v.card, v.space)) return null
  // Boards follow the pick; Talk's thread may have been left too.
  if (onBoards(section)) return boardsFor(v.space)
  if (!talk) return section
  const here = talkInSpace(v, talk)
  if (here === Wait) return Wait
  return here ? section : { kind: "talk", id: "" }
}

// stepLeave is one render's decision. changed: the pick changed just now.
// to: where to move, null to stay.
export function stepLeave(
  state: LeaveState,
  v: SpaceView
): { state: LeaveState; changed: boolean; to: Section | null } {
  const changed = v.space !== state.space
  const shown = shownKey(v)
  let pending = state.pending?.shown === shown ? state.pending : null
  if (changed && !pending && shown) pending = { from: state.space, shown }
  const done = { state: { space: v.space, pending: null }, changed, to: null }
  if (!pending) return done
  // A pick undone before it could act is no switch at all.
  if (v.space === pending.from) return done
  const to = v.spaceKnown ? leaveTarget(v) : Wait
  if (to === Wait)
    return { state: { space: v.space, pending }, changed, to: null }
  return { ...done, to }
}
