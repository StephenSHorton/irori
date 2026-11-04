// Home's "Needs you" list and its progress rows (card #261, #259's
// organization A): what waits on the person comes first, answerable on
// its row, and every other card is one compact row in one of four
// states. Pure, so the order and the words are tested without a window.

import { NotAnswering } from "./agent-counts"
import { spawnLine } from "./spawn-line"
import {
  askLine,
  AskState,
  busy,
  displayName,
  Holding,
  idleUnnamed,
  openAsk,
  over,
  overLabel,
  runningJobNow,
  TaskState,
  workLabel,
  type CardSummary,
  type ReviewPr,
} from "./cards"
import { parseTime, plainLine } from "./irori"
import { State, type Delivery } from "./talk"
import { waitLine, waitName } from "./wait-name"

// RowState is what a person acts on, and Home groups its rows by it in
// this order (card #487; finding 05, card #305):
// - Needs you: a decision, a done check, a question, a review.
// - Stuck: nobody is on it and nothing will put someone on it by itself:
//   nobody took it (never handed out, offered or assigned and not picked
//   up, an ask nobody took), its holder went away, isn't answering or is
//   offline, its agent can't start, or the agent it waits on is gone.
// - Working: someone is on it: busy, running a job it started, between
//   turns, an agent starting for it, or an ask someone took.
// - Waiting: on something named that ends by itself: a lease, another
//   card, an agent, a review, a subtask, the agent reading your answer,
//   a device at its cap, a pause.
// - Idle: held, its holders idle a while with nothing named (card #305).
// - Done.
export type RowState = "needs" | "stuck" | "working" | "waiting" | "idle" | "done"

// RowGroups is Home's group order.
export const RowGroups: readonly RowState[] = ["needs", "stuck", "working", "waiting", "idle", "done"]

export const rowStateLabel: Record<RowState, string> = {
  needs: "Needs you",
  stuck: "Stuck",
  working: "Working",
  waiting: "Waiting",
  idle: "Idle",
  done: "Done",
}

// IDLE_LINE is the row's and the card's words for idleUnnamed (card #305).
export const IDLE_LINE = "Idle, no reason given"

// needsYou says the card waits on the person: an open blocker or done
// check, or its question (input required).
export function needsYou(c: CardSummary): boolean {
  if (over(c)) return false
  return c.openBlockers > 0 || !!c.ready || c.state === TaskState.InputRequired
}

// RowLine is a row's state, the line under its title, and whether an
// agent is busy on it right now (the row's spinner).
export type RowLine = { state: RowState; line: string; spin?: boolean }

function cap(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s
}

// away says a holder isn't there to carry the card: gone, not live,
// not answering its wakes (#110) or on an offline device.
function away(w: CardSummary["working"][number]): boolean {
  return w.status === "gone" || w.status === NotAnswering || !w.live || !!w.offline
}

function awayLine(w: CardSummary["working"][number], h: string): string {
  const name = w.key ? displayName(w) : "Its agent"
  if (h === Holding.HoldSilent || w.status === NotAnswering) return `${name} isn't answering`
  if (h === Holding.HoldOffline || w.offline) return `${name} is on an offline device`
  return `${name} ${w.spawned ? "finished" : "went away"}`
}

// Row is a RowLine and whether nobody took the card (for nobodyTook).
type Row = RowLine & { nobody?: boolean }

