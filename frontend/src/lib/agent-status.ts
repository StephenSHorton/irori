// agentStatus is the one place an agent's roster status becomes words and a
// colour (card #489): the hover card, Settings, the sidebar, talk, the @
// picker, faces and the card holder line all read it, so a `shell` agent
// reads "running a job" everywhere instead of "idle", "shell" or "gone".
import { runningSince } from "./agents"
import { plainLine } from "./irori"

// StatusTone is how the text is coloured: working and job are the live
// green, warn the amber, idle and muted the grey.
export type StatusTone = "working" | "job" | "warn" | "idle" | "muted"
export type AgentStatusText = { text: string; tone: StatusTone }

type Who = {
  status: string
  live?: boolean
  offline?: boolean
  deviceState?: string | null
}

// agentStatus says how an agent stands. since is when its current status
// began (ms or an ISO time): a job older than a minute reads
// "running a job · 12 min".
export function agentStatus(
  a: Who,
  since?: number | string | null,
  now = Date.now()
): AgentStatusText {
  const s = a.status
  // Another member's agent whose state this device doesn't know (card
  // #478): no word, rather than gone or not running.
  if (s === "unknown") return { text: "", tone: "muted" }
  if (s === "gone") return { text: "gone", tone: "muted" }
  if (a.offline || a.deviceState === "offline")
    return { text: "device offline", tone: "muted" }
  if (a.live === false) return { text: "not running", tone: "muted" }
  if (s === "starting") return { text: "starting", tone: "muted" }
  // A person, or Irori itself: neither works nor idles.
  if (s === "person" || s === "irori") return { text: "", tone: "muted" }
  if (s === "busy" || s === "working")
    return { text: "working", tone: "working" }
  if (s === "shell")
    return { text: `running a job${jobFor(since, now)}`, tone: "job" }
  if (s === "unresponsive") return { text: "not answering", tone: "warn" }
  if (s === "idle") return { text: "idle", tone: "idle" }
  // A spawned agent whose process stopped; it keeps its name (card #1215).
  if (s === "resting") return { text: "resting", tone: "muted" }
  // A permission prompt (card #445).
  if (s === "waiting")
    return { text: "waiting on a permission prompt", tone: "warn" }
  // agent-counts counts any other status as working.
  return { text: "working", tone: "working" }
}

// jobFor is " · 12 min" for a job that began at least a minute ago, else "":
// no "· just now", and nothing for an unknown or zero time.
function jobFor(
  since: number | string | null | undefined,
  now: number
): string {
  if (since == null || since === "") return ""
  const at = typeof since === "number" ? since : Date.parse(since)
  if (!Number.isFinite(at) || at <= 0 || now - at < 60_000) return ""
  return ` · ${runningSince(at, now)}`
}

// throughLine says a device has no link of its own here, only rows coming
// through another one (card #544), the same words on every surface.
export function throughLine(via: string): string {
  return `through ${plainLine(via)}`
}
