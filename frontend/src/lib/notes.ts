// Notes gateway (docs/design/notes.md, card #413): short rules, gotchas,
// decisions, pointers and preferences that agents get on wake. Every note
// is the person's own, in their private log, and only their agents read
// it (decision 53, card #616). A note is live as soon as it is written,
// by the person or one of their agents, and so is every edit and removal:
// nothing waits for an approval (card #616).
//
// Like cards.ts, components import from here, never from src/gen. The
// window's notes_* commands (card #411) back it in the app; in a plain
// browser everything runs on the sample below, which the buttons change in
// memory.

import { useCallback, useEffect } from "react"

import { NotesService } from "./host"
import { onChange } from "./changes"
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

export type NoteEvent = { what: string; by: string; at: string; text?: string }

// Note is the window's NoteView (crates/window/src/notes.rs): the folded
// note.
export type Note = {
  id: string
  kind: NoteKind
  text: string
  tags: string[]
  // task:<id>, comment:<id>, blocker:<taskId>/<b1> or proof:<taskId>.
  source?: string
  taskId?: string
  by: string
  // The author is the person, not one of their agents.
  byPerson?: boolean
  at: string
  state: "accepted" | "removed"
  acceptedBy?: string
  acceptedAt?: string
  updated: string
  history: NoteEvent[]
  // Still true? (§7): why it may be out of date, when the window's
  // notes_list says so (a path: tag's path is gone, its source card was
  // deleted).
  stale?: string
}

// NewNote is New note: what note_new takes.
export type NewNote = {
  kind: NoteKind
  text: string
  tags: string[]
}

export type NoteEdit = { kind?: NoteKind; text?: string; tags?: string[] }

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
// "frontend/" for a path, "MLG" for a board: tag (a note about a space).
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

// isPerson reports whether a key is the person's (user:) rather than an
// agent's.
export function isPerson(key: string): boolean {
  return key.startsWith("user:")
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
  list: async (): Promise<Note[]> => {
    if (hostNotes()) return (await NotesService.List()) as Note[]
    // Copies, so a change shows as a new list.
    return sampleNotes()
      .filter((n) => n.state !== "removed")
      .map((n) => ({ ...n }))
  },
  create: async (n: NewNote): Promise<Note> => {
    const problem = noteProblem(n)
    if (problem) throw new Error(problem)
    const tags = cleanTags(n.tags).tags
    if (hostNotes()) {
      const out = (await NotesService.New({
        kind: n.kind,
        text: n.text.trim(),
        tags,
      })) as Note
      changed()
      return out
    }
    const now = new Date().toISOString()
    const note: Note = {
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
    })
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
    changed()
  },
}

// useNotes is the person's notes, fresh on every notes change.
export function useNotes(): {
  notes: Note[]
  loading: boolean
  error: string | null
  reload: () => Promise<void>
} {
  const load = useCallback(() => notes.list(), [])
  const { value, loading, error, reload } = useLatest<Note[]>(load, [], "notes")
  useNotesFresh(reload)
  return { notes: value, loading, error, reload }
}

// useNotesFresh reloads on every notes change: one made here, or (note
// rows fold into the board's changes) board:changed for a note written,
// edited or removed, here or synced.
function useNotesFresh(reload: () => Promise<void>) {
  useEffect(() => {
    void reload()
    listeners.add(reloadFn)
    function reloadFn() {
      void reload()
    }
    const off = inWails()
      ? onChange(EventBoardChanged, () => void reload())
      : () => {}
    return () => {
      listeners.delete(reloadFn)
      off()
    }
  }, [reload])
}

// ---- sample mode -----------------------------------------------------------

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
  // Agents on the sample roster (sample.ts), so their names show.
  const heron = "sample-device/sample02-synthetic-session"
  const otter = "sample-device/sample01-synthetic-session"
  const live = (
    id: string,
    kind: NoteKind,
    text: string,
    tags: string[],
    by: string,
    daysAgo: number,
    more: Partial<Note> = {}
  ): Note => ({
    id,
    kind,
    text,
    tags,
    by,
    byPerson: isPerson(by),
    at: at(daysAgo),
    state: "accepted",
    // An agent's note has no accepter (card #616).
    acceptedBy: isPerson(by) ? by : undefined,
    acceptedAt: isPerson(by) ? at(daysAgo) : undefined,
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
      heron,
      3,
      { source: "task:sample-card-5", taskId: "sample-card-5" }
    ),
    live(
      "sample-note-3",
      "gotcha",
      "PC release builds need CMAKE and LIBCLANG_PATH set before cargo.",
      ["machine:sample-device"],
      otter,
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
      [`path:${irori}:scripts/build.sh`],
      otter,
      9,
      { stale: "scripts/build.sh is gone from irori" }
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
      "Run the full test script with the machine quiet: presence tests time out at a load average over 20.",
      [`repo:${irori}`],
      heron,
      2
    ),
    live(
      "sample-note-8",
      "gotcha",
      "bun can't install a git dependency from a subfolder: ship suzuri-view as a tarball on the release.",
      [`path:${irori}:frontend/`],
      heron,
      0.2,
      { source: "task:sample-card-4", taskId: "sample-card-4" }
    ),
    live(
      "sample-note-9",
      "gotcha",
      "On the PC, test builds never write suzuri's bridge.json: use a scratch LOCALAPPDATA.",
      ["machine:sample-device"],
      otter,
      0.05
    ),
    live(
      "sample-note-11",
      "pointer",
      "Lap times live in the shared sheet, not in the repo.",
      ["board:mlg"],
      "user:sample",
      8
    ),
  ]
  return sample
}