function row(c: CardSummary, now: number): Row {
  if (over(c)) return { state: "done", line: overLabel(c) }
  if (needsYou(c)) {
    if (c.ready && c.openBlockers <= 1)
      return { state: "needs", line: "Says it's done: is it?" }
    if (c.openBlockers > 1)
      return { state: "needs", line: `${c.openBlockers} decisions` }
    return { state: "needs", line: plainLine(c.blockerQuestion) || "A question for you" }
  }
  // The card's holders: a holder or a step owner, not an assignee who
  // hasn't taken it up.
  const held = c.working.filter((w) => !w.assigned)
  if (openAsk(c) && c.ask) {
    const line = askLine(c.ask)
    if (c.ask.state === AskState.Waiting) return { state: "waiting", line }
    if (c.ask.state !== AskState.Taken) return { state: "stuck", line, nobody: true }
    // Taken: Working while its taker is there.
    const gone = held.length > 0 && held.every(away)
    return gone ? { state: "stuck", line: awayLine(held[0], c.holder.state) } : { state: "working", line }
  }
  if (c.paused) return { state: "waiting", line: "Paused" }
  // Card #347: the person's review of the PR it's on, or what its wait
  // on another card comes down to for them.
  if (c.reviewPr) return { state: "needs", line: `Review PR #${c.reviewPr.number}` }
  if (c.state === TaskState.InReview) return { state: "waiting", line: "In review" }
  if (c.subtaskBlocked) return { state: "waiting", line: "A subtask needs a call" }
  if (c.waitingOn && c.waitYou) return { state: "needs", line: `Waits on ${c.waitYou}` }
  if (c.waitingOn && c.waitGone)
    return { state: "stuck", line: `Waiting on ${waitName(c) || "an agent"}, who is gone` }
  if (c.waitingOn) return { state: "waiting", line: waitLine(c) }
  if (c.waiting > 0) return { state: "waiting", line: "Your answer is with the agent" }
  const who = held.map(displayName).slice(0, 2).join(", ")
  if (busy(c)) return { state: "working", line: who ? `${who} working` : "Working", spin: true }
  // Card #431: between turns, but a background job it started still runs.
  const job = runningJobNow(c)
  if (job.length)
    return {
      state: "working",
      line: `${job.map(displayName).slice(0, 2).join(", ")} running a background job`,
      spin: true,
    }
  if (!held.length) {
    // Card #448: an agent Irori is starting (Working), a device at its cap
    // (Waiting), or why one can't start (Stuck), instead of "nobody".
    const sp = spawnLine(c)
    if (sp) {
      const state: RowState = c.starting ? "working" : c.cantStart ? "stuck" : "waiting"
      return { state, line: cap(sp.text) }
    }
    // A parent is never handed out: its subtasks carry it.
    if ((c.subtasks ?? 0) > 0) return { state: "waiting", line: "Waiting on its subtasks" }
    if (c.working.length) {
      const w = workLabel(c)
      return { state: "stuck", line: w === "offered" ? "Offered, not taken up yet" : "Not picked up", nobody: true }
    }
    if (c.returned)
      return {
        state: "stuck",
        line: c.returned.spawned
          ? `Nobody on it: ${displayName(c.returned)} finished`
          : `Nobody on it: ${displayName(c.returned)} went away`,
      }
    return { state: "stuck", line: "Nobody has it yet", nobody: true }
  }
  // Every holder went away, isn't answering or is offline: nobody is on
  // it, whatever the card says (the live roster can say so first). One
  // holder still there carries it.
  if (held.every(away)) return { state: "stuck", line: awayLine(held[0], c.holder.state) }
  if (idleUnnamed(c, now)) return { state: "idle", line: who ? `${IDLE_LINE} (${who})` : IDLE_LINE }
  const there = held.filter((w) => !away(w)).map(displayName).slice(0, 2).join(", ")
  return { state: "working", line: there ? `${there}, between turns` : "Held, between turns" }
}

// rowState is the card's state and the line under its title: what it
// waits on, who is on it, or how it ended. The words are the ones the
// card's own page uses.
export function rowState(c: CardSummary, now: number = Date.now()): RowLine {
  const r: Row = { ...row(c, now) }
  delete r.nobody
  return r
}

// groupRows splits rows into Home's groups, in RowGroups order, each in
// the order given; empty groups are left out.
export function groupRows<T>(
  rows: readonly T[],
  stateOf: (r: T) => RowState
): { state: RowState; rows: T[] }[] {
  const by = new Map<RowState, T[]>()
  for (const r of rows) {
    const s = stateOf(r)
    const g = by.get(s)
    if (g) g.push(r)
    else by.set(s, [r])
  }
  return RowGroups.filter((s) => by.has(s)).map((s) => ({ state: s, rows: by.get(s) ?? [] }))
}

// nobodyTook is the open cards Home files under Stuck because nobody
// took them (card #487): never handed out, offered or assigned and not
// picked up, or an ask nobody took. The idle line names them, so "1
// agent idle, no open work" never sits over them. A holder that went
// away or a wait on a gone agent is Stuck too, but someone took those.
export function nobodyTook(c: CardSummary, now: number = Date.now()): boolean {
  return !!row(c, now).nobody
}

