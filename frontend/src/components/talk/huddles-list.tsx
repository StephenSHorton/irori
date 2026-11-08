// Talk's Huddles list (card #1044; docs/design/call-agents.md §3): the
// huddles of the picked space, newest first, each with its title, where it
// was, who was in it, how long it ran and whether it's on. A row opens the
// huddle's page. huddlesFor is pure, so the tests feed it.

import { HeadphonesIcon } from "lucide-react"

import { dayLabel } from "./conversation"
import { clockText } from "@/lib/call"
import {
  huddleName,
  isLive,
  lengthOf,
  peopleText,
  placeText,
  type TalkHuddle,
} from "@/lib/huddles"
import { AllSpaces, convInSpace } from "@/lib/spaces"
import { cn } from "@/lib/utils"

// huddlesFor is the huddles of the conversations in `space` (all of them
// for "all spaces"), keeping the order given. A search matches the title,
// the place and the people.
export function huddlesFor(
  huddles: readonly TalkHuddle[],
  convs: readonly {
    convId: string
    spaceId?: string
    kind?: string
    members?: readonly { kind: string; key: string }[]
  }[],
  space: string,
  query = ""
): TalkHuddle[] {
  const q = query.trim().toLowerCase()
  const byConv = new Map(convs.map((c) => [c.convId, c]))
  return huddles.filter((h) => {
    // A huddle whose conversation isn't in the list (a channel left, a DM
    // archived) still shows under all spaces.
    const c = byConv.get(h.convId)
    if (c ? !convInSpace(c, space) : space !== AllSpaces) return false
    if (!q) return true
    return [huddleName(h), placeText(h), ...h.people].some((s) =>
      s.toLowerCase().includes(q)
    )
  })
}

// whenShort is when a row's huddle began: "Today 10:02".
export function whenShort(h: TalkHuddle, now: Date): string {
  const at = h.startedAtMs ?? h.lastAtMs
  if (!at) return ""
  return `${dayLabel(new Date(at), now)} ${clockText(at)}`
}

export function HuddleRow({
  h,
  now,
  current,
  onOpen,
}: {
  h: TalkHuddle
  now: Date
  current?: boolean
  onOpen: () => void
}) {
  const live = isLive(h)
  const sub = [placeText(h), peopleText(h.people)].filter(Boolean).join(" · ")
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-current={current ? "page" : undefined}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-ink-2 hover:bg-secondary/60",
        current && "bg-secondary"
      )}
    >
      <HeadphonesIcon
        className={cn(
          "size-4 shrink-0",
          live ? "text-success" : "text-muted-foreground"
        )}
        aria-hidden
      />
      <span className="grid min-w-0 flex-1">
        <span className="flex min-w-0 items-baseline gap-2">
          <span
            className={cn(
              "min-w-0 flex-1 truncate",
              live && "font-medium text-ink"
            )}
          >
            {huddleName(h)}
          </span>
          <span
            className={cn(
              "shrink-0 text-[10.5px] whitespace-nowrap tabular-nums",
              live ? "font-medium text-success" : "text-muted-foreground"
            )}
          >
            {live ? "live" : lengthOf(h)}
          </span>
        </span>
        <span className="truncate text-[11px] text-muted-foreground">
          {sub}
          {sub && " · "}
          {whenShort(h, now)}
        </span>
      </span>
    </button>
  )
}
