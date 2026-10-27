import { useEffect, useState } from "react"
import { XIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { AnchorState, capture, type DecisionView } from "@/lib/capture"

// Shared look for the capture tabs' native selects and text fields, matching
// the Inbox composer.
export const fieldClass =
  "h-8 min-w-0 rounded-lg border border-input bg-white/70 px-2 text-sm text-ink outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"

// areaClass tunes the mokei Textarea, as board/bits textareaClass does.
export const areaClass =
  "min-h-14 bg-white/70 text-sm text-ink dark:bg-white/70"

const fidelityText: Record<string, string> = {
  mod: "Captured from the transcript, with the Irori mod attached",
  passive: "Captured from the transcript only: no Irori mod attached",
  mcp: "A Grok member: only what it reports through Irori tools",
}

// FidelityBadge says how much capture sees of an agent.
export function FidelityBadge({ fidelity }: { fidelity?: string }) {
  if (!fidelity) return null
  return (
    <Badge
      variant={fidelity === "mod" ? "success" : "slate"}
      className="h-4.5 px-1.5 text-[11px]"
      title={fidelityText[fidelity] ?? fidelity}
    >
      {plainLine(fidelity)}
    </Badge>
  )
}

const linkVariant = {
  exact: "success",
  partial: "info",
  heuristic: "warning",
  unattributed: "slate",
} as const

const linkText: Record<string, string> = {
  exact: "The committed file matches a recorded edit exactly",
  partial: "Recorded edits touched it, then it changed again before the commit",
  heuristic: "Only shell commands that changed files explain it",
  unattributed: "No recorded edit explains this file",
}

// LinkChip is one link class, with a count when given.
export function LinkChip({ link, count }: { link: string; count?: number }) {
  const v = linkVariant[link as keyof typeof linkVariant] ?? "slate"
  return (
    <Badge
      variant={v}
      className="h-4.5 px-1.5 text-[11px]"
      title={linkText[link]}
    >
      {count !== undefined && <span className="tabular-nums">{count}</span>}
      {plainLine(link)}
    </Badge>
  )
}

const anchorVariant: Record<string, "success" | "info" | "warning" | "slate"> =
  {
    [AnchorState.OK]: "success",
    [AnchorState.Moved]: "info",
    [AnchorState.Changed]: "warning",
    [AnchorState.Gone]: "warning",
    [AnchorState.Unknown]: "slate",
  }

const anchorText: Record<string, string> = {
  [AnchorState.OK]: "The anchored lines are where they were",
  [AnchorState.Moved]: "The same lines are elsewhere in the file now",
  [AnchorState.Changed]: "The file is there, the anchored lines are not",
  [AnchorState.Gone]: "The anchored file is missing",
  [AnchorState.Unknown]:
    "No worktree of this repo on this device to check against",
}

// AnchorBadge is a decision's re-checked anchor status.
export function AnchorBadge({ d }: { d: DecisionView }) {
  if (!d.anchor) return null
  const state = d.status?.state ?? AnchorState.Unknown
  return (
    <Badge
      variant={anchorVariant[state] ?? "slate"}
      className="h-4.5 px-1.5 text-[11px]"
      title={anchorText[state]}
    >
      {plainLine(state) || "unknown"}
    </Badge>
  )
}

// DiffView fetches a patch blob and shows it as a unified diff.
export function DiffView({
  blobId,
  title,
  onClose,
}: {
  blobId: string
  title: string
  onClose: () => void
}) {
  const [state, setState] = useState<{
    id: string
    text: string | null
    error: string | null
  }>({ id: "", text: null, error: null })

  useEffect(() => {
    let live = true
    capture.patch(blobId).then(
      (text) => live && setState({ id: blobId, text, error: null }),
      (err: unknown) =>
        live && setState({ id: blobId, text: null, error: errorText(err) })
    )
    return () => {
      live = false
    }
  }, [blobId])

  const ready = state.id === blobId
  return (
    <div className="flex min-w-0 flex-col overflow-hidden rounded-lg border border-border bg-white/80">
      <div className="flex min-w-0 items-center gap-2 border-b border-border px-3 py-1.5">
        <span className="truncate font-mono text-xs text-ink" title={title}>
          {title}
        </span>
        <Button
          variant="ghost"
          size="icon-xs"
          className="ml-auto"
          onClick={onClose}
          aria-label="Close diff"
        >
          <XIcon />
        </Button>
      </div>
      {!ready ? (
        <p className="px-3 py-2 text-xs text-muted-foreground">
          Reading the patch…
        </p>
      ) : state.error ? (
        <p className="px-3 py-2 text-xs text-destructive">
          Could not read the patch: {state.error}
        </p>
      ) : (
        <DiffText text={state.text ?? ""} />
      )}
    </div>
  )
}

function DiffText({ text }: { text: string }) {
  const lines = text.replace(/\r\n?/g, "\n").replace(/\n$/, "").split("\n")
  if (text.trim() === "") {
    return (
      <p className="px-3 py-2 text-xs text-muted-foreground">Empty patch.</p>
    )
  }
  return (
    <pre className="max-h-[60vh] overflow-auto py-1 font-mono text-xs leading-5">
      {lines.map((l, i) => (
        <div key={i} className={`px-3 whitespace-pre ${diffLineClass(l)}`}>
          {safeLine(l) || " "}
        </div>
      ))}
    </pre>
  )
}

function diffLineClass(l: string): string {
  if (l.startsWith("+++") || l.startsWith("---")) return "text-muted-foreground"
  if (l.startsWith("@@")) return "bg-info-foreground/60 text-info"
  if (l.startsWith("+")) return "bg-success-foreground/70 text-success"
  if (l.startsWith("-")) return "bg-destructive/10 text-destructive"
  return "text-ink-2"
}

// safeLine keeps a diff line's shape (tabs, spaces) but turns hidden
// control and bidi characters into visible spaces.
function safeLine(l: string): string {
  let out = ""
  for (const ch of l) {
    const c = ch.codePointAt(0) ?? 0
    const hidden =
      (c < 0x20 && c !== 0x09) ||
      (c >= 0x7f && c <= 0x9f) ||
      (c >= 0x200b && c <= 0x200f) ||
      (c >= 0x202a && c <= 0x202e) ||
      (c >= 0x2066 && c <= 0x2069) ||
      c === 0xfeff
    out += hidden ? " " : ch
  }
  return out
}

// Empty is a calm empty state, as in the Work tab.
export function Empty({
  icon,
  title,
  text,
}: {
  icon: React.ReactNode
  title: string
  text: string
}) {
  return (
    <Card
      size="sm"
      className="flex-row items-center gap-3 px-4 text-muted-foreground"
    >
      {icon}
      <div className="flex flex-col">
        <span className="font-medium text-ink">{title}</span>
        <span className="text-[13px]">{text}</span>
      </div>
    </Card>
  )
}

export function ErrorLine({
  what,
  error,
}: {
  what: string
  error: string | null
}) {
  if (!error) return null
  return (
    <p className="rounded-lg bg-destructive/10 px-4 py-2 text-sm text-destructive">
      Could not load {what}: {error}
    </p>
  )
}
