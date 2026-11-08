// SpaceSwitcher sits at the top of the sidebar (card #393, mockup talk-6):
// it picks the space Talk and Boards show (the Dashboard always shows every
// space, card #1001). Personal is you and your agents; then each shared
// space, "with" whoever is in it. A dot on a row says something there waits
// for you. It is the one place for spaces
// (card #689, which dropped the Spaces tab; Boards shows only the picked
// space's): at its foot, Space settings for a shared pick (its page: its
// boards, people, invites and settings) and New or join a space.

import {
  CheckIcon,
  ChevronDownIcon,
  PlusIcon,
  SettingsIcon,
  UserIcon,
} from "lucide-react"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { FreshnessShort } from "@/components/common/freshness-line"
import type { BoardSummary } from "@/lib/boards"
import { plainLine } from "@/lib/irori"
import { go } from "@/lib/nav"
import { previewFlag } from "@/lib/preview"
import { Personal, spacesOf, withLine, type Space } from "@/lib/spaces"
import { cn } from "@/lib/utils"

function Mark({ s, className }: { s: Space; className?: string }) {
  const base = cn(
    "flex size-5 shrink-0 items-center justify-center rounded text-[10.5px] font-semibold",
    className
  )
  if (s.id === Personal)
    return (
      <span className={cn(base, "bg-secondary text-muted-foreground")}>
        <UserIcon className="size-3" aria-hidden />
      </span>
    )
  return (
    <span className={cn(base, "bg-primary/20 text-primary")}>
      {[...plainLine(s.name)][0]?.toUpperCase() ?? "?"}
    </span>
  )
}

// FootIcon is a foot item's icon in a Mark-sized box, so its words line up
// with the spaces' names.
function FootIcon({ Icon }: { Icon: typeof PlusIcon }) {
  return (
    <span className="flex size-5 shrink-0 items-center justify-center">
      <Icon className="size-3.5 text-muted-foreground" aria-hidden />
    </span>
  )
}

export function SpaceSwitcher({
  boards,
  space,
  onSpace,
  onNewSpace,
  folded,
  waiting,
}: {
  boards: BoardSummary[]
  space: string
  onSpace: (id: string) => void
  // Opens New or join a space (App's dialog, which the palette opens too).
  onNewSpace: () => void
  folded: boolean
  // The spaces with something waiting for the person (card #1001).
  waiting?: ReadonlySet<string>
}) {
  const spaces = spacesOf(boards)
  const cur = spaces.find((s) => s.id === space) ?? spaces[0]
  const line = withLine(cur.others.map(plainLine))
  return (
    <DropdownMenu defaultOpen={previewFlag("spaces") === "open"}>
      <DropdownMenuTrigger
        render={
          <button
            type="button"
            aria-label={`Space: ${plainLine(cur.name)}. Pick a space`}
            title={
              folded
                ? `Space: ${plainLine(cur.name)}`
                : "Pick the space Talk and Boards show, or make or join one"
            }
            className={cn(
              "flex h-9 w-full min-w-0 items-center gap-2 rounded-md border border-border bg-background/40 text-left text-sm text-ink hover:bg-secondary pointer-coarse:h-11",
              folded ? "justify-center px-0" : "px-2"
            )}
          />
        }
      >
        <Mark s={cur} />
        {!folded && (
          <>
            <span className="min-w-0 flex-1 truncate">
              <span className="font-medium">{plainLine(cur.name)}</span>
              {line && <span className="text-muted-foreground"> · {line}</span>}
            </span>
            <ChevronDownIcon
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden
            />
          </>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        {spaces.map((s, i) => (
          <div key={s.id}>
            {i === 1 && <DropdownMenuSeparator />}
            <DropdownMenuItem onClick={() => onSpace(s.id)}>
              <Mark s={s} />
              <span className="grid min-w-0 flex-1">
                <span className="truncate">
                  {plainLine(s.name)}
                  {s.others.length > 0 && (
                    <span className="text-muted-foreground">
                      {" "}
                      · {withLine(s.others.map(plainLine))}
                    </span>
                  )}
                </span>
                <FreshnessShort log={s.id} className="text-[11px]" />
              </span>
              {waiting?.has(s.id) && (
                <span
                  role="img"
                  aria-label="Something waits for you here"
                  title="Something waits for you here"
                  className="size-2 shrink-0 rounded-full bg-primary"
                />
              )}
              {s.id === cur.id && (
                <CheckIcon className="size-3.5" aria-hidden />
              )}
            </DropdownMenuItem>
          </div>
        ))}
        <DropdownMenuSeparator />
        {cur.id !== Personal && (
          <DropdownMenuItem
            onClick={() => go({ kind: "board", id: cur.id })}
            title={`${plainLine(cur.name)}'s boards, people, invites and settings`}
          >
            <FootIcon Icon={SettingsIcon} />
            Space settings
          </DropdownMenuItem>
        )}
        <DropdownMenuItem onClick={onNewSpace}>
          <FootIcon Icon={PlusIcon} />
          New or join a space
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
