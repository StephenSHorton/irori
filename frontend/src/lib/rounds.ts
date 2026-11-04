import type { CardThreadItem } from "./models"
import { ThreadKind } from "./thread"

// A done check the person sent back (card #476): their reason, the agent's
// answers on it since, and whether the agent heard it. The window shows
// this on the check while the agent works on it, and on the next check as
// its earlier round, so a re-raised check never reads like a first ask.
export type SentBack = {
  // The person's reason: their last own words on the check.
  reason: string
  reasonAt?: string
  // The agents' answers on the check after the reason, oldest first.
  replies: CardThreadItem[]
  // An agent read the reason (or answered it).
  heard: boolean
  answered: boolean
  // Who has it but hasn't read it yet ("sent to card-81-1, not read yet").
  pending: string[]
}

const person = (it: CardThreadItem) => it.who.status === "person"
const agent = (it: CardThreadItem) =>
  it.who.status !== "person" && it.who.status !== "irori"

// sentBack reads a done check's thread: null when the person said nothing
// on it. `fallback` is the reason when no comment carries it (a card from
// before #476, whose check was resolved with their words as its answer).
export function sentBack(
  thread: CardThreadItem[],
  fallback = ""
): SentBack | null {
  const comments = thread.filter((it) => it.kind === ThreadKind.Comment)
  let at = -1
  comments.forEach((it, i) => {
    if (person(it)) at = i
  })
  if (at < 0) {
    if (!fallback.trim()) return null
    const replies = comments.filter(agent)
    return {
      reason: fallback,
      replies,
      heard: replies.length > 0,
      answered: replies.length > 0,
      pending: [],
    }
  }
  const reason = comments[at]
  const replies = comments.slice(at + 1).filter(agent)
  const answered = replies.length > 0
  return {
    reason: reason.text,
    reasonAt: reason.at,
    replies,
    heard: answered || (reason.heardBy?.length ?? 0) > 0,
    answered,
    pending: answered ? [] : (reason.pending ?? []),
  }
}
