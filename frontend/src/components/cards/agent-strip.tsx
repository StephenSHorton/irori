import { faceColor, Identicon } from "@/components/cards/faces"
import { AgentRef } from "@/gen/models"
import { agentStatus, throughLine } from "@/lib/agent-status"
import { deviceVar } from "@/lib/cards"
import { useDevices } from "@/lib/devices"
import type { OnCard } from "@/lib/roster-groups"

// STRIP_MAX is how many faces the sidebar's strip shows before "+N".
const STRIP_MAX = 8

export type RosterAgent = {
  key: string
  name: string
  status: string
  deviceId: string
  deviceState?: string | null
  deviceVia?: string | null
  // When its status began (card #489).
  statusSince?: string
}

// AgentStrip is the sidebar's row of agent faces (card #407): who is
// around, in their device colours; since 0.11.1 still identicons, not
// animated faces (those kept WindowServer busy). Gone agents
// and agents on an offline device aren't shown.
export function AgentStrip({ agents }: { agents: RosterAgent[] }) {
  const refOf = useRosterRef()
  const here = agents
    .filter((a) => a.status !== "gone" && a.deviceState !== "offline")
    .sort(
      (a, b) =>
        Number(b.status === "busy") - Number(a.status === "busy") ||
        a.name.localeCompare(b.name)
    )
  if (!here.length) return null
  const shown = here.slice(0, STRIP_MAX)
  const rest = here.length - shown.length
  return (
    <div className="flex flex-wrap items-center gap-1 px-2 pt-2">
      {shown.map((a) => {
        const who = refOf(a)
        return (
          <span
            key={a.key}
            title={[
              `${a.name}: ${agentStatus(a, a.statusSince).text}`,
              a.deviceVia && throughLine(a.deviceVia),
            ]
              .filter(Boolean)
              .join(" · ")}
          >
            <span
              className="block size-7"
              style={{ color: faceColor(a.status, deviceVar(who.color)) }}
            >
              <Identicon seed={who.key} full />
            </span>
          </span>
        )
      })}
      {rest > 0 && (
        <span className="text-[11px] text-muted-foreground tabular-nums">
          +{rest}
        </span>
      )}
    </div>
  )
}

// useRosterRef turns a roster agent into the AgentRef its face draws from,
// in its device colour when there is more than one device.
function useRosterRef() {
  const { devices } = useDevices(true)
  const live = devices.filter((d) => !d.revoked)
  const colorOf = (id: string) =>
    live.length > 1 ? (devices.find((d) => d.deviceId === id)?.color ?? "") : ""
  return (a: RosterAgent) =>
    AgentRef({
      key: a.key,
      name: a.name,
      status: a.status,
      live: true,
      deviceId: a.deviceId,
      color: colorOf(a.deviceId),
    })
}

// OnCardLine is what a spawned agent's row says under its name: where it
// runs and what it is doing.
export type OnCardLine = (cardId: string, a: RosterAgent) => string

// OnCards lists the agents Irori started for cards (card #456, sa-1): each
// with its "#N", or together under the card when one card has several.
// A row opens its card.
export function OnCards({
  groups,
  line,
  onOpen,
}: {
  groups: OnCard<RosterAgent>[]
  line: OnCardLine
  onOpen: (cardId: string) => void
}) {
  const refOf = useRosterRef()
  const row = (cardId: string, a: RosterAgent, tag: string) => {
    const who = refOf(a)
    return (
      <button
        key={a.key}
        type="button"
        onClick={() => onOpen(cardId)}
        className="flex w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-left hover:bg-secondary"
      >
        <span
          className="block size-5.5 shrink-0"
          style={{ color: faceColor(a.status, deviceVar(who.color)) }}
        >
          <Identicon seed={who.key} full />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs font-medium">{a.name}</span>
          <span className="block truncate text-[11px] text-muted-foreground">
            {line(cardId, a)}
          </span>
        </span>
        {tag && (
          <span className="shrink-0 rounded bg-secondary px-1 font-mono text-[10px] text-muted-foreground tabular-nums">
            {tag}
          </span>
        )}
      </button>
    )
  }
  return (
    <div className="grid max-h-56 grid-cols-[minmax(0,1fr)] gap-0.5 overflow-y-auto px-1">
      {groups.map((g) =>
        g.agents.length === 1 ? (
          row(g.cardId, g.agents[0], `#${g.short}`)
        ) : (
          <div
            key={g.cardId}
            role="group"
            aria-label={`#${g.short} ${g.title}`}
            className="rounded-md border border-border py-0.5"
          >
            <button
              type="button"
              onClick={() => onOpen(g.cardId)}
              className="flex w-full min-w-0 items-center gap-1.5 px-1.5 py-0.5 text-left text-[11px] text-muted-foreground hover:text-foreground"
            >
              <span className="shrink-0 font-mono tabular-nums">
                #{g.short}
              </span>
              <span className="truncate">
                {g.title} · {g.agents.length} agents
              </span>
            </button>
            {g.agents.map((a) => row(g.cardId, a, ""))}
          </div>
        )
      )}
    </div>
  )
}
