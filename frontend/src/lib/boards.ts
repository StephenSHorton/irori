// Boards gateway (docs/design/v0.5-shared-boards.md §8, step 4): the
// boards on this device, New board, rename, Leave and Forget, and Share to
// board and Withdraw on a card's page. Like cards.ts, components import from
// here, never from src/gen. In a plain browser it runs on two sample
// boards, and sharing moves a sample card onto one.

import { useCallback, useEffect, useState, useSyncExternalStore } from "react"

import { BoardsService, Events } from "./host"
import {
  BoardSettings,
  BoardSummary,
  InviteStatus,
  SharePreview,
  type SummaryMember,
} from "./models"
import {
  sampleBoardGone,
  sampleCancel,
  sampleMoveBoard,
  sampleOpenOn,
  sampleShare,
  sampleTransferOwner,
  sampleWithdraw,
  samplePreview,
  setSampleBoards,
  setSamplePersonal,
} from "./cards"
import { inWails, relativeTime } from "./irori"
import { Personal } from "./spaces"
import { errorText } from "./talk"

export { BoardSettings, BoardSummary, InviteStatus, SharePreview }
export type { SummaryMember }
export type SpaceBoard = BoardSummary["boards"][number]

// presenceLine says how a member is on a board (card #371): "online",
// "seen 5m ago" (heard within ten minutes) or "offline · seen 2h ago",
// then how many of their agents on the board are working. An
// offline person's agent counts are left out: only their own device writes
// an agent's status, so the last ones it wrote may be stale.
export function presenceLine(m: SummaryMember, now: Date): string {
  const seen = m.lastSeen ? new Date(m.lastSeen) : null
  const at = seen && !Number.isNaN(seen.getTime()) ? seen : null
  if (m.presence !== "online" && m.presence !== "recent")
    return at ? `offline · seen ${relativeTime(at, now)}` : "offline"
  const head =
    m.presence === "online"
      ? "online"
      : at
        ? `seen ${relativeTime(at, now)}`
        : "seen recently"
  // Card #512: no "free" count; idle agents just aren't working.
  if (m.working > 0) return `${head} · ${m.working} working`
  return m.free > 0 ? head : `${head} · no agents`
}

// PresenceReload is how often a board page reads presence again while
// it is open: connections come and go without a boards:changed.
export const PresenceReload = 20_000

// EventBoardsChanged mirrors the Go constant in boards_service.go: a board
// opened, went or was renamed.
export const EventBoardsChanged = "boards:changed"

// EventInviteChanged mirrors crates/window invites.rs: every change of the
// open invite.
export const EventInviteChanged = "boards:invite"

// InviteState mirrors the invite flow's states; "" is no invite open.
export const InviteState = {
  Waiting: "waiting",
  Words: "words",
  Joining: "joining",
  Done: "done",
  Failed: "failed",
} as const

// inviteOpen reports whether an invite is still in progress.
export function inviteOpen(s: InviteStatus | null): boolean {
  return (
    !!s &&
    (s.state === InviteState.Waiting ||
      s.state === InviteState.Words ||
      s.state === InviteState.Joining)
  )
}

// Receive presets (design §7.2): what other people on a board may do to
// your agents. Their comments always arrive at the next turn; the preset
// decides wakes and rows to your bypass sessions. Open is the default.
export const ReceivePresets = [
  {
    value: "open",
    label: "Open",
    hint: "They can wake your agents, and their rows reach your bypass sessions.",
  },
  {
    value: "members",
    label: "Members wake",
    hint: "Only the people you pick wake your agents; the rest arrive at the next turn.",
  },
  {
    value: "ask",
    label: "Ask me",
    hint: "Wake requests wait for you to release them.",
  },
  {
    value: "closed",
    label: "Closed",
    hint: "Nothing wakes your agents; everything arrives at the next turn.",
  },
] as const

// Autonomy presets (design §7.3): what your agents may do on a board
// without asking. Trusted is the default.
export const AutonomyPresets = [
  {
    value: "ask",
    label: "Ask me",
    hint: "New cards in the space ask you first; your private cards aren't shared.",
  },
  {
    value: "trusted",
    label: "Trusted",
    hint: "They create cards in the space; sharing a private card asks you.",
  },
  {
    value: "autonomous",
    label: "Autonomous",
    hint: "They also share private cards (after scrub) and mirror all their leases.",
  },
] as const

// ThreadsModes are where your threads on a shared board live (card #185).
export const ThreadsModes = [
  {
    value: "shared",
    label: "Shared",
    hint: "Every member and their agents see your threads in this space.",
  },
  {
    value: "private",
    label: "Private",
    hint: "Only you and your own agents see them; they never reach other members' devices.",
  },
] as const

// MaxBoardName mirrors identity.MaxBoardName.
export const MaxBoardName = 60

// MaxBoardTag mirrors identity.MAX_BOARD_TAG (card #590): a new board's
// tag is this long at most, so refs like agility#1234 stay short.
export const MaxBoardTag = 8

// validTag is identity.valid_board_tag: a lowercase letter, then
// lowercase letters, digits or '-', MaxBoardTag at most.
export function validTag(tag: string): boolean {
  return tag.length <= MaxBoardTag && /^[a-z][a-z0-9-]*$/.test(tag)
}

