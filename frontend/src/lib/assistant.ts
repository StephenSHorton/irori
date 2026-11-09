// Ask Irori (card #783): the small chat in the window's bottom-right corner.
// The person asks anything about their cards, threads and agents, and a
// read-only helper answers in plain words from what Irori knows (the Rust
// side runs it through the person's own Claude Code sign-in). It is not an
// agent: it never shows on the roster and never takes cards.
//
// One assistant per person (docs/design/one-assistant.md): this module is
// the chat's state, built from the conversation's rows (assistant_chat_list,
// read again on assistant:changed). A question is a row the assistant's
// computer answers, wherever the person typed it. A reply streams under its
// question where it runs; elsewhere it appears when its row does. Outside
// the app (the bun preview) a stand-in keeps the entries and answers.

import type {
  AssistantChatItem,
  AssistantEvent,
  AssistantFile,
  AssistantHomeStatus,
} from "../gen/bindings"
import {
  createFollower,
  listenCards,
  parseSeen,
  readWork,
  silentOnly,
  type Act,
  type Fact,
  type Follower,
  type SeenStore,
  type WorkThread,
} from "./assistant-follow"
import { cards, threads } from "./cards"
import { goesByItself, splitGo, type GoTarget } from "./goto"
import { sampleDivideReply } from "./sample-huddle"
import {
  divideNow,
  stopDivide,
  takeDrafts,
  type CardDraft,
  type SettleDraft,
} from "./huddle-divide"
import { screenLabel, screenSnapshot, StatusHead } from "./assistant-screen"
import { AssistantService, Events } from "./host"
import { EventDevicesChanged, inWails } from "./irori"
import { previewFlag } from "./preview"
import { checkUploads, uploadOf } from "./uploads"

export type { AssistantEvent, AssistantFile, AssistantHomeStatus }
export type { Act, WorkThread }

// ChatEntry is one message in the chat. A note is a plain line of the
// chat's own (an error, "Stopped."): shown, kept, never sent as history.
// posted is the thread an assistant message's draft became, once it was
// posted, by itself or by a tap on Post thread (so never twice).
//
// Card #785: an update is a line the chat says about a thread it posted
// (its id), only what waits on the person (card #1000): it needs you, it
// is ready, it wants permission, no agent has started. It goes to the
// assistant as its own words, so "how's that going?" knows. act is what
// the line lets the person do: answer the card's question. A person's
// message with to was a reply sent on a thread (its #ref) from the chat
// by an older build, not a question.
//
// Card #999: files are what the person attached to a message, as references
// only (the bytes are in Core's store). A message may be files and no words.
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
  files?: AssistantFile[]
  // The label of the screen the assistant read for this reply (card
  // #1009), shown under it as "Looked at: …".
  looked?: string
  // Rows (one-assistant card 4): a stable key, the turn it belongs to, a
  // question with no reply yet, the device it was typed on, the turn it
  // asks again, and a note the person may Ask again from.
  id?: string
  turn?: string
  pending?: boolean
  device?: string
  again?: string
  retry?: boolean
}

export type Posted = { id: string; short: string; tag?: string }

// Reply is the answer being written: its text so far, what it is looking
// at now (the "tool" status line), and how it ended.
export type Reply = {
  text: string
  status: string
  end: "" | "done" | "error"
  error: string
  // The screen text it read (card #1009), once it called the screen tool.
  looked: string
  // What the run changed, a line each (card #987).
  changes?: string[]
}

export const freshReply: Reply = {
  text: "",
  status: "",
  end: "",
  error: "",
  looked: "",
}

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
  if (text)
    out.push(
      r.looked
        ? { role: "assistant", text, looked: r.looked }
        : { role: "assistant", text }
    )
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
    const {
      role,
      text,
      note,
      changed,
      posted,
      update,
      act,
      to,
      files,
      looked,
    } = x as Record<string, unknown>
    if ((role !== "person" && role !== "assistant") || typeof text !== "string")
      continue
    const refs = role === "person" ? parseFiles(files) : []
    if (!text.trim() && !refs.length) continue
    const e: ChatEntry =
      note === true ? { role, text, note: true } : { role, text }
    if (note === true && changed === true) e.changed = true
    if (refs.length) e.files = refs
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
      const a = parseAct(act)
      // Lines an older build said that no longer belong here (card #1000).
      if (!a && !waitsOnPerson(text)) continue
      e.update = update
      if (a) e.act = a
    }
    if (role === "person" && typeof to === "string" && to) e.to = to
    if (role === "assistant" && typeof looked === "string" && looked.trim())
      e.looked = looked
    out.push(e)
  }
  return capHistory(out)
}

