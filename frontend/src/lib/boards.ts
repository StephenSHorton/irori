// Boards gateway (docs/design/v0.5-shared-boards.md §8, step 4): the
// boards on this device, New board, rename, Leave and Forget, and Share to
// board and Withdraw on a card's page. Like cards.ts, components import from
// here, never from bindings/. In a plain browser it runs on two sample
// boards, and sharing moves a sample card onto one.

import { useEffect } from "react"
import { Events } from "@wailsio/runtime"

import { BoardsService } from "../../bindings/github.com/StephenSHorton/irori"
import {
  BoardSummary,
  SharePreview,
} from "../../bindings/github.com/StephenSHorton/irori/internal/core/models"
import { sampleShare, sampleWithdraw, samplePreview } from "./cards"
import { inWails } from "./irori"
import { useLatest } from "./talk"

export { BoardSummary, SharePreview }

// EventBoardsChanged mirrors the Go constant in boards_service.go: a board
// opened, went or was renamed.
export const EventBoardsChanged = "boards:changed"

// MaxBoardName mirrors identity.MaxBoardName.
export const MaxBoardName = 60

// validTag is identity.ValidBoardTag: a lowercase letter, then up to 15
// lowercase letters, digits or '-'.
export function validTag(tag: string): boolean {
  return /^[a-z][a-z0-9-]{0,15}$/.test(tag)
}

// suggestTag is a tag from a board's name: its first word, lowercased,
// with only what a tag may hold.
export function suggestTag(name: string): string {
  const word = name
    .toLowerCase()
    .replace(/[^a-z0-9-\s]/g, "")
    .trim()
    .split(/\s+/)[0]
  const t = (word ?? "").replace(/^[^a-z]+/, "").slice(0, 16)
  return validTag(t) ? t : ""
}

// ---- sample mode ------------------------------------------------------------

let sampleBoards: BoardSummary[] | null = null
const sampleListeners = new Set<() => void>()

function boardsState(): BoardSummary[] {
  sampleBoards ??= [
    new BoardSummary({
      boardId: "sample-board-mlg",
      name: "MLG",
      tag: "mlg",
      people: ["Stephen"],
      owner: true,
      devices: 2,
      cards: 2,
      ready: true,
    }),
    new BoardSummary({
      boardId: "sample-board-site",
      name: "Website refresh",
      tag: "site",
      people: ["Stephen"],
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
  if (!b) throw new Error(`core: no such board on this device: ${id}`)
  return b
}

// sampleBoard is a sample board by id, for cards.ts.
export function sampleBoard(id: string): BoardSummary | undefined {
  return boardsState().find((x) => x.boardId === id)
}

// ---- actions ----------------------------------------------------------------

export const boards = {
  async list(): Promise<BoardSummary[]> {
    if (inWails()) return (await BoardsService.List()) ?? []
    return boardsState().map((b) => new BoardSummary(b))
  },

  // create makes a board between your own devices, its cards "<tag>#12".
  async create(name: string, tag: string): Promise<BoardSummary> {
    if (inWails()) return BoardsService.Create(name, tag)
    const n = name.trim()
    const t = tag.trim().toLowerCase()
    if (!n || n.length > MaxBoardName)
      throw new Error(`coord: invalid input: a board name is 1 to ${MaxBoardName} characters`)
    if (!validTag(t))
      throw new Error(
        "coord: invalid input: a board tag is a lowercase letter, then up to 15 lowercase letters, digits or '-'"
      )
    if (boardsState().some((b) => b.tag === t))
      throw new Error(`coord: invalid input: the tag "${t}" is already a board's`)
    const b = new BoardSummary({
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

  async rename(id: string, name: string): Promise<BoardSummary> {
    if (inWails()) return BoardsService.Rename(id, name)
    const b = sampleFind(id)
    b.name = name.trim()
    sampleMoved()
    return b
  },

  // leave takes you off a board on every one of your devices.
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

  // preview is exactly what Share to board would write.
  async preview(taskId: string, boardId: string): Promise<SharePreview> {
    if (inWails()) return BoardsService.SharePreview(taskId, boardId)
    return samplePreview(taskId, sampleFind(boardId))
  },

  // share writes the private card to the board and closes it; the shared
  // card's id comes back.
  async share(taskId: string, boardId: string): Promise<string> {
    if (inWails()) return BoardsService.Share(taskId, boardId)
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
}

// ---- hooks ------------------------------------------------------------------

// useBoards is the boards on this device, kept current from boards:changed.
export function useBoards(): {
  boards: BoardSummary[]
  error: string | null
  reload: () => Promise<void>
} {
  const { value, error, reload } = useLatest<BoardSummary[]>(boards.list, [])
  useEffect(() => {
    void reload()
    if (!inWails()) {
      const fn = () => void reload()
      sampleListeners.add(fn)
      return () => {
        sampleListeners.delete(fn)
      }
    }
    return Events.On(EventBoardsChanged, () => void reload())
  }, [reload])
  return { boards: value, error, reload }
}