// suggestTag is a tag from a board's name: its first word, lowercased,
// with only what a tag may hold.
export function suggestTag(name: string): string {
  const word = name
    .toLowerCase()
    .replace(/[^a-z0-9-\s]/g, "")
    .trim()
    .split(/\s+/)[0]
  const t = (word ?? "").replace(/^[^a-z]+/, "").slice(0, MaxBoardTag)
  return validTag(t) ? t : ""
}

// MaxBoardDescription mirrors core's MAX_BOARD_DESCRIPTION (card #690): a
// board's description is this many characters at most.
export const MaxBoardDescription = 2000

// ClosedBoardNote mirrors core's CLOSED_BOARD_NOTE: the note on a card
// canceled because its board closed.
export const ClosedBoardNote = "its board was closed"

// OpenCards is what Close board does with the board's open cards (card
// #690): keep refuses while any is open, move takes them (each with its
// family) to another open board, cancel ends them.
export type OpenCards = "keep" | "move" | "cancel"

// ---- sample mode ------------------------------------------------------------

// sampleAvatar is a friend's picture in the samples (card #393).
const sampleAvatar =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAE7klEQVR42q3U0YqryAJG4XnOAUEQBEEQhEBhi9gidpB0kHSQRMRIoaGIoYihaF9hPcF5mHN1hL6dYU/ObFjX/4dYVX/8KbAEtsARuAJP4AsCQSjYCITgTZAI3gW54ENQCj4FleBLUAvOglZwEUjBKFCCSaAFs8AIFsEff0ZYEXaEE+FGeBF+RBARRmwiRMRbRBLxHpFHfESUEZ8RVcRXRB1xjmgjLhEyYoxQEVOEjpgjTMQSrUCMFWPHODFujBfjxwQxYcwmRsS8xSQx7zF5zEdMGfMZU8V8xdQx55g25hIjY8YYFTPF6Jg5xsQs8QokWAl2gpPgJngJfkKQECZsEkTCW0KS8J6QJ3wklAmfCVXCV0KdcE5oEy4JMmFMUAlTgk6YE0zCkqxAipVipzgpboqX4qcEKWHKJkWkvKUkKe8pecpHSpnymVKlfKXUKeeUNuWSIlPGFJUypeiUOcWkLOkKZFgZdoaT4WZ4GX5GkBFmbDJExlvGf/9z+8vqjHNGm3HJkBljhsqYMnTGnGEylmwFcqwcO8fJcXO8HD8nyAlzNjki/9v1n845bc4lR+aMOSpnytE5c47JWfIVKLAK7AKnwC3wCvyCoCAs2BS/WP+pLbgUyIKxQBVMBbpgLjAFS7ECW6wt9hZni7vF2+JvCbaE23+0/tNli9wyblFbpi16y7zFbFm2K1BildglTolb4pX4JUFJWL4ClMiSsUSVTCW6ZC4xJUu5AjusHfYOZ4e7w9vh7wh2L6z/JHeMO9SOaYfeMe8wO5bdCuyx9th7nD3uHm+PvyfYvw7sGfeoPdMevWfeY/Ys+xWosCrsCqfCrfAq/Iqgeh2oGCtUxVShK+YKU7FUK3DAOmAfcA64B7wD/oHg8DpwYDygDkwH9IH5gDmwHFbgiHXEPuIccY94R/wjwfF14Mh4RB2Zjugj8xFzZDmuQI1VY9c4NW6NV+PXBDVh/copqpE1Y42qmWp0zVxjapZ6BU5YJ+wTzgn3hHfCPxGcCE+vACfkifGEOjGd0CfmE+bEclqBBqvBbnAa3AavwW8IGsKGTfPPbnLDpUE2jA2qYWrQDXODaViaFWixWuwWp8Vt8Vr8lqAlbNm0iPZXb1FL23JpkS1ji2qZWnTL3GJalnYFOqwOu8PpcDu8Dr8j6Ag7Nh2i4637+9e049zRdlw6ZMfYoTqmDt0xd5iOpVuBHqvH7nF63B6vx+8JesKeTY/oeetJet578p6PnrLns6fq+eqpe849bc+lR/aMPapn6tE9c4/pWfoVkFgSW+JIXIkn8SWBJJRsJELyJknkX3zEl6SWnCWt5CKRklGiJJNES2aJkSxyBQasAXvAGXAHvAF/IBgIBzYDYvjVPxhoBy4DcmAcUAPTgB6YB8zAMqzAFeuKfcW54l7xrvhXgivh9ZVjekVeGa+oK9MVfWW+Yq4s1xVQWApb4ShchafwFYF6/SYrRoVSTAqtmBVGsagVuGHdsG84N9wb3g3/9vL6T+MNdWO6oW/MN8yN5bYCd6w79h3njnvHu/+f6z+pO9MdfWe+Y+4s9xXQWBpb42hcjaf/HaCZNFoza4xm0SvwwHpgP3AeuI9/tf7T9EA/mB+YB8tjBZ5YT+wnzhP3+TuAJ/rJ/MQ8WZ4rYLAMtsExv2H9J22YDcawmBX4xvrG/sb5/n3AN/M35pvlm/8BIz9ypdG5tDYAAAAASUVORK5CYII="

