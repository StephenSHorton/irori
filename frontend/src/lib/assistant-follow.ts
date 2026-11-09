// Ask Irori follows the work it posted (card #785). Once a thread is posted, the chat keeps an eye on that thread and the cards made
// from it, and says in the chat only what waits on the person (card
// #1000): it needs them, it is ready for their check, it wants their
// permission, or no agent has started. The rest of the work's moves stay
// on the threads and cards.
//
// It reads the Board as the window does (the thread list and the cards by
// id) on every card change; Haiku is never asked. What it has said for a
// thread is kept on this device (each line by its key), so a restart
// never says it again. A thread stops being followed once it is resolved
// and every card made from it is over, or it leaves the chat.
//
// facts (pure) is what a thread's work says now; the follower says the
// facts it hasn't said yet.

import {
  BlockerState,
  cardsById,
  displayName,
  onCardsChanged,
  TaskState,
  threads,
  type CardBlocker,
  type CardDetail,
  type ThreadSummary,
} from "./cards"
import { onChange } from "./changes"
import { inWails, plainLine } from "./irori"
import { timedRead } from "./window-log"

// ---- what the follower sees -----------------------------------------------------

// WorkAsk is an open item on a card that waits on the person: a question,
// a done check (Done or say what's missing), or a permission prompt.
export type WorkAsk = {
  id: string
  question: string
  // Who asked, by name.
  by: string
  doneCheck: boolean
  permission: boolean
  // Which round it is in: the newest item in its own thread (an agent's
  // ask-back or revision sends it back to the person under the same id),
  // "" for its first. Each round is said once.
  round?: string
}

export type WorkCard = {
  id: string
  // As the chat cites it: "#12", or "mlg#5" for a space's board.
  ref: string
  title: string
  state: string
  // Who is busy on it now, by name.
  working: string[]
  asks: WorkAsk[]
  // The thread changed this card rather than made it (an edit outcome):
  // said once, not followed as the thread's work.
  changed?: boolean
}

export type WorkThread = {
  id: string
  ref: string
  // Who took it ("" while nobody has).
  taker: string
  // Why no agent could start for it ("" when none failed).
  cantStart: string
  // Resolved, answered or closed with nothing (its ask closed). A thread
  // closed as a card isn't: it is that card now (it heads cards).
  closed: boolean
  // How its ask closed: "answered", "none", "edit", "card", or "".
  outcome: string
  // An agent answered it (still open, the person may reply).
  answered?: boolean
  asks: WorkAsk[]
  // The cards made from it, first first. A thread its agent closed as a
  // card (ask_close card) became that card in place, same # and id, and
  // Irori hands it to a task agent: it comes first here, as itself.
  cards: WorkCard[]
}

// Act is what a line lets the person do from the chat: answer a card's
// open item. The chat never does it itself; the person's tap does (as
// #783 settled: nothing posts without a tap).
export type Act = { kind: "answer"; card: string; blocker: string }

// Fact is one thing about the work, known by its key. A silent one is
// remembered but never said (card #1000): its key still counts, as made:
// keys keep a card followed once the thread's row has left the list.
export type Fact = { key: string; text: string; act?: Act; silent?: true }

export function cardRefOf(c: { short: string; tag?: string | null }): string {
  return `${c.tag ?? ""}#${c.short}`
}

export function over(state: string): boolean {
  return (
    state === TaskState.Completed ||
    state === TaskState.Failed ||
    state === TaskState.Canceled
  )
}

// A question passed on in the chat is cut to this many characters (the
// card has it whole).
export const LineClip = 280

function clip(s: string, n = LineClip): string {
  const t = plainLine(s)
  return t.length > n ? `${t.slice(0, n - 1).trimEnd()}…` : t
}

function names(list: string[]): string {
  if (list.length <= 1) return list[0] ?? ""
  return `${list.slice(0, -1).join(", ")} and ${list[list.length - 1]}`
}

