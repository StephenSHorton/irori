// NotesPage is Notes (docs/design/notes.md §9, card #413): the short rules,
// gotchas, decisions, pointers and preferences agents get on wake. Notes
// are the person's own (card #616): one list, read only by their agents.
// An agent's note, edit or removal applies at once, with no approval; each
// row says who wrote it, and you edit or remove any of them. The list is
// filtered by kind.

import { useMemo, useState } from "react"
import {
  AlertTriangleIcon,
  PencilIcon,
  PlusIcon,
  SearchIcon,
  Trash2Icon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
import type { BoardSummary } from "@/lib/boards"
import { plainLine, relativeTime, type Roster } from "@/lib/irori"
import {
  kindCounts,
  notes,
  tagLabel,
  useNotes,
  type Note,
  type NoteKind,
} from "@/lib/notes"
import { cn } from "@/lib/utils"

import { KindBadge, NoteForm, sourceLabel, useWho, type Who } from "./parts"

export function NotesPage({
  boards,
  roster,
  onOpen,
  now,
}: {
  // For board: tags, which name a space by its tag.
  boards: BoardSummary[]
  roster: Roster
  onOpen: (cardId: string) => void
  now: Date
}) {
  const { notes: list, loading, error } = useNotes()
  const [kind, setKind] = useState<NoteKind | "all">("all")
  const [query, setQuery] = useState("")
  const [creating, setCreating] = useState(false)
  const who = useWho(roster, boards)
  const boardName = (t: string) => {
    const b = boards.find((x) => x.tag.toLowerCase() === t.toLowerCase())
    return b ? plainLine(b.name) : t
  }

  const counts = kindCounts(list)
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase()
    return list
      .filter((n) => n.state === "accepted")
      .filter((n) => kind === "all" || n.kind === kind)
      .filter(
        (n) =>
          !q ||
          n.text.toLowerCase().includes(q) ||
          n.tags.some((t) => t.toLowerCase().includes(q))
      )
      .sort((a, b) => Date.parse(b.updated) - Date.parse(a.updated))
  }, [list, kind, query])

  return (
    <div className="grid content-start gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold text-ink">Notes</h1>
        <div className="ml-auto flex items-center gap-2">
          <label className="relative">
            <span className="sr-only">Search notes</span>
            <SearchIcon
              className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search notes"
              className="h-8 w-56 pl-8 text-[12.5px]"
            />
          </label>
          <Button size="sm" onClick={() => setCreating(true)}>
            <PlusIcon className="size-3.5" aria-hidden />
            New note
          </Button>
        </div>
      </div>
      <p className="max-w-3xl text-[13px] text-muted-foreground">
        Notes stay on your devices and only your agents read them, even when a
        note is about a shared space. Agents write and change them as they
        learn; edit or remove any here. Agents get the few that match their card
        on wake.
      </p>
      {error && (
        <p className="text-xs text-destructive">
          Could not load the notes: {error}
        </p>
      )}

      <section aria-label="Your notes" className="grid gap-2">
        {/* The kind chips and the list wait for the first read together
            (card #598), so neither shows a false 0 or a blank; loading
            never comes back. */}
        <Loading
          loading={loading}
          label="Loading the notes"
          className="min-w-0"
        >
          {loading ? (
            <NotesShape />
          ) : (
            <div className="grid gap-2">
              <div
                role="group"
                aria-label="Kind"
                className="flex flex-wrap items-center gap-1.5 text-[12px]"
              >
                {counts.map((c) => (
                  <button
                    key={c.kind}
                    type="button"
                    aria-pressed={kind === c.kind}
                    onClick={() => setKind(c.kind)}
                    className={cn(
                      "rounded-full px-2.5 py-1 text-ink-2 capitalize",
                      kind === c.kind
                        ? "bg-secondary text-ink"
                        : "border border-border hover:bg-secondary/60"
                    )}
                  >
                    {c.kind === "all" ? "All" : `${c.kind}s`} {c.count}
                  </button>
                ))}
              </div>
              {shown.length > 0 ? (
                <ul
                  className="overflow-hidden p-0! glass-panel"
                  aria-label="Notes"
                >
                  {shown.map((n) => (
                    <NoteRow
                      key={n.id}
                      n={n}
                      who={who}
                      boardName={boardName}
                      onOpen={onOpen}
                      now={now}
                    />
                  ))}
                </ul>
              ) : (
                <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-[13px] text-muted-foreground">
                  {query || kind !== "all"
                    ? "No notes match."
                    : "No notes yet. Agents add them as they learn, or write one with New note."}
                </p>
              )}
            </div>
          )}
        </Loading>
      </section>

      <NoteForm
        open={creating}
        onOpenChange={setCreating}
        title="New note"
        initial={{ kind: "rule", text: "", tags: ["everywhere"] }}
        onSave={async (n) => {
          await notes.create(n)
        }}
      />
    </div>
  )
}

