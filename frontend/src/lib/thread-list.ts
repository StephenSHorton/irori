// Card #1149 (docs/design/list-signals.md §6.3): the thread lists' side of
// the by-id read. A notice names thread ids (board:changed.cards, the read
// cursors in talk:changed.convs); the list reads just those rows with
// threads.byId, upserts them, drops the `gone`, and puts the list in the
// order and size the host's full read has: (resolved, updated_at desc in
// nanoseconds, id), and the newest ResolvedShown resolved ones per bucket.
//
// Nothing in a thread row comes from the inbox, so no list here wakes on
// inbox:changed.

import type { ThreadSummary } from "./models"
import type { Follow, Want } from "./patched"

// ResolvedShown is how many resolved threads a bucket lists (as the host's
// RESOLVED_SHOWN).
export const ResolvedShown = 20

// threadFollow is the signals a thread list wakes on: card changes (the
// thread ids among their cards, queue moves and spawn-grace timers
// included), the read cursors in talk:changed.convs (a thread marked read
// here or on another device), and every device change (names, so `all`).
// focus is on the card signal only.
export const threadFollow: readonly Follow[] = [
  { signal: "board:changed", buckets: ["cards"], focus: true },
  { signal: "talk:changed", buckets: ["convs"] },
  { signal: "devices:changed" },
]

export type ThreadList = { rows: ThreadSummary[]; asOf: number }

// What one read gave: the whole list, or the rows of some ids and the ids
// that have none.
export type ThreadPatch =
  | { full: true; rows: ThreadSummary[]; asOf: number }
  | { full: false; rows: ThreadSummary[]; gone: string[]; asOf: number }

// wantedIds is the thread ids a patch read covers.
export function wantedIds(want: Want): string[] {
  const ids = new Set<string>()
  for (const k of ["board:changed.cards", "talk:changed.convs"])
    for (const id of want.ids[k] ?? []) ids.add(id)
  return [...ids]
}

// threadTime is an RFC 3339 time as [seconds, nanoseconds], to compare
// exact ties as the host does (Date.parse stops at milliseconds).
export function threadTime(s: string | undefined): [number, number] {
  const m = /^(.*?)(?:\.(\d+))?(Z|[+-]\d\d:\d\d)$/.exec(s ?? "")
  if (!m) return [0, 0]
  const secs = Math.floor(Date.parse(m[1] + m[3]) / 1000)
  if (Number.isNaN(secs)) return [0, 0]
  return [secs, Number(((m[2] ?? "") + "000000000").slice(0, 9))]
}

const resolved = (t: ThreadSummary) => t.state === "resolved"

// byNewest is the host's order: open before resolved, newest first, equal
// times by id.
export function byNewest(a: ThreadSummary, b: ThreadSummary): number {
  if (resolved(a) !== resolved(b)) return resolved(a) ? 1 : -1
  const [as, an] = threadTime(a.updatedAt)
  const [bs, bn] = threadTime(b.updatedAt)
  if (as !== bs) return bs - as
  if (an !== bn) return bn - an
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0
}

// bucketOf is the list a thread is capped in, as the host caps them: a
// space's (shared log, or a private thread about it), Personal's, and the
// private threads about a space you left (also space "", told by private).
export function bucketOf(t: ThreadSummary): string {
  const space = t.spaceId ?? ""
  return space === "" && t.private ? "|left" : space
}

// capResolved keeps each bucket's newest ResolvedShown resolved threads.
// rows must be in byNewest order.
export function capResolved(rows: ThreadSummary[]): ThreadSummary[] {
  const seen = new Map<string, number>()
  return rows.filter((t) => {
    if (!resolved(t)) return true
    const b = bucketOf(t)
    const n = (seen.get(b) ?? 0) + 1
    seen.set(b, n)
    return n <= ResolvedShown
  })
}

// mergeThreads puts a read into the list. bucket is the one bucket a
// per-space list shows (rows of another are dropped); null: every bucket.
export function mergeThreads(
  cur: ThreadList,
  patch: ThreadPatch,
  bucket: string | null
): ThreadList {
  const mine = (t: ThreadSummary) => bucket === null || bucketOf(t) === bucket
  if (patch.full) return { rows: patch.rows, asOf: patch.asOf }
  const byId = new Map(cur.rows.map((r) => [r.id, r]))
  for (const id of patch.gone) byId.delete(id)
  for (const r of patch.rows) {
    if (mine(r)) byId.set(r.id, r)
    else byId.delete(r.id)
  }
  const rows = capResolved([...byId.values()].sort(byNewest))
  // A thread started since asOf is shown by withStarted until a full read
  // covers it: only a full read moves asOf.
  return { rows, asOf: cur.asOf }
}

export type ReadDeps = {
  startedSeq(): number
  full(): Promise<ThreadSummary[]>
  byId(ids: string[]): Promise<{ rows: ThreadSummary[]; gone: string[] }>
  // The rows on screen now.
  current(): ThreadSummary[]
}

// readThreads reads what `want` covers: the whole list, or the rows of its
// ids. A thread leaving resolved (reopened, withdrawn, cleared) can't bring
// back the resolved one the cap pushed out, so a patch that shows one
// leave reads the whole list instead (the host sets `all` for those too).
export async function readThreads(
  want: Want,
  d: ReadDeps
): Promise<ThreadPatch> {
  const asOf = d.startedSeq()
  if (want.full) return { full: true, rows: await d.full(), asOf }
  const ids = wantedIds(want)
  if (!ids.length) return { full: false, rows: [], gone: [], asOf }
  const got = await d.byId(ids)
  const was = new Set(
    d
      .current()
      .filter(resolved)
      .map((t) => t.id)
  )
  const left =
    got.rows.some((t) => was.has(t.id) && !resolved(t)) ||
    got.gone.some((id) => was.has(id))
  if (left) return { full: true, rows: await d.full(), asOf }
  return { full: false, rows: got.rows, gone: got.gone, asOf }
}
