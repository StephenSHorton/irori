import { useState } from "react"

// "Agents answer now" (card #391, docs/design/talk-channels.md §3): whether
// a post or thread wakes agents and hands them work now. Off, nobody is
// woken or handed it; the agents in it read it at their next turn and reply
// only if they have something to add or correct.

// namesPerson is whether a picked mention is a person, not an agent:
// another board member (person:<id>) or a person's own key (user:<id>).
// The same rule as coord::names_person.
export function namesPerson(key: string): boolean {
  return key.startsWith("person:") || key.startsWith("user:")
}

// agentsNowDefault is the toggle's starting point for what the text names
// (coord::agents_now_default): on when it names nobody or an agent, off
// when it names only people. Card refs (#12) don't count as naming anyone.
export function agentsNowDefault(
  mentions: readonly { key: string }[]
): boolean {
  const keys = mentions
    .map((m) => m.key.trim())
    .filter((k) => k && k !== "irori")
  return keys.length === 0 || keys.some((k) => !namesPerson(k))
}

// useAgentsNow is a composer's toggle: the default for its mentions until
// the person flips it, which holds until reset (after a send).
export function useAgentsNow(mentions: readonly { key: string }[]): {
  on: boolean
  set: (on: boolean) => void
  reset: () => void
} {
  const [picked, setPicked] = useState<boolean | null>(null)
  return {
    on: picked ?? agentsNowDefault(mentions),
    set: setPicked,
    reset: () => setPicked(null),
  }
}