let sampleBoards: BoardSummary[] | null = null
const sampleListeners = new Set<() => void>()

function boardsState(): BoardSummary[] {
  sampleBoards ??= [
    BoardSummary({
      boardId: "sample-board-mlg",
      name: "MLG",
      tag: "mlg",
      // Shared with Blake (card #268): each card shows whose it is.
      people: ["Stephen", "Blake", "Casey"],
      members: [
        {
          personId: "sample-me",
          name: "Stephen",
          me: true,
          presence: "online",
          working: 2,
          free: 1,
        },
        {
          personId: "sample-person-0",
          name: "Blake",
          me: false,
          presence: "recent",
          color: "indigo",
          // A picked picture (card #393): a 32px sample.
          avatar: sampleAvatar,
          lastSeen: new Date(Date.now() - 5 * 60_000).toISOString(),
          working: 1,
          free: 0,
        },
        {
          personId: "sample-person-1",
          name: "Casey",
          me: false,
          presence: "offline",
          color: "sand",
          lastSeen: new Date(Date.now() - 2 * 3600_000).toISOString(),
          working: 1,
          free: 1,
        },
      ],
      owner: true,
      devices: 2,
      cards: 4,
      doneWeek: 1,
      lastAt: new Date(Date.now() - 7 * 60_000).toISOString(),
      ready: true,
      pinned: true,
      pinRank: 1,
      // A space with a second board (card #392). MLG and Netcode say what
      // they are for, Art has no description, and the prototype is closed
      // (card #690).
      boards: [
        {
          boardId: "sample-board-mlg",
          name: "MLG",
          tag: "mlg",
          description:
            "A two-player boat race people can play over the internet, ready for the spring demo.",
        },
        {
          boardId: "sample-board-mlg-net",
          name: "Netcode",
          tag: "net",
          description:
            "Both clients agree on every boat, every frame, on a normal home connection, so two players across town can race with no desync.\n\nRollback first, then lag compensation. Anything about matchmaking or lobbies goes on the MLG board, not here: this board is only the sync between two running games. Keep the repro captures attached to their cards so whoever picks one up can replay it.",
          mayClose: true,
        },
        {
          boardId: "sample-board-mlg-art",
          name: "Art",
          tag: "art",
          mayClose: true,
        },
        {
          boardId: "sample-board-mlg-proto",
          name: "Prototype",
          tag: "proto",
          description: "Find out if racing boats on one screen is fun.",
          closed: true,
          closedAt: new Date(Date.now() - 4 * 24 * 3600_000).toISOString(),
          mayClose: true,
        },
      ],
    }),
    BoardSummary({
      boardId: "sample-board-site",
      name: "Website refresh",
      tag: "site",
      people: ["Stephen"],
      members: [
        {
          personId: "sample-me",
          name: "Stephen",
          me: true,
          presence: "online",
          working: 0,
          free: 0,
        },
      ],
      owner: true,
      devices: 2,
      cards: 0,
      ready: true,
    }),
  ]
  return sampleBoards
}

function sampleMoved() {
  for (const fn of sampleListeners) fn()
}

function sampleFind(id: string): BoardSummary {
  const b = boardsState().find((x) => x.boardId === id)
  if (!b) throw new Error(`core: no such space on this device: ${id}`)
  return b
}

// sampleList is one board of a sample space or of Personal (space
// Personal), as core's project_board finds it.
function sampleList(space: string, boardId: string): SpaceBoard {
  const list =
    space === Personal ? samplePersonal : boardsOfSample(sampleFind(space))
  const x = list.find((b) => b.boardId === boardId)
  if (!x) throw new Error(`core: no such board: ${boardId}`)
  return x
}

// boardsOfSample is a sample space's boards, its first written out.
function boardsOfSample(b: BoardSummary): SpaceBoard[] {
  if (!b.boards.length)
    b.boards = [{ boardId: b.boardId, name: b.name, tag: b.tag }]
  return b.boards
}

function checkDescription(description: string) {
  if ([...description].length > MaxBoardDescription)
    throw new Error(
      `coord: invalid input: a board's description is ${MaxBoardDescription} characters at most`
    )
}

// cardList is core's card_list: up to eight refs, then "and N more".
function cardList(refs: string[]): string {
  const shown = refs.slice(0, 8).join(", ")
  return refs.length > 8 ? `${shown} and ${refs.length - 8} more` : shown
}

