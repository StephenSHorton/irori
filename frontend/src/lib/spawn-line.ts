// The card line for a card on a spawning board that has no agent yet
// (card #448, docs/design/spawned-agents.md §14 part 4, sa-1): an agent
// starting, its place in a device's queue, why one can't start, or what
// holds the queue (card #450, part 6: a Claude usage limit, the daily
// limit). It replaces "nobody on it yet" there; a repo that isn't trusted
// also gets #449's Trust and start row in Needs you.

export type SpawnState = {
  starting?: string
  atCap?: string
  // Its place in that device's queue, from 1 (card #450).
  queuePlace?: number
  // While a usage limit holds spawns: when they start again (RFC 3339).
  usageLimit?: string | null
  // The daily limit, when today's spawns reached it.
  dailyLimit?: number
  cantStart?: string
}

export const NOT_TRUSTED = "not trusted: "

export type SpawnLine = { text: string; tone: "quiet" | "warn" }

// ordinal is 1st, 2nd, 3rd, 4th … 11th, 12th, 13th, 21st.
export function ordinal(n: number): string {
  const teen = n % 100 >= 11 && n % 100 <= 13
  const end = teen ? "th" : (["th", "st", "nd", "rd"][n % 10] ?? "th")
  return `${n}${end}`
}

// spawnLine is the card's spawn state in words, or null when it has none.
export function spawnLine(c: SpawnState): SpawnLine | null {
  if (c.starting) {
    return { text: `starting an agent on ${c.starting}`, tone: "quiet" }
  }
  if (c.cantStart) {
    if (c.cantStart.startsWith(NOT_TRUSTED)) {
      return { text: "can't start: its repo isn't trusted here", tone: "warn" }
    }
    const why = c.cantStart.replace(/^couldn't start: /, "")
    return { text: `can't start: ${why}`, tone: "warn" }
  }
  if (c.usageLimit) {
    return { text: "queued: usage limit", tone: "quiet" }
  }
  if (c.atCap && c.queuePlace) {
    return {
      text: `queued: ${ordinal(c.queuePlace)} for ${c.atCap}`,
      tone: "quiet",
    }
  }
  if (c.atCap) {
    return { text: `waiting: ${c.atCap} is at its cap`, tone: "quiet" }
  }
  if (c.dailyLimit) {
    return {
      text: `waiting: today's ${c.dailyLimit} agents are started; start this one yourself`,
      tone: "quiet",
    }
  }
  return null
}

// startPastLimit says what Start an agent does for a card a limit keeps
// waiting (a usage limit or the daily limit), for its button's title; ""
// when no limit is what it waits on.
export function startPastLimit(c: SpawnState): string {
  if (c.starting || c.cantStart) return ""
  if (c.usageLimit) {
    return "Tries now instead of waiting for the reset. If the limit still stands, Irori holds again."
  }
  return c.dailyLimit ? "Starts this card's agent now, past today's limit" : ""
}

// queued is whether a card waits in the spawn queue (a place, a usage
// limit or the daily limit), for the roster's "N queued".
export function queued(c: SpawnState): boolean {
  return (
    !c.starting && !c.cantStart && !!(c.atCap || c.usageLimit || c.dailyLimit)
  )
}

// usageResets says when a usage limit's hold ends: "resets 4:30 PM".
export function usageResets(at: string, locale?: string): string {
  const d = new Date(at)
  if (Number.isNaN(d.getTime())) return ""
  return `resets ${d.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" })}`
}
