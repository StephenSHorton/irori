// After the call: divide the canvas into cards (card #1054;
// docs/design/call-agents.md §9). Three things live here:
//
// - the drafts: the assistant ends a reply with `<<cards>>` and `<<settle>>`
//   blocks, which `takeDrafts` pulls out of its text (lib/assistant.ts
//   splitReply calls it first, so a go line inside one never fires);
// - the finished canvas: the items of a huddle that is over, grouped by kind
//   with what still needs a person first, and the items its summary judges
//   answered or superseded (Settle these);
// - the writes: Make cards creates each card by the normal path, then marks
//   its items carded; Apply settles and drops. Nothing runs until a tap.
//
// Divide itself is a small store: which huddle the chat is on, so each of the
// chat's turns carries its `callId` and the Core puts the huddle in the
// prompt.

import { useSyncExternalStore } from "react"

import type { TalkCanvasItem } from "@/gen/bindings"
import type { BoardSummary, SpaceBoard } from "./boards"
import { setCanvasItem } from "./huddles"
import { boardsOf, openBoards } from "./spaces"

// ---- the blocks ----------------------------------------------------------------

export const CardsOpen = "<<cards>>"
export const CardsClose = "<</cards>>"
export const SettleOpen = "<<settle>>"
export const SettleClose = "<</settle>>"

// MaxCards is the most card drafts one reply shows.
export const MaxCards = 12
const MaxTitle = 200
const MaxObjective = 4000

export type CardDraft = {
  title: string
  objective: string
  // The canvas items it covers, by id.
  items: string[]
  // The assistant's suggestion for the board (a name), "" for none.
  board: string
}

export type SettleDraft = { settle: string[]; drop: string[] }

export type Drafts = {
  // The reply without its blocks.
  text: string
  cards: CardDraft[]
  settle: SettleDraft | null
  // A block is still being written (its closing line hasn't come).
  partial: boolean
}

function strings(v: unknown): string[] {
  if (!Array.isArray(v)) return []
  const out: string[] = []
  for (const x of v) {
    if (typeof x !== "string") continue
    const id = x.trim()
    if (id && !out.includes(id)) out.push(id)
  }
  return out
}

function clip(s: string, n: number): string {
  const t = s.trim()
  return t.length > n ? t.slice(0, n - 1).trimEnd() + "…" : t
}

// parseCard reads one `<<cards>>` block's JSON; null when it is not a card.
export function parseCard(body: string): CardDraft | null {
  let v: unknown
  try {
    v = JSON.parse(body.trim())
  } catch {
    return null
  }
  if (!v || typeof v !== "object") return null
  const o = v as Record<string, unknown>
  const title = typeof o.title === "string" ? clip(o.title, MaxTitle) : ""
  if (!title) return null
  return {
    title,
    objective:
      typeof o.objective === "string" ? clip(o.objective, MaxObjective) : "",
    items: strings(o.items),
    board: typeof o.board === "string" ? clip(o.board, 80) : "",
  }
}

// parseSettle reads one `<<settle>>` block's JSON; null when it names nothing.
export function parseSettle(body: string): SettleDraft | null {
  let v: unknown
  try {
    v = JSON.parse(body.trim())
  } catch {
    return null
  }
  if (!v || typeof v !== "object") return null
  const o = v as Record<string, unknown>
  const drop = strings(o.drop)
  // An item named both ways is dropped from neither: the person decides.
  const settle = strings(o.settle).filter((x) => !drop.includes(x))
  const both = strings(o.settle).filter((x) => drop.includes(x))
  const d = { settle, drop: drop.filter((x) => !both.includes(x)) }
  return d.settle.length || d.drop.length ? d : null
}

// cutTail drops a marker's first characters from the end of text being
// written ("…<<car"), so they never flash in the chat.
function cutTail(s: string, markers: string[]): string {
  let out = s
  for (const m of markers)
    for (let n = Math.min(m.length - 1, out.length); n > 0; n--)
      if (out.endsWith(m.slice(0, n))) {
        out = out.slice(0, out.length - n)
        break
      }
  return out
}

