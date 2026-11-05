// The Notes pieces other pages use too (card #413): the kind badge, the
// note form (New note, Edit), and the card page's "Notes its agent got"
// list.

import { useState } from "react"
import { LockIcon, UsersIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { type BoardSummary } from "@/lib/boards"
import { plainLine, type Roster } from "@/lib/irori"
import {
  MineScope,
  NoteKinds,
  NoteMaxChars,
  noteProblem,
  tagLabel,
  useWakeNotes,
  type NewNote,
  type Note,
  type NoteKind,
} from "@/lib/notes"
import { unknownAgent } from "@/lib/names"
import { cn } from "@/lib/utils"

const kindTone: Record<
  NoteKind,
  "destructive" | "warning" | "success" | "info" | "slate"
> = {
  rule: "destructive",
  gotcha: "warning",
  decision: "success",
  pointer: "info",
  preference: "slate",
}

export function KindBadge({ k }: { k: NoteKind }) {
  return (
    <Badge
      variant={kindTone[k] ?? "slate"}
      className="mt-px h-5 shrink-0 px-1.5 text-[10px] capitalize"
    >
      {k}
    </Badge>
  )
}

// Who names a key: you, a board member, or an agent by its roster name.
export type Who = (key: string) => string

export function useWho(roster: Roster, boards: BoardSummary[]): Who {
  return (key: string) => {
    if (!key) return "someone"
    if (key.startsWith("user:")) return "You"
    if (key.startsWith("person:")) {
      const id = key.slice("person:".length)
      for (const b of boards) {
        const m = b.members.find(
          (x) => x.personId === id || `person:${x.personId}` === key
        )
        if (m) return m.me ? "You" : plainLine(m.name)
      }
      return "a member"
    }
    const a = roster.agents.find((x) => x.key === key)
    if (a && plainLine(a.name)) return plainLine(a.name)
    return unknownAgent
  }
}

// sourceLabel is a note's source chip: "#ref b1 answer", "comment",
// "done proof", or "" with none.
export function sourceLabel(n: Pick<Note, "source" | "taskId">): string {
  const s = n.source ?? ""
  if (!s) return ""
  if (s.startsWith("blocker:")) return `${s.split("/").pop()} answer`
  if (s.startsWith("comment:")) return "a comment"
  if (s.startsWith("proof:")) return "done proof"
  if (s.startsWith("task:")) return "the card"
  return s
}

export type NoteDraft = {
  scope: string
  kind: NoteKind
  text: string
  tags: string[]
}

// NoteForm is New note and Edit: kind, text, where it
// applies, and (for a new note) Mine or a board, Mine first (§8).
export function NoteForm(props: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  initial: NoteDraft
  boards?: BoardSummary[]
  // Edit: the note stays where it is.
  fixedScope?: boolean
  save?: string
  onSave: (n: NewNote) => Promise<void>
}) {
  const [busy, setBusy] = useState(false)
  return (
    <Dialog
      open={props.open}
      onOpenChange={(o) => !busy && props.onOpenChange(o)}
    >
      <DialogContent className="gap-3 sm:max-w-lg">
        {/* Mounted only while open: each opening starts from initial. */}
        {props.open && (
          <NoteFormBody {...props} busy={busy} setBusy={setBusy} />
        )}
      </DialogContent>
    </Dialog>
  )
}

