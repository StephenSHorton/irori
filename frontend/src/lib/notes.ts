// Notes gateway (docs/design/notes.md, card #413): short rules, gotchas,
// decisions, pointers and preferences that agents get on wake. Mine lives
// in the person's private log; a shared board's notes in the board's log.
// An agent's note is proposed until a person keeps it (Mine) or any
// member accepts it (a board); a person's note is live at once (§4).
//
// Like cards.ts, components import from here, never from src/gen. The
// window's notes_* commands (card #411) back it in the app; in a plain
// browser everything runs on the sample below, which the buttons change in
// memory.

import { useCallback, useEffect } from "react"

import { Events, NotesService } from "./host"
import { EventBoardChanged } from "./cards"
import { inWails } from "./irori"
import { useLatest } from "./talk"

// ---- the shapes (crates/coord/src/board/notes.rs, §11) ---------------------

export const NoteKinds = [
  "rule",
  "gotcha",
  "decision",
  "pointer",
  "preference",
] as const
export type NoteKind = (typeof NoteKinds)[number]

// The most characters a note's text may have (§11: refused over 500).
export const NoteMaxChars = 500

// MineScope is the private log's scope; a board's scope is its tag.
export const MineScope = "mine"

// NoteChange is an agent's edit or removal of a live note, waiting for a
// person's accept: the live text stays until then.
export type NoteChange = {
  by: string
  at: string
  kind?: string
  text?: string
  // null when unchanged.
  tags?: string[] | null
  remove?: boolean
  why?: string
}

export type NoteEvent = { what: string; by: string; at: string; text?: string }

// Note is the window's NoteView (crates/window/src/notes.rs): the folded
// note and the log it lives in.
export type Note = {
  // "mine", or the board's tag.
  scope: string
  // "" for Mine.
  boardId?: string
  id: string
  kind: NoteKind
  text: string
  tags: string[]
  // task:<id>, comment:<id>, blocker:<taskId>/<b1> or proof:<taskId>.
  source?: string
  taskId?: string
  by: string
  // The author is a person (their note was live at once).
  byPerson?: boolean
  at: string
  state: "proposed" | "accepted" | "removed"
  acceptedBy?: string
  acceptedAt?: string
  updated: string
  pending?: NoteChange | null
  history: NoteEvent[]
  // Still true? (§7): why it may be out of date, when the window's
  // notes_list says so (a path: tag's path is gone, its source card was
  // deleted).
  stale?: string
}

// NewNote is New note: what note_new takes.
export type NewNote = {
  scope: string
  kind: NoteKind
  text: string
  tags: string[]
}

export type NoteEdit = { kind?: NoteKind; text?: string; tags?: string[] }

// WakeNote is one note an agent got on wake, and the tag that matched
// (card #412's card_wake_notes: the latest wake of each agent that got
// notes for the card, newest first).
export type WakeNote = {
  noteId: string
  kind: NoteKind
  text: string
  tag: string
  agent: string
  at: string
}

// ---- tags (§11) ------------------------------------------------------------

export type TagShape =
  "everywhere" | "board" | "repo" | "path" | "machine" | "topic"

export type Tag = { shape: TagShape; value: string; raw: string }

// parseTag reads one tag; null when it isn't a known shape. Repo keys and
// paths keep their case; the rest fold to lower case, as the fold does.
export function parseTag(raw: string): Tag | null {
  const t = raw.trim()
  if (!t) return null
  if (t.toLowerCase() === "everywhere")
    return { shape: "everywhere", value: "", raw: "everywhere" }
  const i = t.indexOf(":")
  if (i <= 0) return null
  const head = t.slice(0, i).toLowerCase()
  const rest = t.slice(i + 1).trim()
  if (!rest) return null
  switch (head) {
    case "repo":
      return { shape: "repo", value: rest, raw: `repo:${rest}` }
    case "path": {
      // path:<repoKey>:<path>; a repo key has colons of its own
      // (remote:github.com/x/y), so the path is after the last one.
      const j = rest.lastIndexOf(":")
      if (j <= 0 || j === rest.length - 1) return null
      return { shape: "path", value: rest, raw: `path:${rest}` }
    }
    case "board":
    case "machine":
    case "topic": {
      const v = rest.toLowerCase()
      return { shape: head, value: v, raw: `${head}:${v}` }
    }
  }
  return null
}