// takeDrafts takes every `<<cards>>` and `<<settle>>` block out of a reply.
// A block that isn't valid JSON is dropped without a draft. One still being
// written is cut from its opening line and reported as partial; it shows
// when it closes. The history keeps the reply whole.
export function takeDrafts(raw: string): Drafts {
  const cards: CardDraft[] = []
  let settle = null as SettleDraft | null
  let partial = false
  let text = ""
  let rest = raw
  for (;;) {
    const a = rest.indexOf(CardsOpen)
    const b = rest.indexOf(SettleOpen)
    const at = a < 0 ? b : b < 0 ? a : Math.min(a, b)
    if (at < 0) break
    const cardsFirst = at === a
    const open = cardsFirst ? CardsOpen : SettleOpen
    const close = cardsFirst ? CardsClose : SettleClose
    const end = rest.indexOf(close, at + open.length)
    text += rest.slice(0, at)
    if (end < 0) {
      partial = true
      rest = ""
      break
    }
    const body = rest.slice(at + open.length, end)
    if (cardsFirst) {
      const c = parseCard(body)
      if (c && cards.length < MaxCards) cards.push(c)
    } else {
      const s = parseSettle(body)
      if (s)
        settle = {
          settle: [...(settle?.settle ?? []), ...s.settle],
          drop: [...(settle?.drop ?? []), ...s.drop],
        }
    }
    rest = rest.slice(end + close.length)
  }
  text += rest
  text = cutTail(text, [CardsOpen, SettleOpen])
  // Blank lines left where blocks were taken out.
  text = text.replace(/\n{3,}/g, "\n\n").trim()
  return { text, cards, settle, partial }
}

// ---- the finished canvas -------------------------------------------------------

// Groups, in the order the finished canvas lists them: what still needs a
// person first.
export const GroupOrder = [
  "question",
  "task",
  "decision",
  "answer",
  "note",
] as const
export type Group = (typeof GroupOrder)[number]

export const GroupTitle: Record<Group, string> = {
  question: "Open questions",
  task: "Proposed tasks",
  decision: "Decisions",
  answer: "Answers",
  note: "Notes",
}

// isOpen is an item that still waits on someone: open, not carded.
export function isOpen(i: Pick<TalkCanvasItem, "status">): boolean {
  return i.status === "open" || i.status === ""
}

export function isCarded(i: Pick<TalkCanvasItem, "status" | "card">): boolean {
  return i.status === "carded" || !!i.card
}

// summaryOf is the chair's summary: the newest `summary` item, or null.
export function summaryOf(
  items: readonly TalkCanvasItem[]
): TalkCanvasItem | null {
  let best: TalkCanvasItem | null = null
  for (const i of items)
    if (i.kind === "summary" && (!best || i.atMs >= best.atMs)) best = i
  return best
}

// settleCandidates are the items the summary's `refs` judge answered or
// superseded that are still open: what Settle these would close.
export function settleCandidates(
  items: readonly TalkCanvasItem[]
): TalkCanvasItem[] {
  const s = summaryOf(items)
  if (!s) return []
  const byId = new Map(items.map((i) => [i.itemId, i]))
  const out: TalkCanvasItem[] = []
  for (const id of s.refs) {
    const i = byId.get(id)
    if (i && i.kind !== "summary" && isOpen(i) && !out.includes(i)) out.push(i)
  }
  return out
}

export type Section = { group: Group; title: string; items: TalkCanvasItem[] }

// finishedSections are the canvas's items grouped by kind, open ones before
// settled, dropped and carded ones, each in time order. The summary is not
// among them (it sits on top). Empty groups are left out.
export function finishedSections(items: readonly TalkCanvasItem[]): Section[] {
  const rank = (i: TalkCanvasItem) =>
    isCarded(i)
      ? 2
      : i.status === "dropped"
        ? 3
        : i.status === "settled"
          ? 1
          : 0
  const out: Section[] = []
  for (const group of GroupOrder) {
    const mine = items
      .filter((i) => i.kind === group)
      .sort((a, b) => rank(a) - rank(b) || a.atMs - b.atMs)
    if (mine.length) out.push({ group, title: GroupTitle[group], items: mine })
  }
  return out
}