// sampleSetClosed is core's close and reopen on the sample (card #690):
// never a first board (or Personal's default), only by who may, and a
// close first keeps, moves or cancels the board's open cards.
export function sampleSetClosed(
  space: string,
  boardId: string,
  closed: boolean,
  open: OpenCards = "keep",
  moveTo = ""
) {
  const personal = space === Personal
  const all = personal ? samplePersonal : boardsOfSample(sampleFind(space))
  const x = sampleList(space, boardId)
  if (all[0]?.boardId === boardId)
    throw new Error(
      personal
        ? "coord: not allowed: Personal's default board is never closed"
        : "coord: not allowed: a space's first board is never closed"
    )
  if (!x.mayClose)
    throw new Error(
      `coord: not allowed: only the person who made board "${x.name}" or an owner of the space may ${closed ? "close" : "reopen"} it`
    )
  // As in core: closing a closed board or reopening an open one does
  // nothing.
  if (!!x.closed === closed) return
  if (closed) sampleClearOpen(personal ? "" : space, x, all, open, moveTo)
  x.closed = closed
  x.closedAt = closed ? new Date().toISOString() : undefined
  if (personal) samplePersonal = [...samplePersonal]
  sampleMoved()
}

// sampleClearOpen is core's clear_open on the sample: board x's open cards
// kept (refused while any is open), moved or canceled.
function sampleClearOpen(
  space: string,
  x: SpaceBoard,
  all: SpaceBoard[],
  open: OpenCards,
  moveTo: string
) {
  const cards = sampleOpenOn(space, x.boardId)
  if (!cards.length) return
  const ref = (c: { tag?: string; short?: string }) =>
    `${c.tag ?? ""}#${c.short ?? ""}`
  if (open === "keep") {
    const n = cards.length
    throw new Error(
      `coord: bad state: board "${x.name}" has ${n} open card${n === 1 ? "" : "s"} (${cardList(cards.map(ref))}): move or cancel them, or close the board with them moved or canceled`
    )
  }
  if (open === "cancel") {
    for (const c of cards) sampleCancel(c.id, ClosedBoardNote)
    return
  }
  const to = moveTo.trim()
  if (!to) throw new Error("coord: invalid input: pick a board to move them to")
  const target = all.find((b) => b.boardId === to)
  if (!target)
    throw new Error(`coord: not found: no board ${to} to move the cards to`)
  if (target.boardId === x.boardId)
    throw new Error(
      "coord: invalid input: pick another board to move the cards to"
    )
  if (target.closed)
    throw new Error(
      `coord: not allowed: board "${target.tag || target.name}" is closed: reopen it first, or pick another board`
    )
  const here = new Set(cards.map((c) => c.id))
  const stuck = cards.flatMap((c) =>
    c.ask && c.ask.state !== "closed"
      ? [`${ref(c)} is an open ask`]
      : c.parent && !here.has(c.parent.id)
        ? [`${ref(c)} is a subtask whose parent isn't open on this board`]
        : []
  )
  if (stuck.length)
    throw new Error(
      `coord: bad state: board "${x.name}" stays open: ${cardList(stuck)} can't move. Close or cancel ${stuck.length === 1 ? "it" : "them"} first, or close the board canceling its cards`
    )
  // A parent's subtasks go with it.
  for (const c of cards) sampleMoveBoard(c.id, target.boardId)
}

// sampleBoard is a sample board by id, for cards.ts.
export function sampleBoard(id: string): BoardSummary | undefined {
  return boardsState().find((x) => x.boardId === id)
}

setSampleBoards(sampleBoard)

// Personal's boards on the sample (card #558), its default first: House
// says what it is for, and Taxes 2025 is closed (card #690).
let samplePersonal: SpaceBoard[] = [
  { boardId: Personal, name: "Personal", tag: "" },
  {
    boardId: "sample-personal-home",
    name: "House",
    tag: "home",
    description: "Keep the house running: repairs, bills and the garden.",
    mayClose: true,
  },
  {
    boardId: "sample-personal-tax",
    name: "Taxes 2025",
    tag: "tax",
    description: "File the 2025 return and get the refund in.",
    closed: true,
    closedAt: new Date(Date.now() - 12 * 24 * 3600_000).toISOString(),
    mayClose: true,
  },
]

setSamplePersonal((id) => samplePersonal.find((x) => x.boardId === id))

const sampleSettings: Record<string, BoardSettings> = {}

// The sample invite: Invite waits for a guest that never comes; Join with
// an invite shows the sample's words.
let sampleInvite: InviteStatus = InviteStatus({})
const inviteListeners = new Set<(s: InviteStatus) => void>()

function setSampleInvite(s: InviteStatus): InviteStatus {
  sampleInvite = s
  for (const fn of inviteListeners) fn(s)
  return s
}

// ---- actions ----------------------------------------------------------------