// parseFiles keeps the well-formed file references of a saved message,
// each file once. A bad one is dropped; the message stays.
function parseFiles(v: unknown): AssistantFile[] {
  if (!Array.isArray(v)) return []
  const out: AssistantFile[] = []
  for (const x of v) {
    if (!x || typeof x !== "object") continue
    const { sha256, name, size, mime, device, can_read } = x as Record<
      string,
      unknown
    >
    if (
      typeof sha256 !== "string" ||
      !/^[0-9a-f]{64}$/.test(sha256) ||
      typeof name !== "string" ||
      !name ||
      typeof size !== "number" ||
      !Number.isFinite(size) ||
      size < 0 ||
      typeof device !== "string" ||
      out.some((f) => f.sha256 === sha256)
    )
      continue
    const f: AssistantFile = { sha256, name, size, device }
    if (typeof mime === "string" && mime) f.mime = mime
    if (typeof can_read === "boolean") f.can_read = can_read
    out.push(f)
  }
  return out
}

function parseAct(v: unknown): Act | undefined {
  if (!v || typeof v !== "object") return undefined
  const a = v as Record<string, unknown>
  const str = (k: string) => (typeof a[k] === "string" ? (a[k] as string) : "")
  if (a.kind === "answer" && str("card") && str("blocker"))
    return { kind: "answer", card: str("card"), blocker: str("blocker") }
  return undefined
}

// waitsOnPerson says a saved update line with nothing to tap is one of
// the two that still belong in the chat: a permission, or no agent yet.
// Everything else with no answer on it was work-following (card #1000).
function waitsOnPerson(text: string): boolean {
  return (
    /^\S*#\S+ is waiting for your permission: /.test(text) ||
    text.startsWith("No agent has started on ")
  )
}

export function serializeHistory(entries: readonly ChatEntry[]): string {
  return JSON.stringify(capHistory(entries))
}

const storeKey = "irori.assistant.history"

// loadHistory reads this device's old history (before chats were shared).
export function loadHistory(): ChatEntry[] {
  try {
    return parseHistory(localStorage.getItem(storeKey))
  } catch {
    return []
  }
}

// ---- the shared state ----------------------------------------------------------

// The chat is the one conversation of rows (one-assistant card 4, design
// §5): entries are built from assistant_chat_list and read again on
// assistant:changed. Nothing is kept here but per-device conveniences: the
// panel, the turns this page asked, what streams, the "seen up to" mark.
// Outside the app (the bun preview) a stand-in keeps the entries locally.

export type AssistantState = {
  open: boolean
  // The conversation, oldest first.
  entries: ChatEntry[]
  // The replies being written, by turn (assistant_chat_watch): only on the
  // assistant's computer; elsewhere nothing streams.
  streams: Readonly<Record<string, Reply>>
  // Something waits on the person: a reply to a question this page asked
  // that came in while the panel was closed, or a follow-up line past
  // "seen up to".
  unread: boolean
  // One of this page's questions is waiting or being answered.
  busy: boolean
  // The work of each thread the chat posted (card #785): the posted draft
  // lists the cards made from it. Not kept: read again.
  work: Record<string, WorkThread>
  // Turns whose draft is being posted by a tap, and why a post failed.
  posting: ReadonlySet<string>
  postFailed: Readonly<Record<string, string>>
  // Where the assistant runs (assistant_home_status), null until read.
  home: AssistantHomeStatus | null
  // Older entries remain beyond those loaded.
  more: boolean
}

let state: AssistantState | null = null
const subs = new Set<() => void>()

// The preview's ?assistant=open starts the panel open on a sample chat,
// ?assistant=busy with a reply half written (lib/preview.ts).
function initial(): AssistantState {
  const flag = typeof window === "undefined" ? null : previewFlag("assistant")
  const base: AssistantState = {
    open: false,
    entries: [],
    streams: {},
    unread: false,
    busy: false,
    work: {},
    posting: new Set(),
    postFailed: {},
    home: null,
    more: false,
  }
  // ?assistant=follow: a thread the chat posted, sample #20, followed
  // from nothing said, so the first read says what waits on the person.
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
    }
  if (flag === "open" || flag === "busy") {
    if (flag === "open") return { ...base, open: true, entries: sampleHistory }
    const turn = "sample-busy"
    return {
      ...base,
      open: true,
      busy: true,
      entries: [
        ...sampleHistory,
        {
          id: `q:${turn}`,
          role: "person",
          text: "What needs me?",
          turn,
          pending: true,
        },
      ],
      streams: {
        [turn]: {
          ...freshReply,
          text: "Two cards are waiting on you. ",
          status: "Reading the Board",
        },
      },
    }
  }
  // ?assistant=files: messages with files (card #999).
  if (flag === "files") return { ...base, open: true, entries: sampleFileChat }
  if (flag === "go")
    return {
      ...base,
      open: true,
      entries: [
        { role: "person", text: "Where do I turn off pausing agents?" },
        { role: "assistant", text: sampleAnswer("pausing agents").text },
      ],
    }
  if (flag === "new") return { ...base, open: true }
  // ?assistant=waiting: a question another device asked, no reply yet.
  if (flag === "waiting")
    return {
      ...base,
      open: true,
      entries: [
        ...sampleHistory,
        {
          id: "q:w1",
          role: "person",
          text: "Who is on the lobby card?",
          turn: "w1",
          pending: true,
        },
        {
          id: "q:w2",
          role: "person",
          text: "And the release notes?",
          turn: "w2",
          pending: true,
        },
      ],
      home: {
        device: "mac",
        name: "Mac",
        here: false,
        state: "connected",
        lastSeenMs: 0,
        claude: true,
        chat: true,
        none: "",
      },
    }
  if (flag === "draft")
    return {
      ...base,
      open: true,
      entries: [
        sampleHistory[2],
        sampleHistory[3],
        {
          role: "person",
          text: "Can you add a changelog page to the release notes? Check with me first.",
        },
        {
          role: "assistant",
          text: sampleWork(
            "Add a changelog page to the release notes, check with me first"
          ),
          turn: "sample-draft",
        },
      ],
    }
  return base
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
  state = { ...assistantState(), ...next }
  for (const f of subs) f()
}

