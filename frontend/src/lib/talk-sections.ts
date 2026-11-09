// Talk's list sections (card #463): the list once held every DM in one tall
// Direct messages group, hundreds deep with agent↔agent conversations. It
// now splits DMs into sections that fold and remembers each fold per
// device. A folded section lists no rows at all (card #484): only its name,
// its count and how many have something new (the Archive only its count,
// #514). Keeping its unread rows
// showing half-collapsed it, rows under a folded heading.

import { useCallback, useState } from "react"

import type { Conversation } from "@/lib/conversations"

export type SectionKey =
  "threads" | "huddles" | "channels" | "dms" | "gone" | "agents"

// foldedByDefault is each section's fold before the person touches it.
export const foldedByDefault: Record<SectionKey, boolean> = {
  threads: false,
  // Huddles (card #1044): newest first, the live ones at the top.
  huddles: false,
  channels: false,
  // Unfolded (card #484): a folded section shows no rows, so folding the
  // person's live DMs by default would hide what waits on them.
  dms: false,
  gone: true,
  agents: true,
}

// shownAtOnce is how many rows an unfolded section lists before "Show N
// more".
export const shownAtOnce = 8

// dmSections splits DMs (newest first, as listed() gives them) three ways:
// yours with a running agent or a person, the Archive of yours with agents
// that have gone (card #473), and agent↔agent DMs the person only reads.
// An agent↔agent DM whose agents have all gone goes to the Archive too
// (card #1037), so Agents talking lists only live conversations.
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
    if (archived(c, gone, keep)) out.gone.push(c)
    else if (c.readOnly) out.agents.push(c)
    else out.yours.push(c)
  }
  return out
}

// archived is whether one of the person's DMs goes in the Archive: its
// agents have gone, read or not. Its unread show beside "Archive" and in
// the Archive, marked unread, not in the main list (Stephen, on #473).
// keep is the one the person opened from yours: it stays there while open,
// so its agent going does not move it out from under them.
export function archived(
  c: Conversation,
  gone: (c: Conversation) => boolean,
  keep = ""
): boolean {
  return c.convId !== keep && gone(c)
}

// sectionRows is what a section lists: nothing when folded, every section
// alike (card #484; the Archive first, on #473), and everything when
// unfolded (capped at shownAtOnce unless all, keeping the open one). hidden
// is how many it leaves out.
export function sectionRows(
  items: readonly Conversation[],
  folded: boolean,
  open: string,
  all: boolean
): { rows: Conversation[]; hidden: number } {
  if (folded) return { rows: [], hidden: items.length }
  if (all || items.length <= shownAtOnce) return { rows: [...items], hidden: 0 }
  const rows = items.slice(0, shownAtOnce)
  const at = items.findIndex((c) => c.convId === open)
  if (at >= shownAtOnce) rows.push(items[at])
  return { rows, hidden: items.length - rows.length }
}

// A broadcast (card #486) is one message an agent sent to many agents. Irori
// sends it as one message per agent, so each copy lands in its own
// agent↔agent DM, and the list once showed owl's "WIND DOWN NOW" as ten
// identical rows. It is one row now, "owl → 10 agents", that unfolds to the
// DMs it went to.
export type Broadcast = {
  // "broadcast:", the sender and the text: it stays the same while copies
  // come in or a recipient answers, so the row keeps its fold.
  id: string
  // The sender's key, the text, and when the newest copy went.
  by: string
  text: string
  lastAt: string
  // The DMs it went to, newest first.
  convs: Conversation[]
}

// AgentRow is one row of Agents talking: a DM, or a broadcast.
export type AgentRow =
  { kind: "dm"; c: Conversation } | { kind: "broadcast"; b: Broadcast }

// broadcastWindow is how far apart one sender's copies of the same text may
// be and still be one broadcast. Irori sends them a second or two apart, and
// the same text a day later is a new message.
export const broadcastWindow = 10 * 60_000

// agentsCap is the most rows Agents talking lists, even after "Show more",
// because the live db has thousands of agent↔agent messages. Search still
// finds the older ones.
export const agentsCap = 50

