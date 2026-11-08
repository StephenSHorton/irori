// Ask Irori (card #783): the small chat in the window's bottom-right corner.
// The person asks anything about their cards, threads and agents, and a
// read-only helper answers in plain words from what Irori knows (the Rust
// side runs it on Haiku through the person's own Claude Code sign-in). It
// is not an agent: it never shows on the roster and never takes cards.
//
// This module is the chat's state: the conversation (kept on this device
// in localStorage, the last HistoryCap messages), whether the panel is
// open, and the reply being written. The reply is folded from the host's
// AssistantEvents by foldEvent, a pure reducer. Outside the app (the bun
// preview) a stand-in streams a canned answer that cites sample cards.

import type { AssistantEvent, AssistantTurn } from "../gen/bindings"
import { threads } from "./cards"
import { AssistantService } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

export type { AssistantEvent, AssistantTurn }

// ChatEntry is one message in the chat. A note is a plain line of the
// chat's own (an error, "Stopped."): shown, kept, never sent as history.
// posted is the thread an assistant message's draft became, once the
// person tapped Post thread (so it is never posted twice).
export type ChatEntry = {
  role: "person" | "assistant"
  text: string
  note?: boolean
  posted?: Posted
}

export type Posted = { id: string; short: string }

// Reply is the answer being written: its text so far, what it is looking
// at now (the "tool" status line), and how it ended.
export type Reply = {
  text: string
  status: string
  end: "" | "done" | "error"
  error: string
}

export const freshReply: Reply = { text: "", status: "", end: "", error: "" }

// The line an error with no words of its own shows.
export const failedLine = "The assistant couldn't answer just now."

// foldEvent folds one host event into the reply: a delta appends, a tool
// event replaces the status line, done and error end it. Once ended it
// takes nothing more (a late event from a stopped run changes nothing).
export function foldEvent(r: Reply, ev: AssistantEvent): Reply {
  if (r.end) return r
  switch (ev.kind) {
    case "delta":
      return ev.text ? { ...r, text: r.text + ev.text } : r
    case "tool":
      return { ...r, status: ev.text.trim() }
    case "done":
      return { ...r, status: "", end: "done" }
    case "error":
      return {
        ...r,
        status: "",
        end: "error",
        error: ev.text.trim() || failedLine,
      }
    default:
      return r
  }
}

// settled is what an ended reply leaves in the chat: its text, if it
// wrote any, and the error as a plain line.
export function settled(r: Reply): ChatEntry[] {
  const out: ChatEntry[] = []
  const text = r.text.trim()
  if (text) out.push({ role: "assistant", text })
  if (r.end === "error")
    out.push({ role: "assistant", text: r.error || failedLine, note: true })
  return out
}

// HistoryCap is how many messages the chat keeps (and sends back).
export const HistoryCap = 50

export function capHistory<T>(entries: readonly T[], n = HistoryCap): T[] {
  return entries.length > n ? entries.slice(entries.length - n) : [...entries]
}

// historyFor is the conversation as the host takes it: the person's and
// the assistant's own words, without the chat's notes.
export function historyFor(entries: readonly ChatEntry[]): AssistantTurn[] {
  return capHistory(
    entries.filter((e) => !e.note).map((e) => ({ role: e.role, text: e.text }))
  )
}

// parseHistory reads saved history back, keeping only well-formed entries
// (the store is this device's, but it may be from an older build).
export function parseHistory(raw: string | null): ChatEntry[] {
  if (!raw) return []
  let v: unknown
  try {
    v = JSON.parse(raw)
  } catch {
    return []
  }
  if (!Array.isArray(v)) return []
  const out: ChatEntry[] = []
  for (const x of v) {
    if (!x || typeof x !== "object") continue
    const { role, text, note, posted } = x as Record<string, unknown>
    if ((role !== "person" && role !== "assistant") || typeof text !== "string")
      continue
    if (!text.trim()) continue
    const e: ChatEntry =
      note === true ? { role, text, note: true } : { role, text }
    const p = posted as Record<string, unknown> | null | undefined
    if (
      role === "assistant" &&
      p &&
      typeof p === "object" &&
      typeof p.id === "string" &&
      p.id &&
      typeof p.short === "string"
    )
      e.posted = { id: p.id, short: p.short }
    out.push(e)
  }
  return capHistory(out)
}

