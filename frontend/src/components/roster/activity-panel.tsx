import { useState } from "react"
import { Collapsible } from "@base-ui/react/collapsible"
import { ChevronRightIcon } from "lucide-react"

import { Card } from "@/components/ui/card"
import {
  hlcWall,
  plainLine,
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

// payloadField reads a string field from a log payload. Payloads arrive as
// parsed JSON, but a raw JSON string is accepted too.
function payloadField(payload: unknown, field: string): string {
  let p = payload
  if (typeof p === "string") {
    try {
      p = JSON.parse(p)
    } catch {
      return ""
    }
  }
  if (!p || typeof p !== "object") return ""
  const v = (p as Record<string, unknown>)[field]
  return typeof v === "string" ? v : ""
}

// agentLabel names an agent key: by its roster name while it is still live,
// then by the name the row carried, otherwise by a short session id.
function agentLabel(
  key: string,
  fallback: string,
  names: Map<string, string>
): string {
  const name = names.get(key) || fallback
  if (name) return plainLine(name)
  const session = key.split("/").pop() ?? key
  return `agent ${plainLine(session).slice(0, 8)}`
}

// subjectLabel names who a row is about. Core writes agent rows with actor
// "irori" and the agent's key in the payload, so the key wins when present;
// an "agent:<key>" actor is read the same way.
function subjectLabel(e: LogEvent, names: Map<string, string>): string {
  const key = payloadField(e.payload, "key")
  if (key) return agentLabel(key, payloadField(e.payload, "name"), names)
  if (e.actor.startsWith("agent:")) {
    return agentLabel(e.actor.slice("agent:".length), "", names)
  }
  return plainLine(e.actor)
}

// subjectTitle is the hover text: the raw actor, plus the agent key if any.
function subjectTitle(e: LogEvent): string {
  const key = payloadField(e.payload, "key")
  return key ? `${e.actor} · ${key}` : e.actor
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
                      title={subjectTitle(e)}
                    >
                      {subjectLabel(e, names)}
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
