// The card line for a card on a spawning board that has no agent yet
// (card #448, docs/design/spawned-agents.md §14 part 4, sa-1): an agent
// starting, a device at its cap, or why one can't start. It replaces
// "nobody on it yet" there; a repo that isn't trusted also gets #449's
// Trust and start row in Needs you.

export type SpawnState = {
  starting?: string
  atCap?: string
  cantStart?: string
  noAgent?: string
}

export const NOT_TRUSTED = "not trusted: "

export type SpawnLine = { text: string; tone: "quiet" | "warn" }

// spawnLine is the card's spawn state in words, or null when it has none.
export function spawnLine(c: SpawnState): SpawnLine | null {
  if (c.starting) {
    return { text: `starting an agent on ${c.starting}`, tone: "quiet" }
  }
  // Card #541: no computer can take it. Already a whole sentence.
  if (c.noAgent) return { text: c.noAgent, tone: "warn" }
  if (c.cantStart) {
    if (c.cantStart.startsWith(NOT_TRUSTED)) {
      return { text: "can't start: its repo isn't trusted here", tone: "warn" }
    }
    const why = c.cantStart.replace(/^couldn't start: /, "")
    return { text: `can't start: ${why}`, tone: "warn" }
  }
  if (c.atCap) {
    return { text: `waiting: ${c.atCap} is at its cap`, tone: "quiet" }
  }
  return null
}
