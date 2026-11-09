// The huddle canvas as data (card #1052; docs/design/call-agents.md §3, §8,
// §13 card 9). Pure: the items a huddle's transcript carries become heads
// (a person and their assistant) and item nodes, placed append-stable, and
// the roster says each assistant's state in words. canvas/huddle.tsx draws
// it; the tests feed it directly.
//
// The three traps of the Board's canvas this avoids (docs/canvas-layout.md,
// "Huddle canvas"):
//   - The graph is built from the items alone, and keyed on their content
//     (`canvasKey`), never on transcript lines, so a line arriving changes
//     nothing the canvas draws.
//   - A node being dragged (or just dropped, its save not back yet) keeps
//     the person's spot when the graph changes (`reconcile`).
//   - Layout is append-stable: an item keeps the spot it first took
//     (`Placer`), a new one goes at the end of its column, and the view
//     refits only while untouched and only when a column is added
//     (`shouldRefit`).

import type {
  TalkAssistant,
  TalkCanvasItem,
  TalkTranscript,
} from "../gen/bindings"
import { personWord } from "./call"

export type Point = { x: number; y: number }

// Sizes of a node on the canvas, in canvas units.
export const ColWidth = 288
export const ItemWidth = 256
export const HeadHeight = 84
export const SlotHeight = 172
const FirstSlot = HeadHeight + 28

// Text shown before it's clamped with "more", in characters. The clamp in
// the node is by lines; this decides whether "More" is offered.
export const ClampChars = 140

// ---- who an item belongs under ------------------------------------------------

// groupOf is the person an item groups under: who asked, else who signed it
// (§8). An assistant's item signed by Stephen's device for Addam's call
// groups under Addam.
export function groupOf(i: Pick<TalkCanvasItem, "askerName" | "byName">) {
  return personWord(i.askerName || i.byName)
}

// byTime orders items oldest first, ties by id, so every device agrees.
export function byTime(items: readonly TalkCanvasItem[]): TalkCanvasItem[] {
  return [...items].sort(
    (a, b) => a.atMs - b.atMs || (a.itemId < b.itemId ? -1 : 1)
  )
}

// ---- words for an item --------------------------------------------------------

export const Kinds = ["note", "question", "decision", "task"] as const
export type AddKind = (typeof Kinds)[number]

const kindWords: Record<string, string> = {
  question: "Question",
  answer: "Answer",
  decision: "Decision",
  task: "Task",
  note: "Note",
  summary: "Summary",
}

export function kindWord(kind: string): string {
  return kindWords[kind] ?? "Note"
}

// answersTo counts the items that reply to `id`.
export function answersTo(items: readonly TalkCanvasItem[], id: string) {
  return items.filter((i) => i.reply === id && i.kind === "answer").length
}

// statusWord is where an item stands, by its kind (§8): a question reads
// open or answered, a decision proposed or agreed, a task proposed (not a
// card yet) or done. A note or summary has none. During the call a question
// an assistant answered reads "open · 1 answer" until a person settles it.
export function statusWord(
  i: Pick<TalkCanvasItem, "kind" | "status" | "card">,
  answers: number
): string {
  if (i.status === "carded")
    return i.card ? `Became card ${i.card}` : "Became a card"
  if (i.status === "dropped") return "Dropped"
  const settled = i.status === "settled"
  switch (i.kind) {
    case "question":
      if (settled) return "Answered"
      return answers > 0
        ? `Open · ${answers} ${answers === 1 ? "answer" : "answers"}`
        : "Open"
    case "decision":
      return settled ? "Agreed" : "Proposed"
    case "task":
      return settled ? "Done" : "Proposed, not a card"
    case "answer":
      return ""
    default:
      return ""
  }
}

// canSettle is whether an item takes Settle and Drop: the kinds with a
// status, while it is still open.
export function canSettle(i: Pick<TalkCanvasItem, "kind" | "status">) {
  return (
    (i.kind === "question" || i.kind === "decision" || i.kind === "task") &&
    (i.status === "" || i.status === "open")
  )
}

export function settleWord(kind: string): string {
  return kind === "question"
    ? "Answered"
    : kind === "decision"
      ? "Agree"
      : "Done"
}

// isLooking is a placeholder an assistant wrote before its look came back
// (card #1051): "Juno is looking: …".
export function isLooking(i: Pick<TalkCanvasItem, "assistant" | "text">) {
  return i.assistant && /^.{1,40} is looking\b/.test(i.text)
}

// clamped is whether an item's text is long enough to need "More".
export function clamped(text: string): boolean {
  return text.length > ClampChars || text.split("\n").length > 3
}

// ---- the graph ------------------------------------------------------------------

export type Head = {
  // The person's name as the canvas shows it ("You" for this person).
  name: string
  mine: boolean
  // The assistant under them: its state words, "" for none known.
  assistant: string
  state: string
  why: string
  // The assistant's own label ("Juno · Addam's assistant") when it has
  // written something, else "".
  label: string
}

