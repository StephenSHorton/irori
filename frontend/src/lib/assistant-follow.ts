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
  | { kind: "reply"; thread: string; to: string }

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
    if (a.doneCheck)
      return {
        key: `ready:${cardId}:${a.id}`,
        text: `${ref} is ready: ${a.by || "its agent"} says it's done.`,
        act,
      }
    return {
      key: `ask:${cardId}:${a.id}`,
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
      act: { kind: "reply", thread: t.id, to: r.by },
    })
  out.push(...askFacts(t.ref, t.id, t.asks))
  for (const c of t.cards) {
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
  return t.closed && t.cards.every((c) => over(c.state))
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
  byId: Map<string, CardDetail | null>
): WorkThread {
  const taker = thread.ask?.taker ? displayName(thread.ask.taker) : ""
  const replies: WorkReply[] = (thread.thread ?? [])
    .filter(
      (it) =>
        it.kind === "comment" && it.who.status !== "person" && it.text.trim()
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
  const made: WorkCard[] = (row?.cards ?? [])
    .filter((c) => c.id !== thread.id)
    .map((rc) => {
      const c = byId.get(rc.id)
      if (!c)
        return {
          id: rc.id,
          ref: cardRefOf(rc),
          title: rc.title,
          state: rc.state,
          working: [],
          asks: [],
        }
      return {
        id: c.id,
        ref: cardRefOf(c),
        title: c.title,
        state: c.state,
        working: busyNames(c),
        asks: asksOf(c),
      }
    })
  return {
    id: thread.id,
    ref: cardRefOf(row ?? thread),
    taker,
    cantStart: row?.cantStart || thread.cantStart || "",
    closed: became
      ? over(thread.state)
      : row?.state === "resolved" ||
        thread.ask?.state === "closed" ||
        over(thread.state),
    outcome:
      thread.ask?.state === "closed" ? (thread.ask.outcome?.kind ?? "") : "",
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
  ids: string[]
): Promise<Map<string, WorkThread | null>> {
  const rows = await threads.listAll()
  const byId = new Map(rows.map((r) => [r.id, r]))
  const madeIds = ids.flatMap(
    (id) => byId.get(id)?.cards.map((c) => c.id) ?? []
  )
  const got = await cardsById([...new Set([...ids, ...madeIds])])
  const out = new Map<string, WorkThread | null>()
  for (const id of ids) {
    const slot = got.get(id)
    if (!slot?.detail) {
      // Gone only when the Board says so; else try again next round.
      if (!byId.has(id) && /no such/i.test(slot?.error ?? "")) out.set(id, null)
      continue
    }
    const made = new Map<string, CardDetail | null>()
    for (const c of byId.get(id)?.cards ?? [])
      made.set(c.id, got.get(c.id)?.detail ?? null)
    out.set(id, workThread(slot.detail, byId.get(id), made))
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
  read: (ids: string[]) => Promise<Map<string, WorkThread | null>>
  listen: (again: () => void) => () => void
  load: () => SeenStore
  save: (s: SeenStore) => void
  // A thread's new lines, and its work as read (null: it is gone).
  say: (id: string, lines: Fact[], work: WorkThread | null) => void
  delayMs?: number
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
        let got: Map<string, WorkThread | null>
        try {
          got = await d.read(ids)
        } catch {
          break
        }
        if (stopped) return
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
          d.say(id, n.say, t)
        }
        forget()
        d.save(store)
      } while (again && !stopped)
    } finally {
      running = false
    }
  }

  const schedule = () => {
    if (stopped || timer !== undefined) return
    timer = setTimeout(() => {
      timer = undefined
      void run()
    }, delay)
  }

  const off = d.listen(schedule)
  void run()

  return {
    follow(id) {
      store[id] = { said: [] }
      d.save(store)
      void run()
    },
    poke: schedule,
    stop() {
      stopped = true
      clearTimeout(timer)
      off()
    },
  }
}

// listenCards is the follower's wake: every card change.
export const listenCards = onCardsChanged
