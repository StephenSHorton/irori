// NotesPage is Notes (docs/design/notes.md §9, card #413): the short rules,
// gotchas, decisions, pointers and preferences agents get on wake. One tab
// for Mine (the private log) and one per board. New from agents sits at
// the top: an agent's proposal reaches no agent until you keep it (Mine)
// or a member accepts it (a board), and an agent's edit or removal of a
// live note waits the same way. Below, the live notes, filtered by kind.

import { useMemo, useState } from "react"
import {
  AlertTriangleIcon,
  CheckIcon,
  LockIcon,
  PencilIcon,
  PlusIcon,
  SearchIcon,
  SparklesIcon,
  Trash2Icon,
  UsersIcon,
  XIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
import type { BoardSummary } from "@/lib/boards"
import { plainLine, relativeTime, type Roster } from "@/lib/irori"
import {
  MineScope,
  kindCounts,
  needsLook,
  notes,
  tagLabel,
  useNotes,
  type Note,
  type NoteKind,
} from "@/lib/notes"
import { cn } from "@/lib/utils"

import { KindBadge, NoteForm, sourceLabel, useWho, type Who } from "./parts"

export function NotesPage({
  tab,
  boards,
  roster,
  onTab,
  onOpen,
  now,
}: {
  // "" for Mine, else a board's tag.
  tab: string
  boards: BoardSummary[]
  roster: Roster
  onTab: (tab: string) => void
  onOpen: (cardId: string) => void
  now: Date
}) {
  const board = tab
    ? boards.find((b) => b.tag.toLowerCase() === tab.toLowerCase())
    : undefined
  const scope = board ? board.tag : MineScope
  const { notes: list, loading, error } = useNotes(scope)
  const [kind, setKind] = useState<NoteKind | "all">("all")
  const [query, setQuery] = useState("")
  const [creating, setCreating] = useState(false)
  const who = useWho(roster, boards)
  const boardName = (t: string) => {
    const b = boards.find((x) => x.tag.toLowerCase() === t.toLowerCase())
    return b ? plainLine(b.name) : t
  }

  const waiting = list.filter(needsLook)
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

  const scopeLabel = board ? plainLine(board.name) : "Mine"
  return (
    <div className="grid content-start gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-lg font-semibold text-ink">Notes</h1>
        <div
          role="tablist"
          aria-label="Whose notes"
          className="flex flex-wrap gap-1 rounded-lg bg-secondary/60 p-0.5 text-[12.5px]"
        >
          <ScopeTab
            label="Mine"
            icon={LockIcon}
            current={!board}
            onClick={() => onTab("")}
          />
          {boards.map((b) => (
            <ScopeTab
              key={b.boardId}
              label={plainLine(b.name) || b.tag}
              icon={UsersIcon}
              current={board?.boardId === b.boardId}
              onClick={() => onTab(b.tag)}
            />
          ))}
        </div>
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
        {board
          ? `Notes on ${scopeLabel} reach every member's agents once anyone in the space accepts them. Any member can edit or remove one.`
          : "Mine stays on your devices and only your agents read it, even when a note is about a shared space. Agents get the few that match their card on wake."}
      </p>
      {error && (
        <p className="text-xs text-destructive">
          Could not load the notes: {error}
        </p>
      )}

      {waiting.length > 0 && (
        <section aria-label="New from agents" className="grid gap-2">
          <h2 className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
            New from agents · {waiting.length}
          </h2>
          {waiting.map((n) => (
            <Proposed
              key={n.id}
              n={n}
              shared={!!board}
              who={who}
              boardName={boardName}
              onOpen={onOpen}
              now={now}
            />
          ))}
        </section>
      )}

      <section aria-label={`${scopeLabel} notes`} className="grid gap-2">
        {/* The kind chips and the list wait for the first read together
            (card #598), so neither shows a false 0 or a blank. Another
            scope's read keeps the shown list; loading never comes back. */}
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
                    : board
                      ? `No notes on ${scopeLabel} yet. Agents propose them here, or write one with New note.`
                      : "No notes yet. Agents propose them here, or write one with New note."}
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
        boards={boards}
        initial={{
          scope,
          kind: "rule",
          text: "",
          tags: board ? ["everywhere"] : ["everywhere"],
        }}
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

function ScopeTab({
  label,
  icon: Icon,
  current,
  onClick,
}: {
  label: string
  icon: typeof LockIcon
  current: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={current}
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-md px-2.5 py-1 text-ink-2",
        current && "bg-background font-medium text-ink shadow-sm"
      )}
    >
      <Icon className="size-3" aria-hidden />
      {label}
    </button>
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
        fixedScope
        initial={{ scope: n.scope, kind: n.kind, text: n.text, tags: n.tags }}
        onSave={async (e) => {
          await notes.edit(n.id, { kind: e.kind, text: e.text, tags: e.tags })
        }}
      />
    </li>
  )
}

