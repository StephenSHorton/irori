// Boards gateway (docs/design/v0.5-shared-boards.md §8, step 4): the
// boards on this device, New board, rename, Leave and Forget, and Share to
// board and Withdraw on a card's page. Like cards.ts, components import from
// here, never from src/gen. In a plain browser it runs on two sample
// boards, and sharing moves a sample card onto one.

import { useEffect, useState } from "react"

import { BoardsService, Events } from "./host"
import { BoardSettings, BoardSummary, InviteStatus, SharePreview } from "./models"
import { sampleShare, sampleWithdraw, samplePreview } from "./cards"
import { inWails } from "./irori"
import { useLatest } from "./talk"

export { BoardSettings, BoardSummary, InviteStatus, SharePreview }

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
  { value: "open", label: "Open", hint: "They can wake your agents, and their rows reach your bypass sessions." },
  { value: "members", label: "Members wake", hint: "Only the people you pick wake your agents; the rest arrive at the next turn." },
  { value: "ask", label: "Ask me", hint: "Wake requests wait for you to release them." },
  { value: "closed", label: "Closed", hint: "Nothing wakes your agents; everything arrives at the next turn." },
] as const

// Autonomy presets (design §7.3): what your agents may do on a board
// without asking. Trusted is the default.
export const AutonomyPresets = [
  { value: "ask", label: "Ask me", hint: "New cards on the board ask you first; your private cards aren't shared." },
  { value: "trusted", label: "Trusted", hint: "They create cards on the board; sharing a private card asks you." },
  { value: "autonomous", label: "Autonomous", hint: "They also share private cards (after scrub) and mirror all their leases." },
] as const

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
    BoardSummary({
      boardId: "sample-board-mlg",
      name: "MLG",
      tag: "mlg",
      people: ["Stephen"],
      owner: true,
      devices: 2,
      cards: 2,
      ready: true,
    }),
    BoardSummary({
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
      throw new Error(`coord: invalid input: a board name is 1 to ${MaxBoardName} characters`)
    if (!validTag(t))
      throw new Error(
        "coord: invalid input: a board tag is a lowercase letter, then up to 15 lowercase letters, digits or '-'"
      )
    if (boardsState().some((b) => b.tag === t))
      throw new Error(`coord: invalid input: the tag "${t}" is already a board's`)
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

  // settings is a board's members and its receive and autonomy presets.
  async settings(boardId: string): Promise<BoardSettings> {
    if (inWails()) return BoardsService.Settings(boardId)
    const b = sampleFind(boardId)
    return (sampleSettings[boardId] ??= BoardSettings({
      boardId: b.boardId,
      members: [{ personId: "sample-me", name: b.people[0] ?? "You", owner: true, me: true, devices: b.devices }],
      receive: "open",
      people: [],
      receiveOwn: false,
      autonomy: "trusted",
      autonomyOwn: false,
      reviewers: [],
      agents: [
        { agent: "sample-dev/alpha", name: "alpha", repo: "remote:github.com/sample-org/example-app" },
        { agent: "sample-dev/beta", name: "beta", repo: "" },
      ],
    }))
  },

  // setReceive sets what of other people's rows reaches your agents on the
  // board; "" goes back to the default.
  async setReceive(boardId: string, preset: string, people: string[]): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetReceive(boardId, preset, people)
    const s = await boards.settings(boardId)
    Object.assign(s, { receive: preset || "open", people, receiveOwn: !!preset })
    return BoardSettings(s)
  },

  // setAutonomy sets what your agents may do on the board without asking.
  async setAutonomy(boardId: string, preset: string): Promise<BoardSettings> {
    if (inWails()) return BoardsService.SetAutonomy(boardId, preset)
    const s = await boards.settings(boardId)
    Object.assign(s, { autonomy: preset || "trusted", autonomyOwn: !!preset })
    return BoardSettings(s)
  },

  // setReviewer dedicates one of your agents as your reviewer on the board
  // (design §10), for one repo or ("") the whole board; agent "" takes the
  // reviewer off.
  async setReviewer(boardId: string, agent: string, repo: string): Promise<BoardSettings> {
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
        link: "irori-invite:SAMPLE0SAMPLE0SAMPLE0SAMPLE0",
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
        boardName: "Sample board",
        words: ["sample", "words", "read", "them", "aloud"],
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
