// Card #1146 (docs/design/list-signals.md §5): the patch hook, next to
// useLatest. A list built on it reads only the rows its signals name, and
// everything when a notice (changes.ts) says `all` or a read of it failed.
// No list uses it yet: every list is still on useLatest and reads
// everything. Cards 6–11 move them over one by one, so the drift check
// below does nothing until one of them patches.
//
// Reads in flight are new code, not something latestReads gives. latestReads
// queues a follow-up only for the same load function, and starts a read
// beside a stuck one (StuckReadMs), whose late answer is then dropped. So a
// patched list has ONE stable load. It never closes over a notice's ids: when
// latestReads starts it, it drains what is pending for the list ({all, ids}).
// Notices, focus and the drift check only add to that pending state and call
// reload() with the same load, so notices during a read make one follow-up
// and never a read beside it.
//
// A read whose answer errors or is dropped puts its ids back (an error also
// marks the next read `all`: a retry after a failure reads everything). The
// patch is merged into the rows on screen when the answer is shown, not
// inside the load.

import { useCallback, useEffect, useRef, useState } from "react"

import { onChange, type Ids, type Notice, type Signal } from "./changes"
import { latestReads, sameAs, type Answer } from "./talk"
import { appPage } from "./visibility"
import {
  noteDrift,
  noteDriftDropped,
  noteRead,
  type ReadReport,
} from "./window-log"

// Follow is one signal a list wakes on, and the id buckets of it that name
// the list's rows. A notice naming none of the buckets (and not `all`) is
// not for the list.
export type Follow = {
  signal: Signal
  buckets?: readonly string[]
  // focus: a full read when the window gets focus (at most once a minute).
  // Put it on one signal of the list only.
  focus?: boolean
}

// Want is what one read covers. `ids` are keyed "<signal>.<bucket>". `full`
// says to read everything: `all`, or a drift sample.
export type Want = { all: boolean; drift: boolean; full: boolean; ids: Ids }

const key = (signal: Signal, bucket: string) => `${signal}.${bucket}`

type Pending = { all: boolean; drift: boolean; ids: Map<string, Set<string>> }

const nothing = (): Pending => ({ all: false, drift: false, ids: new Map() })

const empty = (p: Pending) => !p.all && !p.drift && p.ids.size === 0

export type PatchedOptions<T, P> = {
  name: string
  // read reads what want says: everything when want.full, else the rows
  // named in want.ids.
  read(want: Want): Promise<P>
  // merge puts a read's answer into the rows on screen: a replacement when
  // want.full, else an upsert of the rows read (and a drop of the gone).
  merge(current: T, patch: P, want: Want): T
  current(): T
  set(next: T): void
  fail(error: unknown): void
  // rows lists the list's rows by id, in the order shown, for the drift
  // check. Without it the list has no drift check.
  rows?(value: T): Array<[id: string, row: unknown]>
  // Where the drift check and reads are counted (window-log.ts).
  noteDrift?(rows: number): void
  noteDropped?(): void
  noteRead?: (r: ReadReport) => void
  now?: () => number
  // settleMs is how long a drift sample waits after its read before it is
  // compared (default DriftSettleMs): a change made while the read ran
  // reaches the page a moment after it, and a notice in that wait drops
  // the sample. 0 compares at once.
  settleMs?: number
}

export type PatchedReads = {
  // notice takes a notice from changes.ts: a patch for its ids, or a full
  // read for `all`. A notice naming none of the list's buckets is ignored.
  notice(n: Notice): void
  // reload is the hook's own: always a full read, and its promise ends
  // with a read begun after it was called.
  reload(): Promise<void>
  // driftCheck is one drift sample: a full read through the same queue,
  // compared with the rows shown. Nothing unless a patch has been merged
  // since the last full read.
  driftCheck(): void
  // pending says whether anything waits for the next read.
  pending(): boolean
}