export function setAssistantOpen(open: boolean) {
  const s = assistantState()
  if (s.open === open) return
  if (open) {
    replyDot = false
    markSeen()
    set({ open, unread: false })
    void refreshHome()
  } else set({ open })
}

// newChat appends a New chat row: every device hides the conversation so
// far. The turns being answered stop; the threads it posted are no longer
// followed.
export function newChat() {
  // A new chat is no longer on the huddle it was dividing (card #1054).
  stopDivide()
  if (!inWails()) {
    for (const t of [...localRuns.keys()]) stopTurn(t)
    set({ entries: [], streams: {}, busy: false, work: {} })
    return
  }
  items = []
  limit = PageSize
  derive()
  void AssistantService.ChatClear()
    .catch(() => {})
    .finally(() => void refresh())
}

// ---- the rows ------------------------------------------------------------------

const PageSize = 50
// Entries loaded, newest first, and how many a read asks for.
let items: AssistantChatItem[] = []
let limit = PageSize
let more = false

// stateText is the text of a turn's reply as the chat shows it.
const STOPPED = "Stopped"

// entriesFrom builds the shown conversation, oldest first, from the list's
// items (newest first). A question with no reply is pending. Only the
// person's questions and the assistant's replies, notes and follow-up lines
// are entries; a drafted answer (assistants-talk) isn't shown here yet.
export function entriesFrom(page: readonly AssistantChatItem[]): ChatEntry[] {
  const out: ChatEntry[] = []
  for (let i = page.length - 1; i >= 0; i--) {
    const it = page[i]
    if (it.kind === "update" && it.update) {
      const u = it.update
      const e: ChatEntry = {
        id: `u:${u.key}`,
        role: "assistant",
        text: u.text,
        update: u.thread || u.key,
      }
      if (u.act && u.act.kind === "answer" && u.act.card && u.act.blocker)
        e.act = { kind: "answer", card: u.act.card, blocker: u.act.blocker }
      out.push(e)
      continue
    }
    const t = it.turn
    if (it.kind !== "turn" || !t) continue
    const q: ChatEntry = {
      id: `q:${t.turn}`,
      role: "person",
      text: t.text,
      turn: t.turn,
      device: t.device,
    }
    if (t.files?.length) q.files = t.files
    if (t.again) q.again = t.again
    const r = t.reply
    if (!r) q.pending = true
    out.push(q)
    if (!r) continue
    if (r.text.trim()) {
      const a: ChatEntry = {
        id: `a:${t.turn}`,
        role: "assistant",
        text: r.text,
        turn: t.turn,
      }
      if (t.posted)
        a.posted = {
          id: t.posted.id,
          short: t.posted.short,
          ...(t.posted.tag ? { tag: t.posted.tag } : {}),
        }
      if (r.looked) a.looked = r.looked
      out.push(a)
    }
    r.changed.forEach((line, n) => {
      if (line.trim())
        out.push({
          id: `c:${t.turn}:${n}`,
          role: "assistant",
          text: line,
          note: true,
          changed: true,
          turn: t.turn,
        })
    })
    const why = r.error.trim()
    if (r.skipped || why)
      out.push({
        id: `e:${t.turn}`,
        role: "assistant",
        text:
          why === STOPPED
            ? "Stopped."
            : why ||
              "The assistant's computer didn't get to this. Ask again to send it now.",
        note: true,
        turn: t.turn,
        retry: true,
      })
  }
  return out
}

// postedIds is the threads the chat's messages posted, oldest first.
export function postedIds(entries: readonly ChatEntry[]): string[] {
  const out: string[] = []
  for (const e of entries)
    if (e.posted && !out.includes(e.posted.id)) out.push(e.posted.id)
  return out
}

// The turns this page asked in this run of the app, and when (ms). Only a
// reply to one of these may go somewhere by itself or light the dot, and
// only once, as it arrives (design §5, Markers and The dot).
const asked = new Map<string, number>()
const handled = new Set<string>()
// A reply to one of this page's questions came in while the panel was shut.
let replyDot = false
// The ten minutes after which a reply no longer goes anywhere by itself.
const GoWithinMs = 10 * 60 * 1000