// Proposed is one item of New from agents: a new note an agent proposed,
// or its edit or removal of a live note, with Keep or Accept, Edit, and
// Remove or Reject.
function Proposed({
  n,
  shared,
  who,
  boardName,
  onOpen,
  now,
}: {
  n: Note
  shared: boolean
  who: Who
  boardName: (tag: string) => string
  onOpen: (cardId: string) => void
  now: Date
}) {
  const [editing, setEditing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const p = n.pending
  const proposer = p ? p.by : n.by
  const run = (fn: () => Promise<void>) => {
    setBusy(true)
    setError("")
    fn()
      .catch((e: unknown) =>
        setError(e instanceof Error ? e.message : String(e))
      )
      .finally(() => setBusy(false))
  }
  const what = !p
    ? "proposes a note"
    : p.remove
      ? "proposes removing this note"
      : "proposes an edit"
  const yes = !p
    ? shared
      ? "Accept"
      : "Keep"
    : p.remove
      ? "Remove it"
      : "Accept the edit"
  const no = !p ? (shared ? "Reject" : "Remove") : "Keep it as it is"
  return (
    <div className="grid gap-2 rounded-lg border border-info/40 bg-info-foreground/40 p-3">
      <div className="flex flex-wrap items-center gap-2 text-[11.5px] text-ink-2">
        <SparklesIcon className="size-3.5 text-info" aria-hidden />
        <span>
          <b className="font-medium text-ink">{who(proposer)}</b> {what}
        </span>
        <span className="text-muted-foreground">
          {relativeTime(new Date(p ? p.at : n.at), now)}
        </span>
        <span className="ml-auto">
          <Badge variant="warning" className="h-5 px-1.5 text-[10px]">
            {p ? "Agents still get the live note" : "No agent gets it yet"}
          </Badge>
        </span>
      </div>
      <div className="flex items-start gap-2">
        <KindBadge k={(p?.kind as NoteKind) || n.kind} />
        <div className="grid flex-1 gap-1">
          {p && !p.remove && p.text && p.text !== n.text ? (
            <>
              <span className="text-[13px] leading-snug text-muted-foreground line-through">
                {n.text}
              </span>
              <span className="text-[13px] leading-snug text-ink">
                {p.text}
              </span>
            </>
          ) : (
            <span
              className={cn(
                "text-[13px] leading-snug text-ink",
                p?.remove && "line-through"
              )}
            >
              {n.text}
            </span>
          )}
          {p?.why && (
            <span className="text-[12px] text-ink-2">Why: {p.why}</span>
          )}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <Meta
          n={p?.tags ? { ...n, tags: p.tags } : n}
          who={who}
          boardName={boardName}
          onOpen={onOpen}
          now={now}
        />
        <span className="ml-auto flex gap-1.5">
          <Button
            size="sm"
            disabled={busy}
            onClick={() => run(() => notes.accept(n.id))}
          >
            <CheckIcon className="size-3.5" aria-hidden /> {yes}
          </Button>
          {!p?.remove && (
            <Button
              variant="secondary"
              size="sm"
              disabled={busy}
              onClick={() => setEditing(true)}
            >
              <PencilIcon className="size-3.5" aria-hidden /> Edit
            </Button>
          )}
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() =>
              run(() =>
                p
                  ? // Turning down a change is a person's edit that keeps the
                    // live note: it drops the pending change (§11).
                    notes.edit(n.id, {
                      kind: n.kind,
                      text: n.text,
                      tags: n.tags,
                    })
                  : notes.remove(n.id)
              )
            }
          >
            <XIcon className="size-3.5" aria-hidden /> {no}
          </Button>
        </span>
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
      <NoteForm
        open={editing}
        onOpenChange={setEditing}
        title={
          p
            ? "Edit before accepting"
            : shared
              ? "Edit, then accept"
              : "Edit, then keep"
        }
        fixedScope
        save={shared ? "Accept" : "Keep"}
        initial={{
          scope: n.scope,
          kind: ((p?.kind as NoteKind) || n.kind) as NoteKind,
          text: p?.text || n.text,
          tags: p?.tags ?? n.tags,
        }}
        onSave={async (e) => {
          // A person's edit applies at once; on a proposal it is their word
          // that it goes live.
          await notes.edit(n.id, { kind: e.kind, text: e.text, tags: e.tags })
          if (n.state === "proposed") await notes.accept(n.id).catch(() => {})
        }}
      />
    </div>
  )
}