// repoName is a repo key's short name: remote:github.com/o/irori is irori.
export function repoName(key: string): string {
  const k = key.replace(/^remote:/, "").replace(/\.git$/, "")
  if (k.startsWith("root:")) return k.slice(5, 12)
  const parts = k.split("/")
  return parts[parts.length - 1] || k
}

// tagLabel is how a tag reads on a row: "repo irori", "on the PC",
// "frontend/" for a path, "MLG" for a board.
export function tagLabel(
  raw: string,
  boardName?: (tag: string) => string
): string {
  const t = parseTag(raw)
  if (!t) return raw
  switch (t.shape) {
    case "everywhere":
      return "everywhere"
    case "repo":
      return `repo ${repoName(t.value)}`
    case "path": {
      const j = t.value.lastIndexOf(":")
      return `${repoName(t.value.slice(0, j))} ${t.value.slice(j + 1)}`
    }
    case "board":
      return boardName?.(t.value) || t.value
    case "machine":
      return `on ${t.value}`
    case "topic":
      return `#${t.value}`
  }
}

// cleanTags parses tags as typed (comma- or space-separated), dropping
// repeats; bad holds the ones that aren't a known shape.
export function cleanTags(input: string[]): { tags: string[]; bad: string[] } {
  const tags: string[] = []
  const bad: string[] = []
  for (const raw of input) {
    const t = parseTag(raw)
    if (!t) {
      if (raw.trim()) bad.push(raw.trim())
      continue
    }
    if (!tags.includes(t.raw)) tags.push(t.raw)
  }
  return { tags, bad }
}

// noteProblem is why New note or Edit can't send yet, the same
// checks the write makes (§11), or "".
export function noteProblem(n: {
  kind: string
  text: string
  tags: string[]
}): string {
  if (!(NoteKinds as readonly string[]).includes(n.kind)) return "Pick a kind."
  if (!n.text.trim()) return "Write the note first."
  if ([...n.text.trim()].length > NoteMaxChars)
    return `A note is at most ${NoteMaxChars} characters; long material belongs in the repo's docs.`
  const { tags, bad } = cleanTags(n.tags)
  if (bad.length)
    return `Not a tag Irori knows: ${bad.join(", ")}. Use everywhere, repo:, path:, board:, machine: or topic:.`
  if (!tags.length) return "Say where it applies: at least one tag."
  return ""
}

// isPerson reports whether a key is a person's (user: here, person: on a
// board) rather than an agent's.
export function isPerson(key: string): boolean {
  return key.startsWith("user:") || key.startsWith("person:")
}

// needsLook is a note in New from agents: an agent's proposal, or an
// agent's pending edit or removal of a live note.
export function needsLook(n: Note): boolean {
  return n.state === "proposed" || (n.state === "accepted" && !!n.pending)
}

export type KindCount = { kind: NoteKind | "all"; count: number }

// kindCounts is the filter chips: All, then each kind the live list has.
export function kindCounts(notes: Note[]): KindCount[] {
  const live = notes.filter((n) => n.state === "accepted")
  const out: KindCount[] = [{ kind: "all", count: live.length }]
  for (const k of NoteKinds) {
    const c = live.filter((n) => n.kind === k).length
    if (c) out.push({ kind: k, count: c })
  }
  return out
}

// ---- the gateway -----------------------------------------------------------


const listeners = new Set<() => void>()
function changed() {
  for (const fn of listeners) fn()
}