// cardable is what Make cards could still turn into cards: not carded,
// dropped or a summary.
export function cardable(items: readonly TalkCanvasItem[]): TalkCanvasItem[] {
  return items.filter(
    (i) => i.kind !== "summary" && !isCarded(i) && i.status !== "dropped"
  )
}

// ---- what a draft will do ------------------------------------------------------

export type ShownCard = CardDraft & {
  // Its items that are on the canvas and not carded already, in order.
  ready: TalkCanvasItem[]
  // Items it named that are gone or carded, so the person sees why a line
  // is missing.
  skipped: string[]
}

// shownCards are the drafts with the items that are still open to them. A
// draft whose every item is carded by now (another person divided the
// canvas, or this one was made) is left out; one that names none stays
// (a card may stand on its own).
export function shownCards(
  drafts: readonly CardDraft[],
  items: readonly TalkCanvasItem[]
): ShownCard[] {
  const byId = new Map(items.map((i) => [i.itemId, i]))
  const out: ShownCard[] = []
  for (const d of drafts) {
    const ready: TalkCanvasItem[] = []
    const skipped: string[] = []
    for (const id of d.items) {
      const i = byId.get(id)
      if (i && !isCarded(i)) ready.push(i)
      else skipped.push(id)
    }
    if (d.items.length && !ready.length) continue
    out.push({ ...d, ready, skipped })
  }
  return out
}

export type ShownSettle = {
  settle: TalkCanvasItem[]
  drop: TalkCanvasItem[]
}

// shownSettle is the settle list with the items that exist and aren't
// already in that state (or carded).
export function shownSettle(
  s: SettleDraft | null,
  items: readonly TalkCanvasItem[]
): ShownSettle {
  if (!s) return { settle: [], drop: [] }
  const byId = new Map(items.map((i) => [i.itemId, i]))
  const pick = (ids: string[], status: string) =>
    ids
      .map((id) => byId.get(id))
      .filter(
        (i): i is TalkCanvasItem =>
          !!i && i.kind !== "summary" && !isCarded(i) && i.status !== status
      )
  return { settle: pick(s.settle, "settled"), drop: pick(s.drop, "dropped") }
}

// ---- the writes ----------------------------------------------------------------

export type Made = {
  // The ref of the card, once it was made.
  ref: string
  // Items marked carded.
  marked: string[]
  // Items that could not be marked, with why: the card exists.
  failed: { id: string; why: string }[]
}

// cardText is the objective a card is made with: the assistant's words, then
// the items it covers as quoted lines, so the card keeps where it came from.
export function cardText(c: ShownCard, huddle: string): string {
  const lines = c.ready.map((i) => `- ${i.text.trim().replace(/\s+/g, " ")}`)
  const head = c.objective.trim()
  const from = huddle.trim() ? `From the huddle "${huddle.trim()}".` : ""
  return [
    head,
    lines.length ? `From the canvas:\n${lines.join("\n")}` : "",
    from,
  ]
    .filter(Boolean)
    .join("\n\n")
}

// refOfTask is the ref a canvas item keeps for the card it became:
// "tag#12", or "#12" on a board with no tag.
export function refOfTask(t: { id: string; short?: string; tag?: string }) {
  return t.short ? `${t.tag ?? ""}#${t.short}` : t.id
}

// markCarded writes `carded` and the card's ref on each item, one by one: a
// refusal on one doesn't stop the others.
export async function markCarded(
  callId: string,
  ref: string,
  ids: readonly string[]
): Promise<Pick<Made, "marked" | "failed">> {
  const marked: string[] = []
  const failed: { id: string; why: string }[] = []
  for (const id of ids) {
    try {
      await setCanvasItem(callId, id, { status: "carded", card: ref })
      marked.push(id)
    } catch (err) {
      failed.push({
        id,
        why: err instanceof Error ? err.message : String(err ?? ""),
      })
    }
  }
  return { marked, failed }
}