export function serializeHistory(entries: readonly ChatEntry[]): string {
  return JSON.stringify(capHistory(entries))
}

const storeKey = "irori.assistant.history"

function loadHistory(): ChatEntry[] {
  try {
    return parseHistory(localStorage.getItem(storeKey))
  } catch {
    return []
  }
}

function saveHistory(entries: readonly ChatEntry[]) {
  try {
    if (entries.length)
      localStorage.setItem(storeKey, serializeHistory(entries))
    else localStorage.removeItem(storeKey)
  } catch {
    // Private mode or blocked storage: the chat still works, unsaved.
  }
}

// ---- the shared state ----------------------------------------------------------

export type AssistantState = {
  open: boolean
  entries: ChatEntry[]
  // The answer being written, or null when none is.
  reply: Reply | null
  // A reply ended while the panel was closed.
  unread: boolean
}

let state: AssistantState | null = null
const subs = new Set<() => void>()

// The preview's ?assistant=open starts the panel open on a sample chat,
// ?assistant=busy with a reply half written (lib/preview.ts).
function initial(): AssistantState {
  const flag = typeof window === "undefined" ? null : previewFlag("assistant")
  if (flag === "open" || flag === "busy")
    return {
      open: true,
      entries: sampleHistory,
      reply:
        flag === "busy"
          ? {
              ...freshReply,
              text: "Two cards are waiting on you. ",
              status: "Reading the Board",
            }
          : null,
      unread: false,
    }
  if (flag === "new")
    return { open: true, entries: [], reply: null, unread: false }
  if (flag === "draft")
    return {
      open: true,
      entries: [
        sampleHistory[2],
        sampleHistory[3],
        {
          role: "person",
          text: "Can you add a changelog page to the release notes?",
        },
        {
          role: "assistant",
          text: sampleWork("Add a changelog page to the release notes"),
        },
      ],
      reply: null,
      unread: false,
    }
  return { open: false, entries: loadHistory(), reply: null, unread: false }
}

export function assistantState(): AssistantState {
  state ??= initial()
  return state
}

export function subscribeAssistant(f: () => void): () => void {
  subs.add(f)
  return () => subs.delete(f)
}

function set(next: Partial<AssistantState>) {
  const was = assistantState()
  state = { ...was, ...next }
  if (next.entries && next.entries !== was.entries) saveHistory(next.entries)
  for (const f of subs) f()
}

export function setAssistantOpen(open: boolean) {
  const s = assistantState()
  if (s.open === open) return
  set(open ? { open, unread: false } : { open })
}

// newChat clears the conversation (stopping a reply being written).
export function newChat() {
  stopAsking()
  set({ entries: [], reply: null })
}

// ---- asking --------------------------------------------------------------------

// A run is one question being answered. Events can come before Ask
// returns the host's id, so a run is known by a token of ours; a Stop
// pressed before the id arrives is sent once it does.
type Run = { token: number; id: number | null }
let run: Run | null = null
let tokens = 0
const stoppedEarly = new Set<number>()

function finish(r: Reply) {
  run = null
  const s = assistantState()
  set({
    entries: capHistory([...s.entries, ...settled(r)]),
    reply: null,
    unread: !s.open,
  })
}

// ask sends the person's question with the chat so far. Nothing while a
// reply is being written.
export async function ask(text: string): Promise<void> {
  const body = text.trim()
  const s = assistantState()
  if (!body || run || s.reply) return
  const history = historyFor(s.entries)
  const token = ++tokens
  run = { token, id: null }
  set({
    entries: capHistory([...s.entries, { role: "person", text: body }]),
    reply: freshReply,
  })
  const onEvent = (ev: AssistantEvent) => {
    if (run?.token !== token) return
    const r = foldEvent(assistantState().reply ?? freshReply, ev)
    if (r.end) finish(r)
    else set({ reply: r })
  }
  try {
    const id = inWails()
      ? await AssistantService.Ask(history, body, onEvent)
      : sampleAsk(body, onEvent)
    if (stoppedEarly.delete(token)) void stopRun(id)
    else if (run?.token === token) run.id = id
  } catch (err) {
    if (run?.token !== token) return
    const r = assistantState().reply ?? freshReply
    finish({ ...r, status: "", end: "error", error: errorLine(err) })
  }
}

