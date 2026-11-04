// Talk's list sections (card #463): the list once held every DM in one tall
// Direct messages group, hundreds deep with agent↔agent conversations. It
// now splits DMs into sections that fold, folded by default except Threads
// and Channels, and remembers each fold per device. A folded section still
// shows what needs the person: its DMs with something unread and the one
// open now.

import { useCallback, useState } from "react"

import type { Conversation } from "@/lib/conversations"

export type SectionKey = "threads" | "channels" | "dms" | "gone" | "agents"

// foldedByDefault is each section's fold before the person touches it.
export const foldedByDefault: Record<SectionKey, boolean> = {
  threads: false,
  channels: false,
  dms: true,
  gone: true,
  agents: true,
}

// shownAtOnce is how many rows an unfolded section lists before "Show N
// more".
export const shownAtOnce = 8

// dmSections splits DMs (newest first, as listed() gives them) three ways:
// yours with a running agent or a person, the Archive of yours with agents
// that have gone (card #473), and agent↔agent DMs the person only reads.
// Nothing is deleted: the Archive folds at the bottom of the list and
// search still finds what is in it. When the agent comes back (its session
// reattaches) gone turns false and the DM is back with yours.
export function dmSections(
  dms: readonly Conversation[],
  gone: (c: Conversation) => boolean,
  keep = ""
): { yours: Conversation[]; gone: Conversation[]; agents: Conversation[] } {
  const out = {
    yours: [] as Conversation[],
    gone: [] as Conversation[],
    agents: [] as Conversation[],
  }
  for (const c of dms) {
    if (c.readOnly) out.agents.push(c)
    else if (archived(c, gone, keep)) out.gone.push(c)
    else out.yours.push(c)
  }
  return out
}

// archived is whether one of the person's DMs goes in the Archive: its
// agents have gone and nothing in it is new. A gone agent's DM with
// something unread, or that mentions the person, stays with yours until it
// is read. keep is the one the person opened from yours: it stays there
// while open, so reading it does not move it out from under them.
export function archived(
  c: Conversation,
  gone: (c: Conversation) => boolean,
  keep = ""
): boolean {
  if (c.convId === keep || c.unread > 0 || c.mentions > 0) return false
  return gone(c)
}

// needsLook is whether a conversation shows even in a folded section: it is
// open, or it is the person's and has something unread or mentions them.
export function needsLook(c: Conversation, open: string): boolean {
  if (c.convId === open) return true
  return !c.readOnly && (c.unread > 0 || c.mentions > 0)
}

// sectionRows is what a section lists: everything when unfolded (capped at
// shownAtOnce unless all), only what needsLook when folded. hidden is how
// many it leaves out.
export function sectionRows(
  items: readonly Conversation[],
  folded: boolean,
  open: string,
  all: boolean
): { rows: Conversation[]; hidden: number } {
  if (folded) {
    const rows = items.filter((c) => needsLook(c, open))
    return { rows, hidden: items.length - rows.length }
  }
  if (all || items.length <= shownAtOnce) return { rows: [...items], hidden: 0 }
  const rows = items.slice(0, shownAtOnce)
  const at = items.findIndex((c) => c.convId === open)
  if (at >= shownAtOnce) rows.push(items[at])
  return { rows, hidden: items.length - rows.length }
}

// unreadIn counts a section's conversations with something new for the
// person, shown beside a folded section's name.
export function unreadIn(items: readonly Conversation[]): number {
  return items.filter((c) => !c.readOnly && c.unread > 0).length
}

const foldsKey = "irori.talk.folds"

function readFolds(): Partial<Record<SectionKey, boolean>> {
  try {
    const v = JSON.parse(localStorage.getItem(foldsKey) ?? "null")
    return v && typeof v === "object" ? v : {}
  } catch {
    return {}
  }
}

// useFolds is each section's fold, remembered on this device.
export function useFolds(): [
  (k: SectionKey) => boolean,
  (k: SectionKey) => void,
] {
  const [folds, setFolds] = useState(readFolds)
  const folded = useCallback(
    (k: SectionKey) => folds[k] ?? foldedByDefault[k],
    [folds]
  )
  const toggle = useCallback((k: SectionKey) => {
    setFolds((f) => {
      const next = { ...f, [k]: !(f[k] ?? foldedByDefault[k]) }
      try {
        localStorage.setItem(foldsKey, JSON.stringify(next))
      } catch {
        // No storage: the fold lasts until the window reloads.
      }
      return next
    })
  }, [])
  return [folded, toggle]
}
