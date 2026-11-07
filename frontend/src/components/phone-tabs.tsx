import { HouseIcon, MessagesSquareIcon } from "lucide-react"

import type { Section } from "@/lib/nav"
import { cn } from "@/lib/utils"

// PhoneTab is a bottom tab: Home or Talk (card #395, mockup talk-5). The
// Spaces tab is gone (card #689): spaces are picked, made and opened from
// the space picker at the top of the menu.
export type PhoneTab = "home" | "talk"

// phoneTab is the tab a section sits under: Talk, else Home (a space's
// page is the picked space's, so Home's). A card's page is under the tab
// it was opened from.
export function phoneTab(s: Section): PhoneTab {
  if (s.kind === "talk") return "talk"
  return "home"
}

const tabs: {
  tab: PhoneTab
  label: string
  to: Section
  Icon: typeof HouseIcon
}[] = [
  { tab: "home", label: "Home", to: { kind: "home" }, Icon: HouseIcon },
  {
    tab: "talk",
    label: "Talk",
    to: { kind: "talk", id: "" },
    Icon: MessagesSquareIcon,
  },
]

// PhoneTabs is the phone's bottom bar (design §8): Home and Talk, with
// Talk's count of what waits on you. A tap on the tab you are on goes to
// its top (Home, Talk's list).
export function PhoneTabs({
  at,
  talk,
  onGo,
}: {
  at: PhoneTab
  // Threads that need you and unread DMs, as the sidebar counts them.
  talk: number
  // Moves the window (App's goTo: a card's page closes, and the place it
  // already is adds no history entry).
  onGo: (s: Section) => void
}) {
  return (
    <nav
      aria-label="Tabs"
      className="grid shrink-0 grid-cols-2 border-x-0! border-b-0! pr-[var(--safe-right)] pb-[var(--safe-bottom)] pl-[var(--safe-left)] glass-panel"
    >
      {tabs.map(({ tab, label, to, Icon }) => {
        const on = tab === at
        return (
          <button
            key={tab}
            type="button"
            aria-current={on ? "page" : undefined}
            onClick={() => onGo(to)}
            className={cn(
              "relative flex h-14 flex-col items-center justify-center gap-0.5 text-[11px]",
              on ? "text-primary" : "text-muted-foreground hover:text-ink"
            )}
          >
            <span className="relative">
              <Icon className="size-5" aria-hidden />
              {tab === "talk" && talk > 0 && (
                <span
                  className="absolute -top-1.5 -right-2.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-semibold text-primary-foreground tabular-nums"
                  aria-label={`${talk} waiting`}
                >
                  {talk > 99 ? "99+" : talk}
                </span>
              )}
            </span>
            {label}
          </button>
        )
      })}
    </nav>
  )
}