// hostNotes reports whether the window has the notes commands yet.
function hostNotes(): boolean {
  return inWails()
}

export const notes = {
  list: async (scope: string): Promise<Note[]> => {
    if (hostNotes()) return (await NotesService.List(scope)) as Note[]
    // Copies, so a change shows as a new list.
    return sampleNotes()
      .filter((n) => (!scope || n.scope === scope) && n.state !== "removed")
      .map((n) => ({ ...n }))
  },
  create: async (n: NewNote): Promise<Note> => {
    const problem = noteProblem(n)
    if (problem) throw new Error(problem)
    const tags = cleanTags(n.tags).tags
    if (hostNotes()) {
      const out = (await NotesService.New({
        ...n,
        text: n.text.trim(),
        tags,
      })) as Note
      changed()
      return out
    }
    const now = new Date().toISOString()
    const note: Note = {
      scope: n.scope,
      boardId: n.scope === MineScope ? undefined : sampleBoardId(n.scope),
      id: `sample-note-${Math.random().toString(36).slice(2, 8)}`,
      kind: n.kind,
      text: n.text.trim(),
      tags,
      by: "user:sample",
      byPerson: true,
      at: now,
      state: "accepted",
      acceptedBy: "user:sample",
      acceptedAt: now,
      updated: now,
      history: [{ what: "proposed", by: "user:sample", at: now }],
    }
    sampleNotes().unshift(note)
    changed()
    return note
  },
  accept: async (id: string): Promise<void> => {
    if (hostNotes()) {
      await NotesService.Accept(id)
      changed()
      return
    }
    const n = sampleNote(id)
    const now = new Date().toISOString()
    if (n.pending?.remove) {
      n.state = "removed"
    } else if (n.pending) {
      Object.assign(n, {
        kind: n.pending.kind || n.kind,
        text: n.pending.text || n.text,
        tags: n.pending.tags ?? n.tags,
        updated: now,
      })
    } else {
      n.state = "accepted"
    }
    n.pending = undefined
    n.acceptedBy = "user:sample"
    n.acceptedAt = now
    changed()
  },
  edit: async (id: string, e: NoteEdit): Promise<void> => {
    const problem = noteProblem({
      kind: e.kind ?? "rule",
      text: e.text ?? "x",
      tags: e.tags ?? ["everywhere"],
    })
    if (problem) throw new Error(problem)
    const tags = e.tags ? cleanTags(e.tags).tags : undefined
    if (hostNotes()) {
      await NotesService.Edit(id, { ...e, tags, text: e.text?.trim() })
      changed()
      return
    }
    const n = sampleNote(id)
    Object.assign(n, {
      kind: e.kind ?? n.kind,
      text: e.text?.trim() ?? n.text,
      tags: tags ?? n.tags,
      updated: new Date().toISOString(),
      pending: undefined,
    })
    // A person's edit of a proposal is their word on it (§4).
    if (n.state === "proposed") {
      n.state = "accepted"
      n.acceptedBy = "user:sample"
      n.acceptedAt = n.updated
    }
    changed()
  },
  remove: async (id: string, why = ""): Promise<void> => {
    if (hostNotes()) {
      await NotesService.Remove(id, why)
      changed()
      return
    }
    const n = sampleNote(id)
    n.state = "removed"
    n.pending = undefined
    changed()
  },
}

// useNotes is one scope's notes, fresh on every notes change.
export function useNotes(scope: string): {
  notes: Note[]
  loading: boolean
  error: string | null
  reload: () => Promise<void>
} {
  const load = useCallback(() => notes.list(scope), [scope])
  const { value, loading, error, reload } = useLatest<Note[]>(load, [])
  useNotesFresh(reload)
  return { notes: value, loading, error, reload }
}

