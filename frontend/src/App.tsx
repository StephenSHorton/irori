import { useEffect, useRef, useState } from "react"

import { AppHeader } from "@/components/app-header"
import { CardPanel } from "@/components/cards/card-detail"
import {
  CardGrid,
  EmptyCards,
  FilterBar,
  matches,
  type Filter,
} from "@/components/cards/card-grid"
import { useCard, useCards } from "@/lib/cards"
import { useNow, useRoster } from "@/lib/irori"
import { onOpen } from "@/lib/settings"
import { useGone } from "@/lib/talk"
import { UpdateBanner } from "@/updater"

// narrow mirrors the layout breakpoint: below it the card opens under the
// grid instead of beside it.
const narrow = "(max-width: 979px)"

// App is the whole window (docs/design/v0.4-devices.md §9): cards of shared
// work, and the one opened beside them. Messages, capture and the roster
// keep running underneath for agents and routing; the person reaches them
// through cards and notifications.
export function App() {
  const roster = useRoster()
  const { gone } = useGone()
  const { cards, loading, error } = useCards()
  const now = useNow()
  const [filter, setFilter] = useState<Filter>("all")
  // picked is the person's choice; until there is one the first card opens,
  // and is pinned once shown, so an answer that re-sorts the grid doesn't
  // swap the open card (and its buttons) under the cursor.
  const [picked, setPicked] = useState("")
  const first = cards[0]?.id ?? ""
  if (!picked && first) setPicked(first) // during render: no extra paint
  const selected = picked || first
  const { detail, error: detailError } = useCard(selected)
  const detailRef = useRef<HTMLDivElement>(null)

  const select = (id: string) => {
    setPicked(id)
    if (window.matchMedia(narrow).matches)
      requestAnimationFrame(() =>
        detailRef.current?.scrollIntoView({ block: "start" })
      )
  }

  // A notification's Open: every target lands on the cards, and a task
  // ref opens that card (with the filter cleared so it can be seen).
  // Held messages keep Release and Dismiss on the notification itself.
  useEffect(
    () =>
      onOpen((target) => {
        const ref = target.ref ?? ""
        if (!ref.startsWith("task:")) return
        setFilter("all")
        setPicked(ref.slice("task:".length))
      }),
    []
  )

  const shown = cards.filter((c) => matches(c, filter))
  const decide = cards.filter((c) => c.openBlockers > 0).length

  return (
    <div className="min-h-svh bg-background">
      <AppHeader roster={roster} />
      <main className="mx-auto flex max-w-7xl flex-col gap-3.5 px-4 pt-[18px] pb-10 sm:px-6">
        <UpdateBanner />
        {error && (
          <p className="rounded-lg bg-destructive/10 px-4 py-2 text-sm text-destructive">
            Could not load the cards: {error}
          </p>
        )}
        {loading && cards.length === 0 ? (
          <p className="px-1 text-sm text-muted-foreground">Loading cards…</p>
        ) : cards.length === 0 ? (
          <EmptyCards />
        ) : (
          <>
            <FilterBar filter={filter} onFilter={setFilter} decide={decide} />
            <div className="grid items-start gap-4 min-[980px]:grid-cols-[minmax(0,1fr)_minmax(0,520px)]">
              {shown.length ? (
                <CardGrid cards={shown} selected={selected} onSelect={select} />
              ) : (
                <p className="px-1 text-[12.5px] text-muted-foreground">
                  Nothing here with this filter.
                </p>
              )}
              <div
                ref={detailRef}
                className="min-w-0 scroll-mt-16 min-[980px]:sticky min-[980px]:top-[4.5rem]"
              >
                {selected && (
                  <CardPanel
                    detail={detail}
                    error={detailError}
                    gone={gone}
                    now={now}
                  />
                )}
              </div>
            </div>
          </>
        )}
      </main>
    </div>
  )
}

export default App
