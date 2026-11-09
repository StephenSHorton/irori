// Card #1146 (docs/design/list-signals.md §5): the one place the window
// listens to the list signals. Every list that wakes on board:changed,
// talk:changed, notifications:changed, inbox:changed, devices:changed,
// boards:changed or assistant:changed subscribes here (window-log.ts only
// counts, so it keeps its own listeners).
//
// One Events.On per signal for the whole window. It keeps the last `seq`
// seen, and hands each subscriber a notice: {all: true} (read everything)
// or {ids} (the rows the signal names, for the buckets the subscriber
// follows). These always turn into {all: true}:
//   - a seq gap or a seq out of order (the page missed an event, or heard
//     one late);
//   - `all` in the payload, and every signal that names no rows;
//   - the page showing again after it was hidden. Today's OnShown merged a
//     hidden stretch into one run and so does this: one `all` per stretch,
//     and only if a notice came meanwhile;
//   - a subscriber's first notice after it mounted, or after rekey() (its
//     load key, a conversation or a board, changed): its mount read is a
//     full read, and Tauri's listen goes live some time after it is called,
//     so the first notice can't be told from the one after a miss.
//
// While the page is hidden seq is still counted (so showing again isn't
// followed by a false gap) and only the notice is held. A subscriber with
// `held: false` hears notices while hidden, as an Events.On did.
//
// Focus is a full read, at most once a minute per subscriber: alt-tabbing
// shouldn't read every list again.
//
// A hook's own reload() isn't a notice and stays a full read; the
// patch hook that turns notices into reads is in patched.ts.

import { Events } from "./host"
import { appPage, type Page } from "./visibility"

export type Signal =
  | "board:changed"
  | "talk:changed"
  | "notifications:changed"
  | "inbox:changed"
  | "devices:changed"
  | "boards:changed"
  | "assistant:changed"

// Buckets are the id lists each signal carries, as design §4 has them.
// Signals with none (devices, boards, assistant) are always `all`: their
// lists are small and read whole.
export const Buckets: Record<Signal, readonly string[]> = {
  // desk is a flag on the wire; here it is the bucket "desk" holding "desk".
  "board:changed": ["cards", "notes", "desk"],
  "talk:changed": ["convs", "calls"],
  "notifications:changed": ["keys", "asks"],
  "inbox:changed": ["deliveries", "cards"],
  "devices:changed": [],
  "boards:changed": [],
  "assistant:changed": [],
}

export type Ids = Record<string, string[]>

// Notice is what a subscriber hears: everything changed, or these ids did
// (in the buckets it follows; a notice naming none of them has empty lists).
export type Notice =
  { signal: Signal; all: true } | { signal: Signal; all: false; ids: Ids }

// Names says whether a notice is for a list that follows `bucket`: it said
// `all`, or it names an id there (and, with `id`, that one). Used by a
// subscriber that reads on its own rather than through patched.ts (card
// #1150). A call id matches with or without its `call:` prefix.
export function names(n: Notice, bucket: string, id?: string): boolean {
  if (n.all) return true
  const ids = n.ids[bucket] ?? []
  if (id === undefined) return ids.length > 0
  const bare = (x: string) => x.replace(/^call:/, "")
  return ids.some((x) => bare(x) === bare(id))
}

export type Options = {
  // follow is the buckets to keep ids for; none keeps none (the list reads
  // everything on every notice).
  follow?: readonly string[]
  // held: false hears notices while the page is hidden (default true).
  held?: boolean
  // focus: a full read when the window gets focus, at most once a minute.
  focus?: boolean
}

// Off unsubscribes. rekey says the subscriber's load key changed: its next
// notice is {all: true}.
export type Off = (() => void) & { rekey(): void }

// FocusEveryMs is the least gap between the focus reads of one subscriber.
export const FocusEveryMs = 60_000

export type Deps = {
  // listen subscribes to a host event; fn gets the payload.
  listen(name: string, fn: (payload: unknown) => void): () => void
  page(): Page | null
  now(): number
  // onFocus runs fn when the window gets focus.
  onFocus(fn: () => void): () => void
}