// stopAsking stops the reply being written: what it wrote so far stays,
// with "Stopped." after it. Later events from that run are dropped.
export function stopAsking() {
  const r = run
  if (!r && !assistantState().reply) return
  if (r?.id === null) stoppedEarly.add(r.token)
  else if (r) void stopRun(r.id)
  const reply = assistantState().reply ?? freshReply
  finish({ ...reply, status: "", end: "error", error: "Stopped." })
}

function stopRun(id: number): Promise<void> {
  if (!inWails()) {
    sampleStop(id)
    return Promise.resolve()
  }
  return AssistantService.Stop(id).catch(() => {})
}

function errorLine(err: unknown): string {
  const m = err instanceof Error ? err.message : String(err ?? "")
  const line = m.trim()
  return line ? line : failedLine
}

// ---- drafted threads -----------------------------------------------------------

// Asked for work, the assistant ends its reply with a thread for the
// person to post, between these lines (crates/window/src/assistant.rs,
// DRAFT_OPEN and DRAFT_CLOSE). The chat shows it with a Post thread
// button; the assistant itself never posts.
export const DraftOpen = "<<thread>>"
export const DraftClose = "<</thread>>"

export type Split = {
  // The reply without the draft.
  text: string
  // The drafted thread, "" when there is none.
  draft: string
  // The draft is still being written (its closing line hasn't come).
  partial: boolean
}

// cutMarkerTail drops a marker's first characters from the end of text
// being written ("…<<thr"), so they never flash in the chat.
function cutMarkerTail(s: string, marker: string): string {
  for (let n = Math.min(marker.length - 1, s.length); n > 0; n--)
    if (s.endsWith(marker.slice(0, n))) return s.slice(0, s.length - n)
  return s
}

// splitDraft takes a reply's drafted thread out of its text. The history
// keeps the reply whole; the chat splits it as it shows it.
export function splitDraft(raw: string): Split {
  const at = raw.indexOf(DraftOpen)
  if (at < 0)
    return {
      text: cutMarkerTail(raw, DraftOpen).trim(),
      draft: "",
      partial: false,
    }
  const before = raw.slice(0, at).trim()
  const rest = raw.slice(at + DraftOpen.length)
  const end = rest.indexOf(DraftClose)
  if (end < 0)
    return {
      text: before,
      draft: cutMarkerTail(rest, DraftClose).trim(),
      partial: true,
    }
  const after = rest.slice(end + DraftClose.length).trim()
  return {
    text: after ? `${before}\n\n${after}`.trim() : before,
    draft: rest.slice(0, end).trim(),
    partial: false,
  }
}

// postDraft posts the draft of the chat's message at index as a thread,
// as the ask box does (lib/cards' threads.start: the room's thread, its
// #refs telling the agents on those cards). It notes the thread on the
// message and returns it. Posted already: that thread again.
export async function postDraft(index: number): Promise<Posted> {
  const entry = assistantState().entries[index]
  if (!entry || entry.role !== "assistant" || entry.note)
    throw new Error("That message is no longer in the chat.")
  if (entry.posted) return entry.posted
  const { draft, partial } = splitDraft(entry.text)
  if (!draft || partial) throw new Error("That message has no thread to post.")
  const res = await threads.start(draft)
  const posted: Posted = { id: res.task.id, short: res.task.short ?? "" }
  // The chat may have moved on while it posted: find the message again.
  const now = assistantState().entries
  const i = now[index] === entry ? index : now.indexOf(entry)
  if (i >= 0) {
    const entries = [...now]
    entries[i] = { ...entry, posted }
    set({ entries })
  }
  return posted
}

// ---- reply text ----------------------------------------------------------------

// Replies are Markdown-light: short paragraphs, "- " lists and **bold**
// (rich.ts's boldRuns, through RichText's bold).
// replyText tidies a reply for richBlocks: a Markdown heading ("## Next")
// becomes a bold line.
export function replyText(s: string): string {
  return s
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((l) => {
      const h = /^\s{0,3}#{1,6}\s+(.+?)\s*#*\s*$/.exec(l)
      return h ? `**${h[1].replace(/\*\*/g, "")}**` : l
    })
    .join("\n")
}

// ---- the preview's stand-in -----------------------------------------------------

export const suggestions = [
  "What's blocked?",
  "What are my agents doing?",
  "Find the card about release notes",
]

