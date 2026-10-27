import type { ReactNode } from "react"
import { ExternalLinkIcon, GitCommitIcon, FileIcon, XIcon } from "lucide-react"

import { cn } from "@/lib/utils"
import { plainLine, shortRepo } from "@/lib/irori"
import {
  board,
  BlockerKind,
  isUser,
  LinkKind,
  TaskState,
  type BlockedBy,
  type Link,
  type Task,
} from "@/lib/board"

// Inputs share the Inbox composer's look.
export const fieldClass =
  "h-8 min-w-0 rounded-lg border border-input bg-white/70 px-2 text-sm text-ink outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
// textareaClass tunes the mokei Textarea to match: rows set the height, and
// the white field holds even when the system theme is dark (Irori's tokens
// are light-only, but mokei's dark: variants still apply).
export const textareaClass =
  "min-h-0 bg-white/70 text-sm text-ink dark:bg-white/70"

type Tone = "you" | "block" | "task" | "done" | "shared" | "plain"

const tones: Record<Tone, string> = {
  you: "bg-destructive/10 text-destructive",
  block: "bg-warning-foreground text-warning",
  task: "bg-info-foreground text-info",
  done: "bg-success-foreground text-success",
  shared: "bg-[#ede4fe] text-[#6d28d9]",
  plain: "bg-muted text-muted-foreground",
}

// Pill is a card's small status label.
export function Pill({
  tone,
  children,
  title,
}: {
  tone: Tone
  children: ReactNode
  title?: string
}) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-[19px] max-w-56 items-center truncate rounded-full px-2 text-[11.5px] font-medium whitespace-nowrap",
        tones[tone]
      )}
    >
      {children}
    </span>
  )
}

// LinkChip shows a link by its own name ("Linear ABC-123", "GitHub #57").
// A URL opens in the browser; a path or commit is only named.
export function LinkChip({
  link,
  onRemove,
}: {
  link: Link
  onRemove?: () => void
}) {
  const label = plainLine(link.label) || plainLine(link.ref)
  const Icon =
    link.kind === LinkKind.LinkURL
      ? ExternalLinkIcon
      : link.kind === LinkKind.LinkCommit
        ? GitCommitIcon
        : FileIcon
  const body = (
    <>
      <Icon className="size-3 shrink-0 text-muted-foreground" aria-hidden />
      <span className="truncate">{label}</span>
    </>
  )
  const chip =
    "inline-flex h-[21px] max-w-60 items-center gap-1 rounded-md bg-muted px-1.5 font-mono text-[11.5px] text-ink-2 ring-1 ring-border ring-inset"
  return (
    <span className="inline-flex items-center">
      {link.kind === LinkKind.LinkURL ? (
        <button
          type="button"
          title={link.ref}
          onClick={(e) => {
            e.stopPropagation()
            void board.openLink(link)
          }}
          className={cn(
            chip,
            "outline-none hover:text-ink hover:ring-ring focus-visible:ring-2 focus-visible:ring-ring/50"
          )}
        >
          {body}
        </button>
      ) : (
        <span className={chip} title={link.ref}>
          {body}
        </span>
      )}
      {onRemove && (
        <button
          type="button"
          aria-label={`Remove link ${label}`}
          title="Remove link"
          onClick={onRemove}
          className="ml-0.5 rounded p-0.5 text-muted-foreground outline-none hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <XIcon className="size-3" aria-hidden />
        </button>
      )}
    </span>
  )
}

// blockedLabel says what a blocked task waits on, in a few words.
export function blockedLabel(
  b: BlockedBy | null | undefined,
  name: (key: string) => string,
  title: (id: string) => string
): string {
  if (!b) return "something"
  const ref = plainLine(b.ref ?? "")
  switch (b.kind) {
    case BlockerKind.BlockedOnPerson:
      return !ref || isUser(ref) ? "you" : ref
    case BlockerKind.BlockedOnAgent:
      return plainLine(name(ref))
    case BlockerKind.BlockedOnLease:
      return `lease ${ref}`
    case BlockerKind.BlockedOnTask:
      return `“${title(ref) || ref}”`
    case BlockerKind.BlockedOnExternal:
      return ref || "something outside Irori"
  }
  return ref || "something"
}

// stateLabel is a task state in words.
export function stateLabel(s: TaskState): string {
  switch (s) {
    case TaskState.Submitted:
      return "open"
    case TaskState.Working:
      return "in progress"
    case TaskState.InputRequired:
      return "needs input"
    case TaskState.Blocked:
      return "blocked"
    case TaskState.Completed:
      return "done"
    case TaskState.Canceled:
      return "canceled"
    case TaskState.Failed:
      return "failed"
  }
  return plainLine(s) || "unknown"
}

// repoLabel is a task's repo for a row, or "" for none.
export function repoLabel(t: Task): string {
  return t.repoKey ? shortRepo(t.repoKey) : ""
}

// displayResource shortens a path lease's repo key:
// path:github.com/o/r:src/** -> o/r: src/**.
export function displayResource(r: string): string {
  if (!r.startsWith("path:")) return r
  const rest = r.slice(5)
  const i = rest.lastIndexOf(":")
  if (i <= 0) return r
  return `${shortRepo(rest.slice(0, i))}: ${rest.slice(i + 1)}`
}
