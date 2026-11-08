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
import {
  createFollower,
  finished,
  listenCards,
  parseSeen,
  readWork,
  type Act,
  type Fact,
  type Follower,
  type SeenStore,
  type WorkThread,
} from "./assistant-follow"
import { cards, threads } from "./cards"
import { goesByItself, splitGo, type GoTarget } from "./goto"
import { AssistantService } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

export type { AssistantEvent, AssistantTurn }
export type { Act, WorkThread }

// ChatEntry is one message in the chat. A note is a plain line of the
// chat's own (an error, "Stopped."): shown, kept, never sent as history.
// posted is the thread an assistant message's draft became, once the
// person tapped Post thread (so it is never posted twice).
//
// Card #785: an update is a line the chat says about a thread it posted
// (its id): an agent took it, a card was made, it needs you, it is done.
// It goes to the assistant as its own words, so "how's that going?"
// knows. act is what the line lets the person do: answer the card's
// question, or reply on the thread. A person's message with to was a
// reply sent on that thread (its #ref) from the chat, not a question.
export type ChatEntry = {
  role: "person" | "assistant"
  text: string
  note?: boolean
  // Card #987: a note saying what the chat changed for the person.
  changed?: boolean
  posted?: Posted
  update?: string
  act?: Act
  to?: string
}

export type Posted = { id: string; short: string; tag?: string }

// Reply is the answer being written: its text so far, what it is looking
// at now (the "tool" status line), and how it ended.
export type Reply = {
  text: string
  status: string
  end: "" | "done" | "error"
  error: string
  // What the run changed, a line each (card #987).
  changes?: string[]
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
    case "changed":
      return ev.text.trim()
        ? { ...r, changes: [...(r.changes ?? []), ev.text.trim()] }
        : r
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
  for (const line of r.changes ?? [])
    out.push({ role: "assistant", text: line, note: true, changed: true })
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
// the assistant's own words, without the chat's notes. A posted draft
// says which thread it became, and a reply sent on a thread says where,
// so the assistant can look them up (card #785).
// The chat's own lines on the work it follows go in as one turn per run,
// marked as the chat's, so they don't crowd out the conversation.
export function historyFor(entries: readonly ChatEntry[]): AssistantTurn[] {
  const out: AssistantTurn[] = []
  let ups: string[] = []
  const flush = () => {
    if (ups.length)
      out.push({
        role: "assistant",
        text: `(The chat's own lines on the work it follows:)\n${ups.map((u) => `- ${u}`).join("\n")}`,
      })
    ups = []
  }
  for (const e of entries) {
    if (e.note) continue
    if (e.update) {
      ups.push(e.text)
      continue
    }
    flush()
    out.push({ role: e.role, text: turnText(e) })
  }
  flush()
  return capHistory(out)
}

function turnText(e: ChatEntry): string {
  if (e.to) return `(My reply on ${e.to}) ${e.text}`
  if (e.posted)
    return `${e.text}\n\n(The person posted this draft as thread ${e.posted.tag ?? ""}#${e.posted.short}.)`
  return e.text
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
    const { role, text, note, changed, posted, update, act, to } = x as Record<
      string,
      unknown
    >
    if ((role !== "person" && role !== "assistant") || typeof text !== "string")
      continue
    if (!text.trim()) continue
    const e: ChatEntry =
      note === true ? { role, text, note: true } : { role, text }
    if (note === true && changed === true) e.changed = true
    const p = posted as Record<string, unknown> | null | undefined
    if (
      role === "assistant" &&
      p &&
      typeof p === "object" &&
      typeof p.id === "string" &&
      p.id &&
      typeof p.short === "string"
    ) {
      e.posted = { id: p.id, short: p.short }
      if (typeof p.tag === "string" && p.tag) e.posted.tag = p.tag
    }
    if (role === "assistant" && typeof update === "string" && update) {
      e.update = update
      const a = parseAct(act)
      if (a) e.act = a
    }
    if (role === "person" && typeof to === "string" && to) e.to = to
    out.push(e)
  }
  return capHistory(out)
}

