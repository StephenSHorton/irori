// The sidebar's Agents section split in two (card #456,
// docs/design/spawned-agents.md §10, sa-1): Yours, the terminal agents
// Stephen started, and On cards, the agents Irori started for a card, each
// with its "#N" and grouped under the card when one card has several.
// A spawned agent is known from the cards: a working row marked spawned.
// Only live agents are listed; one that has ended is gone from both.

import { queued as isQueued } from "./spawn-line"

type Agent = { key: string; status: string; deviceState?: string | null }

type Card = {
  id: string
  short: string
  title: string
  working: { key: string; spawned?: boolean; status: string }[]
  starting?: string
  atCap?: string
  // Open blockers on the card (card #512): its idle agents are blocked.
  openBlockers?: number
  // Ready for the person's check: one of openBlockers, not a block.
  ready?: boolean
  queuePlace?: number
  usageLimit?: string | null
  cantStart?: string
}

export type OnCard<A> = {
  cardId: string
  short: string
  title: string
  agents: A[]
}

export type RosterGroups<A> = {
  yours: A[]
  onCards: OnCard<A>[]
  // Agents on cards, cards starting one, and cards queued for one (a
  // slot, a usage limit's reset or the person, card #450).
  spawned: number
  starting: number
  queued: number
  // Live agents that are blocked (card #512): on a permission prompt
  // ("waiting"), or idle on a card with an open blocker (a busy one is
  // working, and a card ready for your check isn't blocked).
  blocked: Set<string>
}

// The roster status of an agent on a permission prompt (card #445).
export const OnPrompt = "waiting"

// rosterGroups sorts the live roster into Yours and On cards. Cards keep
// the order they're given; an agent spawned for two cards (a stale row)
// shows under the first.
export function rosterGroups<A extends Agent>(
  agents: A[],
  cards: Card[]
): RosterGroups<A> {
  const live = agents.filter((a) => a.status !== "gone")
  const byKey = new Map(live.map((a) => [a.key, a]))
  const placed = new Set<string>()
  const onCards: OnCard<A>[] = []
  let starting = 0
  let queued = 0
  const blocked = new Set(
    live.filter((a) => a.status === OnPrompt).map((a) => a.key)
  )
  for (const c of cards) {
    if (c.starting) starting++
    else if (isQueued(c)) queued++
    if ((c.openBlockers ?? 0) - (c.ready ? 1 : 0) > 0)
      for (const w of c.working)
        if (byKey.get(w.key)?.status === "idle") blocked.add(w.key)
    const here: A[] = []
    for (const w of c.working) {
      if (!w.spawned || placed.has(w.key)) continue
      const a = byKey.get(w.key)
      if (!a) continue
      placed.add(w.key)
      here.push(a)
    }
    if (here.length)
      onCards.push({
        cardId: c.id,
        short: c.short,
        title: c.title,
        agents: here,
      })
  }
  return {
    yours: live.filter((a) => !placed.has(a.key)),
    onCards,
    spawned: placed.size,
    starting,
    queued,
    blocked,
  }
}

// spawnedKeys is the agents Irori started for a card, so free counts can
// leave them out: a spawned agent is never handed a second card (§6).
export function spawnedKeys(cards: Card[]): Set<string> {
  const s = new Set<string>()
  for (const c of cards) for (const w of c.working) if (w.spawned) s.add(w.key)
  return s
}