function askFacts(ref: string, cardId: string, asks: WorkAsk[]): Fact[] {
  return asks.map((a): Fact => {
    if (a.permission)
      return {
        key: `perm:${cardId}:${a.id}`,
        text: `${ref} is waiting for your permission: ${clip(a.question)}`,
      }
    const act: Act = { kind: "answer", card: cardId, blocker: a.id }
    const r = a.round ? `:${a.round}` : ""
    if (a.doneCheck)
      return {
        key: `ready:${cardId}:${a.id}${r}`,
        text: `${ref} is ready: ${a.by || "its agent"} says it's done.`,
        act,
      }
    return {
      key: `ask:${cardId}:${a.id}${r}`,
      text: `${ref} needs you: ${clip(a.question)}`,
      act,
    }
  })
}

// facts is everything known of a thread's work now, in the order it
// happens. The follower says each key once, unless it is silent.
export function facts(t: WorkThread): Fact[] {
  const out: Fact[] = []
  if (t.taker)
    out.push({
      key: "taken",
      text: `**${t.taker}** took your thread ${t.ref}.`,
      silent: true,
    })
  else if (t.cantStart && !t.closed)
    out.push({
      key: "cant",
      text: `No agent has started on ${t.ref} yet. ${clip(t.cantStart)}`,
    })
  out.push(...askFacts(t.ref, t.id, t.asks))
  for (const c of t.cards) {
    if (c.changed) {
      out.push({
        key: `made:${c.id}`,
        text: `Your thread ${t.ref} changed ${c.ref}.`,
        silent: true,
      })
      continue
    }
    out.push({
      key: `made:${c.id}`,
      text:
        c.id === t.id
          ? `Your thread ${t.ref} is now a card.`
          : `Your thread ${t.ref} became ${c.ref}.`,
      silent: true,
    })
    if (c.working.length && !over(c.state))
      out.push({
        key: `start:${c.id}`,
        text: `${names(c.working)} started on ${c.ref}.`,
        silent: true,
      })
    if (!over(c.state)) out.push(...askFacts(c.ref, c.id, c.asks))
    if (c.state === TaskState.Completed)
      out.push({ key: `done:${c.id}`, text: `${c.ref} is done.`, silent: true })
    else if (c.state === TaskState.Failed)
      out.push({ key: `done:${c.id}`, text: `${c.ref} failed.`, silent: true })
    else if (c.state === TaskState.Canceled)
      out.push({
        key: `done:${c.id}`,
        text: `${c.ref} was canceled.`,
        silent: true,
      })
  }
  if (t.closed && !t.cards.length)
    out.push({
      key: "closed",
      text:
        t.outcome === "answered"
          ? `${t.ref} was answered.`
          : `${t.ref} was resolved.`,
      silent: true,
    })
  return out
}

// finished says nothing more will happen to the work: the thread is
// resolved and every card made from it is over.
export function finished(t: WorkThread): boolean {
  return t.closed && t.cards.every((c) => c.changed || over(c.state))
}

// quiet says nothing is moving: the thread was answered, every card made
// from it is over (there was at least one) and nothing waits on the
// person. It is still read, but less often (see createFollower). An
// answered thread with no card yet isn't quiet: its agent often makes the
// card next.
export function quiet(t: WorkThread): boolean {
  return (
    t.asks.length === 0 &&
    (t.closed || !!t.answered) &&
    t.cards.length > 0 &&
    t.cards.every((c) => c.changed || over(c.state))
  )
}

// ---- from the Board's cards --------------------------------------------------------

// onPerson says an open item waits on this person: not a wait on other
// work, not another member's decision, not an ownership transfer.
function onPerson(b: CardBlocker): boolean {
  if (b.resolved) return false
  if (b.state !== BlockerState.Open && b.state !== BlockerState.Replied)
    return false
  return !b.waitsOn && !b.decider && !b.transfer && !b.goneAgent
}

function asksOf(c: CardDetail): WorkAsk[] {
  return (c.blockers ?? []).filter(onPerson).map((b) => ({
    id: b.id,
    question: b.question,
    by: b.raisedBy ? displayName(b.raisedBy) : "",
    doneCheck: !!b.doneCheck,
    permission: !!b.permission,
    round: b.thread?.length ? b.thread[b.thread.length - 1].id : "",
  }))
}

function busyNames(c: CardDetail): string[] {
  return (c.working ?? []).map(displayName)
}

