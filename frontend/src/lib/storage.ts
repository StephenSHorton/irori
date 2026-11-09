// Storage gateway (card #728, docs/design/storage.md): what Irori keeps on
// this device, how much space each kind takes, each kind's items, and the
// clean-outs the person asks for. Nothing is cleaned out by itself
// (board.md "No retention rules").
//
// Like cards.ts, components import from here, never from src/gen. The
// window's storage_* commands back it in the app; in a plain browser
// everything runs on the sample below, which the buttons change in memory.

import { useCallback, useEffect } from "react"

import { Events, StorageService } from "./host"
import { EventBoardChanged } from "./cards"
import { inWails } from "./irori"
import { useLatest } from "./talk"

// ---- the shapes (crates/window/src/storage.rs) ------------------------------

export type StorageKindId =
  | "log"
  | "spaces"
  | "files"
  | "builds"
  | "updates"
  | "logs"
  | "briefs"
  | "backups"
  | "received"
  | "voice"
  | "capture"
  | "claude"

export type StorageKind = {
  id: string
  bytes: number
  items: number
  // Build output and Claude Code's folder come unmeasured: the page asks
  // for their items, which measures them.
  measured: boolean
  path: string
}

export type StorageReport = { dir: string; kinds: StorageKind[] }

export type StorageItem = {
  id: string
  name: string
  bytes: number
  at: string
  detail: string
  // Card ids it belongs to, to open.
  cards: string[]
  cleanable: boolean
  keep: string
  // An attachment this device attached itself: maybe the only copy.
  onlyHere: boolean
  // Why the sweep keeps a worktree ("waiting for Done", "canceled, 3
  // unpushed commits"), "" for none (#1118).
  pinned: string
  // Processes with their folder in a worktree: End it ends one (#1118).
  holders: StorageHolder[]
  // A canceled card's worktree, which may be deleted whole (#1118).
  deleteAnyway: boolean
}

export type StorageHolder = { pid: number; name: string }

export type StorageCleaned = {
  removed: number
  freed: number
  kept: { id: string; why: string }[]
}

export type LogPart = { part: string; rows: number; bytes: number }

export type ClearedCard = {
  id: string
  short: string
  title: string
  board: string
  thread: boolean
  updatedAt: string
}

// ---- what each kind is, in the person's words -------------------------------

export type KindInfo = {
  label: string
  // What it holds.
  what: string
  // What deleting does, said plainly; for a kind nothing deletes, why.
  clean: string
  // Whether the page offers deleting its items.
  cleanable: boolean
}

export const kindInfo: Record<StorageKindId, KindInfo> = {
  log: {
    label: "Irori's log",
    what: "Your cards, threads, Talk, notes, and your agents' conversations and messages, in one file shared with your other devices.",
    clean:
      "It can't be trimmed: every row is signed and synced to your other devices, and a row deleted here would break that. Clearing done cards and threads tidies the lists but frees no space.",
    cleanable: false,
  },
  spaces: {
    label: "Shared spaces",
    what: "Each shared space's log, which every member keeps a copy of. Direct messages with another person are a space too.",
    clean:
      "Not deleted here: forget a space from its menu on Boards to remove its file from this device. A space you leave goes once another member has your leave.",
    cleanable: false,
  },
  files: {
    label: "Attachments",
    what: "Files on cards, comments, Talk posts and messages, as this device holds them.",
    clean:
      "Deleting removes the file from this device and can't be undone here. If another device still has it, opening it fetches it again. A file attached on this device may be the only copy.",
    cleanable: true,
  },
  builds: {
    label: "Build output",
    what: "What agents built in the worktrees Irori made for their cards (each one's target folder).",
    clean:
      "Deleting removes the build output for good; the code and its commits stay. An agent that works there again builds from scratch. A tree an agent is working in now is left alone.",
    cleanable: true,
  },
  updates: {
    label: "Downloaded updates",
    what: "Installers Irori downloaded to update itself.",
    clean:
      "Deleted installers are gone for good; the next update downloads again. The one waiting to install stays.",
    cleanable: true,
  },
  logs: {
    label: "Logs",
    what: "Irori's record of what it did, and crash reports.",
    clean:
      "Old logs and crash reports go for good. The log Irori is writing now stays.",
    cleanable: true,
  },
  briefs: {
    label: "Agents' briefs",
    what: "What Irori told each agent it started for a card.",
    clean:
      "Briefs of finished cards go for good. An open card's brief stays, since its agent may still read it.",
    cleanable: true,
  },
  backups: {
    label: "Left from older versions",
    what: "A copy of the log from before Irori 0.4, and what a reset moved aside.",
    clean: "Deleting is permanent: these are the only copies.",
    cleanable: true,
  },
  received: {
    label: "Saved copies",
    what: "Attachments copied out for an agent or you to open.",
    clean: "These are copies; the attachments themselves stay.",
    cleanable: true,
  },
  voice: {
    label: "Speech model",
    what: "The model that writes down what you say.",
    clean: "It downloads again the next time you use voice.",
    cleanable: true,
  },
  capture: {
    label: "Conversation changes",
    what: "The changes and summaries Irori keeps for your agents' conversations.",
    clean:
      "Kept: Irori has no other copy, and your agents' conversations need them.",
    cleanable: false,
  },
  claude: {
    label: "Claude Code's conversations",
    what: "Claude Code's own folder of conversation transcripts, which Irori reads.",
    clean:
      "Irori never deletes these: they are Claude Code's, and deleting them would stop those conversations from being resumed.",
    cleanable: false,
  },
}