function parseAct(v: unknown): Act | undefined {
  if (!v || typeof v !== "object") return undefined
  const a = v as Record<string, unknown>
  const str = (k: string) => (typeof a[k] === "string" ? (a[k] as string) : "")
  if (a.kind === "answer" && str("card") && str("blocker"))
    return { kind: "answer", card: str("card"), blocker: str("blocker") }
  if (a.kind === "reply" && str("thread"))
    return { kind: "reply", thread: str("thread"), to: str("to") }
  return undefined
}

export function serializeHistory(entries: readonly ChatEntry[]): string {
  return JSON.stringify(capKeeping(entries, live))
}

// live is the threads the chat still follows (card #785): their posted
// messages stay through the cap, or following would stop while the work
// goes on (the follower's own lines would push them out).
const live = new Set<string>()

// capKeeping is capHistory that drops the oldest messages other than the
// posted ones of threads in keep.
export function capKeeping(
  entries: readonly ChatEntry[],
  keep: ReadonlySet<string>,
  n = HistoryCap
): ChatEntry[] {
  let drop = entries.length - n
  if (drop <= 0) return [...entries]
  const out: ChatEntry[] = []
  for (const e of entries) {
    if (drop > 0 && !(e.posted && keep.has(e.posted.id))) {
      drop--
      continue
    }
    out.push(e)
  }
  return out
}

function capChat(entries: readonly ChatEntry[]): ChatEntry[] {
  return capKeeping(entries, live)
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
  // The work of each thread the chat posted, as last read (card #785):
  // the posted draft lists the cards made from it. Not kept: read again.
  work: Record<string, WorkThread>
}

let state: AssistantState | null = null
const subs = new Set<() => void>()