const sampleHistory: ChatEntry[] = [
  { role: "person", text: "What's blocked?" },
  {
    role: "assistant",
    text: "**Two cards are waiting on you.**\n\n- #1 has 3 open questions; the first asks whether to ship today or wait for the two-PC temple check.\n- #3 needs one Firewall click on the PC.\n\n#8 is moving: sample-otter is on it, 1 of 2 steps done.",
  },
  { role: "person", text: "Where's the card about the lobby?" },
  {
    role: "assistant",
    text: "That's #3. It's blocked on the Firewall prompt, and #1 waits on it.",
  },
]

// sampleWork is the stand-in's answer to a request for work: a line and
// a drafted thread.
const sampleWork = (q: string) =>
  `I found #8, which is close: drafted a thread that names it.\n\n${DraftOpen}\n${q
    .trim()
    .replace(/^(please|can you|could you)\s+/i, "")
    .replace(/^./, (c) => c.toUpperCase())
    .replace(
      /[?.!]*$/,
      "."
    )} #8 already covers the release notes, so build on what it has.\n${DraftClose}`

// sampleAnswer picks a canned answer by the question's words.
export function sampleAnswer(q: string): { tools: string[]; text: string } {
  const s = q.toLowerCase()
  if (
    /^(please|can you|could you)\b|\b(add|fix|build|make|change|rename)\b/.test(
      s
    )
  )
    return { tools: [`Searching for “${q.trim()}”`], text: sampleWork(q) }
  if (/agent|doing|working|busy/.test(s))
    return {
      tools: ["Reading the agents", "Reading the Board"],
      text: "Three agents are working:\n\n- sample-otter is on #1, waiting for your call on its blockers.\n- sample-heron is watching #1 and answering in #suzuri.\n- sample-wren waits until #1 is done.\n\nTwo more are idle.",
    }
  if (/block|stuck|wait|need/.test(s))
    return {
      tools: ["Reading the Board", "Opening #1"],
      text: sampleHistory[1].text,
    }
  const about = /about\s+(.+?)\??$/.exec(q.trim())?.[1] ?? q.trim()
  return {
    tools: [`Searching for “${about}”`, "Opening #8"],
    text: `I think you mean #8: sample-otter is writing the release notes, 1 of 2 steps done. #1 waits on it.`,
  }
}

const sampleRuns = new Map<number, ReturnType<typeof setTimeout>[]>()
let sampleIds = 0

function sampleAsk(q: string, on: (ev: AssistantEvent) => void): number {
  const id = ++sampleIds
  const { tools, text } = sampleAnswer(q)
  const timers: ReturnType<typeof setTimeout>[] = []
  let at = 150
  const after = (ms: number, ev: AssistantEvent) => {
    at += ms
    timers.push(setTimeout(() => on(ev), at))
  }
  for (const t of tools) after(450, { kind: "tool", text: t })
  for (const w of text.match(/\S+\s*/g) ?? [])
    after(28, { kind: "delta", text: w })
  after(60, { kind: "done", text: "" })
  timers.push(setTimeout(() => sampleRuns.delete(id), at + 10))
  sampleRuns.set(id, timers)
  return id
}

function sampleStop(id: number) {
  for (const t of sampleRuns.get(id) ?? []) clearTimeout(t)
  sampleRuns.delete(id)
}

// ---- keeping clear of the page's own controls ----------------------------------

// The button sits in the bottom-right corner, lifted above whatever bar is
// docked under it there: the phone's tabs and call bar, a conversation's
// composer. Those carry data-assistant-avoid; "bar" ones are avoided by
// the open panel too, the rest only by the button (the panel covers them).
export type Box = { left: number; right: number; top: number; bottom: number }

// liftOver is how far above the viewport's bottom the button must sit to
// clear the boxes that reach into its column (from left to right), by
// their tops. Boxes with no size, off screen, or outside the column don't
// count.
export function liftOver(
  boxes: readonly Box[],
  viewport: { height: number },
  column: { left: number; right: number }
): number {
  let lift = 0
  for (const b of boxes) {
    if (b.right <= b.left || b.bottom <= b.top) continue
    if (b.top >= viewport.height || b.bottom <= 0) continue
    if (b.right <= column.left || b.left >= column.right) continue
    lift = Math.max(lift, viewport.height - b.top)
  }
  return Math.round(lift)
}
