// Card #1152 (docs/design/list-signals.md §6.6): what the small lists share.
// These lists have no by-id reads. They only decide, from a notice
// (changes.ts), whether to read again: desk, notes, the idle line, #ref
// chips, the Done tab, devices, boards, inbox and held, repo names. The
// decisions are plain functions here, so each list's trigger is in a test.

import { errorText, latestReads, sameAs, type Answer } from "./talk"
import type { Notice } from "./changes"
import { noteRead, type ReadReport } from "./window-log"

// Follows are the id buckets each small list keeps from its signal (the
// `follow` of its onChange).
export const Follows = {
  desk: ["desk", "cards"],
  notes: ["notes", "cards"],
  cards: ["cards"],
  deliveries: ["deliveries"],
} as const

// ---- reading a notice --------------------------------------------------------

// idsOf is the ids a notice names in a bucket ([] for `all`, which names
// none and is asked with isAll).
export function idsOf(n: Notice, bucket: string): string[] {
  return n.all ? [] : (n.ids[bucket] ?? [])
}

export const isAll = (n: Notice) => n.all

// hits says whether a notice names any id of `bucket` that `has` holds.
export function hits(
  n: Notice,
  bucket: string,
  has: (id: string) => boolean
): boolean {
  return idsOf(n, bucket).some(has)
}

// ---- a store shared by every reader ----------------------------------------------

export type StoreState<T> = {
  value: T
  // True until the first read ends, either way; a reload never sets it.
  loading: boolean
  error: string | null
}

// createStore is one list for the whole window. The first reader starts it
// (a read, and `start` for what wakes it); a later reader reads nothing. The
// last reader stops it. reload() is a full read, the one callers await after
// an action; `reload` handed to start() is the same read for a trigger.
export function createStore<T>(o: {
  name: string
  initial: T
  read(): Promise<T>
  // start follows whatever wakes the list and returns the stop. It calls
  // `reload` for each wake.
  start(reload: () => void): () => void
  now?: () => number
  noteRead?: (r: ReadReport) => void
}) {
  let state: StoreState<T> = { value: o.initial, loading: true, error: null }
  const readers = new Set<() => void>()
  let stop: (() => void) | null = null
  const shown: { current: string | null } = { current: null }

  const publish = (next: StoreState<T>) => {
    state = next
    for (const fn of [...readers]) fn()
  }

  // Made on first use: this module and talk.ts import each other.
  let readsOf: ReturnType<typeof latestReads<T>> | null = null
  const reads = (load: () => Promise<T>) => {
    readsOf ??= latestReads<T>(
      (answer: Answer<T>) => {
        if ("error" in answer) {
          shown.current = null
          publish({ ...state, loading: false, error: errorText(answer.error) })
          return
        }
        const same = sameAs(shown, answer.value)
        if (same && !state.loading && state.error === null) return
        publish({ value: answer.value, loading: false, error: null })
      },
      o.now ?? Date.now,
      o.noteRead ?? ((r) => noteRead(o.name, r))
    )
    return readsOf(load)
  }

  // One stable load, so a wake during a read makes one follow-up and never
  // a read beside it.
  const load = () => o.read()

  const reload = () => reads(load)

  return {
    reload,
    subscribe(reader: () => void): () => void {
      readers.add(reader)
      if (!stop) {
        stop = o.start(() => void reload())
        void reload()
      }
      return () => {
        readers.delete(reader)
        // After the commit: a page swapped for another (or React's
        // strict-mode remount) keeps the store.
        queueMicrotask(() => {
          if (readers.size > 0 || !stop) return
          stop()
          stop = null
        })
      }
    },
    snapshot: () => state,
    readers: () => readers.size,
  }
}

// ---- at most one read in a while ---------------------------------------------------

export type Timers = {
  setTimeout(fn: () => void, ms: number): unknown
  clearTimeout(id: unknown): void
  now(): number
}

const realTimers: Timers = {
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (id) =>
    globalThis.clearTimeout(id as ReturnType<typeof setTimeout>),
  now: () => Date.now(),
}

// IdleEveryMs is the least gap between two reads of Home's idle line.
export const IdleEveryMs = 5000

// trailing runs fn at most once every `ms`: the first call runs at once, a
// call inside the gap waits for its end and runs once (the trailing run),
// however many came. cancel() drops a waiting run.
export function trailing(
  fn: () => void,
  ms: number,
  t: Timers = realTimers
): { call(): void; cancel(): void } {
  let last: number | null = null
  let timer: unknown = null
  const run = () => {
    timer = null
    last = t.now()
    fn()
  }
  return {
    call() {
      if (timer !== null) return
      const wait = last === null ? 0 : last + ms - t.now()
      if (wait <= 0) run()
      else timer = t.setTimeout(run, wait)
    },
    cancel() {
      if (timer !== null) t.clearTimeout(timer)
      timer = null
    },
  }
}

// ---- the Done tab --------------------------------------------------------------------