export const boards = {
  async list(): Promise<BoardSummary[]> {
    if (inWails()) return (await BoardsService.List()) ?? []
    return boardsState().map((b) => BoardSummary(b))
  },

  // create makes a board between your own devices, its cards "<tag>#12".
  async create(name: string, tag: string): Promise<BoardSummary> {
    if (inWails()) return BoardsService.Create(name, tag)
    const n = name.trim()
    const t = tag.trim().toLowerCase()
    if (!n || n.length > MaxBoardName)
      throw new Error(
        `coord: invalid input: a space name is 1 to ${MaxBoardName} characters`
      )
    if (!validTag(t))
      throw new Error(
        `coord: invalid input: a board tag is a lowercase letter, then lowercase letters, digits or '-', ${MaxBoardTag} characters at most`
      )
    if (boardsState().some((b) => b.tag === t))
      throw new Error(
        `coord: invalid input: the tag "${t}" is already a board's`
      )
    const b = BoardSummary({
      boardId: `sample-board-${Date.now()}`,
      name: n,
      tag: t,
      people: ["Stephen"],
      owner: true,
      devices: 2,
      cards: 0,
      ready: true,
    })
    boardsState().push(b)
    sampleMoved()
    return b
  },

  // pin puts a board in the sidebar's Pinned boards, or takes it out (card
  // #135): a pin in your own log, so it follows you to your other devices.
  async pin(id: string, pinned: boolean): Promise<void> {
    if (inWails()) return BoardsService.Pin(id, pinned)
    const b = sampleFind(id)
    const top = Math.max(0, ...boardsState().map((x) => x.pinRank ?? 0))
    b.pinned = pinned
    b.pinRank = pinned ? top + 1 : 0
    sampleMoved()
  },

  // renameInSpace renames one board of a space, its first too (card #558):
  // any member may, and the space keeps its own name.
  async renameInSpace(
    spaceId: string,
    boardId: string,
    name: string
  ): Promise<BoardSummary> {
    if (inWails()) return BoardsService.RenameInSpace(spaceId, boardId, name)
    const b = sampleFind(spaceId)
    const n = name.trim()
    if (!n || n.length > MaxBoardName)
      throw new Error(
        `coord: invalid input: a board name is 1 to ${MaxBoardName} characters`
      )
    if (!b.boards.length)
      b.boards = [{ boardId: b.boardId, name: b.name, tag: b.tag }]
    b.boards = b.boards.map((x) =>
      x.boardId === boardId ? { ...x, name: n } : x
    )
    sampleMoved()
    return b
  },

  // personal is Personal's boards, its default first (card #558): the
  // person's own log holds them, so no BoardSummary does.
  async personal(): Promise<SpaceBoard[]> {
    if (inWails()) return (await BoardsService.Personal()) ?? []
    return samplePersonal.map((x) => ({ ...x }))
  },

  // addToPersonal makes one of Personal's boards, with its description
  // (card #690; "" for none).
  async addToPersonal(
    name: string,
    tag: string,
    description = ""
  ): Promise<SpaceBoard[]> {
    const d = description.trim()
    if (inWails())
      return (await BoardsService.AddToPersonal(name, tag, d)) ?? []
    checkDescription(d)
    const t = tag.trim().toLowerCase()
    if (!validTag(t))
      throw new Error(
        "coord: invalid input: a board tag is a lowercase letter, then up to 15 lowercase letters, digits or '-'"
      )
    if (
      samplePersonal.some((x) => x.tag === t) ||
      boardsState().some((x) => x.tag === t)
    )
      throw new Error(`core: the tag "${t}" is already a board's`)
    samplePersonal = [
      ...samplePersonal,
      {
        boardId: `sample-personal-${t}`,
        name: name.trim(),
        tag: t,
        description: d || undefined,
        mayClose: true,
      },
    ]
    sampleMoved()
    return boards.personal()
  },

  async renameInPersonal(boardId: string, name: string): Promise<SpaceBoard[]> {
    if (inWails())
      return (await BoardsService.RenameInPersonal(boardId, name)) ?? []
    const n = name.trim()
    if (!n || n.length > MaxBoardName)
      throw new Error(
        `coord: invalid input: a board name is 1 to ${MaxBoardName} characters`
      )
    samplePersonal = samplePersonal.map((x) =>
      x.boardId === boardId ? { ...x, name: n } : x
    )
    sampleMoved()
    return boards.personal()
  },

  // addToSpace puts another board in space `spaceId` (card #392): one
  // invite, one set of members, many boards. Refused, naming the devices,
  // until every member device runs an Irori that reads spaces.
  async addToSpace(
    spaceId: string,
    name: string,
    tag: string,
    description = ""
  ): Promise<BoardSummary> {
    const d = description.trim()
    if (inWails()) return BoardsService.AddToSpace(spaceId, name, tag, d)
    const b = sampleFind(spaceId)
    checkDescription(d)
    const t = tag.trim().toLowerCase()
    if (
      boardsState().some(
        (x) => x.tag === t || x.boards.some((y) => y.tag === t)
      )
    )
      throw new Error(`core: the tag "${t}" is already a board's`)
    if (!b.boards.length)
      b.boards = [{ boardId: b.boardId, name: b.name, tag: b.tag }]
    b.boards = [
      ...b.boards,
      {
        boardId: `${spaceId}-${t}`,
        name: name.trim(),
        tag: t,
        description: d || undefined,
        mayClose: true,
      },
    ]
    sampleMoved()
    return b
  },

  // deleteFromSpace deletes a board (card #563): its cards go to the space's
  // first board with new numbers there, and their old refs still find them.
  // Any member may. The first board can't go, and it's refused, naming the
  // devices, until every member device runs an Irori that reads spaces.
  async deleteFromSpace(
    spaceId: string,
    boardId: string
  ): Promise<BoardSummary> {
    if (inWails()) return BoardsService.DeleteInSpace(spaceId, boardId)
    const b = sampleFind(spaceId)
    if (b.boards[0]?.boardId === boardId)
      throw new Error(
        "coord: not allowed: a space's first board can't be deleted"
      )
    b.boards = b.boards.filter((x) => x.boardId !== boardId)
    sampleBoardGone(boardId, b.tag)
    sampleMoved()
    return b
  },

  // deleteFromPersonal deletes one of Personal's boards (card #563): its
  // cards go to the default board with new numbers there, and their old
  // refs still find them. The default board can't go.
  async deleteFromPersonal(boardId: string): Promise<SpaceBoard[]> {
    if (inWails()) return (await BoardsService.DeleteInPersonal(boardId)) ?? []
    if (boardId === Personal)
      throw new Error(
        "coord: not allowed: Personal's default board can't be deleted"
      )
    samplePersonal = samplePersonal.filter((x) => x.boardId !== boardId)
    sampleBoardGone(boardId, "")
    sampleMoved()
    return boards.personal()
  },

  // setDescription writes a board's description (card #690): any member
  // may; "" clears it. space is the space's id, or Personal.
  async setDescription(
    space: string,
    boardId: string,
    description: string
  ): Promise<void> {
    const d = description.trim()
    if (inWails()) {
      if (space === Personal)
        await BoardsService.SetDescriptionInPersonal(boardId, d)
      else await BoardsService.SetDescriptionInSpace(space, boardId, d)
    } else {
      checkDescription(d)
      const x = sampleList(space, boardId)
      if (x.closed)
        throw new Error(
          `coord: not allowed: board "${x.tag || x.name}" is closed: reopen it first, or pick another board`
        )
      x.description = d || undefined
      if (space === Personal) samplePersonal = [...samplePersonal]
      sampleMoved()
    }
    boardsTouched()
  },

  // close makes a board read-only and puts it under Closed boards (card
  // #690), its open cards first kept (refused while any is open), moved
  // to board moveTo (a board id; the space's id is its first board,
  // Personal Personal's default) or canceled. Its maker or a space owner
  // may (anyone in Personal); never a first board.
  async close(
    space: string,
    boardId: string,
    open: OpenCards,
    moveTo = ""
  ): Promise<void> {
    if (inWails()) {
      if (space === Personal)
        await BoardsService.CloseInPersonal(boardId, open, moveTo)
      else await BoardsService.CloseInSpace(space, boardId, open, moveTo)
    } else sampleSetClosed(space, boardId, true, open, moveTo)
    boardsTouched()
  },

  // reopen brings a closed board back as it was, its description too.
  async reopen(space: string, boardId: string): Promise<void> {
    if (inWails()) {
      if (space === Personal) await BoardsService.ReopenInPersonal(boardId)
      else await BoardsService.ReopenInSpace(space, boardId)
    } else sampleSetClosed(space, boardId, false)
    boardsTouched()
  },

  async rename(id: string, name: string): Promise<BoardSummary> {
    if (inWails()) return BoardsService.Rename(id, name)
    const b = sampleFind(id)
    b.name = name.trim()
    sampleMoved()
    return b
  },

  // leave takes you off a board on every one of your devices; with other
  // people on it, the board shows "Leaving…" until one of them has your
  // leave (design §4.4).
  async leave(id: string): Promise<void> {
    if (inWails()) return BoardsService.Leave(id)
    sampleFind(id)
    sampleBoards = boardsState().filter((b) => b.boardId !== id)
    sampleMoved()
  },

  // forget drops a board on this device only.
  async forget(id: string): Promise<void> {
    if (inWails()) return BoardsService.Forget(id)
    return boards.leave(id)
  },

  // removeMember takes someone off a board you own (§4.3).
  async removeMember(
    boardId: string,
    personId: string
  ): Promise<BoardSettings> {
    if (inWails()) return BoardsService.RemoveMember(boardId, personId)
    const s = await boards.settings(boardId)
    s.members = s.members.filter((m) => m.personId !== personId)
    return BoardSettings(s)
  },

  // setRole makes a member an owner, or an owner a member (§4.2).
  async setRole(
    boardId: string,
    personId: string,
    role: "owner" | "member"
  ): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetRole(boardId, personId, role)
    const s = await boards.settings(boardId)
    const owners = s.members.filter((m) => m.owner).length
    const m = s.members.find((x) => x.personId === personId)
    if (!m) throw new Error(`coord: not found: ${personId} isn't in the space`)
    if (role === "member" && m.owner && owners <= 1)
      throw new Error(
        "coord: not allowed: the last owner can't stop being one: make someone else an owner first"
      )
    m.owner = role === "owner"
    return BoardSettings(s)
  },

  // preview is exactly what Share to board would write.
  async preview(taskId: string, boardId: string): Promise<SharePreview> {
    if (inWails()) return BoardsService.SharePreview(taskId, boardId)
    return samplePreview(taskId, sampleFind(boardId))
  },

  // share writes the private card to the board and closes it; the shared
  // card's id comes back. hash is the confirmed preview's: a card that
  // changed since is refused.
  async share(taskId: string, boardId: string, hash: string): Promise<string> {
    if (inWails()) return BoardsService.Share(taskId, boardId, hash)
    const id = sampleShare(taskId, sampleFind(boardId))
    sampleFind(boardId).cards++
    sampleMoved()
    return id
  },

  // withdraw takes a shared card back off its board; the private card the
  // work carries on in comes back ("" when none).
  async withdraw(taskId: string): Promise<string> {
    if (inWails()) return BoardsService.Withdraw(taskId)
    const back = sampleWithdraw(taskId)
    sampleMoved()
    return back
  },

  // transferOwner offers shared card taskId to member `to` (a person id)
  // when the person owns it, or asks its owner for it when they don't
  // (to is then ""): an Accept/Decline question for the other side (card
  // #432). It returns that question's id.
  async transferOwner(taskId: string, to: string): Promise<string> {
    if (inWails()) return BoardsService.TransferOwner(taskId, to)
    const id = sampleTransferOwner(taskId, to)
    sampleMoved()
    return id
  },

  // settings is a board's members and its receive and autonomy presets.
  async settings(boardId: string): Promise<BoardSettings> {
    if (inWails()) return BoardsService.Settings(boardId)
    const b = sampleFind(boardId)
    return (sampleSettings[boardId] ??= BoardSettings({
      boardId: b.boardId,
      members: [
        {
          personId: "sample-me",
          name: b.people[0] ?? "You",
          owner: true,
          me: true,
          devices: b.devices,
          fingerprint: "AB12 CD34 EF56",
        },
        ...b.people.slice(1).map((name, i) => ({
          personId: `sample-person-${i}`,
          name,
          owner: false,
          me: false,
          devices: 1,
          fingerprint: "9F3A 01BC 77D2",
        })),
      ],
      receive: "open",
      people: [],
      receiveOwn: false,
      autonomy: "trusted",
      autonomyOwn: false,
      threads: "shared",
      leaving: false,
      conflicts: [],
      reviewers: [],
      agents: [
        {
          agent: "sample-dev/alpha",
          name: "alpha",
          repo: "remote:github.com/sample-org/example-app",
        },
        { agent: "sample-dev/beta", name: "beta", repo: "" },
      ],
    }))
  },

  // setReceive sets what of other people's rows reaches your agents on the
  // board; "" goes back to the default.
  async setReceive(
    boardId: string,
    preset: string,
    people: string[]
  ): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetReceive(boardId, preset, people)
    const s = await boards.settings(boardId)
    Object.assign(s, {
      receive: preset || "open",
      people,
      receiveOwn: !!preset,
    })
    return BoardSettings(s)
  },

  // setAutonomy sets what your agents may do on the board without asking.
  async setAutonomy(boardId: string, preset: string): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetAutonomy(boardId, preset)
    const s = await boards.settings(boardId)
    Object.assign(s, { autonomy: preset || "trusted", autonomyOwn: !!preset })
    return BoardSettings(s)
  },

  // setThreads sets where your threads on the board live: shared or
  // private (card #185).
  async setThreads(boardId: string, mode: string): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetThreads(boardId, mode)
    const s = await boards.settings(boardId)
    Object.assign(s, { threads: mode || "shared" })
    return BoardSettings(s)
  },

  // setReviewer dedicates one of your agents as your reviewer on the board
  // (design §10), for one repo or ("") the whole board; agent "" takes the
  // reviewer off.
  async setReviewer(
    boardId: string,
    agent: string,
    repo: string
  ): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetReviewer(boardId, agent, repo)
    const s = await boards.settings(boardId)
    const rest = s.reviewers.filter((r) => r.repo !== repo)
    const a = s.agents.find((x) => x.agent === agent)
    s.reviewers = a ? [...rest, { repo, agent, name: a.name }] : rest
    return BoardSettings(s)
  },

  // invite opens a one-hour invite window on a board you own; the status
  // carries the irori-invite: link to send.
  async invite(boardId: string): Promise<InviteStatus> {
    if (inWails()) return BoardsService.Invite(boardId)
    if (inviteOpen(sampleInvite)) throw new Error("an invite is already open")
    const b = sampleFind(boardId)
    return setSampleInvite(
      InviteStatus({
        role: "host",
        state: InviteState.Waiting,
        boardId: b.boardId,
        boardName: b.name,
        // As long as a real ticket (code, endpoint address, relay, fingerprint).
        link: "irori-invite:" + "SAMPLE0".repeat(30),
        expiresAt: new Date(Date.now() + 3600_000).toISOString(),
        myFingerprint: "3f2a 91c0 7be4 d018",
      })
    )
  },

  // joinInvite takes a pasted irori-invite: link.
  async joinInvite(link: string): Promise<InviteStatus> {
    if (inWails()) return BoardsService.JoinInvite(link)
    if (!link.trim().toLowerCase().startsWith("irori-invite:"))
      throw new Error("that isn't an irori-invite: link")
    return setSampleInvite(
      InviteStatus({
        role: "guest",
        state: InviteState.Words,
        boardName: "Sample space",
        words: ["sample", "words", "to", "give", "them"],
        peerName: "Sam",
        peerFingerprint: "3f2a 91c0 7be4 d018",
        myFingerprint: "a7d1 0c55 e29b 4f63",
      })
    )
  },

  // confirmInviteWords is the inviter typing the words the invitee reads.
  async confirmInviteWords(words: string): Promise<InviteStatus> {
    if (inWails()) return BoardsService.ConfirmInviteWords(words)
    throw new Error("no invite is waiting for words")
  },

  async inviteStatus(): Promise<InviteStatus> {
    if (inWails()) return BoardsService.InviteStatus()
    return sampleInvite
  },

  async cancelInvite(): Promise<void> {
    if (inWails()) return BoardsService.CancelInvite()
    setSampleInvite(InviteStatus({}))
  },
}