// NotesShape is the notes list's skeleton: the kind chips, then rows in
// NoteRow's frame, plain markup with no hooks.
function NotesShape() {
  const chips = ["All 14", "rules 5", "gotchas 3", "decisions 4", "pointers 2"]
  const rows = [
    "Run the two-PC check before calling a netcode change done.",
    "The dock colliders live in the harbour scene, not the prefab; edit them there.",
    "Prefer the smaller change first, then ask.",
    "Release builds sign on the Mac only.",
  ]
  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap items-center gap-1.5 text-[12px]">
        {chips.map((c) => (
          <span
            key={c}
            className="rounded-full border border-border px-2.5 py-1"
          >
            {c}
          </span>
        ))}
      </div>
      <ul className="overflow-hidden p-0! glass-panel">
        {rows.map((t) => (
          <li
            key={t}
            className="grid gap-1.5 border-b border-border/70 px-4 py-3 last:border-b-0"
          >
            <div className="flex items-start gap-2">
              <span className="block h-5 w-12 rounded-md" />
              <span className="flex-1 text-[13px] leading-snug text-ink">
                {t}
              </span>
              <span className="block size-7 rounded-md" />
              <span className="block size-7 rounded-md" />
            </div>
            <span className="text-[11px] text-muted-foreground">
              everywhere · sample-heron · 3 days ago
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}

// Meta is a row's second line: tags, source, author and accepter.
function Meta({
  n,
  who,
  boardName,
  onOpen,
  now,
}: {
  n: Note
  who: Who
  boardName: (tag: string) => string
  onOpen: (cardId: string) => void
  now: Date
}) {
  const src = sourceLabel(n)
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
      {n.tags.map((t) => (
        <span
          key={t}
          title={t}
          className="inline-flex items-center rounded-md border border-border px-1.5 py-0.5 text-[10.5px]"
        >
          {tagLabel(t, boardName)}
        </span>
      ))}
      {src && n.taskId && (
        <button
          type="button"
          onClick={() => onOpen(n.taskId!)}
          title="Open the card it came from"
          className="inline-flex items-center gap-1 rounded-md bg-secondary px-1.5 py-0.5 text-[11px] text-ink-2 hover:text-ink"
        >
          {src}
        </button>
      )}
      <span>
        {who(n.by)}
        {n.acceptedBy && n.acceptedBy !== n.by && (
          <> · kept by {who(n.acceptedBy)}</>
        )}
        {" · "}
        {relativeTime(new Date(n.updated), now)}
      </span>
      {n.stale && (
        <span
          className="flex items-center gap-1 text-warning"
          title="Irori found something it names has changed"
        >
          <AlertTriangleIcon className="size-3" aria-hidden /> Still true?{" "}
          {n.stale}
        </span>
      )}
    </div>
  )
}

function NoteRow({
  n,
  who,
  boardName,
  onOpen,
  now,
}: {
  n: Note
  who: Who
  boardName: (tag: string) => string
  onOpen: (cardId: string) => void
  now: Date
}) {
  const [editing, setEditing] = useState(false)
  const [error, setError] = useState("")
  return (
    <li className="grid gap-1.5 border-b border-border/70 px-4 py-3 last:border-b-0">
      <div className="flex items-start gap-2">
        <KindBadge k={n.kind} />
        <span className="flex-1 text-[13px] leading-snug whitespace-pre-wrap text-ink">
          {n.text}
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          title="Edit the note"
          aria-label="Edit the note"
          onClick={() => setEditing(true)}
        >
          <PencilIcon className="size-3.5" aria-hidden />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          title="Remove the note: no agent gets it any more"
          aria-label="Remove the note"
          onClick={() => {
            setError("")
            notes
              .remove(n.id)
              .catch((e: unknown) =>
                setError(e instanceof Error ? e.message : String(e))
              )
          }}
        >
          <Trash2Icon className="size-3.5" aria-hidden />
        </Button>
      </div>
      <Meta n={n} who={who} boardName={boardName} onOpen={onOpen} now={now} />
      {error && <p className="text-xs text-destructive">{error}</p>}
      <NoteForm
        open={editing}
        onOpenChange={setEditing}
        title="Edit note"
        initial={{ kind: n.kind, text: n.text, tags: n.tags }}
        onSave={async (e) => {
          await notes.edit(n.id, { kind: e.kind, text: e.text, tags: e.tags })
        }}
      />
    </li>
  )
}