export function infoOf(id: string): KindInfo {
  return (
    kindInfo[id as StorageKindId] ?? {
      label: id,
      what: "",
      clean: "",
      cleanable: false,
    }
  )
}

// partLabel names a share of the log.
export const partLabels: Record<string, string> = {
  conversations: "Agents' conversations",
  messages: "Messages to and between agents",
  cards: "Cards, boards and threads",
  talk: "Talk",
  notes: "Notes",
  code: "Commits and merges",
  agents: "Agents starting and stopping",
  assistant: "Your assistant's chat",
  other: "Everything else",
}

// ---- sizes ------------------------------------------------------------------

// bytesText is a size as the page shows it: 0 B, 812 KB, 1.5 GB.
export function bytesText(n: number): string {
  if (n < 1024) return `${n} B`
  const units = ["KB", "MB", "GB", "TB"]
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`
}

// total is every measured kind's bytes.
export function total(kinds: StorageKind[]): number {
  return kinds.reduce((n, k) => n + (k.measured ? k.bytes : 0), 0)
}

// cleanableBytes is what a clean of these items would free.
export function cleanableBytes(items: StorageItem[]): number {
  return items.reduce((n, i) => n + (i.cleanable ? i.bytes : 0), 0)
}

// measuredFrom fills a kind that came unmeasured from its items.
export function measuredFrom(
  k: StorageKind,
  items: StorageItem[]
): StorageKind {
  return {
    ...k,
    measured: true,
    bytes: items.reduce((n, i) => n + i.bytes, 0),
    items: items.length,
  }
}

// ---- the sample (a plain browser) -------------------------------------------

const GB = 1 << 30
const MB = 1 << 20
const day = 24 * 3600 * 1000
const ago = (d: number) => new Date(Date.now() - d * day).toISOString()

function item(
  p: Partial<StorageItem> & { id: string; bytes: number }
): StorageItem {
  return {
    name: p.id,
    at: ago(3),
    detail: "",
    cards: [],
    cleanable: true,
    keep: "",
    onlyHere: false,
    pinned: "",
    holders: [],
    deleteAnyway: false,
    ...p,
  }
}

function freshSample(): Record<string, StorageItem[]> {
  return {
    spaces: [
      item({
        id: "b1",
        name: "Moon Lander Game",
        bytes: 104 * MB,
        cleanable: false,
        keep: "Shared with the space's members: leave or forget a space from its menu on Boards",
      }),
      item({
        id: "b2",
        name: "Direct messages",
        bytes: 2 * MB,
        cleanable: false,
        detail:
          "A space for direct messages with another person, or one this device has left",
        keep: "Shared with the space's members: leave or forget a space from its menu on Boards",
      }),
    ],
    files: [
      item({
        id: "a".repeat(64),
        name: "screen-recording.mp4",
        bytes: 241 * MB,
        at: ago(40),
        detail: "From the MacBook",
      }),
      item({
        id: "b".repeat(64),
        name: "crash-dump.bin",
        bytes: 125 * MB,
        at: ago(12),
        detail: "Attached on this device",
        onlyHere: true,
      }),
      item({
        id: "c".repeat(64),
        name: "board-after.png",
        bytes: 1.2 * MB,
        at: ago(1),
        detail: "From the MacBook",
      }),
    ],
    builds: [
      item({
        id: "c:/work/irori/.claude/worktrees/card-664-1",
        name: "card-664-1",
        bytes: 27.9 * GB,
        at: ago(2),
        detail: "#664 Voice calls: echo cancelling",
        cards: ["01SAMPLE664"],
      }),
      item({
        id: "c:/work/irori/.claude/worktrees/card-687-1",
        name: "card-687-1",
        bytes: 23.1 * GB,
        at: ago(5),
        detail: "#687 Merge desk: rebase in place",
        cards: ["01SAMPLE687"],
      }),
      item({
        id: "c:/work/irori/.claude/worktrees/card-728-1",
        name: "card-728-1",
        bytes: 4.2 * GB,
        at: ago(0),
        detail: "#728 Storage page",
        cards: ["01SAMPLE728"],
        cleanable: false,
        keep: "an agent is working in it",
      }),
      item({
        id: "c:/work/irori/.claude/worktrees/card-973-1",
        name: "card-973-1",
        bytes: 33 * GB,
        at: ago(4),
        detail: "#973 Relay health",
        cards: ["01SAMPLE973"],
        pinned: "waiting for Done",
      }),
      item({
        id: "c:/work/irori/.claude/worktrees/card-655-1",
        name: "card-655-1",
        bytes: 31 * GB,
        at: ago(6),
        detail: "#655 Share a screen",
        cards: ["01SAMPLE655"],
        cleanable: false,
        keep: "held by cat (pid 4242)",
        holders: [{ pid: 4242, name: "cat" }],
      }),
      item({
        id: "c:/work/irori/.claude/worktrees/card-610-1",
        name: "card-610-1",
        bytes: 12 * GB,
        at: ago(9),
        detail: "#610 Old idea",
        cards: ["01SAMPLE610"],
        pinned: "canceled, 3 unpushed commits",
        deleteAnyway: true,
      }),
    ],
    updates: [
      item({ id: "Irori_0.12.5_x64-setup.exe", bytes: 24 * MB, at: ago(9) }),
      item({
        id: "Irori_0.12.6_x64-setup.exe",
        bytes: 24 * MB,
        at: ago(1),
        cleanable: false,
        keep: "it's the update waiting to install",
      }),
    ],
    logs: [
      item({
        id: "irori.log",
        bytes: 1.1 * MB,
        at: ago(0),
        cleanable: false,
        keep: "Irori is writing to it now",
      }),
      item({ id: "irori.log.1", bytes: 4 * MB, at: ago(2) }),
      item({ id: "crash-1657.log", bytes: 188 * 1024, at: ago(60) }),
    ],
    briefs: [
      item({
        id: "01SAMPLE664",
        name: "#664 Voice calls: echo cancelling",
        bytes: 12 * 1024,
        detail: "2 agent starts",
        cards: ["01SAMPLE664"],
      }),
      item({
        id: "01SAMPLE728",
        name: "#728 Storage page",
        bytes: 6 * 1024,
        detail: "1 agent start",
        cards: ["01SAMPLE728"],
        cleanable: false,
        keep: "its card is still open",
      }),
    ],
    backups: [
      item({
        id: "irori.db.v0.3",
        name: "The log before Irori 0.4",
        detail: "A copy kept when Irori moved to a new log format",
        bytes: 19 * MB,
        at: ago(120),
      }),
    ],
    received: [
      item({
        id: "dev_1",
        name: "trace.txt",
        bytes: 92 * 1024,
        detail: "A copy saved out for an agent; the attachment itself stays",
      }),
    ],
    voice: [],
    claude: [
      item({
        id: "C--work-irori",
        name: "C--work-irori",
        bytes: 3.4 * GB,
        detail: "1,204 files",
        cleanable: false,
        keep: "Claude Code's folder: Irori reads it and never deletes from it",
      }),
      item({
        id: "C--work-moon",
        name: "C--work-moon",
        bytes: 812 * MB,
        detail: "233 files",
        cleanable: false,
        keep: "Claude Code's folder: Irori reads it and never deletes from it",
      }),
    ],
  }
}

let sample = freshSample()
let sampleCleared: ClearedCard[] = []
let sampleThreads = 4

const sampleLog: LogPart[] = [
  { part: "conversations", rows: 35473, bytes: 89 * MB },
  { part: "messages", rows: 138000, bytes: 73 * MB },
  { part: "code", rows: 41000, bytes: 29 * MB },
  { part: "cards", rows: 52000, bytes: 18 * MB },
  { part: "agents", rows: 30000, bytes: 6 * MB },
  { part: "talk", rows: 410, bytes: 0.3 * MB },
  { part: "notes", rows: 120, bytes: 0.1 * MB },
]

const kindOrder: StorageKindId[] = [
  "log",
  "spaces",
  "files",
  "builds",
  "updates",
  "logs",
  "briefs",
  "backups",
  "received",
  "voice",
  "capture",
  "claude",
]

function sampleReport(): StorageReport {
  const sum = (id: string) =>
    (sample[id] ?? []).reduce((n, i) => n + i.bytes, 0)
  return {
    dir: "C:\\Users\\you\\AppData\\Local\\irori",
    kinds: kindOrder.map((id) => ({
      id,
      bytes:
        id === "log"
          ? 406 * MB
          : id === "capture"
            ? 18 * MB
            : id === "builds" || id === "claude"
              ? 0
              : sum(id),
      items: id === "log" || id === "capture" ? 0 : (sample[id] ?? []).length,
      measured: id !== "builds" && id !== "claude",
      path: "",
    })),
  }
}

// resetSample puts the browser's sample back (tests).
export function resetSample() {
  sample = freshSample()
  sampleCleared = [
    {
      id: "01SAMPLEOLD1",
      short: "#512",
      title: "Card colours for agents",
      board: "",
      thread: false,
      updatedAt: ago(30),
    },
  ]
  sampleThreads = 4
}
resetSample()

// ---- the gateway --------------------------------------------------------------

// Each listener follows a scope: a kind's items follow that kind, the
// report follows every change ("*"), the log's lists follow "log". A clean
// of one kind so never re-walks another (build output is a long walk).
const listeners = new Map<() => void, string>()
function changed(kind?: string) {
  for (const [fn, scope] of listeners)
    if (!kind || scope === "*" || scope === kind) fn()
}

export const storage = {
  report: (): Promise<StorageReport> =>
    inWails() ? StorageService.Report() : Promise.resolve(sampleReport()),

  log: (): Promise<LogPart[]> =>
    inWails() ? StorageService.Log() : Promise.resolve(sampleLog),

  items: (kind: string): Promise<StorageItem[]> =>
    inWails()
      ? StorageService.Items(kind)
      : Promise.resolve(
          [...(sample[kind] ?? [])].sort((a, b) => b.bytes - a.bytes)
        ),

  // clean deletes the items given of one kind, or with none every one a
  // clean may take.
  clean: async (kind: string, ids: string[] = []): Promise<StorageCleaned> => {
    let out: StorageCleaned
    if (inWails()) {
      out = await StorageService.Clean(kind, ids)
    } else {
      out = { removed: 0, freed: 0, kept: [] }
      const want = new Set(ids)
      const left: StorageItem[] = []
      for (const i of sample[kind] ?? []) {
        if (want.size && !want.has(i.id)) {
          left.push(i)
        } else if (!i.cleanable) {
          if (want.size) out.kept.push({ id: i.id, why: i.keep })
          left.push(i)
        } else {
          out.removed++
          out.freed += i.bytes
        }
      }
      sample[kind] = left
    }
    changed(kind)
    return out
  },

  // endHolder ends process `pid`, which has its folder in worktree `tree`
  // (#1118): the sweep can't remove a tree past it.
  endHolder: async (tree: string, pid: number): Promise<string> => {
    let who: string
    if (inWails()) who = await StorageService.EndHolder(tree, pid)
    else {
      const t = (sample.builds ?? []).find((i) => i.id === tree)
      who = t?.holders.find((h) => h.pid === pid)?.name ?? `pid ${pid}`
      if (t) {
        t.holders = t.holders.filter((h) => h.pid !== pid)
        if (!t.holders.length) {
          t.keep = ""
          t.cleanable = true
        }
      }
    }
    changed("builds")
    return who
  },

  // deleteTree deletes a canceled card's worktree whole, unpushed work and
  // all (#1118).
  deleteTree: async (tree: string): Promise<void> => {
    if (inWails()) await StorageService.DeleteTree(tree)
    else sample.builds = (sample.builds ?? []).filter((i) => i.id !== tree)
    changed("builds")
  },

  cleared: (): Promise<ClearedCard[]> =>
    inWails() ? StorageService.Cleared() : Promise.resolve([...sampleCleared]),

  unclear: async (ids: string[]): Promise<number> => {
    let n: number
    if (inWails()) n = await StorageService.Unclear(ids)
    else {
      const before = sampleCleared.length
      sampleCleared = sampleCleared.filter((c) => !ids.includes(c.id))
      n = before - sampleCleared.length
    }
    changed("log")
    return n
  },

  clearThreadsCount: (olderThanSecs: number): Promise<number> =>
    inWails()
      ? StorageService.ClearThreadsCount(olderThanSecs)
      : Promise.resolve(
          olderThanSecs > 0 ? Math.min(2, sampleThreads) : sampleThreads
        ),

  clearThreads: async (olderThanSecs: number): Promise<number> => {
    let n: number
    if (inWails()) n = await StorageService.ClearThreads(olderThanSecs)
    else {
      n = olderThanSecs > 0 ? Math.min(2, sampleThreads) : sampleThreads
      for (let i = 0; i < n; i++)
        sampleCleared.unshift({
          id: `01SAMPLETHREAD${sampleThreads - i}`,
          short: `#${700 + sampleThreads - i}`,
          title: "Which port does the relay use?",
          board: "",
          thread: true,
          updatedAt: ago(10),
        })
      sampleThreads -= n
    }
    changed("log")
    return n
  },
}

