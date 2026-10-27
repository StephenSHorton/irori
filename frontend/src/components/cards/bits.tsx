import { Badge } from "@/components/ui/badge"
import { percent, TaskState, type CardSummary } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { cn } from "@/lib/utils"

// ProgressBar is a card's done steps over all steps; amber while the card
// waits on a decision. Cards without steps show no bar.
export function ProgressBar({
  progress,
  blocked,
  label,
}: {
  progress: { done: number; total: number }
  blocked?: boolean
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
      className="block h-1.5 overflow-hidden rounded-full bg-muted ring-1 ring-border ring-inset"
    >
      <span
        className={cn(
          "block h-full rounded-full",
          blocked ? "bg-warning" : "bg-primary"
        )}
        style={{ width: `${p}%` }}
      />
    </span>
  )
}

// StatePill is the card's first badge: its blockers, else in progress.
export function StatePill({ card }: { card: CardSummary }) {
  if (card.openBlockers > 0)
    return (
      <Badge variant="destructive" className="rounded-full">
        {card.openBlockers} {card.openBlockers === 1 ? "blocker" : "blockers"}
      </Badge>
    )
  if (card.state === TaskState.Completed)
    return (
      <Badge variant="success" className="rounded-full">
        Done
      </Badge>
    )
  if (card.state === TaskState.Canceled || card.state === TaskState.Failed)
    return (
      <Badge variant="slate" className="rounded-full">
        {card.state === TaskState.Failed ? "Failed" : "Canceled"}
      </Badge>
    )
  if (card.working.length === 0)
    return (
      <Badge variant="slate" className="rounded-full">
        Open
      </Badge>
    )
  return (
    <Badge variant="info" className="rounded-full">
      In progress
    </Badge>
  )
}

// ScopeBadge says where the card lives: "My devices" until Shared Boards.
export function ScopeBadge({ card }: { card: CardSummary }) {
  return (
    <Badge
      variant="outline"
      className="rounded-full bg-muted/60 font-normal text-muted-foreground"
    >
      {plainLine(card.scopeLabel) || "My devices"}
    </Badge>
  )
}