export type CItem = {
  item: TalkCanvasItem
  group: string
  answers: number
  status: string
  // The spot is the person's, saved on the item.
  kept: boolean
}

export type CNode =
  | { id: string; type: "head"; pos: Point; head: Head }
  | { id: string; type: "item"; pos: Point; c: CItem }

export type CEdge = {
  id: string
  source: string
  target: string
  refs: boolean
}

export type CanvasGraph = {
  nodes: CNode[]
  edges: CEdge[]
  columns: number
}

export function headId(name: string): string {
  return `head:${name}`
}

// Placer remembers where each column and item first went, so a later build
// never moves them: new items go at the end of their column, a new person
// gets the next column. One lives as long as a page does.
export class Placer {
  private cols = new Map<string, number>()
  private rows = new Map<string, number>()
  private spots = new Map<string, Point>()

  columns(): number {
    return this.cols.size
  }

  col(name: string): number {
    let c = this.cols.get(name)
    if (c === undefined) {
      c = this.cols.size
      this.cols.set(name, c)
    }
    return c
  }

  head(name: string): Point {
    return { x: this.col(name) * ColWidth, y: 0 }
  }

  // spot is where item `id` of column `name` sits when the person hasn't
  // placed it.
  spot(id: string, name: string): Point {
    const had = this.spots.get(id)
    if (had) return had
    const c = this.col(name)
    const r = this.rows.get(name) ?? 0
    this.rows.set(name, r + 1)
    const p = { x: c * ColWidth, y: FirstSlot + r * SlotHeight }
    this.spots.set(id, p)
    return p
  }
}

// rosterHeads are the people across the top, in a stable order: the
// roster's, then anyone else who is on the huddle's list.
export function rosterHeads(
  t: Pick<TalkTranscript, "assistants" | "canvas">,
  people: readonly string[]
): Head[] {
  const out: Head[] = []
  const seen = new Set<string>()
  const add = (name: string, mine: boolean, a?: TalkAssistant) => {
    if (seen.has(name)) return
    seen.add(name)
    const own = t.canvas.find(
      (i) => i.assistant && i.label && groupOf(i) === name
    )
    out.push({
      name,
      mine,
      assistant: a ? a.state : "",
      state: a ? a.state : "",
      why: a?.why ?? "",
      label: own?.label ?? "",
    })
  }
  for (const a of t.assistants) add(personWord(a.name), a.mine, a)
  for (const p of people) add(personWord(p), personWord(p) === "You")
  return out
}

// buildCanvas is the graph for a huddle's canvas: a head per person, an
// item node per item, a line from an answer to its question and a dotted
// one from a summary to what it judges. Positions come from `placer`,
// or the item's own saved spot.
export function buildCanvas(
  placer: Placer,
  t: Pick<TalkTranscript, "assistants" | "canvas">,
  people: readonly string[]
): CanvasGraph {
  const heads = rosterHeads(t, people)
  const items = byTime(t.canvas)
  for (const h of heads) placer.col(h.name)
  for (const i of items) placer.col(groupOf(i))
  const nodes: CNode[] = heads.map((head) => ({
    id: headId(head.name),
    type: "head",
    pos: placer.head(head.name),
    head,
  }))
  const have = new Set(items.map((i) => i.itemId))
  const edges: CEdge[] = []
  for (const item of items) {
    const group = groupOf(item)
    const kept = item.x !== null && item.y !== null
    const pos = kept
      ? { x: item.x as number, y: item.y as number }
      : placer.spot(item.itemId, group)
    const answers = answersTo(items, item.itemId)
    nodes.push({
      id: item.itemId,
      type: "item",
      pos,
      c: { item, group, answers, status: statusWord(item, answers), kept },
    })
    if (item.reply && have.has(item.reply))
      edges.push({
        id: `${item.itemId}>${item.reply}`,
        source: item.reply,
        target: item.itemId,
        refs: false,
      })
    for (const r of item.refs)
      if (have.has(r))
        edges.push({
          id: `${item.itemId}~${r}`,
          source: item.itemId,
          target: r,
          refs: true,
        })
  }
  return { nodes, edges, columns: placer.columns() }
}

// canvasKey is what the canvas is made of, as text: the same items give the
// same key, however many times the transcript is read again. A transcript
// line, a changed `over` or a new array of equal items changes nothing.
export function canvasKey(
  t: Pick<TalkTranscript, "assistants" | "canvas">,
  people: readonly string[]
): string {
  return JSON.stringify([
    t.assistants.map((a) => [a.name, a.mine, a.state, a.why]),
    people,
    t.canvas.map((i) => [
      i.itemId,
      i.kind,
      i.text,
      i.status,
      i.reply,
      i.refs,
      i.assistant,
      i.label,
      i.askerName || i.byName,
      i.x,
      i.y,
      i.card,
    ]),
  ])
}

// ---- keeping the person's hand on a node -----------------------------------------

function same(a: Point, b: Point): boolean {
  return Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5
}

