import { useState } from "react"
import { Collapsible } from "@base-ui/react/collapsible"
import { ChevronRightIcon } from "lucide-react"

import { Card } from "@/components/ui/card"
import {
  hlcWall,
  relativeTime,
  type AgentCard,
  type LogEvent,
} from "@/lib/irori"

const openKey = "irori.activity.open"

function readOpen(): boolean {
  try {
    return localStorage.getItem(openKey) !== "0"
  } catch {
    return true
  }
}

function writeOpen(open: boolean) {
  try {
    localStorage.setItem(openKey, open ? "1" : "0")
  } catch {
    // Remembering the panel is a convenience only.
  }
}

// actorLabel names an actor: agents by their roster name when they are still
// live, otherwise by a short session id.
function actorLabel(actor: string, names: Map<string, string>): string {
  if (!actor.startsWith("agent:")) return actor
  const key = actor.slice("agent:".length)
  const name = names.get(key)
  if (name) return name
  const session = key.split("/").pop() ?? key
  return `agent ${session.slice(0, 8)}`
}

// ActivityPanel lists the newest log rows. It is collapsible and remembers
// whether it was open.
export function ActivityPanel({
  events,
  agents,
  error,
  now,
}: {
  events: LogEvent[]
  agents: AgentCard[]
  error: string | null
  now: Date
}) {
  const [open, setOpen] = useState(readOpen)
  const names = new Map(agents.map((a) => [a.key, a.name]))

  return (
    <Card size="sm" className="gap-0 py-0!">
      <Collapsible.Root
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          writeOpen(next)
        }}
      >
        <Collapsible.Trigger className="group flex w-full items-center gap-2 px-4 py-3 text-left text-sm font-medium text-ink outline-none focus-visible:ring-3 focus-visible:ring-ring/50">
          <ChevronRightIcon
            className="size-4 text-muted-foreground transition-transform group-data-[panel-open]:rotate-90"
            aria-hidden
          />
          Activity
          <span className="text-xs font-normal text-muted-foreground tabular-nums">
            {events.length}
          </span>
        </Collapsible.Trigger>
        <Collapsible.Panel>
          {error && (
            <p className="border-t px-4 py-2 text-xs text-destructive">
              Could not read the log: {error}
            </p>
          )}
          {events.length === 0 && !error ? (
            <p className="border-t px-4 py-3 text-xs text-muted-foreground">
              Nothing logged yet.
            </p>
          ) : (
            <ol className="max-h-80 overflow-y-auto border-t">
              {events.map((e) => {
                const at = hlcWall(e.hlc)
                return (
                  <li
                    key={`${e.deviceId}:${e.seq}`}
                    className="grid grid-cols-[8.5rem_1fr_auto] items-center gap-3 px-4 py-1.5 text-xs odd:bg-muted/40"
                  >
                    <span className="font-mono text-ink-2">{e.kind}</span>
                    <span
                      className="truncate text-muted-foreground"
                      title={e.actor}
                    >
                      {actorLabel(e.actor, names)}
                    </span>
                    <time
                      className="text-muted-foreground tabular-nums"
                      dateTime={at?.toISOString()}
                      title={at?.toLocaleString()}
                    >
                      {relativeTime(at, now)}
                    </time>
                  </li>
                )
              })}
            </ol>
          )}
        </Collapsible.Panel>
      </Collapsible.Root>
    </Card>
  )
}
