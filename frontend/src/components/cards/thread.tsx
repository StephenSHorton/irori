import { useState, type ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import type { CardThreadItem } from "@/lib/cards"
import { parseTime, plainBlock, plainLine, relativeTime } from "@/lib/irori"
import { threadLabel, ThreadKind, visible } from "@/lib/thread"
import { cn } from "@/lib/utils"
import { Face } from "./faces"

// Thread is a card's thread, oldest first: comments, messages about the
// card ("A -> B") and one muted line per thing that happened, each with
// who wrote it and, once the person has a second device, where. The
// newest page shows first; "Show earlier" opens the rest. composer renders
// under the list (the comment box's slot; nothing passes one yet).
export function Thread({
  items,
  now,
  earlier = 0,
  historyTrimmed,
  empty = "Nothing yet.",
  compact,
  composer,
}: {
  items: CardThreadItem[]
  now: Date
  // earlier counts entries older than the ones sent (BoardService.Card
  // sends the newest MaxThreadView).
  earlier?: number
  // historyTrimmed: the card's oldest activity lines are no longer kept.
  historyTrimmed?: boolean
  empty?: ReactNode
  compact?: boolean
  composer?: ReactNode
}) {
  const [showAll, setShowAll] = useState(false)
  const shown = visible(items, showAll)
  const hidden = items.length - shown.length
  return (
    <div className={cn("grid text-[12.5px]", compact ? "gap-1" : "gap-2")}>
      {hidden > 0 ? (
        <Button
          size="xs"
          variant="ghost"
          className="justify-self-start text-muted-foreground"
          onClick={() => setShowAll(true)}
        >
          Show {hidden} earlier
        </Button>
      ) : (
        (earlier > 0 || historyTrimmed) &&
        items.length > 0 && (
          <p className="text-xs text-muted-foreground">
            {earlier > 0
              ? `${earlier} earlier ${earlier === 1 ? "entry isn't" : "entries aren't"} shown here.`
              : "Earlier history not kept."}
          </p>
        )
      )}
      {shown.length ? (
        shown.map((it) => <ThreadRow key={it.id} item={it} now={now} />)
      ) : (
        <p className="text-muted-foreground">{empty}</p>
      )}
      {composer}
    </div>
  )
}

function ThreadRow({ item, now }: { item: CardThreadItem; now: Date }) {
  const at = parseTime(item.at)
  const time = (
    <time
      className="text-muted-foreground tabular-nums"
      dateTime={at?.toISOString()}
      title={at?.toLocaleString()}
    >
      {relativeTime(at, now)}
    </time>
  )
  if (item.kind === ThreadKind.Event)
    return (
      <div className="grid grid-cols-[56px_minmax(0,1fr)] gap-2">
        {time}
        <span className="text-muted-foreground">
          {plainLine(item.text)}
          {item.device && <DeviceTag device={item.device} />}
        </span>
      </div>
    )
  const l = threadLabel(item)
  const body = plainBlock(item.text)
  return (
    <div className="grid grid-cols-[56px_minmax(0,1fr)] gap-2">
      {time}
      <div className="grid min-w-0 gap-1">
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          <Face who={item.who} noDot />
          <span className="font-medium text-ink">{plainLine(l.author)}</span>
          {item.to && l.arrow && (
            <>
              <span className="text-muted-foreground" aria-hidden>
                {"->"}
              </span>
              <span className="sr-only">to</span>
              <Face who={item.to} noDot />
              <span className="font-medium text-ink">{plainLine(l.arrow)}</span>
            </>
          )}
          {l.device && <DeviceTag device={l.device} />}
          {l.badges.map((b) => (
            <Badge
              key={b.label}
              variant={b.tone}
              className="h-4 px-1.5 text-[10.5px]"
            >
              {b.label}
            </Badge>
          ))}
        </div>
        {item.held ? (
          <p className="text-muted-foreground">
            Held by your wake policy, so its text isn't on the card.
          </p>
        ) : (
          body && (
            <p className="max-w-[64ch] whitespace-pre-wrap text-ink-2">
              {body}
            </p>
          )
        )}
      </div>
    </div>
  )
}

// DeviceTag names the device an entry was written on.
function DeviceTag({ device }: { device: string }) {
  return (
    <Badge
      variant="slate"
      className="ml-1.5 h-4 px-1.5 align-middle font-mono text-[10.5px]"
      title={`Written on ${plainLine(device)}`}
    >
      {plainLine(device)}
    </Badge>
  )
}