export function patchedReads<T, P>(o: PatchedOptions<T, P>): PatchedReads {
  // The first read is a full one.
  let pending: Pending = { ...nothing(), all: true }
  // Notices that mattered to this list, to tell a drift sample that one
  // came during.
  let touched = 0
  // A patch has been merged since the last full read.
  let patched = false

  type Result = { want: Want; patch: P | null; touched: number }

  const wanted = (p: Pending): Want => {
    const ids: Ids = {}
    for (const [k, set] of p.ids) ids[k] = [...set]
    return { all: p.all, drift: p.drift, full: p.all || p.drift, ids }
  }

  const putBack = (want: Want, failed: boolean) => {
    if (want.all || failed) pending.all = true
    for (const [k, ids] of Object.entries(want.ids)) {
      const set = pending.ids.get(k) ?? new Set<string>()
      for (const id of ids) set.add(id)
      pending.ids.set(k, set)
    }
  }

  // The one load. It drains pending as it starts.
  const load = async (): Promise<Result> => {
    const drained = pending
    pending = nothing()
    const want = wanted(drained)
    const since = touched
    if (empty(drained)) return { want, patch: null, touched: since }
    try {
      return { want, patch: await o.read(want), touched: since }
    } catch (err) {
      putBack(want, true)
      throw err
    }
  }

  const dropSample = () => (o.noteDropped ?? noteDriftDropped)()

  const compare = (before: T, after: T) => {
    if (!o.rows) return
    const n = rowsDiffer(o.rows(before), o.rows(after))
    ;(o.noteDrift ?? ((rows) => noteDrift(o.name, rows)))(n)
  }

  const reads: (l: () => Promise<Result>) => Promise<void> =
    latestReads<Result>(
      (answer) => {
        if ("error" in answer) {
          o.fail(answer.error)
          return
        }
        const { want, patch, touched: since } = answer.value
        if (patch === null) return
        const before = o.current()
        const after = o.merge(before, patch, want)
        if (want.drift) {
          // A notice for this list during the read, a patch waiting for
          // one, or notices drained into this very read make the two lists
          // differ by the notice, not by a miss.
          const drained = want.all || Object.keys(want.ids).length > 0
          if (drained || touched !== since || !empty(pending)) dropSample()
          else if ((o.settleMs ?? DriftSettleMs) <= 0) compare(before, after)
          else {
            // The pump delivers a notice a moment after its change, so a
            // read that ran meanwhile can already hold a change whose
            // notice has not come yet: wait for it before comparing.
            const at = touched
            setTimeout(() => {
              if (touched !== at || !empty(pending)) dropSample()
              else compare(before, after)
            }, o.settleMs ?? DriftSettleMs)
          }
        }
        patched = want.full ? false : true
        o.set(after)
      },
      o.now ?? Date.now,
      o.noteRead ?? ((r) => noteRead(o.name, r)),
      (answer: Answer<Result>) => {
        // Not shown: an older answer than the last, or a stuck read's late
        // one. What it covered goes back, and a read follows.
        if ("value" in answer) {
          if (answer.value.want.drift) dropSample()
          putBack(answer.value.want, false)
        }
        if (!empty(pending)) void reads(load)
      }
    )

  const relevant = (n: Notice) =>
    n.all || Object.values(n.ids).some((ids) => ids.length > 0)

  return {
    notice(n) {
      if (!relevant(n)) return
      touched++
      if (n.all) pending.all = true
      else
        for (const [bucket, ids] of Object.entries(n.ids)) {
          if (!ids.length) continue
          const k = key(n.signal, bucket)
          const set = pending.ids.get(k) ?? new Set<string>()
          for (const id of ids) set.add(id)
          pending.ids.set(k, set)
        }
      void reads(load)
    },
    reload() {
      pending.all = true
      return reads(load)
    },
    driftCheck() {
      if (!patched || !o.rows) return
      pending.drift = true
      void reads(load)
    },
    pending: () => !empty(pending),
  }
}

// rowsDiffer is how many rows of two lists differ: ids in only one, rows
// that serialize differently, and one more if the rows both hold are in
// another order.
export function rowsDiffer(
  a: Array<[string, unknown]>,
  b: Array<[string, unknown]>
): number {
  const ser = (row: unknown) => {
    try {
      return JSON.stringify(row) ?? ""
    } catch {
      return String(row)
    }
  }
  const inB = new Map(b.map(([id, row]) => [id, ser(row)]))
  const inA = new Set(a.map(([id]) => id))
  let n = 0
  for (const [id, row] of a) {
    const other = inB.get(id)
    if (other === undefined || other !== ser(row)) n++
  }
  for (const [id] of b) if (!inA.has(id)) n++
  const common = (list: Array<[string, unknown]>, within: Set<string>) =>
    list.map(([id]) => id).filter((id) => within.has(id))
  const ca = common(a, new Set(inB.keys()))
  const cb = common(b, inA)
  if (ca.some((id, i) => id !== cb[i])) n++
  return n
}