const ms = (s: string) => {
  const t = Date.parse(s)
  return Number.isNaN(t) ? 0 : t
}

// agentRows groups read-only DMs (newest first) whose newest post has the
// same sender and text, each copy within broadcastWindow of the next, into
// one broadcast row where its newest DM was. A DM that someone answered has
// a different last line, so it stays its own row.
export function agentRows(items: readonly Conversation[]): AgentRow[] {
  const out: AgentRow[] = []
  const growing = new Map<string, Broadcast>()
  for (const c of items) {
    const text = c.lastText ?? ""
    const by = c.lastBy ?? ""
    if (!c.readOnly || !text || !by || !c.members.some((m) => m.key === by)) {
      out.push({ kind: "dm", c })
      continue
    }
    const key = `${by}\u0000${text}`
    const b = growing.get(key)
    if (
      b &&
      ms(b.convs[b.convs.length - 1].lastAt) - ms(c.lastAt) <= broadcastWindow
    ) {
      b.convs.push(c)
      continue
    }
    const nb: Broadcast = {
      id: `broadcast:${key}`,
      by,
      text,
      lastAt: c.lastAt,
      convs: [c],
    }
    growing.set(key, nb)
    out.push({ kind: "broadcast", b: nb })
  }
  return out.map((r) =>
    r.kind === "broadcast" && r.b.convs.length === 1
      ? { kind: "dm", c: r.b.convs[0] }
      : r
  )
}

// broadcastTo is who a broadcast went to: each DM's other side.
export function broadcastTo(b: Broadcast): Conversation["members"] {
  return b.convs.map(
    (c) => c.members.find((m) => m.key !== b.by) ?? c.members[0]
  )
}

// agentRowHas is whether a row holds conversation id.
export function agentRowHas(r: AgentRow, id: string): boolean {
  if (!id) return false
  return r.kind === "dm"
    ? r.c.convId === id
    : r.b.convs.some((c) => c.convId === id)
}

// agentSectionRows is what Agents talking lists. Folded, it lists nothing,
// as every section (card #484). Unfolded, it lists the newest
// shownAtOnce, or up to agentsCap after "Show more", plus the open one's
// row wherever it is. Search lists every match. hidden is how many more
// "Show more" would list. capped is how many lie past agentsCap.
export function agentSectionRows(
  rows: readonly AgentRow[],
  folded: boolean,
  open: string,
  all: boolean,
  searching = false
): { rows: AgentRow[]; hidden: number; capped: number } {
  if (folded) return { rows: [], hidden: 0, capped: 0 }
  const i = rows.findIndex((r) => agentRowHas(r, open))
  if (searching) return { rows: [...rows], hidden: 0, capped: 0 }
  const limit = all ? agentsCap : shownAtOnce
  const shown = rows.slice(0, limit)
  if (i >= limit) shown.push(rows[i])
  const reach = Math.min(rows.length, agentsCap)
  const beyond = i >= agentsCap ? 1 : 0
  return {
    rows: shown,
    hidden: reach - (shown.length - beyond),
    capped: Math.max(0, rows.length - agentsCap - beyond),
  }
}

// unreadIn counts a section's conversations with something new for the
// person, shown beside a folded section's name.
export function unreadIn(items: readonly Conversation[]): number {
  return items.filter((c) => !c.readOnly && c.unread > 0).length
}

// sectionHeader is what a section's name shows besides its count: how
// many have something new (the pill a folded section shows) and whether it
// offers Mark all read. The Archive is there to put gone agents' DMs out of
// the way, so folded it is only "Archive · N": no pill inviting a look, no
// Mark all read (Stephen, #513). Opened, its rows keep their unread marks
// and Mark all read shows while something is new. Mark all read is the
// Archive's alone (card #473).
export function sectionHeader(
  k: SectionKey,
  folded: boolean,
  items: readonly Conversation[]
): { unread: number; markAll: boolean } {
  const unread = unreadIn(items)
  if (k !== "gone") return { unread, markAll: false }
  if (folded) return { unread: 0, markAll: false }
  return { unread, markAll: unread > 0 }
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