// workThread reads one followed thread from its card (an ask card), its
// row in the thread list and the cards made from it (by id; an unread
// one stands in from its row).
export function workThread(
  thread: CardDetail,
  row: ThreadSummary | undefined,
  byId: Map<string, CardDetail | null>,
  // Cards already said to be made from it: kept when its row is gone
  // (the thread list keeps only the newest resolved threads).
  known: readonly string[] = []
): WorkThread {
  const taker = thread.ask?.taker ? displayName(thread.ask.taker) : ""
  // Closed as a card: the thread is that card now.
  const became =
    thread.ask?.state === "closed" && thread.ask.outcome?.kind === "card"
  const self: WorkCard[] = became
    ? [
        {
          id: thread.id,
          ref: cardRefOf(row ?? thread),
          title: thread.title,
          state: thread.state,
          working: busyNames(thread),
          asks: asksOf(thread),
        },
      ]
    : []
  const listed = (row?.cards ?? []).filter((c) => c.id !== thread.id)
  const ids = [
    ...listed.map((c) => c.id),
    ...known.filter(
      (id) => id !== thread.id && !listed.some((c) => c.id === id)
    ),
  ]
  const made: WorkCard[] = []
  for (const id of ids) {
    const c = byId.get(id)
    const rc = listed.find((x) => x.id === id)
    if (c)
      made.push({
        id: c.id,
        ref: cardRefOf(c),
        title: c.title,
        state: c.state,
        working: busyNames(c),
        asks: asksOf(c),
        // A card the thread changed was made from something else.
        ...(c.fromThread && c.fromThread.id !== thread.id
          ? { changed: true }
          : {}),
      })
    else if (rc)
      made.push({
        id: rc.id,
        ref: cardRefOf(rc),
        title: rc.title,
        state: rc.state,
        working: [],
        asks: [],
      })
  }
  return {
    id: thread.id,
    ref: cardRefOf(row ?? thread),
    taker,
    cantStart:
      row?.cantStart ||
      thread.cantStart ||
      row?.noAgent ||
      thread.noAgent ||
      "",
    closed: became
      ? over(thread.state)
      : row?.state === "resolved" ||
        thread.ask?.state === "closed" ||
        over(thread.state),
    outcome:
      thread.ask?.state === "closed" ? (thread.ask.outcome?.kind ?? "") : "",
    answered: row?.state === "answered",
    // As a card, its items are the card's (said once, there).
    asks: became ? [] : asksOf(thread),
    cards: [...self, ...made],
  }
}

// readWork reads the followed threads: each id's work, null when the
// thread is gone (deleted, or no longer on this device). Throws when the
// Board couldn't be read at all, so the round is tried again later.
export function readWork(
  ids: string[],
  known: ReadonlyMap<string, readonly string[]> = new Map()
): Promise<Map<string, WorkThread | null>> {
  return timedRead("follower", () => readWorkRound(ids, known))
}

async function readWorkRound(
  ids: string[],
  known: ReadonlyMap<string, readonly string[]>
): Promise<Map<string, WorkThread | null>> {
  const { rows } = await threads.byId(ids)
  const byId = new Map(rows.map((r) => [r.id, r]))
  const madeIds = ids.flatMap((id) => [
    ...(byId.get(id)?.cards.map((c) => c.id) ?? []),
    ...(known.get(id) ?? []),
  ])
  const got = await cardsById([...new Set([...ids, ...madeIds])])
  const out = new Map<string, WorkThread | null>()
  for (const id of ids) {
    const slot = got.get(id)
    if (!slot?.detail) {
      // Gone only when the Board says so; else try again next round.
      if (!byId.has(id) && /no such/i.test(slot?.error ?? "")) out.set(id, null)
      continue
    }
    const row = byId.get(id)
    const had = known.get(id) ?? []
    const made = new Map<string, CardDetail | null>()
    for (const c of [...(row?.cards.map((x) => x.id) ?? []), ...had])
      made.set(c, got.get(c)?.detail ?? null)
    // A card it made that can't be read now, with no row to stand in:
    // skip the thread this round rather than lose that card.
    const listed = new Set(row?.cards.map((x) => x.id) ?? [])
    if (had.some((c) => c !== id && !listed.has(c) && !made.get(c))) continue
    out.set(id, workThread(slot.detail, row, made, had))
  }
  return out
}

