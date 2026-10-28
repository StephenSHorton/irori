import { Badge } from "@/components/ui/badge"
import { percent, TaskState, type CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"

// Tone is a card's status colour, shared by its state chip and its progress
// bar so the two always agree.
export type Tone = "destructive" | "success" | "slate" | "info"

const barTone: Record<Tone, string> = {
  destructive: "bg-destructive",
  success: "bg-success",
  slate: "bg-muted-foreground",
  info: "bg-info",
}

// cardTone: waiting on the person or blocked is red, done green, canceled,
// failed or unheld grey, else in progress blue.
export function cardTone(card: {
  state: string
  openBlockers: number
  working: readonly unknown[]
}): Tone {
  if (card.state === TaskState.InputRequired || card.openBlockers > 0)
    return "destructive"
  if (card.state === TaskState.Completed) return "success"
  if (card.state === TaskState.Canceled || card.state === TaskState.Failed)
    return "slate"
  if (card.working.length === 0) return "slate"
  return "info"
}

// ProgressBar is a card's done steps over all steps, in the card's status
// colour. Cards without steps show no bar.
export function ProgressBar({
  progress,
  label,
  tone = "info",
}: {
  progress: { done: number; total: number }
  label: string
  tone?: Tone
}) {
  const p = percent(progress)
  if (p === null) return null
  return (
    <span
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={p}
      className="block h-1 overflow-hidden bg-muted"
    >
      <span
        className={`block h-full ${barTone[tone]} transition-[width] duration-300`}
        style={{ width: `${p}%` }}
      />
    </span>
  )
}

// StatePill is the card's first badge: waiting on the person's input, its
// blockers, else in progress.
export function StatePill({ card }: { card: CardSummary }) {
  if (card.state === TaskState.InputRequired)
    return (
      <Badge variant="destructive">
        Needs your input
      </Badge>
    )
  if (card.openBlockers > 0)
    return (
      <Badge variant="destructive">
        {card.openBlockers} {card.openBlockers === 1 ? "blocker" : "blockers"}
      </Badge>
    )
  if (card.state === TaskState.Completed)
    return (
      <Badge variant="success">
        Done
      </Badge>
    )
  if (card.state === TaskState.Canceled || card.state === TaskState.Failed)
    return (
      <Badge variant="slate">
        {card.state === TaskState.Failed ? "Failed" : "Canceled"}
      </Badge>
    )
  if (card.working.length === 0)
    return (
      <Badge variant="slate">
        Open
      </Badge>
    )
  return (
    <Badge variant="info">
      In progress
    </Badge>
  )
}

// ScopeBadge says where the card lives: "My devices" until Shared Boards.
export function ScopeBadge({ card }: { card: CardSummary }) {
  return (
    <Badge
      variant="outline"
      className="font-normal text-muted-foreground"
    >
      {plainLine(card.scopeLabel) || "My devices"}
    </Badge>
  )
}