// liveCards remembers the cards the grid last showed. A card the grid had
// and no longer has was closed (or cleared, or its board went): the Done
// tab reads then, not on every change to a live card. Until the grid has
// answered once, `known()` is false and the tab can't tell.
export function createLiveCards() {
  let live: Set<string> | null = null
  const left = new Set<(ids: string[]) => void>()
  return {
    known: () => live !== null,
    has: (id: string) => live?.has(id) ?? false,
    // note takes the grid's cards after a read.
    note(ids: Iterable<string>) {
      const next = new Set(ids)
      const gone = live ? [...live].filter((id) => !next.has(id)) : []
      live = next
      if (gone.length) for (const fn of [...left]) fn(gone)
    },
    onLeft(fn: (ids: string[]) => void): () => void {
      left.add(fn)
      return () => left.delete(fn)
    },
    reset() {
      live = null
    },
  }
}

export const liveCards = createLiveCards()

// doneWakes says whether the Done tab reads again for a notice: it names a
// card on the page, or (the grid unable to say which closed) any card. A
// card the grid shows is not on the Done tab, so a change to it waits for
// liveCards.onLeft.
export function doneWakes(
  n: Notice,
  onPage: ReadonlySet<string>,
  live: { known(): boolean; has(id: string): boolean }
): boolean {
  if (n.all) return true
  const ids = idsOf(n, "cards")
  if (ids.some((id) => onPage.has(id))) return true
  return !live.known() && ids.length > 0
}

// ---- #ref chips ------------------------------------------------------------------------

// A chip's entry: the card its ref resolved to ("" none), and whether the
// answer is in.
export type RefEntry = { id: string; settled: boolean }

// refsToDrop is which cached refs a notice stales: those that resolved to a
// card the notice names, plus every ref that resolved to nothing or is
// still pending (a new card or a renumber always comes with ids, and null
// entries are few). `all` drops every one. Empty: the notice is not for the
// chips.
export function refsToDrop(
  n: Notice,
  entries: ReadonlyMap<string, RefEntry>
): string[] {
  if (n.all) return [...entries.keys()]
  const named = new Set(idsOf(n, "cards"))
  if (named.size === 0) return []
  const out: string[] = []
  for (const [ref, e] of entries)
    if (!e.settled || e.id === "" || named.has(e.id)) out.push(ref)
  return out
}

// ---- notes -------------------------------------------------------------------------------

// notesWake says whether the notes list reads again: a note changed, a card
// a note names (its source, for "Still true?") changed, or `all`.
export function notesWake(n: Notice, taskIds: ReadonlySet<string>): boolean {
  if (n.all) return true
  if (idsOf(n, "notes").length > 0) return true
  return hits(n, "cards", (id) => taskIds.has(id))
}

// ---- the desk ------------------------------------------------------------------------------

// deskWakes says whether the desks read again: a desk row changed, a card
// in a desk's verdicts (a PR's author card) changed, or `all`.
export function deskWakes(n: Notice, verdictCards: ReadonlySet<string>) {
  if (n.all) return true
  if (idsOf(n, "desk").length > 0) return true
  return hits(n, "cards", (id) => verdictCards.has(id))
}

// ---- devices -------------------------------------------------------------------------------

// deviceStateSig is what a roster:changed payload says of the devices: each
// device with an agent in it, and its state. A roster change that leaves it
// as it was is no news for the devices list.
export function deviceStateSig(payload: unknown): string {
  const agents = (payload as { agents?: unknown } | null)?.agents
  if (!Array.isArray(agents)) return "?"
  const seen = new Set<string>()
  for (const a of agents as Array<Record<string, unknown>>) {
    seen.add(`${String(a?.deviceId ?? "")}=${String(a?.deviceState ?? "")}`)
  }
  return [...seen].sort().join(",")
}

// rosterWatch tells, for each roster:changed payload, whether the device
// state changed since the last one (the first always counts).
export function rosterWatch() {
  let last: string | null = null
  return (payload: unknown): boolean => {
    const sig = deviceStateSig(payload)
    const changed = last === null || sig !== last || sig === "?"
    last = sig
    return changed
  }
}

// ---- inbox and held ------------------------------------------------------------------------

// InboxKeep is how many deliveries the inbox list keeps (inbox_list(200)).
export const InboxKeep = 200

export type Row = { id: string }

// upsertInbox puts the rows read for `ids` into the list: a delivery that
// came back replaces its row (or joins at the top), one that didn't come
// back is dropped, and rows past `keep` are dropped unless `keepRow` holds
// them (held ones stay). `order` ranks two rows, the first the newer.
export function upsertRows<T extends Row>(
  current: readonly T[],
  ids: readonly string[],
  got: ReadonlyMap<string, T>,
  order: (a: T, b: T) => number,
  keep: number,
  keepRow: (r: T) => boolean = () => false
): T[] {
  const named = new Set(ids)
  const rest = current.filter((r) => !named.has(r.id))
  const next = [...rest, ...got.values()].sort(order)
  const out: T[] = []
  let n = 0
  for (const r of next) {
    if (n < keep || keepRow(r)) out.push(r)
    n++
  }
  return out
}

// heldUpsert is the held list after a patch: a row whose new read is not
// held, or failed, is gone.
export function heldUpsert<T extends Row>(
  current: readonly T[],
  ids: readonly string[],
  got: ReadonlyMap<string, T | null>,
  isHeld: (r: T) => boolean,
  order: (a: T, b: T) => number
): T[] {
  const named = new Set(ids)
  const rest = current.filter((r) => !named.has(r.id))
  const add: T[] = []
  for (const id of ids) {
    const r = got.get(id)
    if (r && isHeld(r)) add.push(r)
  }
  return [...rest, ...add].sort(order)
}