// ---- what it has said ----------------------------------------------------------------

// Seen is what the chat has said of one thread: its facts' keys (null:
// none known yet, so the first read says nothing and only remembers;
// that is a thread posted by a build before this one), and whether it is
// over.
export type Seen = { said: string[] | null; done?: boolean }
export type SeenStore = Record<string, Seen>

export function parseSeen(raw: string | null): SeenStore {
  if (!raw) return {}
  let v: unknown
  try {
    v = JSON.parse(raw)
  } catch {
    return {}
  }
  if (!v || typeof v !== "object" || Array.isArray(v)) return {}
  const out: SeenStore = {}
  for (const [id, x] of Object.entries(v as Record<string, unknown>)) {
    if (!id || !x || typeof x !== "object") continue
    const { said, done } = x as Record<string, unknown>
    out[id] = {
      said: Array.isArray(said)
        ? said.filter((k): k is string => typeof k === "string")
        : null,
      ...(done === true ? { done: true } : {}),
    }
  }
  return out
}

// news is a read's facts the chat hasn't said, and what it has said once
// it says them. Silent facts count as said without being said.
export function news(
  seen: Seen | undefined,
  t: WorkThread
): { say: Fact[]; seen: Seen } {
  const all = facts(t)
  const done = finished(t) || undefined
  if (!seen || seen.said === null)
    return {
      say: [],
      seen: { said: all.map((f) => f.key), ...(done ? { done } : {}) },
    }
  const had = new Set(seen.said)
  const fresh = all.filter((f) => !had.has(f.key))
  return {
    say: fresh.filter((f) => !f.silent),
    seen: {
      said: [...seen.said, ...fresh.map((f) => f.key)],
      ...(done ? { done } : {}),
    },
  }
}

// isSilentKey says a fact is one the chat never says (taken, made, start,
// done, closed): only this device remembers it. The lines it does say are
// follow-up rows, and the rows are what remembers those (card 4).
export function isSilentKey(k: string): boolean {
  return !(k === "cant" || /^(perm|ready|ask):/.test(k))
}

// silentOnly is a store cut to what only this device remembers.
export function silentOnly(st: SeenStore): SeenStore {
  const out: SeenStore = {}
  for (const [id, x] of Object.entries(st))
    out[id] = {
      said: x.said === null ? null : x.said.filter(isSilentKey),
      ...(x.done ? { done: true } : {}),
    }
  return out
}

// ---- the follower ------------------------------------------------------------------

export type FollowDeps = {
  // The threads the chat posted, as its messages hold them now.
  ids: () => string[]
  // known: each thread's cards already said to be made from it.
  read: (
    ids: string[],
    known: ReadonlyMap<string, readonly string[]>
  ) => Promise<Map<string, WorkThread | null>>
  // listen calls `again` on a change. With ids it names the rows that
  // changed; the follower skips a round that touches none of its own (the
  // threads, the cards made from them). Without ids: everything may have.
  listen: (again: (ids?: readonly string[]) => void) => () => void
  load: () => SeenStore
  // The facts of a thread already said as rows, on any device. A thread
  // this device has no entry for starts from them, not from nothing.
  shown?: (id: string) => string[]
  save: (s: SeenStore) => void
  // A thread's new lines, and its work as read (null: it is gone).
  say: (id: string, lines: Fact[], work: WorkThread | null) => void
  delayMs?: number
  // How long a card change waits while every followed thread is quiet
  // (answered or resolved, its cards over, nothing on the person).
  quietMs?: number
  // The least time between one read's start and the next (card #1136): a
  // busy board changes many times a minute, and each change used to read
  // the threads and their cards again. Default 0: no gap.
  minGapMs?: number
}

export type Follower = {
  // A thread was just posted: follow it from nothing said.
  follow: (id: string) => void
  // Read again soon (the chat's messages changed).
  poke: () => void
  stop: () => void
}