// useStorageFresh re-reads on every change made here (a clean, a clear,
// Put back, Refresh) and, with board, on board:changed too. Sizes walk the
// disk and the log, so only the cleared list follows board:changed; the
// rest is read when the page opens and after each change made here.
export function useStorageFresh(
  reload: () => Promise<void>,
  board = false,
  scope = "*"
) {
  useEffect(() => {
    void reload()
    const fn = () => void reload()
    listeners.set(fn, scope)
    const off = board && inWails() ? Events.On(EventBoardChanged, fn) : () => {}
    return () => {
      listeners.delete(fn)
      off()
    }
  }, [reload, board, scope])
}

// refreshStorage re-reads everything the page shows (its Refresh).
export function refreshStorage() {
  changed()
}

// useStorageReport is the overview's report.
export function useStorageReport() {
  const load = useCallback(() => storage.report(), [])
  const { value, loading, error, reload } = useLatest<StorageReport>(
    load,
    { dir: "", kinds: [] },
    "storage"
  )
  useStorageFresh(reload)
  return { report: value, loading, error, reload }
}

// useStorageItems is one kind's items ("" for none).
export function useStorageItems(kind: string) {
  const load = useCallback(
    () => (kind ? storage.items(kind) : Promise.resolve([] as StorageItem[])),
    [kind]
  )
  const { value, loading, error, reload } = useLatest<StorageItem[]>(
    load,
    [],
    "the items"
  )
  useStorageFresh(reload, false, kind)
  return { items: value, loading, error, reload }
}