let loaded = false

// derive rebuilds the shown entries from the loaded items and opens a
// watch for each question still waiting.
function derive() {
  const entries = entriesFrom(items)
  const s = assistantState()
  const pending = entries.filter((e) => e.pending && e.turn)
  watchTurns(pending.map((e) => e.turn as string))
  const busy = pending.some((e) => asked.has(e.turn as string))
  set({
    entries,
    more,
    busy,
    unread: s.open ? false : replyDot || updatesUnread(),
  })
  follower?.poke()
}

let reading = false
let again = false

// refresh reads the conversation again (assistant:changed, or after a
// write) and treats what newly arrived: a reply to this page's question
// may go to its page and light the dot.
export async function refresh(): Promise<void> {
  if (!inWails()) return
  if (reading) {
    again = true
    return
  }
  reading = true
  try {
    do {
      again = false
      const page = await AssistantService.ChatList("", limit)
      items = page.items
      more = page.more
      const first = !loaded
      loaded = true
      if (!first) arrived()
      else for (const it of items) if (it.turn?.reply) handled.add(it.turn.turn)
      derive()
    } while (again)
  } catch {
    // The host couldn't read: the next change reads again.
  } finally {
    reading = false
  }
}

// arrived acts on replies that came in since the last read, for questions
// this page asked: Take me there (card #784) once, and the dot.
function arrived() {
  for (const it of items) {
    const t = it.turn
    if (!t?.reply || handled.has(t.turn)) continue
    handled.add(t.turn)
    const at = asked.get(t.turn)
    if (at === undefined) continue
    const s = assistantState()
    const r = t.reply
    if (!r.error && !r.skipped && Date.now() - at < GoWithinMs) {
      const go = splitReply(r.text).go
      if (go && s.open && goesByItself(go)) goHandler?.(go)
    }
    if (!s.open) replyDot = true
  }
}

// loadOlder shows 50 more of the conversation.
export function loadOlder() {
  if (!more) return
  limit += PageSize
  void refresh()
}

// ---- the dot -------------------------------------------------------------------

// Each device keeps its own "seen up to" (a cursor) in localStorage: a
// per-device convenience, not part of the conversation.
const seenKey = "irori.assistant.seen"

// after says cursor a sorts after b: cursors read `later.hlc.seq.device`.
export function cursorAfter(a: string, b: string): boolean {
  const pa = a.split(".")
  const pb = b.split(".")
  for (let i = 0; i < 3; i++) {
    const x = Number(pa[i])
    const y = Number(pb[i])
    if (x !== y) return x > y
  }
  return pa.slice(3).join(".") > pb.slice(3).join(".")
}

function readSeen(): string | null {
  try {
    return localStorage.getItem(seenKey)
  } catch {
    return null
  }
}

// newestUpdate is the newest follow-up line's cursor (not an imported one).
function newestUpdate(): string {
  let best = ""
  for (const it of items) {
    if (it.kind !== "update" || it.update?.imported) continue
    if (!best || cursorAfter(it.cursor, best)) best = it.cursor
  }
  return best
}

// updatesUnread: a follow-up line past "seen up to", whether it came live
// or while this device was closed: it waits on the person wherever they
// are. With nothing seen yet, what is there now counts as seen.
function updatesUnread(): boolean {
  const newest = newestUpdate()
  if (!newest) return false
  const seen = readSeen()
  if (seen === null) {
    markSeen()
    return false
  }
  return cursorAfter(newest, seen)
}

function markSeen() {
  const newest = newestUpdate()
  if (!newest) return
  try {
    localStorage.setItem(seenKey, newest)
  } catch {
    // Blocked storage: the dot may light again after a restart.
  }
}

// ---- where the assistant runs --------------------------------------------------

export async function refreshHome(): Promise<void> {
  if (!inWails()) return
  try {
    const home = await AssistantService.HomeStatus()
    const was = assistantState().home
    if (JSON.stringify(was) !== JSON.stringify(home)) set({ home })
  } catch {
    // Not readable now: the line keeps what it had.
  }
}

// ---- the streamed reply --------------------------------------------------------

// A watch is opened for each question still waiting that this computer may
// run, and kept until the turn ends. Elsewhere it only gets a stream when
// card 7 lands.
const watched = new Set<string>()

function watchTurns(turns: string[]) {
  for (const turn of turns) {
    if (watched.has(turn)) continue
    watched.add(turn)
    void AssistantService.ChatWatch(turn, (ev) => onStream(turn, ev)).catch(
      () => {}
    )
  }
  // A turn that has its reply no longer streams.
  const still = new Set(turns)
  const s = assistantState()
  const gone = Object.keys(s.streams).filter((t) => !still.has(t))
  if (gone.length) {
    const streams = { ...s.streams }
    for (const t of gone) delete streams[t]
    set({ streams })
  }
}

