import { Identicon } from "@/components/cards/faces"
import { AgentRef } from "@/gen/models"
import { deviceVar } from "@/lib/cards"
import { useDevices } from "@/lib/devices"

// STRIP_MAX is how many faces the sidebar's strip shows before "+N".
const STRIP_MAX = 8

type RosterAgent = {
  key: string
  name: string
  status: string
  deviceId: string
  deviceState?: string | null
}

// AgentStrip is the sidebar's row of agent faces (card #407): who is
// around, in their device colours; since 0.11.1 still identicons, not
// animated faces (those kept WindowServer busy). Gone agents
// and agents on an offline device aren't shown.
export function AgentStrip({ agents }: { agents: RosterAgent[] }) {
  const { devices } = useDevices(true)
  const live = devices.filter((d) => !d.revoked)
  const colorOf = (id: string) =>
    live.length > 1 ? (devices.find((d) => d.deviceId === id)?.color ?? "") : ""
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
        const who = AgentRef({
          key: a.key,
          name: a.name,
          status: a.status,
          live: true,
          deviceId: a.deviceId,
          color: colorOf(a.deviceId),
        })
        return (
          <span
            key={a.key}
            title={`${a.name}: ${a.status === "busy" ? "working" : a.status}`}
          >
            <span
              className="block size-7"
              style={{ color: deviceVar(who.color) || "var(--color-primary)" }}
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