// ---- hooks ------------------------------------------------------------------

// touched are readers to tell when this window changed a board itself,
// so the header and the page over it (separate readers of Personal's
// boards) never disagree, whether or not a boards:changed follows.
const touched = new Set<() => void>()

function boardsTouched() {
  void reloadBoards()
  for (const fn of touched) fn()
}

// The boards list is one list for the whole window (card #233): the
// sidebar's Pinned boards, the space picker and every other reader share
// it, so they can never disagree. It loads when its first reader mounts,
// again on every boards:changed (a board opened, went or was renamed, or
// a board synced from another device got its rows), and on reload.

// loaded is false until the window's first read ends, either way (card
// #598): before it, an empty list is not yet "no boards".
export type BoardsList = {
  boards: BoardSummary[]
  error: string | null
  loaded: boolean
}

const listState = {
  value: { boards: [], error: null, loaded: false } as BoardsList,
  readers: new Set<() => void>(),
  off: null as (() => void) | null,
  req: 0,
}

// onBoardsMoved calls fn whenever the list may have changed: the host's
// boards:changed, or a sample board moving in a plain browser.
function onBoardsMoved(fn: () => void): () => void {
  if (inWails()) return Events.OnShown(EventBoardsChanged, fn)
  sampleListeners.add(fn)
  return () => {
    sampleListeners.delete(fn)
  }
}