// useNotesFresh reloads on every notes change: one made here, or (note
// rows fold into the board's changes) board:changed for a note proposed,
// accepted, edited or removed, here or synced.
function useNotesFresh(reload: () => Promise<void>) {
  useEffect(() => {
    void reload()
    listeners.add(reloadFn)
    function reloadFn() {
      void reload()
    }
    const off = inWails()
      ? Events.On(EventBoardChanged, () => void reload())
      : () => {}
    return () => {
      listeners.delete(reloadFn)
      off()
    }
  }, [reload])
}

// useNotesWaiting is the sidebar's Notes count: notes in Mine and every
// board waiting on the person's look. It reads notes_waiting, a count,
// not every note with its Still true? checks (card #468).
export function useNotesWaiting(): number {
  const load = useCallback(async (): Promise<number> => {
    if (hostNotes()) return NotesService.Waiting()
    return (await notes.list("")).filter(needsLook).length
  }, [])
  const { value, reload } = useLatest<number>(load, 0)
  useNotesFresh(reload)
  return value
}

// wakeNotes is what card_wake_notes says the card's agents got on wake
// (card #412).
export async function wakeNotes(taskId: string): Promise<WakeNote[]> {
  if (inWails()) {
    return (await NotesService.CardWakeNotes(taskId)) as WakeNote[]
  }
  return sampleWake(taskId)
}

// useWakeNotes is wakeNotes for the open card, re-read when notes change.
export function useWakeNotes(taskId: string): WakeNote[] {
  const load = useCallback(() => wakeNotes(taskId), [taskId])
  const { value, reload } = useLatest<WakeNote[]>(load, [])
  useEffect(() => {
    void reload()
  }, [reload])
  return value
}

// ---- sample mode -----------------------------------------------------------

function sampleBoardId(scope: string): string {
  return scope === "mlg" ? "sample-board-mlg" : scope
}

function sampleWake(taskId: string): WakeNote[] {
  if (taskId !== "sample-card-4" && taskId !== "sample-card-1") return []
  const at = new Date(Date.now() - 20 * 60 * 1000).toISOString()
  const pick = (id: string, tag: string): WakeNote => {
    const n = sampleNotes().find((x) => x.id === id)!
    return {
      noteId: n.id,
      kind: n.kind,
      text: n.text,
      tag,
      agent: "sample-device/sample-heron",
      at,
    }
  }
  return [
    pick("sample-note-1", "everywhere"),
    pick("sample-note-3", "machine:the pc"),
    pick("sample-note-2", "repo:remote:github.com/StephenSHorton/irori"),
  ]
}

let sample: Note[] | null = null

function sampleNote(id: string): Note {
  const bare = id.replace(/^note:/, "")
  const n = sampleNotes().find((x) => x.id === bare)
  if (!n) throw new Error(`coord: no such note: ${id}`)
  return n
}

// resetSampleNotes is for tests.
export function resetSampleNotes(): void {
  sample = null
}

