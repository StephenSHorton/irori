// Agent counts that match who a thread or ask can go to (card #172).
// Irori hands work only to an agent whose roster status is exactly "idle"
// on a device that isn't offline (coord asks.rs free_agents), and (card
// #453) only to a terminal agent the person turned "Offer it cards" on
// for: an idle agent without it, or one Irori started for a card, works
// only on what it was given, so it isn't free. Everything
// else that is around is working: "busy", and also "shell" (waiting on a
// command it started, still on its card) or any other status Claude Code
// reports. "unresponsive" (#110) is idle but not answering its wakes, so it
// is neither free nor working. Gone agents aren't counted.

// The roster status of an agent whose wakes went unread (coord roster.rs
// NOT_ANSWERING, #110).
export const NotAnswering = "unresponsive"

type Agent = {
  status: string
  deviceState?: string | null
  repoKey?: string
  offersCards?: boolean
  spawned?: boolean
}

// takesOffers is whether Irori hands this agent unheld cards, asks and
// threads at all (card #453).
export function takesOffers(a: Agent): boolean {
  return !!a.offersCards && !a.spawned
}

export type AgentCounts = {
  // Agents still around (not gone).
  total: number
  // Busy, or waiting on their own command ("shell"), or any other status.
  working: number
  // Of working: the ones running a job ("shell"), card #489.
  running: number
  // Idle on an online device with "Offer it cards" on: who a thread or
  // ask goes to.
  free: number
  // Idle on an online device, but not taking offered work (card #453).
  ownWork: number
  // Idle, but their wakes went unread (#110).
  silent: number
  // On a device that is offline.
  offline: number
  // Blocked (card #512): on a permission prompt, or on a card with an open
  // blocker, from roster-groups. Neither working nor free.
  blocked: number
}

export function agentCounts(
  agents: (Agent & { key?: string })[],
  blocked: Set<string> = new Set()
): AgentCounts {
  const c: AgentCounts = {
    total: 0,
    working: 0,
    running: 0,
    free: 0,
    ownWork: 0,
    silent: 0,
    offline: 0,
    blocked: 0,
  }
  for (const a of agents) {
    if (a.status === "gone") continue
    c.total++
    if (a.deviceState === "offline") c.offline++
    else if (a.key !== undefined && blocked.has(a.key)) c.blocked++
    else if (a.status === NotAnswering) c.silent++
    else if (a.status === "idle") {
      if (takesOffers(a)) c.free++
      else c.ownWork++
    } else {
      c.working++
      if (a.status === "shell") c.running++
    }
  }
  return c
}

// freeIn is how many free agents work in repoKey ("" for any repo).
export function freeIn(agents: Agent[], repoKey = ""): number {
  return agents.filter(
    (a) =>
      a.status === "idle" &&
      a.deviceState !== "offline" &&
      takesOffers(a) &&
      (!repoKey || a.repoKey === repoKey)
  ).length
}

// whyWaiting says why a thread or ask that nobody has taken is still
// waiting, from the same rule Irori hands work out by. "" when someone
// is free (it should go to them on the next sweep).
export function whyWaiting(agents: Agent[], repoKey = ""): string {
  const c = agentCounts(agents)
  if (c.total === 0) return "No agents are running."
  if (!agents.some((a) => a.status !== "gone" && takesOffers(a)))
    return "No agent takes offered work: Irori starts one for it when spawning is on."
  if (c.free > 0) {
    if (repoKey && freeIn(agents, repoKey) === 0)
      return `${n(c.free, "agent is", "agents are")} idle, but none works in ${repoKey}.`
    return ""
  }
  const parts: string[] = []
  if (c.working > 0)
    parts.push(
      c.running > 0
        ? `${c.working} working (${c.running} running a job)`
        : `${c.working} working`
    )
  if (c.ownWork > 0) parts.push(`${c.ownWork} idle, not taking offered work`)
  if (c.silent > 0) parts.push(`${c.silent} idle but not answering`)
  if (c.offline > 0) parts.push(`${c.offline} on an offline device`)
  return `Nobody is free: ${parts.join(", ")}. Irori starts an agent for it when spawning is on.`
}

function n(k: number, one: string, many: string): string {
  return `${k} ${k === 1 ? one : many}`
}

// IdleReport is Home's idle line's input (card #341): idle agents and the
// unheld cards nobody has been handed yet, with why (coord idle_report).
export type IdleReport = {
  idle: number
  unoffered: { id: string; short: string; title: string; why: string }[]
}

// IdleLineShown is how many unhanded cards the line names before "and N more".
export const IdleLineShown = 3

// Untaken is an open card Home files under Stuck because nobody took it
// (card #487, needs.ts nobodyTook): never handed out, offered or assigned
// and not picked up, or an ask nobody took.
export type Untaken = { id: string; short: string }

// idleLine says on Home whether idle agents mean a work shortage or a
// dispatch gap (cards #341, #487): "3 agents idle, no open work" only when
// no open card waits for a taker; else "1 agent idle; 3 cards nobody
// took: #25, #6, #14", with the idle report's why for a card Irori hasn't
// handed out yet. A report card Home doesn't list at all (homeIds: the ids
// of the cards Home lists) still counts; one Home lists under another
// group (an agent starting, a device at its cap) goes by its row.
// "" when nobody is idle and nothing waits.
export function idleLine(
  r: IdleReport,
  untaken: readonly Untaken[] = [],
  homeIds: ReadonlySet<string> = new Set()
): string {
  const why = new Map(r.unoffered.map((u) => [u.id, u.why]))
  const cards: { short: string; why: string }[] = untaken.map((u) => ({
    short: u.short,
    why: why.get(u.id) ?? "",
  }))
  const listed = new Set(untaken.map((u) => u.id))
  for (const u of r.unoffered)
    if (!listed.has(u.id) && !homeIds.has(u.id))
      cards.push({ short: u.short, why: u.why })
  const k = cards.length
  if (k === 0)
    return r.idle > 0
      ? `${n(r.idle, "agent", "agents")} idle, no open work.`
      : ""
  const shown = cards
    .slice(0, IdleLineShown)
    .map((u) => (u.why ? `${u.short} (${u.why})` : u.short))
    .join(", ")
  const more = k > IdleLineShown ? `, and ${k - IdleLineShown} more` : ""
  const who =
    r.idle > 0 ? `${n(r.idle, "agent", "agents")} idle` : "No agent is free"
  return `${who}; ${n(k, "card", "cards")} nobody took: ${shown}${more}.`
}