// applySettle writes the person's own `settled` and `dropped` on the items,
// one by one. Returns how many it set and the failures.
export async function applySettle(
  callId: string,
  s: ShownSettle
): Promise<{ set: number; failed: { id: string; why: string }[] }> {
  let set = 0
  const failed: { id: string; why: string }[] = []
  const todo: [TalkCanvasItem, string][] = [
    ...s.settle.map((i): [TalkCanvasItem, string] => [i, "settled"]),
    ...s.drop.map((i): [TalkCanvasItem, string] => [i, "dropped"]),
  ]
  for (const [i, status] of todo) {
    try {
      await setCanvasItem(callId, i.itemId, { status })
      set++
    } catch (err) {
      failed.push({
        id: i.itemId,
        why: err instanceof Error ? err.message : String(err ?? ""),
      })
    }
  }
  return { set, failed }
}

// ---- which board a card goes on ------------------------------------------------

export type BoardChoice = {
  key: string
  label: string
  // What NewCardInput takes: the space ("" for Personal) and the board
  // inside it.
  board: string
  spaceBoard: string
}

// boardChoices are the boards a card from a huddle can go on: Personal's,
// then each space's open boards. A space with one board shows its name; one
// with several shows "Space › Board".
export function boardChoices(
  personal: readonly SpaceBoard[],
  spaces: readonly BoardSummary[]
): BoardChoice[] {
  const out: BoardChoice[] = []
  const mine = openBoards([...personal])
  for (const b of mine)
    out.push({
      key: `p:${b.boardId}`,
      label: mine.length > 1 ? `Personal › ${b.name}` : "Personal",
      board: "",
      spaceBoard: b.boardId,
    })
  for (const sp of spaces) {
    const bs = openBoards(boardsOf(sp))
    for (const b of bs)
      out.push({
        key: `s:${sp.boardId}:${b.boardId}`,
        label: bs.length > 1 ? `${sp.name} › ${b.name}` : sp.name,
        board: sp.boardId,
        spaceBoard: b.boardId,
      })
  }
  return out
}

// defaultChoice is where a huddle's cards go unless the person picks: its
// space's first board when the channel is in a space, else Personal (a
// person DM's goes to the promoter's Personal, §9).
export function defaultChoice(
  choices: readonly BoardChoice[],
  spaceId: string | undefined
): BoardChoice | undefined {
  return (
    (spaceId ? choices.find((c) => c.board === spaceId) : undefined) ??
    choices.find((c) => c.board === "") ??
    choices[0]
  )
}

// suggestedChoice is the board the assistant named, matched by the name it
// gave (case aside) against the choices' labels and board names; undefined
// when it named none or none fits.
export function suggestedChoice(
  choices: readonly BoardChoice[],
  name: string
): BoardChoice | undefined {
  const n = name.trim().toLowerCase()
  if (!n) return undefined
  return (
    choices.find((c) => c.label.toLowerCase() === n) ??
    choices.find((c) =>
      c.label
        .toLowerCase()
        .split("›")
        .some((p) => p.trim() === n)
    )
  )
}

// ---- the chat's huddle ---------------------------------------------------------

export type Divide = { callId: string; title: string }

let dividing: Divide | null = null
const subs = new Set<() => void>()

export function divideNow(): Divide | null {
  return dividing
}

function subscribe(f: () => void): () => void {
  subs.add(f)
  return () => subs.delete(f)
}

// startDivide puts the chat on huddle `callId`: each question it asks carries
// the id until stopDivide.
export function startDivide(callId: string, title: string) {
  dividing = { callId, title }
  for (const f of [...subs]) f()
}

export function stopDivide() {
  if (!dividing) return
  dividing = null
  for (const f of [...subs]) f()
}

export function useDivide(): Divide | null {
  return useSyncExternalStore(subscribe, divideNow, divideNow)
}

// divideAsk is what the page sends when the person taps Divide: the first
// message, so the assistant starts from the canvas without a prompt typed.
export const DivideAsk =
  "Divide this huddle's canvas into cards. Group what belongs together, and tell me what you would settle instead."