function NoteFormBody({
  onOpenChange,
  title,
  description,
  initial,
  boards,
  fixedScope,
  save = "Keep",
  onSave,
  busy,
  setBusy,
}: Parameters<typeof NoteForm>[0] & {
  busy: boolean
  setBusy: (b: boolean) => void
}) {
  const [draft, setDraft] = useState(initial)
  const [tagText, setTagText] = useState(initial.tags.join(", "))
  const [error, setError] = useState("")
  const tags = tagText
    .split(/[,\n]/)
    .map((t) => t.trim())
    .filter(Boolean)
  const problem = noteProblem({ kind: draft.kind, text: draft.text, tags })
  const chars = [...draft.text.trim()].length
  const submit = async () => {
    if (problem) {
      setError(problem)
      return
    }
    setBusy(true)
    setError("")
    try {
      await onSave({
        scope: draft.scope,
        kind: draft.kind,
        text: draft.text,
        tags,
      })
      onOpenChange(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <>
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>
          {description ??
            "A short fact an agent should know before it acts. Long material belongs in the repo's docs."}
        </DialogDescription>
      </DialogHeader>
      <div className="grid gap-1">
        <Textarea
          aria-label="The note"
          value={draft.text}
          rows={3}
          onChange={(e) => setDraft({ ...draft, text: e.target.value })}
          className="text-[13px]"
        />
        <span
          className={cn(
            "justify-self-end text-[11px] tabular-nums",
            chars > NoteMaxChars ? "text-destructive" : "text-muted-foreground"
          )}
        >
          {chars}/{NoteMaxChars}
        </span>
      </div>
      <div className="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-2 text-[12.5px]">
        <span className="text-ink-2">Kind</span>
        <ChoiceSelect
          label="Kind"
          size="sm"
          value={draft.kind}
          onValueChange={(v) => setDraft({ ...draft, kind: v as NoteKind })}
          choices={NoteKinds.map((k) => ({
            value: k,
            label: <span className="capitalize">{k}</span>,
          }))}
        />
        <span className="text-ink-2">Applies to</span>
        <div className="grid gap-1">
          <Input
            aria-label="Applies to"
            value={tagText}
            onChange={(e) => setTagText(e.target.value)}
            className="h-7 font-mono text-[12px]"
          />
          <span className="text-[11px] text-muted-foreground">
            Comma-separated: everywhere, repo:&lt;repo key&gt;, path:&lt;repo
            key&gt;:&lt;path&gt;, board:&lt;tag&gt;, machine:&lt;device&gt;,
            topic:&lt;word&gt;
          </span>
          {tags.length > 0 && (
            <span className="flex flex-wrap gap-1">
              {tags.map((t) => (
                <span
                  key={t}
                  className="rounded-md border border-border px-1.5 py-0.5 text-[10.5px] text-muted-foreground"
                >
                  {tagLabel(t)}
                </span>
              ))}
            </span>
          )}
        </div>
        {!fixedScope && (
          <>
            <span className="text-ink-2">Who reads it</span>
            <ChoiceSelect
              label="Who reads it"
              size="sm"
              value={draft.scope}
              onValueChange={(v) => setDraft({ ...draft, scope: v })}
              choices={[
                {
                  value: MineScope,
                  label: (
                    <span className="flex items-center gap-1.5">
                      <LockIcon className="size-3" aria-hidden /> Mine: only my
                      agents
                    </span>
                  ),
                },
                ...(boards ?? []).map((b) => ({
                  value: b.tag,
                  label: (
                    <span className="flex items-center gap-1.5">
                      <UsersIcon className="size-3" aria-hidden />{" "}
                      {plainLine(b.name)}: every member's agents
                    </span>
                  ),
                })),
              ]}
            />
          </>
        )}
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
      <DialogFooter>
        <Button
          variant="ghost"
          size="sm"
          disabled={busy}
          onClick={() => onOpenChange(false)}
        >
          Cancel
        </Button>
        <Button size="sm" disabled={busy} onClick={() => void submit()}>
          {busy ? "Saving…" : save}
        </Button>
      </DialogFooter>
    </>
  )
}

// WakeNotes is the card page's "Notes its agent got": which notes each
// agent got on its latest wake for this card, and the tag that matched
// (card #412). Nothing when none did.
export function WakeNotes({ cardId, who }: { cardId: string; who: Who }) {
  const got = useWakeNotes(cardId)
  if (!got.length) return null
  const agents = [...new Set(got.map((g) => g.agent))]
  return (
    <div className="grid gap-2">
      {agents.map((a) => (
        <div key={a} className="grid gap-1">
          <p className="text-[12px] text-ink-2">
            {who(a)} got{" "}
            {got.filter((g) => g.agent === a).length === 1
              ? "this note"
              : "these notes"}{" "}
            on wake:
          </p>
          <ul className="grid gap-1" aria-label={`Notes ${who(a)} got`}>
            {got
              .filter((g) => g.agent === a)
              .map((g) => (
                <li
                  key={g.noteId}
                  className="flex items-start gap-2 text-[12.5px]"
                >
                  <KindBadge k={g.kind} />
                  <span className="flex-1 leading-snug text-ink">{g.text}</span>
                  <span
                    className="shrink-0 text-[11px] text-muted-foreground"
                    title={g.tag}
                  >
                    {tagLabel(g.tag)}
                  </span>
                </li>
              ))}
          </ul>
        </div>
      ))}
    </div>
  )
}