function sampleNotes(): Note[] {
  if (sample) return sample
  const day = 24 * 3600 * 1000
  const at = (daysAgo: number) =>
    new Date(Date.now() - daysAgo * day).toISOString()
  const irori = "remote:github.com/StephenSHorton/irori"
  const live = (
    id: string,
    kind: NoteKind,
    text: string,
    tags: string[],
    by: string,
    daysAgo: number,
    more: Partial<Note> = {}
  ): Note => ({
    scope: MineScope,
    id,
    kind,
    text,
    tags,
    by,
    byPerson: isPerson(by),
    at: at(daysAgo),
    state: "accepted",
    acceptedBy: isPerson(by) ? by : "user:sample",
    acceptedAt: at(daysAgo),
    updated: at(daysAgo),
    history: [],
    ...more,
  })
  sample = [
    live(
      "sample-note-1",
      "rule",
      "Never drive Stephen's mouse or keyboard. UI checks use passive screenshots or the browser preview.",
      ["everywhere"],
      "user:sample",
      1,
      { source: "comment:sample-comment-1", taskId: "sample-card-1" }
    ),
    live(
      "sample-note-2",
      "gotcha",
      "A scratch Irori dev build still hands asks to live agents. Never send from one.",
      [`repo:${irori}`],
      "sample-device/sample-heron",
      3,
      { source: "task:sample-card-5", taskId: "sample-card-5" }
    ),
    live(
      "sample-note-3",
      "gotcha",
      "PC release builds need CMAKE and LIBCLANG_PATH set before cargo.",
      [`repo:${irori}`, "machine:the pc"],
      "sample-device/sample-kestrel",
      4
    ),
    live(
      "sample-note-4",
      "decision",
      "Devices meet through n0's public relays, on by default (decision 43).",
      [`repo:${irori}`],
      "user:sample",
      6,
      { source: "blocker:sample-card-2/b1", taskId: "sample-card-2" }
    ),
    live(
      "sample-note-5",
      "gotcha",
      "Each worktree builds into its own target/. A shared target mixes agents' crates.",
      [`path:${irori}:scripts/check.sh`],
      "sample-device/sample-kestrel",
      9,
      { stale: "scripts/check.sh changed since it was kept" }
    ),
    live(
      "sample-note-6",
      "preference",
      "The app says Version N, never take.",
      ["repo:remote:github.com/StephenSHorton/pipeline"],
      "user:sample",
      12
    ),
    live(
      "sample-note-7",
      "gotcha",
      "Run scripts/check.sh with the machine quiet: presence tests time out at a load average over 20.",
      [`repo:${irori}`],
      "sample-device/sample-heron",
      2,
      {
        pending: {
          by: "sample-device/sample-kestrel",
          at: at(0.1),
          text: "Run scripts/check.sh with the machine quiet: presence tests time out at a load average over 20; rerun a failed file alone.",
          why: "A failing file passes when run alone.",
        },
      }
    ),
    {
      ...live(
        "sample-note-8",
        "gotcha",
        "bun can't install a git dependency from a subfolder: ship suzuri-view as a tarball on the release.",
        [`path:${irori}:frontend/`],
        "sample-device/sample-heron",
        0.2,
        { source: "task:sample-card-4", taskId: "sample-card-4" }
      ),
      state: "proposed",
      acceptedBy: undefined,
      acceptedAt: undefined,
    },
    {
      ...live(
        "sample-note-9",
        "gotcha",
        "On the PC, test builds never write suzuri's bridge.json: use a scratch LOCALAPPDATA.",
        ["repo:remote:github.com/StephenSHorton/suzuri", "machine:the pc"],
        "sample-device/sample-kestrel",
        0.05
      ),
      state: "proposed",
      acceptedBy: undefined,
      acceptedAt: undefined,
    },
    live(
      "sample-note-10",
      "decision",
      "Boats stop at the dock's edge: the collider is wider, two-PC check passed.",
      ["everywhere"],
      "person:sample-person-0",
      5,
      {
        scope: "mlg",
        boardId: "sample-board-mlg",
        source: "task:sample-board-1",
        taskId: "sample-board-1",
      }
    ),
    live(
      "sample-note-11",
      "pointer",
      "Lap times live in the shared sheet, not in the repo.",
      ["everywhere"],
      "user:sample",
      8,
      {
        scope: "sample-board-mlg",
        boardId: "sample-board-mlg",
      }
    ),
    {
      ...live(
        "sample-note-12",
        "gotcha",
        "Run the lobby test with both PCs on the same build hash, or the join silently fails.",
        ["topic:lobby"],
        "sample-blake-device/sample-kite",
        0.3,
        {
          scope: "mlg",
          boardId: "sample-board-mlg",
          source: "task:sample-board-3",
          taskId: "sample-board-3",
        }
      ),
      state: "proposed",
      acceptedBy: undefined,
      acceptedAt: undefined,
    },
  ]
  return sample
}
