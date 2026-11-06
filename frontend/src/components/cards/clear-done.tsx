import { useEffect, useState } from "react"

import { Button } from "@/components/ui/button"
import { ConfirmButton } from "@/components/common/confirm-button"
import { cards as cardsApi } from "@/lib/cards"
import { errorText } from "@/lib/talk"

const Day = 24 * 60 * 60

// clearAges are the ages a clear can take (card #138): closed more than a
// day, a week or a month ago, or every done card.
export const clearAges = [
  {
    id: "day",
    label: "Older than a day",
    secs: Day,
    phrase: "closed more than a day ago",
  },
  {
    id: "week",
    label: "Older than a week",
    secs: 7 * Day,
    phrase: "closed more than a week ago",
  },
  {
    id: "month",
    label: "Older than a month",
    secs: 30 * Day,
    phrase: "closed more than a month ago",
  },
  { id: "all", label: "All", secs: 0, phrase: "" },
] as const

type Age = (typeof clearAges)[number]["id"]

const plural = (n: number) => `${n} done ${n === 1 ? "card" : "cards"}`

// ClearDone clears done cards by age (card #138): pick an age, see how
// many it takes, confirm. Cleared cards leave the app's lists, on your
// boards too; agents still find them with search and show. Used on the
// Closed view and in Settings > Maintenance.
export function ClearDone({
  onCleared,
  compact,
  scope = "",
}: {
  onCleared?: (n: number) => void
  // scope: whose done cards ("" all, "own" Home's, else a board id).
  scope?: string
  // compact: the Closed view's one-line form.
  compact?: boolean
}) {
  const [age, setAge] = useState<Age>("week")
  const [count, setCount] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [last, setLast] = useState<number | null>(null)
  const pick = clearAges.find((a) => a.id === age) ?? clearAges[1]

  // tick re-counts after a clear.
  const [tick, setTick] = useState(0)
  useEffect(() => {
    let live = true
    cardsApi
      .clearDoneCount(pick.secs, scope)
      .then((n) => {
        if (!live) return
        setCount(n)
        setError(null)
      })
      .catch((err: unknown) => live && setError(errorText(err)))
    return () => {
      live = false
    }
  }, [pick.secs, scope, tick])

  const what = pick.phrase
    ? `${plural(count ?? 0)} ${pick.phrase}`
    : `all ${plural(count ?? 0)}`

  return (
    <div
      className={compact ? "flex flex-wrap items-center gap-2" : "grid gap-2.5"}
    >
      <div
        className="flex flex-wrap items-center gap-1"
        role="group"
        aria-label="Clear done cards by age"
      >
        {compact && (
          <span className="mr-1 text-xs text-muted-foreground">
            Clear done:
          </span>
        )}
        {clearAges.map((a) => (
          <Button
            key={a.id}
            size="xs"
            aria-pressed={age === a.id}
            variant={age === a.id ? "secondary" : "ghost"}
            onClick={() => {
              setAge(a.id)
              setLast(null)
            }}
          >
            {a.label}
          </Button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {!compact && (
          <span className="text-[12.5px] text-muted-foreground tabular-nums">
            {count === null
              ? "Counting…"
              : count === 0
                ? "Nothing to clear."
                : `Clears ${what}.`}
          </span>
        )}
        <ConfirmButton
          label={count ? `Clear ${count}…` : "Clear"}
          disabled={!count}
          confirmLabel={count ? `Clear ${count}` : "Clear"}
          question={
            <>
              Clear {what}? They leave the app's lists, in your spaces too.
              Agents still find them with search and show. Nothing is deleted.
            </>
          }
          onConfirm={async () => {
            const n = await cardsApi.clearDone(pick.secs, [], scope)
            setLast(n)
            onCleared?.(n)
            setTick((t) => t + 1)
          }}
        />
        {last !== null && (
          <span className="text-xs text-muted-foreground" role="status">
            Cleared {plural(last)}.
          </span>
        )}
        {error && <span className="text-xs text-destructive">{error}</span>}
      </div>
    </div>
  )
}
