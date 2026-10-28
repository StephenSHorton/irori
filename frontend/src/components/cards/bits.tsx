import { Badge } from "@/components/ui/badge"
import { percent, TaskState, type CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"

// ProgressBar is a card's done steps over all steps, always in the primary
// blue (the state chip carries the status). Cards without steps show no bar.
export function ProgressBar({
  progress,
  label,
}: {
  progress: { done: number; total: number }
  label: string
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
        className="block h-full bg-primary transition-[width] duration-300"
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