// The preview's ?assistant=open starts the panel open on a sample chat,
// ?assistant=busy with a reply half written (lib/preview.ts).
function initial(): AssistantState {
  const flag = typeof window === "undefined" ? null : previewFlag("assistant")
  const base = { work: {} }
  // ?assistant=follow: a thread the chat posted, sample #20, followed
  // from nothing said, so the first read says how its work went (#785).
  if (flag === "follow")
    return {
      ...base,
      open: true,
      entries: [
        { role: "person", text: "Is #3 still moving? Get someone on it." },
        {
          role: "assistant",
          text: `#3 is waiting on a call about the lobby list. I drafted a thread.\n\n${DraftOpen}\nIs #3 still moving? And did anyone finish the release notes (#8)?\n${DraftClose}`,
          posted: { id: "sample-thread-1", short: "20" },
        },
      ],
      reply: null,
      unread: false,
    }
  if (flag === "open" || flag === "busy")
    return {
      ...base,
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
  if (flag === "go")
    return {
      ...base,
      open: true,
      entries: [
        { role: "person", text: "Where do I turn off pausing agents?" },
        { role: "assistant", text: sampleAnswer("pausing agents").text },
      ],
      reply: null,
      unread: false,
    }
  if (flag === "new")
    return { ...base, open: true, entries: [], reply: null, unread: false }
  if (flag === "draft")
    return {
      ...base,
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
  return {
    ...base,
    open: false,
    entries: loadHistory(),
    reply: null,
    unread: false,
  }
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

// newChat clears the conversation (stopping a reply being written). The
// threads it posted are no longer followed.
export function newChat() {
  stopAsking()
  set({ entries: [], reply: null, work: {} })
  follower?.poke()
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
    entries: capChat([...s.entries, ...settled(r)]),
    reply: null,
    unread: !s.open,
  })
  // Take me there (card #784): a whole reply with a go line, come in
  // while the chat is open, goes there by itself, once. Never from saved
  // history, never a reply stopped or cut short, never a PR (that opens
  // the browser: only a tap does).
  const go = r.end === "done" ? splitReply(r.text).go : null
  if (go && s.open && goesByItself(go)) goHandler?.(go)
}

// addNote puts a plain line of the chat's own at its end (a go that
// couldn't get there, card #784).
export function addNote(text: string) {
  const s = assistantState()
  set({
    entries: capHistory([
      ...s.entries,
      { role: "assistant", text, note: true },
    ]),
  })
}

// The chat's way to go somewhere (components/assistant/assistant-chat.tsx
// registers it while it is mounted).
let goHandler: ((t: GoTarget) => void) | null = null
export function onGo(f: ((t: GoTarget) => void) | null) {
  goHandler = f
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
    entries: capChat([...s.entries, { role: "person", text: body }]),
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

// splitReply splits a reply as the chat shows it: its drafted thread
// (splitDraft) and its go line (card #784, lib/goto.ts), taken only from
// the text outside the draft, so the draft posts exactly as shown.
// writing: the reply is still being written.
export function splitReply(
  raw: string,
  writing = false
): Split & { go: GoTarget | null } {
  const d = splitDraft(raw)
  const g = splitGo(d.text, writing)
  return { ...d, text: g.text, go: g.go }
}

// postDraft posts the draft of the chat's message at index as a thread,
// as the ask box does (lib/cards' threads.start: the room's thread, its
// #refs telling the agents on those cards). It notes the thread on the
// message and returns it. Posted already: that thread again; being
// posted (a second tap after the panel closed and opened): that post.
const posting = new Map<ChatEntry, Promise<Posted>>()

export function postDraft(index: number): Promise<Posted> {
  const entry = assistantState().entries[index]
  if (!entry || entry.role !== "assistant" || entry.note)
    return Promise.reject(new Error("That message is no longer in the chat."))
  if (entry.posted) return Promise.resolve(entry.posted)
  const going = posting.get(entry)
  if (going) return going
  const { draft, partial } = splitDraft(entry.text)
  if (!draft || partial)
    return Promise.reject(new Error("That message has no thread to post."))
  const p = threads
    .start(draft)
    .then((res) => {
      const posted: Posted = { id: res.task.id, short: res.task.short ?? "" }
      if (res.task.tag) posted.tag = res.task.tag
      // The chat may have moved on while it posted: find the message again.
      const now = assistantState().entries
      const i = now[index] === entry ? index : now.indexOf(entry)
      if (i >= 0) {
        const entries = [...now]
        entries[i] = { ...entry, posted }
        set({ entries })
        follower?.follow(posted.id)
      }
      return posted
    })
    .finally(() => posting.delete(entry))
  posting.set(entry, p)
  return p
}

// ---- following the work it posted (card #785) -------------------------------------

// The chat follows each thread it posted (lib/assistant-follow.ts) and
// says in the chat, as updates, when the work moves. What it has said is
// kept on this device beside the history.
const seenKey = "irori.assistant.follow"
let follower: Follower | null = null

// postedIds is the threads the chat's messages posted, oldest first.
export function postedIds(entries: readonly ChatEntry[]): string[] {
  const out: string[] = []
  for (const e of entries)
    if (e.posted && !out.includes(e.posted.id)) out.push(e.posted.id)
  return out
}

// updateEntries is a thread's new lines as the chat's messages.
export function updateEntries(id: string, lines: readonly Fact[]): ChatEntry[] {
  return lines.map((f) => {
    const e: ChatEntry = { role: "assistant", text: f.text, update: id }
    if (f.act) e.act = f.act
    return e
  })
}

function said(id: string, lines: Fact[], work: WorkThread | null) {
  const s = assistantState()
  // The chat may have moved on (New chat) while it read.
  if (!postedIds(s.entries).includes(id)) return
  const next: Partial<AssistantState> = {}
  if (work && !finished(work)) live.add(id)
  else live.delete(id)
  if (work) {
    if (JSON.stringify(s.work[id]) !== JSON.stringify(work))
      next.work = { ...s.work, [id]: work }
  } else if (s.work[id]) {
    const w = { ...s.work }
    delete w[id]
    next.work = w
  }
  if (lines.length) {
    next.entries = capChat([...s.entries, ...updateEntries(id, lines)])
    if (!s.open) next.unread = true
  }
  if (Object.keys(next).length) set(next)
}

// startFollowing starts the follower for the window's life (the chat's
// corner button mounts once); it returns its stop.
export function startFollowing(): () => void {
  if (follower) return () => {}
  const flag = previewFlag("assistant")
  // The preview's chats are samples: what they said is never kept.
  let mem: SeenStore =
    flag === "follow" ? { "sample-thread-1": { said: [] } } : {}
  const kept = flag === null
  follower = createFollower({
    ids: () => postedIds(assistantState().entries),
    read: readWork,
    listen: listenCards,
    load: () => {
      if (!kept) return mem
      try {
        return parseSeen(localStorage.getItem(seenKey))
      } catch {
        return {}
      }
    },
    save: (st) => {
      if (!kept) {
        mem = st
        return
      }
      try {
        if (Object.keys(st).length)
          localStorage.setItem(seenKey, JSON.stringify(st))
        else localStorage.removeItem(seenKey)
      } catch {
        // Blocked storage: it may say a line again after a restart.
      }
    },
    say: said,
  })
  const f = follower
  return () => {
    f.stop()
    if (follower === f) follower = null
  }
}

// replyOn sends the person's reply on a thread the chat posted, as the
// thread's own reply box does (its agents hear it), and shows it in the
// chat. ref is the thread's #ref.
export async function replyOn(
  thread: string,
  ref: string,
  text: string
): Promise<void> {
  const body = text.trim()
  if (!body) return
  await cards.comment(thread, body)
  const s = assistantState()
  set({
    entries: capChat([
      ...s.entries,
      { role: "person", text: body, to: ref || "the thread" },
    ]),
  })
}

// answerAsk answers a card's open item from the chat, as its card page
// does: option is an index into its options, or -1 with the person's own
// words.
export async function answerAsk(
  card: string,
  blocker: string,
  option: number,
  text = ""
): Promise<void> {
  await cards.answer(card, blocker, option, text.trim())
}

// ---- the window check's question -------------------------------------------------

// A window check (scripts/window-shot.sh, ?dev) can't type: its
// ?assistant-ask=<question> opens the chat on a new conversation and asks
// it, through the real assistant in the app, and with assistant-post then
// posts the drafted thread as a tap on Post thread would, handing it to
// onPosted. Once a page load; nothing without the flag.
let autoAsked = false
export function autoAsk(onPosted: (p: Posted) => void) {
  const q = previewFlag("assistant-ask")
  if (autoAsked || !q) return
  autoAsked = true
  const post = previewFlag("assistant-post") !== null
  newChat()
  setAssistantOpen(true)
  void ask(q)
  if (!post) return
  const off = subscribeAssistant(() => {
    const s = assistantState()
    if (s.reply) return
    off()
    const i = s.entries.length - 1
    const last = s.entries[i]
    if (last?.role !== "assistant" || !splitDraft(last.text).draft) return
    postDraft(i).then(onPosted, () => {})
  })
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
  // Take me there (card #784).
  if (/paus|stop/.test(s))
    return {
      tools: ["Reading the agents"],
      text: "Pausing agents is now **Stop here**, the button in the top bar, not in Settings. While it's on it reads Stopped here; one click starts this machine again.\n\n<<go setting stop-here>>",
    }
  if (/\b(model|effort)\b/.test(s))
    return {
      tools: [],
      text: "A new agent's model and effort are in **Settings › Agents**, under What a new agent starts with. An agent already working keeps its own.\n\n<<go setting agents.model>>",
    }
  const shown =
    /\b(?:show|open|take me to|where(?:'s| is))\b.*?(?:^|\s)([a-z0-9_-]*#\d+)/.exec(
      s
    )
  if (shown)
    return {
      tools: [`Opening ${shown[1]}`],
      text: `Here's ${shown[1]}. Opening it for you.\n\n<<go card ${shown[1]}>>`,
    }
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
