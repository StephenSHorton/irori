// Ask Irori follows the work it posted (card #785). Once the person taps
// Post thread, the chat keeps an eye on that thread and the cards made
// from it, and says in the chat when the work moves: an agent took the
// thread or replied on it, a card was made from it, an agent started on
// that card, it needs the person, it is ready, it is done.
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
import { plainLine } from "./irori"

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

export type WorkReply = { id: string; by: string; text: string }

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
  // The agents' replies on it, oldest first.
  replies: WorkReply[]
  asks: WorkAsk[]
  // The cards made from it, first first. A thread its agent closed as a
  // card (ask_close card) became that card in place, same # and id, and
  // Irori hands it to a task agent: it comes first here, as itself.
  cards: WorkCard[]
}

// Act is what a line lets the person do from the chat: answer a card's
// open item, or reply on the thread. The chat never does either itself;
// the person's tap does (as #783 settled: nothing posts without a tap).
export type Act =
  | { kind: "answer"; card: string; blocker: string }
  | { kind: "reply"; thread: string; to: string; ref?: string }

// Fact is one line the chat says about the work, known by its key.
export type Fact = { key: string; text: string; act?: Act }

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

// A reply passed on in the chat is cut to this many characters (the thread
// has it whole).
export const ReplyClip = 280

function clip(s: string, n = ReplyClip): string {
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

// facts is everything a thread's work says now, in the order it happens.
// The follower says each key once.
export function facts(t: WorkThread): Fact[] {
  const out: Fact[] = []
  if (t.taker)
    out.push({
      key: "taken",
      text: `**${t.taker}** took your thread ${t.ref}.`,
    })
  else if (t.cantStart && !t.closed)
    out.push({
      key: "cant",
      text: `No agent could start for ${t.ref} yet: ${clip(t.cantStart)}`,
    })
  for (const r of t.replies)
    out.push({
      key: `reply:${r.id}`,
      text: `${r.by} replied on ${t.ref}: ${clip(r.text)}`,
      act: { kind: "reply", thread: t.id, to: r.by, ref: t.ref },
    })
  out.push(...askFacts(t.ref, t.id, t.asks))
  for (const c of t.cards) {
    if (c.changed) {
      out.push({
        key: `made:${c.id}`,
        text: `Your thread ${t.ref} changed ${c.ref}.`,
      })
      continue
    }
    out.push({
      key: `made:${c.id}`,
      text:
        c.id === t.id
          ? `Your thread ${t.ref} is now a card.`
          : `Your thread ${t.ref} became ${c.ref}.`,
    })
    if (c.working.length && !over(c.state))
      out.push({
        key: `start:${c.id}`,
        text: `${names(c.working)} started on ${c.ref}.`,
      })
    if (!over(c.state)) out.push(...askFacts(c.ref, c.id, c.asks))
    if (c.state === TaskState.Completed)
      out.push({ key: `done:${c.id}`, text: `${c.ref} is done.` })
    else if (c.state === TaskState.Failed)
      out.push({ key: `done:${c.id}`, text: `${c.ref} failed.` })
    else if (c.state === TaskState.Canceled)
      out.push({ key: `done:${c.id}`, text: `${c.ref} was canceled.` })
  }
  if (t.closed && !t.cards.length)
    out.push({
      key: "closed",
      text:
        t.outcome === "answered"
          ? `${t.ref} was answered.`
          : `${t.ref} was resolved.`,
    })
  return out
}

// finished says nothing more will happen to the work: the thread is
// resolved and every card made from it is over.
export function finished(t: WorkThread): boolean {
  return t.closed && t.cards.every((c) => c.changed || over(c.state))
}

// quiet says nothing is moving: the thread is answered or closed, every
// card made from it is over and nothing waits on the person. A later
// reply is still said, but read less often (see createFollower).
export function quiet(t: WorkThread): boolean {
  return (
    t.asks.length === 0 &&
    (t.closed || !!t.answered) &&
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
  const replies: WorkReply[] = (thread.thread ?? [])
    .filter(
      (it) =>
        it.kind === "comment" &&
        it.who.status !== "person" &&
        it.who.status !== "irori" &&
        it.text.trim()
    )
    .map((it) => ({ id: it.id, by: displayName(it.who), text: it.text }))
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
    replies,
    // As a card, its items are the card's (said once, there).
    asks: became ? [] : asksOf(thread),
    cards: [...self, ...made],
  }
}

// readWork reads the followed threads: each id's work, null when the
// thread is gone (deleted, or no longer on this device). Throws when the
// Board couldn't be read at all, so the round is tried again later.
export async function readWork(
  ids: string[],
  known: ReadonlyMap<string, readonly string[]> = new Map()
): Promise<Map<string, WorkThread | null>> {
  const rows = await threads.listAll()
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
// it says them.
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
  const say = all.filter((f) => !had.has(f.key))
  return {
    say,
    seen: {
      said: [...seen.said, ...say.map((f) => f.key)],
      ...(done ? { done } : {}),
    },
  }
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
  listen: (again: () => void) => () => void
  load: () => SeenStore
  save: (s: SeenStore) => void
  // A thread's new lines, and its work as read (null: it is gone).
  say: (id: string, lines: Fact[], work: WorkThread | null) => void
  delayMs?: number
  // How long a card change waits while every followed thread is quiet
  // (answered or resolved, its cards over, nothing on the person).
  quietMs?: number
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
  // Every thread read last round was quiet: card changes wait longer.
  let allQuiet = false
  let timerQuiet = false

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
        for (const id of ids) {
          if (!got.has(id)) continue
          const t = got.get(id) ?? null
          if (!t) {
            store[id] = { said: store[id]?.said ?? [], done: true }
            d.say(id, [], null)
            continue
          }
          const n = news(store[id], t)
          store[id] = n.seen
          if (!n.seen.done && !quiet(t)) calm = false
          d.say(id, n.say, t)
        }
        allQuiet = calm
        forget()
        d.save(store)
      } while (again && !stopped)
    } finally {
      running = false
    }
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
    timer = setTimeout(
      () => {
        timer = undefined
        void run()
      },
      slow ? quietDelay : delay
    )
  }

  const off = d.listen(() => schedule(false))
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

// listenCards is the follower's wake: every card change.
export const listenCards = onCardsChanged
