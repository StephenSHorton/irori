import type { ReactNode } from "react"
import { MenuIcon, PlusIcon } from "lucide-react"

import { MetersStrip } from "@/components/meters/meters-strip"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { PauseAgentsButton } from "@/components/pause-agents-button"
import { WindDownButton } from "@/components/wind-down-button"
import type { Roster } from "@/lib/irori"
import type { Section } from "@/lib/nav"
import type { WindDown } from "@/lib/wind-down"

// AppHeader is the page header (card #135): the page's title and its own
// actions. The nav lives in the sidebar. New card on Home and on a board's
// page (card #116, there it goes on that board); a board's page adds its own actions (pin, rename, leave) as
// `actions`.
export function AppHeader({
  roster,
  title,
  sub,
  bar,
  actions,
  onNewCard,
  onWindDown,
  windDown,
  section,
  onMenu,
}: {
  roster: Roster
  // The page's title (Home, Talk, a space's name) and a line beside it.
  title: ReactNode
  sub?: ReactNode
  // bar replaces the title (a board's page: its name, rename and actions).
  bar?: ReactNode
  // The page's own actions, before New card.
  actions?: ReactNode
  onNewCard: () => void
  // Stop everywhere (card #220): opens its dialog.
  onWindDown?: () => void
  // Stop everywhere's switch (cards #773, #787), App's one copy.
  windDown?: WindDown
  // Where the window is; null on a card's page.
  section?: Section | null
  // Opens the nav drawer; its button shows only on a narrow window (card #227).
  onMenu?: () => void
}) {
  return (
    <header className="sticky top-0 z-20 border-x-0! border-t-0! pt-[var(--safe-top)] pr-[var(--safe-right)] pl-[var(--safe-left)] glass-panel">
      <div className="@container/header mx-auto flex h-[calc(3.5rem_-_min(var(--safe-top),0.75rem))] max-w-[1920px] items-center gap-2 px-2 sm:gap-3 sm:px-6 md:px-4">
        {onMenu && (
          <Button
            variant="ghost"
            size="icon"
            className="-ml-1 md:hidden"
            aria-label="Open the menu"
            onClick={onMenu}
          >
            <MenuIcon className="size-5" aria-hidden />
          </Button>
        )}
        {bar ?? (
          <h1 className="flex min-w-0 items-baseline gap-2 truncate text-lg font-semibold text-ink">
            {title}
          </h1>
        )}
        {!bar && sub && (
          <span className="hidden min-w-0 truncate text-xs text-muted-foreground md:inline">
            {sub}
          </span>
        )}
        {roster.source === "sample" && (
          <Badge
            variant="warning"
            className="hidden sm:inline-flex"
            title="Not connected to the Irori app"
          >
            Sample data
          </Badge>
        )}
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          {/* This computer's meters (card #599): desktop only. */}
          <MetersStrip crowded={Boolean(bar)} />
          {actions}
          {/* Stop here (cards #665, #773): this machine, one click each
              way. While Stop everywhere is on this machine's stop is part
              of it, so this hides (card #787). */}
          {!windDown?.status.on && <PauseAgentsButton />}
          {/* Stop everywhere (cards #773, #787): a switch too. Its off button
              isn't on the sample roster; its on state shows wherever it is
              on. */}
          {windDown && (
            <WindDownButton
              windDown={windDown}
              onOpen={roster.source !== "sample" ? onWindDown : undefined}
            />
          )}
          {section && (
            <Button
              size="sm"
              onClick={onNewCard}
              aria-label="New card"
              title={
                section.kind === "board"
                  ? section.board
                    ? "Make a card on this board"
                    : "Make a card in this space"
                  : "Make a card for an agent to pick up"
              }
            >
              <PlusIcon aria-hidden className="sm:hidden" />
              <span className="hidden sm:inline">+ New card</span>
            </Button>
          )}
        </div>
      </div>
    </header>
  )
}