type Sub = {
  fn: (n: Notice) => void
  follow: readonly string[]
  held: boolean
  first: boolean
  owed: boolean
  focusAt: number | null
  offFocus: () => void
}

type Feed = { subs: Set<Sub>; off: () => void; last: number | null }

export function createChanges(deps: Deps) {
  const feeds = new Map<Signal, Feed>()
  let watching: Page | null = null

  const hidden = () => deps.page()?.hidden === true

  // Shown again: whoever was sent a notice meanwhile reads everything once.
  const onVisible = () => {
    if (hidden()) return
    for (const [signal, feed] of feeds) {
      for (const sub of [...feed.subs]) {
        if (!sub.owed) continue
        sub.owed = false
        sub.first = false
        sub.fn({ signal, all: true })
      }
    }
  }

  const pick = (
    payload: Record<string, unknown>,
    follow: readonly string[]
  ) => {
    const ids: Ids = {}
    for (const bucket of follow) {
      if (bucket === "desk") {
        ids.desk = payload.desk === true ? ["desk"] : []
        continue
      }
      const v = payload[bucket]
      ids[bucket] = Array.isArray(v) ? v.map(String) : []
    }
    return ids
  }

  const heard = (signal: Signal, feed: Feed, payload: unknown) => {
    const p =
      payload && typeof payload === "object"
        ? (payload as Record<string, unknown>)
        : {}
    const seq = typeof p.seq === "number" ? p.seq : null
    let gap = false
    if (seq === null) gap = true
    else if (feed.last === null) feed.last = seq
    else {
      if (seq !== feed.last + 1) gap = true
      feed.last = Math.max(feed.last, seq)
    }
    const everything = gap || p.all === true || Buckets[signal].length === 0
    for (const sub of [...feed.subs]) {
      const all = everything || sub.first
      sub.first = false
      if (sub.held && hidden()) {
        sub.owed = true
        continue
      }
      sub.fn(
        all
          ? { signal, all: true }
          : { signal, all: false, ids: pick(p, sub.follow) }
      )
    }
  }

  function on(
    signal: Signal,
    fn: (n: Notice) => void,
    opts: Options = {}
  ): Off {
    let feed = feeds.get(signal)
    if (!feed) {
      const made: Feed = { subs: new Set(), off: () => {}, last: null }
      made.off = deps.listen(signal, (payload) => heard(signal, made, payload))
      feed = made
      feeds.set(signal, feed)
    }
    if (!watching) {
      watching = deps.page()
      watching?.addEventListener("visibilitychange", onVisible)
    }
    const sub: Sub = {
      fn,
      follow: opts.follow ?? [],
      held: opts.held ?? true,
      first: true,
      owed: false,
      focusAt: null,
      offFocus: () => {},
    }
    if (opts.focus) {
      sub.offFocus = deps.onFocus(() => {
        const t = deps.now()
        if (sub.focusAt !== null && t - sub.focusAt < FocusEveryMs) return
        sub.focusAt = t
        sub.fn({ signal, all: true })
      })
    }
    feed.subs.add(sub)
    const made = feed
    const off = (() => {
      sub.offFocus()
      sub.offFocus = () => {}
      sub.owed = false
      made.subs.delete(sub)
      if (made.subs.size > 0 || feeds.get(signal) !== made) return
      made.off()
      feeds.delete(signal)
      if (feeds.size === 0 && watching) {
        watching.removeEventListener("visibilitychange", onVisible)
        watching = null
      }
    }) as Off
    off.rekey = () => {
      sub.first = true
    }
    return off
  }

  return { on }
}

export const changes = createChanges({
  listen: (name, fn) => Events.On(name, (ev) => fn(ev.data)),
  page: () => appPage(),
  now: () => Date.now(),
  onFocus: (fn) => {
    window.addEventListener("focus", fn)
    return () => window.removeEventListener("focus", fn)
  },
})

// onChange subscribes to a list signal. See the header.
export const onChange = changes.on