function onStream(turn: string, ev: AssistantEvent) {
  if (ev.kind === "changed") return
  const s = assistantState()
  const r = foldEvent(s.streams[turn] ?? freshReply, ev)
  // The row, not the stream, is the reply: when it lands it replaces this.
  set({ streams: { ...s.streams, [turn]: r } })
}

// ---- asking --------------------------------------------------------------------

// addNote puts a plain line of the chat's own at its end (a go that
// couldn't get there, card #784). It lives with this page only.
export function addNote(text: string) {
  const s = assistantState()
  set({
    entries: [
      ...s.entries,
      { id: `n:${++notes}`, role: "assistant", text, note: true },
    ],
  })
}
let notes = 0

// The chat's way to go somewhere (components/assistant/assistant-chat.tsx
// registers it while it is mounted).
let goHandler: ((t: GoTarget) => void) | null = null
export function onGo(f: ((t: GoTarget) => void) | null) {
  goHandler = f
}

// attachFiles stores files for the chat (card #999) and says what became
// of each, one record per distinct file. It throws what the window refuses.
export async function attachFiles(files: File[]): Promise<AssistantFile[]> {
  checkUploads(files)
  if (!inWails()) return sampleAttach(files)
  const stored = await AssistantService.Attach(
    await Promise.all(files.map(uploadOf))
  )
  return stored.filter(
    (f, i) => stored.findIndex((o) => o.sha256 === f.sha256) === i
  )
}

// unreadableNote is the chat's own line for files the assistant can't
// read: said here, every time, not left to the model.
export function unreadableNote(files: readonly AssistantFile[]): string {
  const names = files.filter((f) => f.can_read === false).map((f) => f.name)
  if (!names.length) return ""
  return `Can't read ${names.join(", ")}: the assistant reads pictures, PDFs and text files.`
}

// ask sends the person's question as a row; the assistant's computer
// answers it, wherever it is. The composer stays on while questions wait:
// a second one may follow the first. again is the turn Ask again repeats.
export async function ask(
  text: string,
  files: AssistantFile[] = [],
  again = ""
): Promise<void> {
  const body = text.trim()
  if (!body && !files.length) return
  if (!inWails()) return localAsk(body, files)
  try {
    // What the window shows, sent along for the screen tool (card #1009).
    const screen = await screenSnapshot().catch(() => "")
    // Divide (card #1054): while the chat is on a huddle, each question
    // carries its id and the assistant's computer puts the huddle in the
    // prompt.
    const turn = await AssistantService.ChatAsk(
      body,
      screen,
      again,
      files,
      divideNow()?.callId
    )
    asked.set(turn, Date.now())
    await refresh()
  } catch (err) {
    addNote(errorLine(err))
  }
}

// stopTurn stops one question: from any device, before or while it runs.
export function stopTurn(turn: string) {
  if (!inWails()) {
    localStop(turn)
    return
  }
  void AssistantService.ChatStop(turn).catch(() => {})
}

function errorLine(err: unknown): string {
  const m = err instanceof Error ? err.message : String(err ?? "")
  const line = m.trim()
  return line ? line : failedLine
}

// ---- drafted threads -----------------------------------------------------------

// Asked for work, the assistant ends its reply with a thread between
// these lines (crates/window/src/assistant.rs, DRAFT_OPEN and DRAFT_CLOSE).
// The chat posts it by itself when the reply ends (card #988). Asked to
// check first, the assistant opens it with DraftOpenCheck instead, and the
// chat shows it with a Post thread button and posts only on a tap.
export const DraftOpen = "<<thread>>"
export const DraftOpenCheck = "<<thread check>>"
export const DraftClose = "<</thread>>"

export type Split = {
  // The reply without the draft.
  text: string
  // The drafted thread, "" when there is none.
  draft: string
  // The draft is still being written (its closing line hasn't come).
  partial: boolean
  // The person asked to check it first: only a tap posts it.
  check: boolean
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
  const open = /<<thread( check)?>>/.exec(raw)
  if (!open)
    return {
      text: cutMarkerTail(cutMarkerTail(raw, DraftOpen), DraftOpenCheck).trim(),
      draft: "",
      partial: false,
      check: false,
    }
  const check = open[1] !== undefined
  const before = raw.slice(0, open.index).trim()
  const rest = raw.slice(open.index + open[0].length)
  const end = rest.indexOf(DraftClose)
  if (end < 0)
    return {
      text: before,
      draft: cutMarkerTail(rest, DraftClose).trim(),
      partial: true,
      check,
    }
  const after = rest.slice(end + DraftClose.length).trim()
  return {
    text: after ? `${before}\n\n${after}`.trim() : before,
    draft: rest.slice(0, end).trim(),
    partial: false,
    check,
  }
}

