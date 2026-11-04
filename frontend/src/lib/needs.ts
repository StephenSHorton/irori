// Home's "Needs you" list and its progress rows (card #261, #259's
// organization A): what waits on the person comes first, answerable on
// its row, and every other card is one compact row in one of four
// states. Pure, so the order and the words are tested without a window.

import { spawnLine } from "./spawn-line"
import {
  askLine,
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
  type CardRef,
  type CardSummary,
} from "./cards"
import { parseTime, plainLine } from "./irori"
import { HoldReason, State, type Delivery } from "./talk"

// RowState is one of the states a person acts on (finding 05): Needs you
// (a decision, a done check, a question), Working (an agent is on it right
// now), Waiting (on an agent, a review, another card, a taker), Idle (held,
// its holders idle a while with nothing named it waits on: card #305) and
// Done.
export type RowState = "needs" | "working" | "waiting" | "idle" | "done"

export const rowStateLabel: Record<RowState, string> = {
  needs: "Needs you",
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

// rowState is the card's state and the line under its title: what it
// waits on, who is on it, or how it ended. The words are the ones the
// card's own page uses.
export function rowState(
  c: CardSummary,
  now: number = Date.now()
): { state: RowState; line: string } {
  if (over(c)) return { state: "done", line: overLabel(c) }
  if (needsYou(c)) {
    if (c.ready && c.openBlockers <= 1)
      return { state: "needs", line: "Says it's done: is it?" }
    if (c.openBlockers > 1)
      return { state: "needs", line: `${c.openBlockers} decisions` }
    return { state: "needs", line: plainLine(c.blockerQuestion) || "A question for you" }
  }
  if (openAsk(c) && c.ask) return { state: "waiting", line: askLine(c.ask) }
  if (c.paused) return { state: "waiting", line: "Paused" }
  // Card #471: a card on a PR waits on the PR (agents merge it, another
  // member's agents approve it), never on the person; its chip links it.
  if (c.state === TaskState.InReview) return { state: "waiting", line: "In review" }
  if (c.subtaskBlocked) return { state: "waiting", line: "A subtask needs a call" }
  // Card #347: what its wait on another card comes down to for them.
  if (c.waitingOn && c.waitYou) return { state: "needs", line: `Waits on ${c.waitYou}` }
  // Card #481: waits nothing will end.
  if (c.waitLoop?.length) return { state: "needs", line: loopLine(c.waitLoop) }
  if (c.waitStuck)
    return { state: "needs", line: `Waits on ${refOf(c.waitStuck)}, which nobody is on` }
  if (c.waitingOn)
    return {
      state: "waiting",
      line: c.waitOn ? `Waiting on ${plainLine(c.waitOn)}` : "Waiting on something else",
    }
  if (c.waiting > 0) return { state: "waiting", line: "Your answer is with the agent" }
  const who = c.working
    .filter((w) => !w.assigned)
    .map(displayName)
    .slice(0, 2)
    .join(", ")
  if (busy(c)) return { state: "working", line: who ? `${who} working` : "Working" }
  // Card #431: between turns, but a background job it started still runs.
  const job = runningJobNow(c)
  if (job.length)
    return {
      state: "working",
      line: `${job.map(displayName).slice(0, 2).join(", ")} running a job`,
    }
  if (!c.working.length || c.holder.state === Holding.HoldNobody) {
    // Card #448: an agent Irori is starting, a device at its cap, or why
    // one can't start, instead of "nobody".
    const sp = spawnLine(c)
    if (sp) return { state: "waiting", line: sp.text[0].toUpperCase() + sp.text.slice(1) }
    return { state: "waiting", line: "Nobody has it yet" }
  }
  const w = workLabel(c)
  if (w === "not picked up" || w === "offered")
    return { state: "waiting", line: w === "offered" ? "Offered" : "Not picked up" }
  if (idleUnnamed(c, now)) return { state: "idle", line: who ? `${IDLE_LINE} (${who})` : IDLE_LINE }
  return { state: "waiting", line: who ? `${who}, not working on it now` : "Held, not working on it now" }
}

// Need is one item of the Needs you list.
// NOT_TRUSTED starts a card's cantStart when its agent couldn't start
// because its repo isn't trusted for agents on that device (card #449).
export const NOT_TRUSTED = "not trusted: "

// prChip is the small "waits on PR #N" chip an in-review card shows on
// itself (card #471), with the PR's link and who it waits on: agents merge
// it, and where the repo wants an approval another member's agents give
// it. Never the person, so it is never a Needs you row.
export function prChip(c: CardSummary): { text: string; title: string; url: string } | null {
  const pr = c.waitPr
  if (!pr || over(c)) return null
  const who = pr.approval
    ? "another member's agents approve it, then it merges"
    : "an agent merges it once its checks pass"
  return {
    text: `waits on PR #${pr.number}`,
    title: `Waits on PR #${pr.number} in ${pr.repo}: ${who}. Opens ${pr.url}`,
    url: pr.url,
  }
}

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

// refOf names another card as the person reads it: "#392".
export function refOf(r: CardRef): string {
  return r.short ? `#${plainLine(r.short)}` : plainLine(r.title) || "a card"
}

// andList joins names as a sentence does: "a", "a and b", "a, b and c".
export function andList(xs: string[]): string {
  if (xs.length < 2) return xs.join("")
  return `${xs.slice(0, -1).join(", ")} and ${xs[xs.length - 1]}`
}

// loopLine is a wait loop's line (card #481): "#392, #396 and #394 wait
// on each other".
export function loopLine(ring: CardRef[]): string {
  return `${andList(ring.map(refOf))} wait on each other`
}

// stuckLine is a chain's that ends at a card nobody is on (card #481):
// "#12 waits on #34, which nobody is on".
export function stuckLine(end: CardRef, cards: CardSummary[]): string {
  const who = andList(cards.map((c) => (c.short ? `#${plainLine(c.short)}` : plainLine(c.title))))
  return `${who} ${cards.length === 1 ? "waits" : "wait"} on ${refOf(end)}, which nobody is on`
}

export type Need =
  | { kind: "check" | "decision"; id: string; at: number; card: CardSummary }
  | { kind: "trust"; id: string; at: number; card: CardSummary }
  | { kind: "held"; id: string; at: number; delivery: Delivery }
  | { kind: "heldQuiet"; id: string; at: number; deliveries: Delivery[] }
  // Card #481: cards that wait on each other, once for the whole loop,
  // and cards whose waits end at a card nobody is on, once per card.
  | { kind: "loop"; id: string; at: number; ring: CardRef[]; cards: CardSummary[] }
  | { kind: "stuck"; id: string; at: number; end: CardRef; cards: CardSummary[] }

// HELD_QUIET is the id of Needs you's one line for the holds that don't
// need the person one by one (card #488).
export const HELD_QUIET = "held-quiet"

// asksPerson says a held message is the person's to decide on its own
// row in Needs you (card #488): their own message no agent in its repo
// can take yet, one from another person on a shared board, or an agent's
// message to them. Holds between agents (a long chain of messages, the
// allow or block list, an older Irori's rule) and the person's own Pause
// all wait behind one "N held" line instead.
export function asksPerson(d: Delivery): boolean {
  if (d.state !== State.Held) return false
  if (d.hold === HoldReason.HoldPaused) return false
  if (d.to?.startsWith("user:")) return true
  if (d.message.from.kind === "board") return true
  return d.hold === HoldReason.HoldUnowned && fromPerson(d)
}

// fromPerson says the person at this app sent d.
export function fromPerson(d: Delivery): boolean {
  return d.message.from.kind === "user" || d.message.from.key.startsWith("user:")
}

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
// person (a done check, else a decision, else its repo's trust: card #449)
// and every message Irori held for them. A PR is never theirs to review
// (card #471): its card shows a small chip instead. An agent's message to the person is a DM in Talk now (card #390,
// design b1), not a row here. held is the Held list: only the holds that
// ask the person (asksPerson) are rows; the rest are one heldQuiet line,
// always last (card #488).
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
  const loops = new Map<string, Extract<Need, { kind: "loop" | "stuck" }>>()
  for (const c of cards) {
    if (over(c)) continue
    const ring = c.waitLoop ?? []
    const end = c.waitStuck
    if (!ring.length && !end) continue
    const id = ring.length ? ring.map((r) => r.id).join(",") : end!.id
    const at = time(c.updatedAt)
    const n = loops.get(id)
    if (n) {
      n.cards.push(c)
      n.at = Math.max(n.at, at)
      continue
    }
    loops.set(
      id,
      ring.length
        ? { kind: "loop", id, at, ring, cards: [c] }
        : { kind: "stuck", id, at, end: end!, cards: [c] }
    )
  }
  out.push(...loops.values())
  const seen = new Set<string>()
  const quiet: Delivery[] = []
  for (const d of held) {
    if (seen.has(d.id)) continue
    seen.add(d.id)
    if (!asksPerson(d)) {
      quiet.push(d)
      continue
    }
    out.push({ kind: "held", id: d.id, at: time(d.createdAt), delivery: d })
  }
  out.sort((a, b) => b.at - a.at)
  if (quiet.length)
    out.push({
      kind: "heldQuiet",
      id: HELD_QUIET,
      at: Math.max(...quiet.map((d) => time(d.createdAt))),
      deliveries: quiet,
    })
  return out
}