// Need is one item of the Needs you list.
// NOT_TRUSTED starts a card's cantStart when its agent couldn't start
// because its repo isn't trusted for agents on that device (card #449).
export const NOT_TRUSTED = "not trusted: "

// waitsOnTrust says the card's agent can't start until the person trusts
// its repo: a Trust and start row in Needs you.
export function waitsOnTrust(c: CardSummary): boolean {
  return !over(c) && !!c.cantStart?.startsWith(NOT_TRUSTED)
}

// The questions Irori's own sweep raises on a spawned agent's card (card
// #456, sa-7). They must match the Rust constants word for word:
// WAITING_PERMISSION in crates/core/src/service/spawn_waiting.rs (the
// blocker reads "Waiting for your permission: <name> is stopped on a
// prompt") and CRASHED_QUESTION in crates/coord/src/board/spawn.rs.
export const WAITING_PERMISSION = "Waiting for your permission"
export const CRASHED_QUESTION = "Its agent crashed twice in a row: start another?"

// spawnNeed says a card's decision is one of those: its agent stopped on
// a permission prompt nobody is there to answer, or two agents in a row
// went away before finishing, so Irori stopped starting them.
export function spawnNeed(c: CardSummary): "permission" | "crashed" | null {
  const q = c.blockerQuestion ?? ""
  if (q.startsWith(WAITING_PERMISSION)) return "permission"
  if (q === CRASHED_QUESTION) return "crashed"
  return null
}

export type Need =
  | { kind: "check" | "decision"; id: string; at: number; card: CardSummary }
  | { kind: "trust"; id: string; at: number; card: CardSummary }
  | { kind: "review"; id: string; at: number; pr: ReviewPr; cards: CardSummary[] }
  | { kind: "held"; id: string; at: number; delivery: Delivery }

// toPerson says the delivery is an agent's message to the person, still
// unanswered: sent to a user key (the person's, on any of their devices)
// and neither held nor closed.
export function toPerson(d: Delivery): boolean {
  if (!d.to?.startsWith("user:")) return false
  return (
    d.state === State.Queued ||
    d.state === State.Delivered ||
    d.state === State.Engaged ||
    d.state === State.Deferred
  )
}

const time = (s: string | undefined) => parseTime(s ?? "")?.getTime() ?? 0

// needsOf is the Needs you list, newest first: every card waiting on the
// person (a done check, else a decision, else its repo's trust: card #449), each PR waiting on their review
// once with every card on it (card #347), and every message Irori held for
// them. An agent's message to the person is a DM in Talk now (card #390,
// design b1), not a row here. held is the Held list.
export function needsOf(cards: CardSummary[], held: Delivery[]): Need[] {
  const out: Need[] = []
  for (const c of cards) {
    if (!needsYou(c)) continue
    const check = !!c.ready && c.openBlockers <= 1
    out.push({
      kind: check ? "check" : "decision",
      id: c.id,
      at: time(c.updatedAt),
      card: c,
    })
  }
  for (const c of cards) {
    if (!waitsOnTrust(c) || needsYou(c)) continue
    out.push({ kind: "trust", id: c.id, at: time(c.updatedAt), card: c })
  }
  const reviews = new Map<string, Extract<Need, { kind: "review" }>>()
  for (const c of cards) {
    if (!c.reviewPr || over(c) || needsYou(c)) continue
    const r = reviews.get(c.reviewPr.url)
    const at = time(c.updatedAt)
    if (r) {
      r.cards.push(c)
      r.at = Math.max(r.at, at)
      continue
    }
    reviews.set(c.reviewPr.url, {
      kind: "review",
      id: c.reviewPr.url,
      at,
      pr: c.reviewPr,
      cards: [c],
    })
  }
  out.push(...reviews.values())
  const seen = new Set<string>()
  for (const d of held) {
    if (seen.has(d.id)) continue
    seen.add(d.id)
    out.push({ kind: "held", id: d.id, at: time(d.createdAt), delivery: d })
  }
  return out.sort((a, b) => b.at - a.at)
}