// splitReply splits a reply as the chat shows it: the card and settle
// drafts of a huddle's Divide (card #1054, lib/huddle-divide.ts), its
// drafted thread (splitDraft) and its go line (card #784, lib/goto.ts),
// taken only from the text outside the blocks, in that order, so a go line
// inside a block never fires and a draft posts exactly as shown.
// writing: the reply is still being written.
export function splitReply(
  raw: string,
  writing = false
): Split & {
  go: GoTarget | null
  cards: CardDraft[]
  settle: SettleDraft | null
} {
  const k = takeDrafts(raw)
  const d = splitDraft(k.text)
  const g = splitGo(d.text, writing)
  return { ...d, text: g.text, go: g.go, cards: k.cards, settle: k.settle }
}

// postDraft posts the draft of the reply at `turn` as a thread, as the
// ask box does (lib/cards' threads.start: the room's thread, its #refs
// telling the agents on those cards), marked as the assistant's so Irori
// doesn't rename it (card #991). A tap does this only when the reply did not
// post by itself (the assistant's computer posts a whole draft itself, and
// says so in its row). It then appends `posted`, so every device hides the
// button. Posted already: that thread again; being posted (a second tap
// meanwhile): that post.
const posting = new Map<string, Promise<Posted>>()

export function postDraft(turn: string): Promise<Posted> {
  const entry = assistantState().entries.find(
    (e) => e.turn === turn && e.role === "assistant" && !e.note && !e.update
  )
  if (!entry)
    return Promise.reject(new Error("That message is no longer in the chat."))
  if (entry.posted) return Promise.resolve(entry.posted)
  const going = posting.get(turn)
  if (going) return going
  const { draft, partial } = splitDraft(entry.text)
  if (!draft || partial)
    return Promise.reject(new Error("That message has no thread to post."))
  const busy = (on: boolean) => {
    const s = assistantState()
    const next = new Set(s.posting)
    if (on) next.add(turn)
    else next.delete(turn)
    set({ posting: next })
  }
  busy(true)
  const p = threads
    .start(draft, [], [], "", null, { assistant: true })
    .then(async (res) => {
      const posted: Posted = { id: res.task.id, short: res.task.short ?? "" }
      if (res.task.tag) posted.tag = res.task.tag
      if (inWails()) {
        await AssistantService.ChatPosted(
          turn,
          posted.id,
          posted.short,
          posted.tag ?? ""
        ).catch(() => {})
        await refresh()
      } else {
        const entries = assistantState().entries.map((e) =>
          e === entry ? { ...e, posted } : e
        )
        set({ entries })
        follower?.follow(posted.id)
      }
      const failed = { ...assistantState().postFailed }
      delete failed[turn]
      set({ postFailed: failed })
      return posted
    })
    .catch((err) => {
      set({
        postFailed: { ...assistantState().postFailed, [turn]: errorLine(err) },
      })
      throw err
    })
    .finally(() => {
      posting.delete(turn)
      busy(false)
    })
  posting.set(turn, p)
  return p
}

// ---- following the work it posted (card #785) -------------------------------------

// The chat follows each thread it posted (lib/assistant-follow.ts) and
// says as follow-up rows what waits on the person: the first row per key
// wins, so every device saying it at once shows one line. What it has said
// comes from the rows. The facts it never says (taken, made, started, done,
// closed) are kept on this device only, in `irori.assistant.follow.silent`.
const silentKey = "irori.assistant.follow.silent"
let follower: Follower | null = null

// updateEntries is a thread's new lines as the chat's messages (the
// preview, which has no rows).
export function updateEntries(id: string, lines: readonly Fact[]): ChatEntry[] {
  return lines.map((f) => {
    const e: ChatEntry = { role: "assistant", text: f.text, update: id }
    if (f.act) e.act = f.act
    return e
  })
}

// updateKey is the row key of a thread's fact.
export function updateKey(thread: string, fact: string): string {
  return `${thread}|${fact}`
}

// shownKeys is the facts the rows say a thread already had said.
function shownKeys(thread: string): string[] {
  const out: string[] = []
  for (const it of items) {
    const u = it.update
    if (it.kind !== "update" || !u || u.thread !== thread) continue
    const i = u.key.indexOf("|")
    if (i >= 0) out.push(u.key.slice(i + 1))
  }
  return out
}

function said(id: string, lines: Fact[], work: WorkThread | null) {
  const s = assistantState()
  // The chat may have moved on (New chat) while it read.
  if (!postedIds(s.entries).includes(id)) return
  const next: Partial<AssistantState> = {}
  if (work) {
    if (JSON.stringify(s.work[id]) !== JSON.stringify(work))
      next.work = { ...s.work, [id]: work }
  } else if (s.work[id]) {
    const w = { ...s.work }
    delete w[id]
    next.work = w
  }
  if (lines.length) {
    if (inWails()) {
      for (const f of lines)
        void AssistantService.ChatUpdate({
          key: updateKey(id, f.key),
          thread: id,
          text: f.text,
          act: f.act
            ? { kind: f.act.kind, card: f.act.card, blocker: f.act.blocker }
            : null,
        }).catch(() => {})
    } else {
      next.entries = [...s.entries, ...updateEntries(id, lines)]
      if (!s.open) next.unread = true
    }
  }
  if (Object.keys(next).length) set(next)
}