// reconcile is the nodes to show when the graph changes: the new graph's,
// except that a node being dragged keeps where it is now, and one just
// dropped keeps its dropped spot until the saved position comes back
// (`pending` forgets it then). Nothing is reset at render time.
export function reconcile<N extends { id: string; position: Point }>(
  prev: readonly N[],
  next: readonly N[],
  dragging: ReadonlySet<string>,
  pending: Map<string, Point>
): N[] {
  const was = new Map(prev.map((n) => [n.id, n]))
  return next.map((n) => {
    if (dragging.has(n.id)) {
      const p = was.get(n.id)
      return p ? { ...n, position: p.position } : n
    }
    const keep = pending.get(n.id)
    if (keep) {
      if (same(keep, n.position)) {
        pending.delete(n.id)
        return n
      }
      return { ...n, position: keep }
    }
    return n
  })
}

// shouldRefit is whether the view fits the whole canvas again: only while
// the person hasn't panned, zoomed or dragged, and only when a column has
// come or gone. An item landing in a column changes nothing here.
export function shouldRefit(o: {
  touched: boolean
  columns: number
  fitted: number
}): boolean {
  return !o.touched && o.columns !== o.fitted
}

// ---- the roster ---------------------------------------------------------------

export type RosterRow = {
  name: string
  mine: boolean
  // connected, taking notes, reconnecting, not connected, or "" for a
  // person with no assistant row (an older Irori, or it is over).
  state: "connected" | "notes" | "reconnecting" | "off" | ""
  words: string
  // Further lines under it.
  notes: string[]
}

// capped is whether an assistant has said it is done for this huddle (the
// cap, §6): its note reads "Juno has done its looks for this huddle."
export function capped(items: readonly TalkCanvasItem[], name: string) {
  return items.some(
    (i) =>
      i.assistant &&
      groupOf(i) === name &&
      /has done its looks for this huddle/.test(i.text)
  )
}

// rosterRows says, for each person, their assistant's state in words (§3):
// connected, taking notes, reconnecting or not connected (with the why),
// the cap, "lines reach assistants only at the end" in a space that isn't
// ready, and "lines aren't heard by assistants (older Irori)" in a DM
// whose person's lines carry no agents mark. `live` is the huddle being on.
export function rosterRows(
  t: Pick<
    TalkTranscript,
    "assistants" | "canvas" | "lines" | "convKind" | "over"
  >,
  people: readonly string[],
  o: { live: boolean; agentsOff: boolean }
): RosterRow[] {
  const names: { name: string; mine: boolean }[] = []
  const add = (name: string, mine: boolean) => {
    if (!names.some((n) => n.name === name)) names.push({ name, mine })
  }
  for (const a of t.assistants) add(personWord(a.name), a.mine)
  for (const p of people) add(personWord(p), personWord(p) === "You")
  return names.map(({ name, mine }) => {
    const a = t.assistants.find((x) => personWord(x.name) === name)
    const notes: string[] = []
    let state: RosterRow["state"] = ""
    let words = ""
    if (o.live && a) {
      const wrote = t.canvas.some((i) => i.assistant && groupOf(i) === name)
      if (a.state === "connected") {
        state = wrote ? "notes" : "connected"
        words = wrote ? "Taking notes" : "Connected"
      } else if (a.state === "reconnecting") {
        state = "reconnecting"
        words = "Reconnecting"
      } else {
        state = "off"
        words = a.why ? `Not connected: ${a.why}` : "Not connected"
      }
      if (capped(t.canvas, name))
        notes.push("Its looks for this huddle are used up.")
      const own = mine ? "Your" : `${name}'s`
      if (o.agentsOff)
        notes.push(`${own} lines reach assistants only at the end.`)
      else if (t.convKind === "dm") {
        const said = t.lines.filter((l) =>
          mine ? l.mine : personWord(l.name) === name
        )
        if (said.length > 0 && said.every((l) => !l.agents))
          notes.push(`${own} lines aren't heard by assistants (older Irori).`)
      }
    }
    return { name, mine, state, words, notes }
  })
}

// askAvailable is whether the Ask box shows: the huddle is on and the
// viewer's own assistant is connected (§3).
export function askAvailable(
  t: Pick<TalkTranscript, "assistants" | "over">,
  live: boolean
): boolean {
  return (
    live &&
    !t.over &&
    t.assistants.some((a) => a.mine && a.state === "connected")
  )
}

// emptyWords is what the canvas says when it has nothing yet.
export function emptyWords(o: {
  live: boolean
  agentsOff: boolean
  anyAssistant: boolean
}): string {
  if (o.agentsOff)
    return "Agents and the canvas are off: someone's Irori in this space needs updating."
  if (!o.anyAssistant)
    return o.live
      ? "No assistant is connected, so there are no notes. You can still add one."
      : "No notes from this huddle."
  return o.live
    ? "No notes yet. Assistants add what they hear as the huddle goes on."
    : "No notes from this huddle."
}