// reloadBoards reads the list again and hands it to every reader; an older
// read that finishes after a newer one is dropped.
export async function reloadBoards(): Promise<void> {
  const n = ++listState.req
  let next: BoardsList
  try {
    next = { boards: await boards.list(), error: null, loaded: true }
  } catch (err) {
    next = { ...listState.value, error: errorText(err), loaded: true }
  }
  if (n !== listState.req) return
  listState.value = next
  for (const fn of listState.readers) fn()
}

// subscribeBoards adds a reader of the shared list; the first one starts
// following boards:changed and the last one stops it. Every new reader
// reads the list again, so a page that opens shows it as it is now.
export function subscribeBoards(reader: () => void): () => void {
  listState.readers.add(reader)
  listState.off ??= onBoardsMoved(() => void reloadBoards())
  void reloadBoards()
  return () => {
    listState.readers.delete(reader)
    if (listState.readers.size === 0) {
      listState.off?.()
      listState.off = null
    }
  }
}

// boardsNow is the shared list as last read.
export function boardsNow(): BoardsList {
  return listState.value
}

// useBoards is the boards on this device: the shared list, kept current.
export function useBoards(): {
  boards: BoardSummary[]
  error: string | null
  // loaded: the first read has ended.
  loaded: boolean
  reload: () => Promise<void>
} {
  const {
    boards: value,
    error,
    loaded,
  } = useSyncExternalStore(subscribeBoards, boardsNow, boardsNow)
  return { boards: value, error, loaded, reload: reloadBoards }
}