// startFollowing starts the chat's live parts for the window's life (the
// chat's corner button mounts once): it reads the conversation, listens for
// changes and follows the threads it posted. It returns its stop.
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
    shown: shownKeys,
    // Card #1136: at most one read of the threads and cards in 3 s.
    minGapMs: 3000,
    load: () => {
      if (!kept) return mem
      try {
        return parseSeen(localStorage.getItem(silentKey))
      } catch {
        return {}
      }
    },
    save: (st) => {
      if (!kept) {
        mem = st
        return
      }
      const silent = silentOnly(st)
      try {
        if (Object.keys(silent).length)
          localStorage.setItem(silentKey, JSON.stringify(silent))
        else localStorage.removeItem(silentKey)
      } catch {
        // Blocked storage: it may say a line again after a restart.
      }
    },
    say: said,
  })
  const f = follower
  const offs: Array<() => void> = []
  if (inWails()) {
    offs.push(
      Events.OnShown(EventAssistantChanged, () => void refresh()),
      Events.OnShown(EventAssistantHome, () => void refreshHome()),
      Events.OnShown(EventDevicesChanged, () => void refreshHome())
    )
    const timer = setInterval(() => {
      if (assistantState().open) void refreshHome()
    }, HomeRefreshMs)
    offs.push(() => clearInterval(timer))
    void refresh()
    void refreshHome()
  }
  return () => {
    for (const off of offs) off()
    f.stop()
    if (follower === f) follower = null
  }
}

const EventAssistantChanged = "assistant:changed"
const EventAssistantHome = "assistant:home"
const HomeRefreshMs = 5000

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
// posts the drafted thread as a tap on Post thread would (a check-first
// one too), handing it to onPosted. Once a page load; nothing without the
// flag.
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
    if (s.busy) return
    const last = s.entries[s.entries.length - 1]
    if (last?.role !== "assistant" || !last.turn || last.posted) return
    if (!splitDraft(last.text).draft) return
    off()
    postDraft(last.turn).then(onPosted, () => {})
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

// ?assistant=files (card #999): a picture with words, files alone, and a
// type the assistant can't read.
const sampleRef = (
  n: number,
  name: string,
  size: number,
  mime: string,
  can_read = true
): AssistantFile => ({
  sha256: n.toString(16).padStart(64, "a"),
  name,
  size,
  mime,
  device: "sample-device",
  can_read,
})

const sampleFileChat: ChatEntry[] = [
  {
    role: "person",
    text: "What's wrong with this screen?",
    files: [sampleRef(1, "screenshot.png", 182_400, "image/png")],
  },
  {
    role: "assistant",
    text: "The Send button is cut off at the right edge.",
  },
  {
    role: "person",
    text: "",
    files: [
      sampleRef(
        2,
        "a-very-long-quarterly-report-name-final-v2.pdf",
        2_400_000,
        "application/pdf"
      ),
      sampleRef(3, "notes.md", 4_100, "text/markdown"),
    ],
  },
  {
    role: "assistant",
    text: "The report's second page says the launch moved to March; your notes still say February.",
  },
  {
    role: "person",
    text: "",
    files: [
      sampleRef(
        4,
        "report.docx",
        51_200,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        false
      ),
    ],
  },
  {
    role: "assistant",
    text: "Can't read report.docx: the assistant reads pictures, PDFs and text files.",
    note: true,
  },
]