export function createFollower(d: FollowDeps): Follower {
  const store = d.load()
  let running = false
  let again = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let stopped = false
  const delay = d.delayMs ?? 400
  const quietDelay = d.quietMs ?? 60_000
  const minGap = d.minGapMs ?? 0
  // When the last read started.
  let lastStart = -Infinity
  // Every thread read last round was quiet: card changes wait longer.
  let allQuiet = false
  let timerQuiet = false

  // The cards of the threads as last read: with the threads and the made
  // cards said so far, the ids a change must touch to matter.
  let lastCards = new Set<string>()
  const touches = (ids: readonly string[]): boolean => {
    const mine = new Set<string>([...d.ids(), ...lastCards])
    for (const e of Object.values(store))
      for (const k of e.said ?? [])
        if (k.startsWith("made:")) mine.add(k.slice("made:".length))
    return ids.some((id) => mine.has(id))
  }

  const active = () => {
    const ids = [...new Set(d.ids())]
    return ids.filter((id) => !store[id]?.done)
  }

  // forget drops threads that left the chat (New chat, or past the
  // history's cap); it says whether any went.
  const forget = (): boolean => {
    const ids = new Set(d.ids())
    let gone = false
    for (const id of Object.keys(store))
      if (!ids.has(id)) {
        delete store[id]
        gone = true
      }
    return gone
  }

  async function run() {
    if (running) {
      again = true
      return
    }
    running = true
    lastStart = Date.now()
    try {
      do {
        again = false
        const ids = active()
        if (!ids.length) {
          if (forget()) d.save(store)
          break
        }
        const known = new Map<string, string[]>()
        for (const id of ids) {
          const made = (store[id]?.said ?? [])
            .filter((k) => k.startsWith("made:"))
            .map((k) => k.slice("made:".length))
          if (made.length) known.set(id, made)
        }
        let got: Map<string, WorkThread | null>
        try {
          got = await d.read(ids, known)
        } catch {
          break
        }
        if (stopped) return
        let calm = true
        lastCards = new Set(
          [...got.values()].flatMap((t) => t?.cards.map((c) => c.id) ?? [])
        )
        for (const id of ids) {
          if (!got.has(id)) continue
          const t = got.get(id) ?? null
          if (!t) {
            store[id] = { said: store[id]?.said ?? [], done: true }
            d.say(id, [], null)
            continue
          }
          const rows = d.shown?.(id)
          const base = store[id]
          const n = news(
            rows
              ? {
                  ...(base ?? { said: [] }),
                  said: [...new Set([...(base?.said ?? []), ...rows])],
                }
              : base,
            t
          )
          store[id] = n.seen
          if (!n.seen.done && !quiet(t)) calm = false
          d.say(id, n.say, t)
        }
        allQuiet = calm
        forget()
        d.save(store)
        // Asked again meanwhile: after the gap, not at once.
        if (again && minGap > 0) break
      } while (again && !stopped)
    } finally {
      running = false
    }
    if (again && !stopped && minGap > 0) schedule(true)
  }

  const schedule = (soon: boolean) => {
    if (stopped) return
    const slow = !soon && allQuiet
    if (timer !== undefined) {
      // A quiet wait gives way to a sooner one.
      if (!timerQuiet || slow) return
      clearTimeout(timer)
    }
    timerQuiet = slow
    const wait = Math.max(
      slow ? quietDelay : delay,
      lastStart + minGap - Date.now()
    )
    timer = setTimeout(() => {
      timer = undefined
      void run()
    }, wait)
  }

  const off = d.listen((changed) => {
    if (changed && !touches(changed)) return
    schedule(false)
  })
  void run()

  return {
    follow(id) {
      store[id] = { said: [] }
      d.save(store)
      void run()
    },
    poke: () => schedule(true),
    stop() {
      stopped = true
      clearTimeout(timer)
      off()
    },
  }
}

// listenCards is the follower's wake (card #1149): the cards a change names
// (board:changed.cards includes thread ids), and every device change. The
// preview's changes name none.
export function listenCards(again: (ids?: readonly string[]) => void) {
  if (!inWails()) return onCardsChanged(() => again())
  const offs = [
    onChange("board:changed", (n) => again(n.all ? undefined : n.ids.cards), {
      follow: ["cards"],
    }),
    onChange("devices:changed", () => again()),
  ]
  return () => {
    for (const off of offs) off()
  }
}