// usePersonalBoards reads Personal's boards, and again when they may have
// moved: added here, or on another of the person's devices.
export function usePersonalBoards(): {
  boards: SpaceBoard[]
  error: string | null
  reload: () => Promise<void>
} {
  const [state, setState] = useState<{
    boards: SpaceBoard[]
    error: string | null
  }>({
    boards: [],
    error: null,
  })
  const reload = useCallback(async () => {
    try {
      const list = await boards.personal()
      setState({ boards: list, error: null })
    } catch (err) {
      setState((s) => ({ ...s, error: errorText(err) }))
    }
  }, [])
  useEffect(() => {
    void reload()
    const on = () => void reload()
    touched.add(on)
    const off = onBoardsMoved(on)
    return () => {
      touched.delete(on)
      off()
    }
  }, [reload])
  return { ...state, reload }
}

// useInvite follows the open (or last) invite while active. An invite that
// had already ended when the dialog opened isn't shown again.
export function useInvite(active: boolean): {
  invite: InviteStatus | null
  setInvite: (s: InviteStatus | null) => void
} {
  const [invite, setInvite] = useState<InviteStatus | null>(null)
  useEffect(() => {
    if (!active) return
    let live = true
    void boards
      .inviteStatus()
      .then((s) => live && setInvite(inviteOpen(s) ? s : null))
      .catch(() => live && setInvite(null))
    const on = (s: InviteStatus) => live && setInvite(s)
    let off: () => void
    if (inWails()) {
      off = Events.On<InviteStatus>(EventInviteChanged, (ev) =>
        on(InviteStatus(ev.data ?? {}))
      )
    } else {
      inviteListeners.add(on)
      off = () => inviteListeners.delete(on)
    }
    return () => {
      live = false
      off()
    }
  }, [active])
  return { invite, setInvite }
}