// checkFirst is how the stand-in hears "check with me first".
const checkFirst =
  /[,;.]?\s*(?:but\s+)?(?:check(?: with me)? first|show me first|let me (?:see|look|check)[^,.?!]*|(?:don't|do not) post[^,.?!]*)/i

// sampleWork is the stand-in's answer to a request for work: a line and
// a thread, which waits for a tap if the request said to check first.
const sampleWork = (q: string) => {
  const check = checkFirst.test(q)
  const line = check
    ? "I found #8, which is close: here's a thread that names it, waiting for you."
    : "I found #8, which is close: posting a thread that names it."
  return `${line}\n\n${check ? DraftOpenCheck : DraftOpen}\n${q
    .replace(checkFirst, "")
    .trim()
    .replace(/^(please|can you|could you)\s+/i, "")
    .replace(/^./, (c) => c.toUpperCase())
    .replace(
      /[?.!]*$/,
      "."
    )} #8 already covers the release notes, so build on what it has.\n${DraftClose}`
}

// sampleAnswer picks a canned answer by the question's words.
export function sampleAnswer(
  q: string,
  screen = ""
): { tools: string[]; text: string } {
  const s = q.toLowerCase()
  // Card #1009: a question about where the person is reads the screen.
  if (screen && /\b(this|here)\b|looking at|where am i/.test(s)) {
    const label = screenLabel(screen)
    return {
      tools: [`${StatusHead}: ${label}`],
      text: `You're on ${label}. Nothing there is waiting on you right now.`,
    }
  }
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

// sampleFiles is the stand-in's answer to a message with files: nothing
// reads them in the preview, and it says so.
export function sampleFiles(
  q: string,
  files: readonly AssistantFile[]
): { tools: string[]; text: string } {
  const read = files.filter((f) => f.can_read !== false).map((f) => f.name)
  if (!read.length) return sampleAnswer(q)
  return {
    tools: [`Looking at ${read[0]}`],
    text: `This is the preview, so nothing read ${read.join(", ")}. In the app the assistant reads it and answers${q ? ` “${q}”` : ""}.`,
  }
}

// The stand-in's store: pictures, PDFs and text read, the rest refused,
// as the app decides from the bytes.
const sampleReads =
  /^(image\/(png|jpeg|gif|webp|hei[cf])|application\/(pdf|json)|text\/.*)$/
const sampleReadsExt =
  /\.(txt|md|csv|json|ya?ml|log|toml|xml|html|[jt]sx?|rs|go|py)$/i
let sampleStored = 0

function sampleAttach(files: File[]): AssistantFile[] {
  return files.map((f) => ({
    sha256: (++sampleStored).toString(16).padStart(64, "0"),
    name: f.name || "pasted.png",
    size: f.size,
    mime: f.type || "application/octet-stream",
    device: "sample-device",
    can_read: sampleReads.test(f.type) || sampleReadsExt.test(f.name),
  }))
}

const sampleRuns = new Map<number, ReturnType<typeof setTimeout>[]>()
let sampleIds = 0

function sampleAsk(
  q: string,
  files: readonly AssistantFile[],
  on: (ev: AssistantEvent) => void,
  screen = ""
): number {
  const id = ++sampleIds
  // Divide (card #1054): the chat is on a huddle.
  const dividing = divideNow()
  const { tools, text } = dividing
    ? {
        tools: ["Reading the canvas"],
        text: sampleDivideReply(dividing.callId),
      }
    : files.length
      ? sampleFiles(q, files)
      : sampleAnswer(q, screen)
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

// ---- the preview's chat (no host, so no rows) -------------------------------------

// Outside the app the entries live here and the stand-in answers.
const localRuns = new Map<string, number>()
let localTurns = 0

function localAsk(body: string, files: readonly AssistantFile[]) {
  const s = assistantState()
  const turn = `local-${++localTurns}`
  const mine: ChatEntry[] = [
    {
      id: `q:${turn}`,
      role: "person",
      text: body,
      turn,
      pending: true,
      ...(files.length ? { files: [...files] } : {}),
    },
  ]
  const note = unreadableNote(files)
  if (note) mine.push({ role: "assistant", text: note, note: true })
  if (!body && files.every((f) => f.can_read === false)) {
    mine[0] = { ...mine[0], pending: false }
    set({ entries: [...s.entries, ...mine] })
    return
  }
  set({
    entries: [...s.entries, ...mine],
    streams: { ...s.streams, [turn]: freshReply },
    busy: true,
  })
  const screen = ""
  const id = sampleAsk(
    body,
    files,
    (ev) => {
      if (!localRuns.has(turn)) return
      const cur = assistantState()
      let r = foldEvent(cur.streams[turn] ?? freshReply, ev)
      if (ev.kind === "tool" && ev.text.startsWith(StatusHead) && screen)
        r = { ...r, looked: screenLabel(screen) }
      if (!r.end) {
        set({ streams: { ...cur.streams, [turn]: r } })
        return
      }
      localRuns.delete(turn)
      const streams = { ...cur.streams }
      delete streams[turn]
      const entries = cur.entries.flatMap((e) =>
        e.turn === turn && e.pending
          ? [
              { ...e, pending: false },
              ...settled(r).map((x) => ({ ...x, turn })),
            ]
          : [e]
      )
      set({ entries, streams, busy: localRuns.size > 0, unread: !cur.open })
      if (r.end === "done" && cur.open) {
        const go = splitReply(r.text).go
        if (go && goesByItself(go)) goHandler?.(go)
      }
    },
    screen
  )
  localRuns.set(turn, id)
}

function localStop(turn: string) {
  const id = localRuns.get(turn)
  if (id === undefined) return
  sampleStop(id)
  localRuns.delete(turn)
  const cur = assistantState()
  const r = cur.streams[turn] ?? freshReply
  const streams = { ...cur.streams }
  delete streams[turn]
  const entries = cur.entries.flatMap((e) =>
    e.turn === turn && e.pending
      ? [
          { ...e, pending: false },
          ...settled({ ...r, end: "error", error: "Stopped." }).map((x) => ({
            ...x,
            turn,
          })),
        ]
      : [e]
  )
  set({ entries, streams, busy: localRuns.size > 0 })
}