// ---- the drift check's schedule -------------------------------------------------

// DriftEveryMs is how often a patched list checks itself against a full
// read, while the window is shown. DriftSlots is how many places the lists
// are staggered over, so they don't all read at once.
export const DriftEveryMs = 5 * 60_000
// DriftSettleMs is how long a drift sample waits for the notice of a change
// that came in during its read before it compares (patchedReads).
export const DriftSettleMs = 3000
export const DriftSlots = 10

export type DriftDeps = {
  setTimeout(fn: () => void, ms: number): unknown
  clearTimeout(id: unknown): void
  hidden(): boolean
}

export function createDriftSchedule(
  deps: DriftDeps = {
    setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
    clearTimeout: (id) =>
      globalThis.clearTimeout(id as ReturnType<typeof setTimeout>),
    hidden: () => appPage()?.hidden === true,
  }
) {
  let registered = 0
  return {
    // register runs fn every DriftEveryMs, first at its slot's turn, and
    // not while the window is hidden. It returns the stop.
    register(fn: () => void): () => void {
      const slot = registered++ % DriftSlots
      let id: unknown
      let stopped = false
      const tick = () => {
        if (stopped) return
        if (!deps.hidden()) fn()
        id = deps.setTimeout(tick, DriftEveryMs)
      }
      id = deps.setTimeout(tick, ((slot + 1) * DriftEveryMs) / DriftSlots)
      return () => {
        stopped = true
        deps.clearTimeout(id)
      }
    },
  }
}

export const driftSchedule = createDriftSchedule()

// ---- the hook ---------------------------------------------------------------------

// RetryMs is the wait before reading everything again after a failed read.
const RetryMs = 4000

// usePatched is useLatest for a list that patches. `follow` is the signals
// it wakes on; `rekey` (a conversation or a board id) makes the next notice
// and a read of its own `all`. Its reload is a full read.
export function usePatched<T, P>(o: {
  name: string
  initial: T
  follow: readonly Follow[]
  rekey?: string
  read: (want: Want) => Promise<P>
  merge: (current: T, patch: P, want: Want) => T
  rows?: (value: T) => Array<[id: string, row: unknown]>
}) {
  const [state, setState] = useState<{
    value: T
    loading: boolean
    error: string | null
  }>({ value: o.initial, loading: true, error: null })
  const mounted = useRef(true)
  const value = useRef(o.initial)
  const shown = useRef<string | null>(null)
  const opts = useRef(o)
  useEffect(() => {
    opts.current = o
  })

  const subs = useRef<Array<{ rekey(): void }>>([])
  const core = useRef<PatchedReads | null>(null)
  core.current ??= patchedReads<T, P>({
    name: o.name,
    read: (want) => opts.current.read(want),
    merge: (cur, patch, want) => opts.current.merge(cur, patch, want),
    rows: o.rows ? (v) => opts.current.rows?.(v) ?? [] : undefined,
    current: () => value.current,
    set(next) {
      value.current = next
      if (!mounted.current) return
      const same = sameAs(shown, next)
      setState((s) =>
        same && !s.loading && s.error === null
          ? s
          : { value: next, loading: false, error: null }
      )
    },
    fail(error) {
      shown.current = null
      if (!mounted.current) return
      setState((s) => ({
        ...s,
        loading: false,
        error: error instanceof Error ? error.message : String(error),
      }))
    },
  })

  const reload = useCallback(() => core.current!.reload(), [])

  useEffect(() => {
    mounted.current = true
    const c = core.current!
    void c.reload()
    const offs = o.follow.map((f) =>
      onChange(f.signal, (n) => c.notice(n), {
        follow: f.buckets,
        focus: f.focus,
      })
    )
    const stop = driftSchedule.register(() => c.driftCheck())
    subs.current = offs
    return () => {
      mounted.current = false
      subs.current = []
      for (const off of offs) off()
      stop()
    }
    // follow is fixed for the life of the list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // A new load key (another conversation or board) reads everything.
  const first = useRef(true)
  useEffect(() => {
    if (first.current) {
      first.current = false
      return
    }
    for (const sub of subs.current) sub.rekey()
    void core.current!.reload()
  }, [o.rekey])

  useEffect(() => {
    if (!state.error) return
    const id = window.setTimeout(() => void core.current!.reload(), RetryMs)
    return () => window.clearTimeout(id)
  }, [state.error])

  return { ...state, reload }
}
