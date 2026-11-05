import type { ReactNode } from "react"
import { MenuIcon, PlusIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { Roster } from "@/lib/irori"
import type { Section } from "@/lib/nav"

// AppHeader is the page header (card #135): the page's title and its own
// actions. The nav lives in the sidebar. New card on Home and on a board's
// page (card #116, there it goes on that board), New board on Boards (card
// #126); a board's page adds its own actions (pin, rename, leave) as
// `actions`.
export function AppHeader({
  roster,
  title,
  sub,
  bar,
  actions,
  onNewCard,
  onNewBoard,
  onWindDown,
  section,
  onMenu,
}: {
  roster: Roster
  // The page's title (Home, Boards, a board's name) and a line beside it.
  title: ReactNode
  sub?: ReactNode
  // bar replaces the title (a board's page: its name, rename and actions).
  bar?: ReactNode
  // The page's own actions, before New card / New board.
  actions?: ReactNode
  onNewCard: () => void
  onNewBoard: () => void
  // Wind down all (card #220): opens its dialog.
  onWindDown?: () => void
  // Where the window is; null on a card's page.
  section?: Section | null
  // Opens the nav drawer; its button shows only on a narrow window (card #227).
  onMenu?: () => void
}) {
  return (
    <header className="sticky top-0 z-20 border-x-0! border-t-0! pt-[var(--safe-top)] pl-[var(--safe-left)] pr-[var(--safe-right)] glass-panel">
      <div className="mx-auto flex h-14 max-w-[1920px] items-center gap-2 px-2 sm:gap-3 sm:px-6 md:px-4">
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
          <Badge variant="warning" className="hidden sm:inline-flex" title="Not connected to the Irori app">
            Sample data
          </Badge>
        )}
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          {actions}
          {onWindDown && roster.source !== "sample" && (
            <Button
              size="sm"
              variant="ghost"
              onClick={onWindDown}
              title="Ask every agent to save its place, hand back its cards and stop"
            >
              Wind down
            </Button>
          )}
          {section?.kind === "boards" ? (
            <Button
              size="sm"
              onClick={onNewBoard}
              aria-label="New space"
              title="Make a board for a project, or join one with an invite"
            >
              <PlusIcon aria-hidden className="sm:hidden" />
              <span className="hidden sm:inline">+ New space</span>
            </Button>
          ) : (
            section && (
              <Button
                size="sm"
                onClick={onNewCard}
                aria-label="New card"
                title={
                  section.kind === "board"
                    ? "Make a card on this board"
                    : "Make a card for an agent to pick up"
                }
              >
                <PlusIcon aria-hidden className="sm:hidden" />
                <span className="hidden sm:inline">+ New card</span>
              </Button>
            )
          )}
        </div>
      </div>
    </header>
  )
}
