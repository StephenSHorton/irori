// Cards gateway (docs/design/v0.4-devices.md §9): the grid, one opened card,
// and the person's answer to a blocker. Like talk.ts, components import from
// here, never from src/gen, and in a plain browser everything runs on the
// sample in sample-cards.ts (answers change it in memory so the buttons can
// be tried).

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react"

import { agentsNowDefault } from "./agents-now"
import { BoardService, Browser, Events } from "./host"
import { checkUploads, uploadOf } from "./uploads"
import {
  AnswerResult,
  AskRepo,
  CardAsk,
  CardMention,
  CardMessage,
  CardResolution,
  CardSummary,
  CardLater,
  CardThreadItem,
  CommentResult,
  DonePage,
  Holding,
  IdleReport,
  UnofferedCard,
  MentionCandidate,
  Told,
  TaskResult,
  ThreadSummary,
  type RefTarget,
  type AgentRef,
  type CardActivity,
  type CardBlocker,
  CardDetail,
  type CardHolder,
  type CardOption,
  type CardPR,
  type CardReview,
  type CardRef,
  type WaitPr,
  type GoneWait,
  type CardSubtask,
  type CardWait,
  type CardWeight,
  LinkKind,
  TaskState,
  type Link,
  type Mention,
  type NewCardInput,
  Step,
  ThreadCard,
} from "./models"
import {
  EventDevicesChanged,
  EventRosterChanged,
  inHost,
  inWails,
  parseTime,
  plainLine,
} from "./irori"
import { safeHref } from "./rich"
import { EventInboxChanged, useLatest } from "./talk"
import {
  NoteItem,
  sampleAgents,
  sampleCards,
  sampleHolder,
  sampleYou,
} from "./sample-cards"
import { sampleAgents as sampleAgentSettings } from "./agents"
import { nameOr, unknownAgent } from "./names"
import { pending, type Pending } from "./pending"
import { spawnLine, type SpawnLine, type SpawnState } from "./spawn-line"

export type {
  AgentRef,
  AnswerResult,
  AskRepo,
  CardAsk,
  CardActivity,
  CardBlocker,
  CardDetail,
  CardMention,
  CardMessage,
  CardHolder,
  CardOption,
  CardPR,
  CardReview,
  CardRef,
  WaitPr,
  GoneWait,
  CardSubtask,
  CardSummary,
  CardLater,
  CardThreadItem,
  CardWait,
  CardWeight,
  CommentResult,
  Link,
  Mention,
  MentionCandidate,
  NewCardInput,
  Step,
  ThreadCard,
  ThreadSummary,
  Told,
}
export { Holding, LinkKind, NoteItem, TaskState }

// EventBoardChanged mirrors the Go constant in board_service.go: it fires on
// every card change.
export const EventBoardChanged = "board:changed"

// The person's comments, asks and blocker answers have no box limit: coord
// holds them to MAX_PERSON_TEXT (512 KiB) and agents hear them clipped with
// a show pointer (card #265).

// BlockerState mirrors the Go constants on CardBlocker.state: open (no
// answer yet), answered (the person answered; it waits on the raiser,
// also after a plain reply from the agent, card #204), replied (an agent
// asked back with what it still needs; back with the person),
// resolved, and closed without an answer.
export const BlockerState = {
  Open: "open",
  Answered: "answered",
  Replied: "replied",
  Resolved: "resolved",
  Closed: "closed",
} as const

// AskState mirrors the Go constants on CardAsk.state (board.md "Asks").
export const AskState = {
  Open: "open",
  Waiting: "waiting",
  Unclaimed: "unclaimed",
  Taken: "taken",
  Handed: "handed",
  Closed: "closed",
} as const

// openAsk says the card is an ask still finding its taker or its outcome:
// the person's prompt to the room, before it became a card.
// holderLineShows says whether a card with steps also shows its holder
// line under the bar (card #16): whenever nobody is busy on it, so a card
// whose agent is idle, offline or gone doesn't look like work in progress.
// A card without steps always shows it in place of the bar.
export function holderLineShows(c: { holder: { state: string } }): boolean {
  return (
    c.holder.state !== Holding.HoldActive &&
    c.holder.state !== Holding.HoldPerson
  )
}

// nobodyOn says the card has nobody live on it: its holder went away, or
// Irori took that holder off and it is open again (card #16).
export function nobodyOn(c: {
  holder: { state: string }
  returned?: AgentRef | null
}): boolean {
  return !!c.returned || c.holder.state === Holding.HoldGone
}

// PeopleRow is one agent in a card's Who's on it (card #490): its
// strongest role, the titles of the open steps it owns, and its waits.
export type PeopleRow = {
  who: AgentRef
  role: "working" | "watching" | "waiting"
  stepTitles: string[]
  waits: CardWait[]
}

// stepTitle is a step's title for the person, never its id (card #490).
export function stepTitle(steps: Step[], id: string): string {
  return plainLine(steps.find((s) => s.id === id)?.text ?? "") || "a step"
}

// peopleRows gives one row per agent (card #490): working (holders, then
// step owners), then watchers, then agents that only wait. A holder that
// also waits shows its wait on its own row, not as a second one.
export function peopleRows(d: {
  working: AgentRef[]
  watching: AgentRef[]
  waits?: CardWait[] | null
  steps: Step[]
}): PeopleRow[] {
  const rows: PeopleRow[] = []
  const at = new Map<string, PeopleRow>()
  const add = (who: AgentRef, role: PeopleRow["role"]) => {
    const r = at.get(who.key)
    if (!r) {
      const row: PeopleRow = { who, role, stepTitles: [], waits: [] }
      at.set(who.key, row)
      rows.push(row)
      return row
    }
    if (r.role === "working" && role === "working") {
      r.who = {
        ...r.who,
        assigned: r.who.assigned || who.assigned,
        waiting: r.who.waiting || who.waiting,
        spawned: r.who.spawned || who.spawned,
        handoff: r.who.handoff || who.handoff,
        steps: [...(r.who.steps ?? []), ...(who.steps ?? [])],
      }
    }
    return r
  }
  for (const w of d.working) add(w, "working")
  for (const w of d.watching) add(w, "watching")
  for (const w of d.waits ?? []) add(w.who, "waiting").waits.push(w)
  for (const r of rows) {
    const ids = [...new Set(r.who.steps ?? [])]
    r.stepTitles = ids.map((id) => stepTitle(d.steps, id))
  }
  return rows
}

// deviceVar is a device colour's CSS variable (card #22); "" for none.
export function deviceVar(color?: string | null): string {
  return color ? `var(--device-${color})` : ""
}

// DeviceChip is one device in the grid's device filter.
export type DeviceChip = { id: string; name: string; color: string }

// deviceChips are the devices the cards' agents run on, for the filter row
// (card #22): only once there is more than one, since a lone device's
// chip would filter nothing. A colour is set only while the person has
// more than one device, which is also when a face carries one.
export function deviceChips(
  cards: readonly { working: readonly AgentRef[] }[]
): DeviceChip[] {
  const seen = new Map<string, DeviceChip>()
  for (const c of cards)
    for (const w of c.working)
      if (w.deviceId && w.color && !seen.has(w.deviceId))
        seen.set(w.deviceId, { id: w.deviceId, name: w.where, color: w.color })
  const out = [...seen.values()].sort((a, b) => a.name.localeCompare(b.name))
  return out.length > 1 ? out : []
}

export function openAsk(c: { ask?: CardAsk | null }): boolean {
  return !!c.ask && c.ask.state !== AskState.Closed
}

// askSpawn is what Irori is doing to get an ask nobody took an agent
// (card #739), from its card's spawn state: starting one, its place in a
// queue, or why it can't. null once an agent took it or it closed, or
// while Irori isn't getting it one. Without it a reopened thread whose
// agent had gone read "Nobody took this yet" while its new agent started.
export function askSpawn(
  a: CardAsk,
  spawn?: SpawnState | null
): SpawnLine | null {
  if (
    !spawn ||
    a.state === AskState.Taken ||
    a.state === AskState.Handed ||
    a.state === AskState.Closed
  )
    return null
  return spawnLine(spawn)
}

// askLine is an open ask's line in place of the holder line: how many
// it told, who it waits on, or who took it; while nobody took it and
// Irori gets it an agent, that (card #739).
// A reason it can't start is a long sentence: the line says it short, and
// the card page's queue row (and a tile's title) carries it in full.
export function askLine(a: CardAsk, spawn?: SpawnState | null): string {
  const sp = askSpawn(a, spawn)
  if (sp?.tone === "warn") return "Can't start an agent"
  if (sp) return sp.text.charAt(0).toUpperCase() + sp.text.slice(1)
  switch (a.state) {
    case AskState.Taken:
      return `${a.taker ? displayName(a.taker) : "An agent"} took it`
    case AskState.Handed:
      return `Handed to ${a.taker ? displayName(a.taker) : "an agent"}`
    case AskState.Waiting:
      return `Waiting for ${plainLine(a.owner) || "the device it was posted from"} to hand it out`
    case AskState.Unclaimed:
      return "Nobody took this yet"
    case AskState.Closed:
      switch (a.outcome?.kind) {
        case "edit":
          return `Changed ${a.outcomeRef || "a card"}: ${plainLine(a.outcome.note)}`
        case "answered":
          return `Answered: ${plainLine(a.outcome.note)}`
      }
      return `Closed: ${plainLine(a.outcome?.note ?? "")}`
  }
  if (a.told === 0) return "Nobody was free to hear it"
  return `Told ${a.told} ${a.told === 1 ? "agent" : "agents"}; nobody took it yet`
}

// ---- sample mode ------------------------------------------------------------

let sample: CardDetail[] | null = null
const sampleListeners = new Set<() => void>()

function sampleState(): CardDetail[] {
  if (!sample) {
    sample = sampleCards(new Date())
    sample = [...sampleThreadSeed(new Date()), ...sample]
  }
  return sample
}

// ---- threads (card #116) ------------------------------------------------------

// sampleThreadIds are the sample's threads: cards in the sample state (so
// a thread opens like a card) kept off the grid, as BoardService.Cards
// keeps the real ones.
const sampleThreadIds = new Set<string>()
const sampleThreadState = new Map<string, string>()
// How many replies the person has read in each sample thread (card #719).
const sampleThreadRead = new Map<string, number>()
// sampleThreadBoard is the shared board a sample thread was started on.
const sampleThreadBoard = new Map<string, string>()
// sampleThreadAgentsNow is a sample thread's "Agents answer now" (card
// #391) once it differs from on.
const sampleThreadAgentsNow = new Map<string, boolean>()

// sampleThread is one of the sample's threads: an ask card (ask.thread
// in the app), so a thread opens in the ask card view (card #780).
// extra makes one real-looking: an agent on it, its steps.
function sampleThread(
  id: string,
  short: string,
  text: string,
  at: string,
  thread: CardThreadItem[],
  state: string,
  extra: Partial<CardDetail> = {}
): CardDetail {
  sampleThreadIds.add(id)
  sampleThreadState.set(id, state)
  const working = extra.working ?? []
  const taker = working[0] ?? null
  return CardDetail({
    id,
    short,
    shortWas: "",
    title: text.split("\n")[0].slice(0, 80),
    goal: text,
    doneWhen: "",
    note: "",
    state: state === "resolved" ? TaskState.Completed : TaskState.Submitted,
    repoKey: "",
    scope: "mine",
    scopeLabel: "My devices",
    steps: [],
    blockers: [],
    prs: [],
    links: [],
    comments: [],
    watching: [],
    progress: { done: 0, total: 0 },
    openBlockers: 0,
    waiting: 0,
    blockerQuestion: "",
    recent: [],
    threadEarlier: 0,
    historyTrimmed: false,
    updatedAt: at,
    talking: [],
    ...extra,
    working,
    holder: sampleHolder(working),
    thread,
    ask: CardAsk({
      state:
        state === "resolved"
          ? AskState.Closed
          : taker
            ? AskState.Taken
            : state === "handed"
              ? AskState.Handed
              : AskState.Open,
      told: 3,
      owner: "Macbook Pro (this one)",
      wake: true,
      taker,
      outcome: null,
      outcomeRef: "",
    }),
  })
}

// sampleThreadAsk closes or reopens a sample thread's ask with it, as
// Resolve and Reopen do in coord (card #780): the card view follows.
function sampleThreadAsk(id: string, closed: boolean) {
  sample = sampleState().map((c) =>
    c.id !== id || !c.ask
      ? c
      : {
          ...c,
          state: closed ? TaskState.Completed : TaskState.Submitted,
          ask: CardAsk({
            ...c.ask,
            state: closed
              ? AskState.Closed
              : c.ask.taker
                ? AskState.Taken
                : AskState.Open,
          }),
        }
  )
}

// sampleThreadSeed is the preview's threads: one sample-heron answered,
// one its named agent heard, one handed to an idle agent, one resolved.
function sampleThreadSeed(now: Date): CardDetail[] {
  const ago = (s: number) => new Date(now.getTime() - s * 1000).toISOString()
  sampleThreadRead.set("sample-thread-6", 1)
  return [
    sampleThread(
      "sample-thread-1",
      "20",
      "Is #3 still moving? And did anyone finish the release notes (#8)?",
      ago(120),
      [
        CardThreadItem({
          id: "sample-thread-1-asked",
          kind: "event",
          at: ago(120),
          who: sampleYou,
          text: "You asked the room",
        }),
        CardThreadItem({
          id: "sample-thread-1-took",
          kind: "event",
          at: ago(100),
          who: sampleAgents.heron,
          text: "sample-heron took it",
        }),
        CardThreadItem({
          id: "sample-thread-1-a",
          kind: "comment",
          at: ago(60),
          who: sampleAgents.heron,
          text: "#3 is waiting on you: it needs your call on the lobby list (Steam or our relay). #8 is half done; I'll finish it after the build. Where the builds go was settled in #26, and the notes export (#5) is done.",
        }),
        // A long unbroken line (a path, a log key) must wrap inside the
        // Threads column, never widen it (card #215).
        CardThreadItem({
          id: "sample-thread-1-b",
          kind: "comment",
          at: ago(30),
          who: sampleAgents.heron,
          text: "The build log is at builds/release/mac-universal/2026-11-01T18-37-38Z/irori-desktop-aarch64-apple-darwin-signed-notarized-stapled.log and the failing key was RELEASE_NOTES_EXPORT_PIPELINE_STAGE_THREE_MISSING_ARTIFACT_MANIFEST.",
          attachments: [
            {
              sha256: "7".repeat(64),
              name: "release-notes-draft.png",
              size: 41870,
              mime: "image/png",
              device: sampleYou.deviceId ?? "",
            },
          ],
        }),
      ],
      "answered",
      // An agent on it, with its steps (card #780): the ask view's
      // Progress and Who's on it have something to show.
      {
        working: [sampleAgents.heron],
        steps: [
          Step({ id: "s1", text: "Check #3's blocker", done: true }),
          Step({
            id: "s2",
            text: "Finish the release notes (#8)",
            doing: true,
          }),
          Step({ id: "s3", text: "Post the build log" }),
        ],
        progress: { done: 1, total: 3 },
      }
    ),
    // Answered and opened since (card #719): not in Talk's count, no dot.
    sampleThread(
      "sample-thread-6",
      "28",
      "Which port does the preview use?",
      ago(300),
      [
        CardThreadItem({
          id: "sample-thread-6-a",
          kind: "comment",
          at: ago(240),
          who: sampleAgents.otter,
          text: "The first free one from 9246 up; it writes it to frontend/.preview-url.",
        }),
      ],
      "answered"
    ),
    sampleThread(
      "sample-thread-2",
      "21",
      "Why is the Mac build slower than the PC's?",
      ago(20),
      [],
      "waiting"
    ),
    sampleThread(
      "sample-thread-3",
      "25",
      "Can someone check why the lobby list is empty after a restart?",
      ago(90),
      [],
      "handed"
    ),
    // Nobody with Offer it cards on was free: Irori starts an agent for
    // it (card #470).
    sampleThread(
      "sample-thread-5",
      "27",
      "What did we ship this week?",
      ago(10),
      [],
      "waiting"
    ),
    // Card #739: answered by an agent that has gone since, resolved,
    // reopened, and written on. Nobody live heard the follow-up; Irori
    // starts a new agent for it, and the thread says so while it starts.
    {
      ...sampleThread(
        "sample-thread-6",
        "28",
        "Can we add shortcuts to the app, like Cmd+[ to go back?",
        ago(600),
        [
          CardThreadItem({
            id: "sample-thread-6-a",
            kind: "comment",
            at: ago(540),
            who: { ...sampleAgents.heron, status: "gone", live: false },
            text: "Yes: #733 adds Back/Forward, Cmd+1–5 and Cmd+B, and #734 a terminal panel on Ctrl+`.",
          }),
          CardThreadItem({
            id: "sample-thread-6-b",
            kind: "comment",
            at: ago(15),
            who: sampleYou,
            text: "we're doing equivalent windows shortcuts right?",
            unheard: true,
          }),
        ],
        "waiting"
      ),
      ask: CardAsk({
        state: "unclaimed",
        told: 0,
        owner: "",
        wake: true,
        outcomeRef: "",
      }),
      starting: "Macbook Pro (this one)",
    } as CardDetail,
    sampleThread(
      "sample-thread-4",
      "26",
      "Where do the release builds go now?",
      ago(3600),
      [],
      "resolved"
    ),
  ]
}

// sampleThreads is BoardService.Threads on the sample; null is ThreadsAll.
function sampleThreads(boardId: string | null = ""): ThreadSummary[] {
  const all = sampleState().filter(
    (c) =>
      sampleThreadIds.has(c.id) &&
      (boardId === null || (sampleThreadBoard.get(c.id) ?? "") === boardId)
  )
  const rows = all.map((c) => {
    const replies = c.thread.filter((it) => it.kind === "comment")
    const last = replies[replies.length - 1]
    const agents = replies.filter((it) => it.who.key !== sampleYou.key)
    const state = sampleThreadState.get(c.id) ?? "waiting"
    // Unread (card #719): an agent replied after what the person read and
    // after their own newest reply.
    const seen = Math.max(
      sampleThreadRead.get(c.id) ?? 0,
      replies.findLastIndex((it) => it.who.key === sampleYou.key) + 1
    )
    const unread =
      state !== "resolved" &&
      replies.slice(seen).some((it) => it.who.key !== sampleYou.key)
    return ThreadSummary({
      id: c.id,
      short: c.short,
      title: c.title,
      text: c.goal,
      // A follow-up nobody live heard waits on an agent (card #739).
      state:
        state === "resolved"
          ? "resolved"
          : agents.length && c.id !== "sample-thread-6"
            ? "answered"
            : state,
      told: c.id === "sample-thread-1" ? [sampleAgents.heron] : [],
      answeredBy: agents
        .map((it) => it.who)
        .filter((w, i, a) => a.findIndex((x) => x.key === w.key) === i),
      replies: replies.length,
      lastBy: last?.who,
      lastText: last?.text ?? "",
      // Thread 1 made a card with a long title: its chip truncates
      // inside the column (card #215).
      cards:
        c.id === "sample-thread-1"
          ? [
              ThreadCard({
                id: "sample-card-3",
                short: "3",
                title:
                  "Lobby list: pick Steam or our own relay for the public lobby browser and wire it into the join flow",
                state: "working",
              }),
            ]
          : [],
      at: c.updatedAt,
      updatedAt: last?.at ?? c.updatedAt,
      agentsNow: sampleThreadAgentsNow.get(c.id) ?? true,
      spaceId: sampleThreadBoard.get(c.id),
      unread,
      starting:
        c.id === "sample-thread-5" || c.id === "sample-thread-6"
          ? "Macbook Pro (this one)"
          : undefined,
    })
  })
  const t = (s: ThreadSummary) => Date.parse(s.updatedAt) || 0
  const open = rows
    .filter((r) => r.state !== "resolved")
    .sort((a, b) => t(b) - t(a))
  const done = rows
    .filter((r) => r.state === "resolved")
    .sort((a, b) => t(b) - t(a))
  return [...open, ...done]
}

// threads is the Threads panel's side of the Board (card #116): the
// header box starts one, the panel lists them, the person resolves them.
// Replies go through cards.comment, like any card's.
// boardId (card #134) is a shared board's: its threads, started from its
// page, are in its log; "" is your own board.
export const threads = {
  async list(boardId = ""): Promise<ThreadSummary[]> {
    if (inWails()) return (await BoardService.Threads(boardId)) ?? []
    return sampleThreads(boardId)
  },

  // listAll is every thread on this device, each with its spaceId (card
  // #558): Talk filters them by the pick, as it does channels.
  async listAll(): Promise<ThreadSummary[]> {
    if (inWails()) return (await BoardService.ThreadsAll()) ?? []
    return sampleThreads(null)
  },

  // start is the header box's message as a thread: the agents on the
  // cards it names (#12) and the ones it @mentions hear it, and at once
  // one idle agent or Irori's own agent gets it (card #512).
  // files (picked, dropped or pasted) go on it as an ask's do (card
  // #124). boardId starts it on that shared board (card #134): no
  // mentions or files there. agentsNow is the box's "Agents answer now"
  // (card #391); null: the default for its mentions.
  async start(
    text: string,
    mentions: Mention[] = [],
    files: File[] = [],
    boardId = "",
    agentsNow: boolean | null = null
  ): Promise<TaskResult> {
    checkUploads(files)
    if (inWails()) {
      if (boardId && files.length)
        throw new Error(
          "files go with a thread in Personal only; in a shared space, attach them on a card"
        )
      const res = files.length
        ? await BoardService.StartThreadWithFiles(
            text,
            mentions,
            await Promise.all(files.map(uploadOf)),
            agentsNow
          )
        : await BoardService.StartThread(text, mentions, boardId, agentsNow)
      rememberStarted(boardId, startedRow(res))
      return res
    }
    const t = text.trim()
    if (!t) throw new Error("coord: invalid input: an empty comment")
    const cs = sampleState()
    const top = Math.max(0, ...cs.map((c) => parseInt(c.short, 10) || 0))
    const id = `sample-thread-${Date.now()}`
    sample = [
      sampleThread(
        id,
        String(top + 1),
        t,
        new Date().toISOString(),
        [],
        "waiting"
      ),
      ...cs,
    ]
    if (boardId) sampleThreadBoard.set(id, boardId)
    sampleThreadAgentsNow.set(id, agentsNow ?? agentsNowDefault(mentions))
    sampleChanged()
    return TaskResult({
      task: { id, short: String(top + 1) } as TaskResult["task"],
      told: Told({ to: [], mode: "wake" }),
    })
  },

  // setAgentsNow turns an open thread's "Agents answer now" on or off
  // (card #391).
  async setAgentsNow(id: string, on: boolean): Promise<void> {
    if (inWails()) return BoardService.ThreadAgentsNow(id, on)
    sampleThreadAgentsNow.set(id, on)
    sampleChanged()
  },

  // read marks what is in an open thread now as read (card #719): Talk's
  // count drops it until an agent replies again.
  async read(id: string): Promise<void> {
    if (inWails()) return BoardService.ReadThread(id)
    const n = sampleState()
      .find((c) => c.id === id)
      ?.thread.filter((it) => it.kind === "comment").length
    if (n === undefined || sampleThreadRead.get(id) === n) return
    sampleThreadRead.set(id, n)
    sampleChanged()
  },

  async resolve(id: string): Promise<void> {
    if (inWails()) return BoardService.ResolveThread(id)
    sampleThreadState.set(id, "resolved")
    sampleThreadAsk(id, true)
    sampleChanged()
  },

  // reopen brings a resolved thread back (card #169): live again, the
  // person's next reply going to the agents that answered.
  async reopen(id: string): Promise<void> {
    if (inWails()) return BoardService.ReopenThread(id)
    sampleThreadState.set(id, "answered")
    sampleThreadAsk(id, false)
    sampleChanged()
  },
}

// cardRef is what a #ref names across every board on this device, done
// cards and old threads too (card #144), or null.
export async function cardRef(r: string): Promise<RefTarget | null> {
  if (inWails()) return (await BoardService.CardRef(r)) ?? null
  return sampleCardRef(r)
}

// sampleCardRef is BoardService.CardRef on the sample.
export function sampleCardRef(r: string): RefTarget | null {
  const m = /^([a-z0-9_-]*)#(\d+[a-z]*)$/i.exec(r.trim())
  if (!m) return null
  const tag = m[1].toLowerCase()
  const hit = sampleState().filter(
    (c) => c.short === m[2] && (c.tag ?? "") === tag
  )
  if (hit.length !== 1) return null
  const c = hit[0]
  const thread = sampleThreadIds.has(c.id)
  return {
    id: c.id,
    short: c.short,
    title: c.title,
    thread,
    done: thread
      ? sampleThreadState.get(c.id) === "resolved"
      : c.state === TaskState.Completed ||
        c.state === TaskState.Failed ||
        c.state === TaskState.Canceled,
    state: c.state,
    blocked:
      (c.openBlockers > 0 && !(c.ready && c.openBlockers === 1)) ||
      c.state === TaskState.InputRequired,
    ready: !!c.ready && c.openBlockers === 1,
    progress: c.progress,
  }
}

// refCache keeps each looked-up ref until the board changes, so a long
// thread full of "#12" asks once per change; a minute at most, as a net.
const refCache = new Map<string, { at: number; v: Promise<RefTarget | null> }>()
const RefTTL = 60_000

// refGen counts board changes while some chip watches (card #343): each
// one drops the cache, and every useCardRef asks again, so a looked-up
// card's chip moves with the card, no reload. One subscription for all.
let refGen = 0
const refWatchers = new Set<() => void>()
let refOff: (() => void) | null = null
function watchRefs(fn: () => void): () => void {
  refWatchers.add(fn)
  refOff ??= subscribe(() => {
    refCache.clear()
    refGen++
    for (const w of refWatchers) w()
  })
  return () => {
    refWatchers.delete(fn)
    if (!refWatchers.size && refOff) {
      refOff()
      refOff = null
    }
  }
}

// useCardRef is cardRef(r) for a component: undefined while it first
// loads, then the target or null, fresh on every board change (the last
// answer stays while the next loads). Off (r "") it asks nothing.
export function useCardRef(r: string): RefTarget | null | undefined {
  const [got, setGot] = useState<{ r: string; v: RefTarget | null } | null>(
    null
  )
  const [gen, setGen] = useState(refGen)
  useEffect(() => {
    if (!r) return
    return watchRefs(() => setGen(refGen))
  }, [r])
  useEffect(() => {
    if (!r) return
    let live = true
    let e = refCache.get(r)
    if (!e || Date.now() - e.at > RefTTL) {
      e = { at: Date.now(), v: cardRef(r).catch(() => null) }
      refCache.set(r, e)
    }
    void e.v.then((v) => live && setGot({ r, v }))
    return () => {
      live = false
    }
  }, [r, gen])
  return got && got.r === r ? got.v : undefined
}

// RefStatus is what a #ref's chip says of its card (card #343): a word
// and a dot in the card's status colour, and its steps when it has any.
export type RefTone = "info" | "destructive" | "success" | "warning" | "slate"
export type RefStatus = {
  label: string
  tone: RefTone
  progress: { done: number; total: number } | null
}

// refStatus reads a card (live or looked up) or a thread as its chip:
// a thread open or resolved; a card blocked on the person, in review,
// done, failed, canceled, working or open, as its page's pill would.
export function refStatus(x: {
  thread: boolean
  done: boolean
  state: string
  blocked: boolean
  ready?: boolean
  working?: boolean
  progress?: { done: number; total: number }
}): RefStatus {
  const progress =
    !x.thread && x.progress && x.progress.total > 0 ? x.progress : null
  const s = (label: string, tone: RefTone): RefStatus => ({
    label,
    tone,
    progress,
  })
  if (x.thread)
    return x.done ? s("Resolved", "slate") : s("Open thread", "info")
  if (x.state === TaskState.Completed) return s("Done", "success")
  if (x.state === TaskState.Failed) return s("Failed", "warning")
  if (x.state === TaskState.Canceled) return s("Canceled", "slate")
  // Done but for your word (card #112): green, as its pill.
  if (x.ready) return s("Ready", "success")
  if (
    x.blocked ||
    x.state === TaskState.InputRequired ||
    x.state === TaskState.Blocked
  )
    return s("Blocked", "destructive")
  if (x.state === TaskState.InReview) return s("In review", "warning")
  if (x.state === TaskState.Working && x.working !== false)
    return s("Working", "info")
  return s("Open", "slate")
}

// cardStatus is refStatus for a card the window holds.
export function cardStatus(c: CardSummary): RefStatus {
  return refStatus({
    thread: false,
    done: over(c),
    state: c.state,
    blocked:
      (c.openBlockers > 0 && !(c.ready && c.openBlockers === 1)) ||
      c.state === TaskState.InputRequired,
    ready: !!c.ready && c.openBlockers === 1,
    working: c.working.length > 0,
    progress: c.progress,
  })
}

// A thread the person just started shows at once (card #551), from what
// StartThread answered: on a slow window the list's next read can be
// seconds off, and Talk opens the new thread by finding it in the list.
// The row stands in until a read begun after it was remembered: that read
// is the truth, with the row in it or not.
export type StartedThread = { seq: number; boardId: string; row: ThreadSummary }

let startedThreads: StartedThread[] = []
let startedSeq = 0
const startedListeners = new Set<() => void>()

function rememberStarted(boardId: string, row: ThreadSummary) {
  startedThreads = [...startedThreads, { seq: ++startedSeq, boardId, row }]
  for (const fn of startedListeners) fn()
}

// wasStarted says StartThread made id in this window (card #780): it is a
// thread even before a list read has it.
export function wasStarted(id: string): boolean {
  return startedThreads.some((s) => s.row.id === id)
}

function onStarted(fn: () => void): () => void {
  startedListeners.add(fn)
  return () => startedListeners.delete(fn)
}

// startedRow is the list's row for a thread StartThread just made. It has
// no replies yet; who heard it comes with the next read.
export function startedRow(res: TaskResult): ThreadSummary {
  const t = res.task
  return ThreadSummary({
    id: t.id,
    short: t.short ?? "",
    title: t.title,
    text: t.objective,
    state: t.ask?.taker ? "handed" : "waiting",
    at: t.createdAt,
    updatedAt: t.updatedAt,
    private: !!t.ask?.board,
    agentsNow: !!t.ask?.wake,
  })
}

// withStarted is rows, a read of boardId's threads begun at seq asOf, with
// the threads started since on top, newest first.
// A null boardId is every board's (ThreadsAll, card #780).
export function withStarted(
  rows: ThreadSummary[],
  asOf: number,
  boardId: string | null,
  started: StartedThread[]
): ThreadSummary[] {
  const fresh = started
    .filter(
      (s) =>
        s.seq > asOf &&
        (boardId === null || s.boardId === boardId) &&
        !rows.some((r) => r.id === s.row.id)
    )
    .map((s) => s.row)
    .reverse()
  return fresh.length ? [...fresh, ...rows] : rows
}

// useThreads is the Threads panel's rows, fresh on every card change.
export function useThreads(boardId = ""): {
  threads: ThreadSummary[]
  // True until the first read lands (card #598), never on a reload.
  loading: boolean
  error: string | null
} {
  const load = useCallback(async () => {
    const asOf = startedSeq
    return { rows: await threads.list(boardId), asOf }
  }, [boardId])
  const { value, loading, error, reload } = useLatest(
    load,
    { rows: [] as ThreadSummary[], asOf: startedSeq },
    "threads"
  )
  useFresh(reload, error, true)
  const started = useSyncExternalStore(onStarted, () => startedThreads)
  const list = useMemo(
    () => withStarted(value.rows, value.asOf, boardId, started),
    [value, boardId, started]
  )
  return { threads: list, loading, error }
}

// useAllThreads is every thread on this device, fresh on every card change.
export function useAllThreads(): {
  threads: ThreadSummary[]
  // True until the first read lands (card #598), never on a reload.
  loading: boolean
  error: string | null
} {
  const load = useCallback(async () => {
    const asOf = startedSeq
    return { rows: await threads.listAll(), asOf }
  }, [])
  const { value, loading, error, reload } = useLatest(
    load,
    { rows: [] as ThreadSummary[], asOf: startedSeq },
    "threads-all"
  )
  useFresh(reload, error, true)
  // A thread just started on any board is in it at once (card #780): a
  // board's Threads panel and its ask box open it in Talk, which finds it
  // here, before the next read has it.
  const started = useSyncExternalStore(onStarted, () => startedThreads)
  const list = useMemo(
    () => withStarted(value.rows, value.asOf, null, started),
    [value, started]
  )
  // A thread's read marker is a Talk read cursor (card #719): reading one
  // here or on another of the person's devices fires talk:changed
  // (conversations.ts's EventTalkChanged), not board:changed.
  useEffect(() => {
    if (!inWails()) return
    return Events.On("talk:changed", () => void reload())
  }, [reload])
  return { threads: list, loading, error }
}

function sampleChanged() {
  for (const fn of sampleListeners) fn()
}

function sampleCard(id: string): CardDetail {
  const c = sampleState().find((x) => x.id === id)
  if (!c) throw new Error(`coord: no such task or lease: task "${id}"`)
  return c
}

function summary(c: CardDetail): CardSummary {
  return CardSummary({
    id: c.id,
    short: c.short,
    shortWas: c.shortWas,
    title: c.title,
    state: c.state,
    repoKey: c.repoKey,
    scope: c.scope,
    scopeLabel: c.scopeLabel,
    board: c.board,
    ours: c.ours,
    tag: c.tag,
    spaceBoard: c.spaceBoard,
    progress: c.progress,
    openBlockers: c.openBlockers,
    blockerQuestion: c.blockerQuestion,
    ready: c.ready,
    where: c.where,
    leftOff: c.leftOff,
    waiting: c.waiting,
    waitingOn: c.waitingOn,
    waitOn: c.waitOn,
    waitFull: c.waitFull,
    waitQuestion: c.waitQuestion,
    waitYou: c.waitYou,
    waitTask: c.waitTask,
    waitLoop: c.waitLoop,
    waitStuck: c.waitStuck,
    cantStart: c.cantStart,
    noAgent: c.noAgent,
    starting: c.starting,
    atCap: c.atCap,
    queuePlace: c.queuePlace,
    usageLimit: c.usageLimit,
    waitPr: c.waitPr,
    goneWait: c.goneWait,
    working: c.working,
    watching: c.watching,
    holder: c.holder,
    updatedAt: c.updatedAt,
    returned: c.returned,
    stale: c.stale,
    devices: c.devices,
    accent: c.accent,
    talking: c.talking,
    ask: c.ask,
    parent: c.parent,
    subtasks: c.subtasks,
    subtaskBlocked: c.subtaskBlocked,
    pinned: c.pinned,
    pinRank: c.pinRank,
    pos: c.pos,
    owner: c.owner,
    later: c.later,
    order: c.order,
    weight: c.weight,
    // As crates/window's search_text (card #171), less the creator.
    searchText: [c.goal, c.doneWhen, ...c.steps.map((s) => s.text)]
      .map((w) => w.trim())
      .filter(Boolean)
      .join("\n"),
  })
}

// sampleNotYet is Not yet on the sample (card #570): it reads until the
// way the host does, roughly, so the popover can be tried.
function sampleNotYet(id: string, until: string, why: string) {
  const c = sampleCard(id)
  const u = until.trim()
  const by = "You"
  const reason = why.trim() || "You held it back"
  let later: CardLater
  if (!u)
    later = CardLater({ why: reason, back: "word", line: "when you say", by })
  else if (/#|^\d+$/.test(u)) {
    // Numbers are per board (card #558): a bare one means the card's own.
    const ref = u.includes("#") ? u : `${c.tag ?? ""}#${u}`
    const hit =
      sampleCardRef(ref.startsWith("#") ? `${c.tag ?? ""}${ref}` : ref) ??
      sampleCardRef(ref)
    const k = hit && sampleCard(hit.id)
    if (!k) throw new Error(`coord: not found: no card ${u}`)
    if (k.id === c.id)
      throw new Error("coord: invalid input: a card can't wait on itself")
    later = CardLater({
      why: reason,
      back: "card",
      line: `when ${k.tag ?? ""}#${k.short} is done`,
      by,
      card: k.id,
    })
  } else if (/^v?\d+\.\d/.test(u))
    later = CardLater({
      why: reason,
      back: "version",
      line: `when this Mac runs v${u.replace(/^v/, "")}`,
      by,
    })
  else if (
    /^(\d{4}-\d\d-\d\d|\d{1,2}\s+[a-z]{3,}|[a-z]{3,}\s+\d{1,2})$/i.test(u)
  )
    later = CardLater({ why: reason, back: "date", line: `on ${u}`, by })
  else
    throw new Error(
      "Hold it until a card (#83), a day (1 Dec) or a version (v0.13.0); left empty, it waits for you"
    )
  c.later = later
  c.state = TaskState.Blocked
  c.working = []
  sampleChanged()
}

// sampleBoardOf finds a sample board by id; lib/boards.ts sets it (it
// imports this file, so this file doesn't import it back).
let sampleBoardOf: (id: string) => SampleBoard | undefined = () => undefined
export function setSampleBoards(fn: (id: string) => SampleBoard | undefined) {
  sampleBoardOf = fn
}

// samplePersonalOf finds one of Personal's sample boards, the same way.
let samplePersonalOf: (id: string) => SampleList | undefined = () => undefined
export function setSamplePersonal(fn: (id: string) => SampleList | undefined) {
  samplePersonalOf = fn
}

// sampleCreate is BoardService.Create on the sample: a new open card,
// handed to the agent picked or to sample-heron (the free one).
function sampleCreate(input: NewCardInput): TaskResult {
  const title = (input.title ?? "").trim()
  if (!title) throw new Error("coord: invalid input: give the card a title")
  const now = new Date().toISOString()
  const cs = sampleState()
  // From a board's page (card #126): on that board, unheld unless picked.
  const board = input.board ? sampleBoardOf(input.board) : undefined
  // The list it was made from (card #558), each with its own numbers.
  const list = board
    ? board.boards?.find((x) => x.boardId === input.spaceBoard)
    : input.spaceBoard
      ? samplePersonalOf(input.spaceBoard)
      : undefined
  refuseClosed(list)
  const tag = list?.tag ?? board?.tag ?? ""
  const top = Math.max(
    0,
    ...cs
      .filter((c) => (c.tag ?? "") === tag)
      .map((c) => parseInt(c.short, 10) || 0)
  )
  const to = input.assignTo || (board ? "" : "sample-heron")
  const onBoard = board
    ? {
        scope: "board",
        scopeLabel: board.name,
        board: board.boardId,
        tag,
        spaceBoard: list?.boardId ?? board.boardId,
        onBoard: {
          id: board.boardId,
          name: board.name,
          tag,
          spaceBoard: list?.boardId ?? board.boardId,
          visibleTo: board.people,
          canWithdraw: true,
          canTransfer: true,
        },
      }
    : { board: "", onBoard: null, tag, spaceBoard: list?.boardId ?? "" }
  const c = {
    ...cs[0],
    id: `sample-new-${Date.now()}`,
    short: String(top + 1),
    shortWas: "",
    title,
    goal: (input.objective ?? "").trim(),
    doneWhen: (input.doneWhen ?? "").trim(),
    note: "",
    state: TaskState.Submitted,
    repoKey: input.repoKey ?? "",
    steps: [],
    blockers: [],
    prs: [],
    links: [],
    comments: [],
    working: [],
    watching: [],
    progress: { done: 0, total: 0 },
    openBlockers: 0,
    waiting: 0,
    blockerQuestion: "",
    recent: [],
    thread: [
      CardThreadItem({
        id: "sample-new-post",
        kind: "event",
        at: now,
        who: sampleYou,
        text: "made the card",
      }),
    ],
    threadEarlier: 0,
    historyTrimmed: false,
    updatedAt: now,
    ask: null,
    ...onBoard,
  } as CardDetail
  sample = [c, ...cs]
  sampleChanged()
  return TaskResult({
    task: { id: c.id } as TaskResult["task"],
    told: Told(to ? { to: [to], mode: "wake" } : {}),
  })
}

// sampleMentions are the person's mentions as the sample's thread shows
// them: the agent as the sample knows it, gone if it left (or isn't one).
function sampleMentions(mentions: readonly Mention[]): CardMention[] {
  const agents = Object.values(sampleAgents)
  return mentions.map((m) => {
    const who = agents.find((a) => a.key === m.key)
    return CardMention({
      name: m.name,
      who: who ?? {
        key: m.key,
        name: m.name,
        status: "gone",
        live: false,
        where: "",
      },
      gone: !who || who.status === "gone",
    })
  })
}

// sampleOutcomes is each mention's outcome on the sample: a gone agent
// isn't delivered; the others are woken.
function sampleOutcomes(named: readonly CardMention[]) {
  return named.map((m) => ({
    key: m.who.key,
    name: m.name,
    state: m.gone ? "gone" : "woke",
  }))
}

// sampleCandidates are BoardService.MentionCandidates on the sample.
function sampleCandidates(): MentionCandidate[] {
  return Object.values(sampleAgents).map((who) =>
    MentionCandidate({
      who,
      cwd: who.device
        ? "~/projects/irori"
        : "C:\\Users\\sample\\projects\\example-app",
      repo: who.device ? "irori" : "example-app",
    })
  )
}

// samplePin and sampleMovePin are BoardService.Pin and MovePin on the
// sample: ranks as coord makes them.
function samplePin(id: string, pinned: boolean) {
  const c = sampleCard(id)
  const top = Math.max(
    0,
    ...sampleState()
      .filter((x) => x.pinned)
      .map((x) => x.pinRank ?? 0)
  )
  c.pinned = pinned
  c.pinRank = pinned ? top + 1 : 0
  sampleChanged()
}

// sampleSetWeight is BoardService.SetWeight on the sample (card #963): the
// person's weight, mapped as the preview's Settings › Agents maps it; ""
// clears it.
function sampleSetWeight(id: string, level: string, reason: string) {
  const c = sampleCard(id)
  const why = reason.trim()
  if (why.length > 200)
    throw new Error("coord: invalid input: a weight's reason is 200 at most")
  if (!level) {
    c.weight = null
    sampleChanged()
    return
  }
  const a = sampleAgentSettings()
  const [model, effort] =
    level === "light"
      ? [a.lightModel, a.lightEffort]
      : level === "heavy"
        ? [a.heavyModel, a.heavyEffort]
        : level === "normal"
          ? [a.model, a.effort]
          : ["", ""]
  if (!model)
    throw new Error(`coord: invalid input: weight "${level}" isn't one`)
  c.weight = {
    level,
    reason: why,
    setBy: "person",
    byName: "you",
    model,
    effort,
    at: new Date().toISOString(),
  } satisfies CardWeight
  sampleChanged()
}

function sampleMovePin(id: string, before: string) {
  const list = sampleState()
    .filter((x) => x.pinned && x.id !== id)
    .sort((a, b) => (a.pinRank ?? 0) - (b.pinRank ?? 0))
  const i = list.findIndex((x) => x.id === before)
  const c = sampleCard(id)
  if (!before || i < 0) c.pinRank = (list.at(-1)?.pinRank ?? 0) + 1
  else if (i === 0) c.pinRank = (list[0].pinRank ?? 0) - 1
  else c.pinRank = ((list[i - 1].pinRank ?? 0) + (list[i].pinRank ?? 0)) / 2
  sampleChanged()
}

// sampleCloseAsk is BoardService.CloseAsk on the sample: nothing came of
// it, so it leaves the grid.
function sampleCloseAsk(id: string, note: string) {
  const c = sampleCard(id)
  c.state = TaskState.Canceled
  c.ask = CardAsk({
    ...c.ask,
    state: AskState.Closed,
    outcome: { kind: "none", note: note.trim() || "closed by the person" },
  })
  sample = sampleState().filter((x) => x.id !== id)
  sampleChanged()
}

// sampleClose is BoardService.Close on the sample: the person ends a card
// as done or canceled, and it moves to the Closed filter.
function sampleClose(id: string, state: TaskState, note: string) {
  const c = sampleCard(id)
  c.state = state
  c.note = note.trim()
  c.updatedAt = new Date().toISOString()
  sampleChanged()
}

// sampleCleared is the sample's cleared done cards (card #138).
const sampleCleared = new Set<string>()

// sampleToClear is BoardService.ClearDone's pick on the sample.
function sampleToClear(
  olderThanSecs: number,
  ids: string[],
  scope: string
): string[] {
  const inScope = (c: { board?: string | null }) =>
    !scope || (scope === "own" ? !c.board : c.board === scope)
  const done = sampleState().filter(
    (c) =>
      over(c) &&
      !sampleThreadIds.has(c.id) &&
      !sampleCleared.has(c.id) &&
      inScope(c)
  )
  if (ids.length) return done.filter((c) => ids.includes(c.id)).map((c) => c.id)
  const cut = Date.now() - Math.max(0, olderThanSecs) * 1000
  return done
    .filter((c) => olderThanSecs <= 0 || updated(c) < cut)
    .map((c) => c.id)
}

// sampleDelete is BoardService.Delete on the sample: the card is gone.
function sampleDelete(id: string) {
  sampleCard(id)
  sample = sampleState().filter((x) => x.id !== id)
  sampleChanged()
}

// sampleMoveBoard is BoardService.Move on the sample: the card shows the
// new board and its tag at once, and the sample keeps no numbers per board.
export function sampleMoveBoard(id: string, board: string) {
  const c = sampleCard(id)
  const space = c.board ? sampleBoardOf(c.board) : undefined
  const list = space
    ? space.boards?.find((x) => x.boardId === board)
    : samplePersonalOf(board)
  refuseClosed(list)
  c.tag = list?.tag || space?.tag || ""
  c.spaceBoard = board
  sampleChanged()
}

// sampleRename is BoardService.Rename on the sample (card #779): cards
// and threads alike, as the host's checks.
export function sampleRename(id: string, title: string) {
  const t = title.trim()
  if (!t) throw new Error("coord: a card needs a title")
  if ([...t].length > 200)
    throw new Error("coord: title longer than 200 characters")
  const c = sampleCard(id)
  if (c.title === t) return
  c.title = t
  sampleChanged()
}

// sampleUndoTitle is BoardService.UndoTitle on the sample (card #782):
// the title Irori's rename replaced comes back as the person's own, and
// the line loses its Undo.
export function sampleUndoTitle(id: string) {
  const c = sampleCard(id)
  const line = [...c.thread].reverse().find((it) => it.undoTitle)
  if (!line?.undoTitle) throw new Error("Irori's name is already gone")
  const was = c.title
  c.title = line.undoTitle
  c.thread = [
    ...c.thread.map((it) =>
      it.undoTitle ? { ...it, undoTitle: undefined } : it
    ),
    CardThreadItem({
      id: `${id}-undo-${c.thread.length}`,
      kind: "event",
      at: new Date().toISOString(),
      who: sampleYou,
      text: `You renamed it from “${was}” to “${c.title}”`,
    }),
  ]
  sampleChanged()
}

// sampleUndoWeight is BoardService.UndoWeight on the sample (card #965):
// what Irori replaced comes back as the person's, and the line loses its
// Undo.
export function sampleUndoWeight(id: string) {
  const c = sampleCard(id)
  const line = [...c.thread].reverse().find((it) => it.undoWeight)
  if (!line?.undoWeight) throw new Error("Irori's weight is already gone")
  sampleSetWeight(
    id,
    line.undoWeight === "no weight" ? "" : line.undoWeight,
    ""
  )
  c.thread = c.thread.map((it) =>
    it.undoWeight ? { ...it, undoWeight: undefined } : it
  )
  sampleChanged()
}

// sampleCancel is BoardService.Close as canceled on the sample, with its
// note (card #690: a closed board's open cards).
export function sampleCancel(id: string, note: string) {
  sampleClose(id, TaskState.Canceled, note)
}

// refuseClosed is coord's read-only closed board on the sample (card
// #690): nothing new goes onto one.
function refuseClosed(list: SampleList | undefined) {
  if (list?.closed)
    throw new Error(
      `coord: not allowed: board "${list.tag || list.name}" is closed: reopen it first, or pick another board`
    )
}

// sampleOpenOn is the open cards on one board of a space (or of Personal
// when space is ""), for the sample's Close board (card #690). Only a
// board that isn't the first can close, so a card with no inner board
// never counts.
export function sampleOpenOn(space: string, boardId: string): CardDetail[] {
  return sampleState().filter(
    (c) =>
      (c.board ?? "") === space &&
      c.spaceBoard === boardId &&
      !over(c) &&
      !sampleThreadIds.has(c.id)
  )
}

// sampleBoardGone is a board delete on the sample (card #563): its cards go
// to the default board, shown by its tag.
export function sampleBoardGone(boardId: string, tag: string) {
  for (const c of sampleState()) {
    if (c.spaceBoard !== boardId) continue
    c.spaceBoard = ""
    c.tag = tag
  }
  sampleChanged()
}

// sampleReopen is BoardService.Reopen on the sample: working again when
// someone holds it, else open.
function sampleReopen(id: string) {
  const c = sampleCard(id)
  c.state = c.working.length ? TaskState.Working : TaskState.Submitted
  sampleChanged()
}

// The boards' sample side (boards.ts): what Share to board writes, and
// Withdraw. board is a sample BoardSummary.
type SampleBoard = {
  boardId: string
  name: string
  tag: string
  people: string[]
  boards?: SampleList[]
}

// SampleList is one board inside a sample space or Personal.
type SampleList = {
  boardId: string
  name?: string
  tag: string
  closed?: boolean
}

// strictPass is internal/scrub.Strict for the sample: absolute paths and
// home directories become [path].
function strictPass(s: string): string {
  return s
    .replace(/(^|[^A-Za-z0-9])file:\/\/[^\s"'<>|*?)\]`]*/gi, "$1[path]")
    .replace(/(^|[^A-Za-z0-9])[A-Za-z]:[\\/][^\s"'<>|*?)\]`]*/g, "$1[path]")
    .replace(
      /(^|[\s"'([=,:*`])(?:~|\$HOME)(?:[\\/][^\s"'<>|*?)\]`]*)?/g,
      "$1[path]"
    )
    .replace(
      /(^|[\s"'([=,:*`])\/[A-Za-z0-9._@+-]+\/[^\s"'<>|*?)\]`]*/g,
      "$1[path]"
    )
}

export function samplePreview(
  id: string,
  board: SampleBoard
): SharePreviewShape {
  const c = sampleCard(id)
  if (c.scope === "board")
    throw new Error("coord: invalid input: the card is already in a space")
  if (c.parent)
    throw new Error(
      `coord: invalid input: #${c.short} is a subtask of #${c.parent.short}: share #${c.parent.short} and its subtasks go with it`
    )
  if (over(c))
    throw new Error(
      `coord: bad state: #${c.short} is ${c.state}; share a card that is still in play`
    )
  const dropped: string[] = []
  const clean = (what: string, s: string) => {
    const t = strictPass(s)
    if (t !== s) dropped.push(`a path in the ${what}`)
    return t
  }
  const out = {
    boardId: board.boardId,
    boardName: board.name,
    title: clean("title", c.title),
    goal: clean("goal", c.goal),
    doneWhen: clean("done-when", c.doneWhen),
    steps: c.steps.map((s) => clean("steps", s.text)),
    links: c.links
      .filter((l) => l.kind !== LinkKind.LinkPath)
      .map((l) => l.ref),
    repo: c.repoKey,
    visibleTo: board.people,
    dropped: [] as string[],
    // A parent's open subtasks go with it; ended ones stay (card #180).
    subtasks: (c.subtaskList ?? [])
      .filter((k) => !over(k))
      .map((k) => ({
        taskId: k.id,
        ref: `#${k.short}`,
        title: clean(`#${k.short}'s title`, k.title),
        goal: "",
        doneWhen: "",
        steps: [] as string[],
        links: [] as string[],
        repo: c.repoKey,
      })),
    keptPrivate: (c.subtaskList ?? [])
      .filter((k) => over(k))
      .map((k) => `#${k.short} ${k.title} (${k.state})`),
    hash: "sample",
  }
  if (c.links.some((l) => l.kind === LinkKind.LinkPath))
    dropped.push("a link to a path on this machine")
  out.dropped = [...new Set(dropped)]
  return out
}

// SharePreviewShape is core.SharePreview's fields.
export type SharePreviewShape = {
  boardId: string
  boardName: string
  title: string
  goal: string
  doneWhen: string
  steps: string[]
  links: string[]
  repo: string
  visibleTo: string[]
  dropped: string[]
  subtasks: {
    taskId: string
    ref: string
    title: string
    goal: string
    doneWhen: string
    steps: string[]
    links: string[]
    repo: string
  }[]
  keptPrivate: string[]
  hash: string
}

export function sampleShare(id: string, board: SampleBoard): string {
  const p = samplePreview(id, board)
  const c = sampleCard(id)
  const cs = sampleState()
  const top = Math.max(
    0,
    ...cs
      .filter((x) => x.board === board.boardId)
      .map((x) => parseInt(x.short, 10) || 0)
  )
  const shared = {
    ...c,
    id: `sample-shared-${Date.now()}`,
    short: String(top + 1),
    shortWas: "",
    title: p.title,
    goal: p.goal,
    doneWhen: p.doneWhen,
    steps: c.steps.map((s, i) => ({ ...s, text: p.steps[i] })),
    links: c.links.filter((l) => l.kind !== LinkKind.LinkPath),
    comments: [],
    thread: [],
    scope: "board",
    scopeLabel: board.name,
    board: board.boardId,
    tag: board.tag,
    spaceBoard: board.boardId,
    onBoard: {
      id: board.boardId,
      name: board.name,
      tag: board.tag,
      spaceBoard: board.boardId,
      visibleTo: board.people,
      canWithdraw: true,
      canTransfer: true,
    },
    sharedTo: undefined,
    pinned: false,
  } as CardDetail
  cs.unshift(shared)
  c.state = TaskState.Canceled
  c.note = `Shared to ${board.name} as ${board.tag}#${shared.short} (visible to: ${board.people.join(", ")}). Work it there; this private card is closed.`
  c.sharedTo = {
    boardId: board.boardId,
    boardName: board.name,
    taskId: shared.id,
    ref: `${board.tag}#${shared.short}`,
  }
  sampleChanged()
  return shared.id
}

// sampleTransferOwner is Transfer ownership on the sample (card #432): an
// Accept/Decline item for the other side, as the window's would be.
export function sampleTransferOwner(id: string, to: string): string {
  const c = sampleCard(id)
  if (!c.onBoard)
    throw new Error(
      `coord: invalid input: ${id} is a private card: only a card in a shared space changes owner`
    )
  if (c.blockers.some((b) => !b.resolved && b.transfer))
    throw new Error(
      `coord: conflict: ${c.short} already has an ownership offer or request waiting`
    )
  const now = new Date().toISOString()
  const bid = `b${c.blockers.length + 1}`
  const option = (answer: string) => ({
    answer,
    why: "",
    by: sampleYou,
    at: now,
    recommended: false,
  })
  c.blockers.unshift({
    id: bid,
    question: to ? "Take over this card?" : "Hand this card over?",
    context: to
      ? "Its owner offers it to you. As its owner you decide Done and answer the questions its agents raise."
      : "Another member asks to own it. They would decide Done and answer the questions its agents raise; you would no longer.",
    raisedBy: sampleYou,
    at: now,
    options: [option("Accept"), option("Decline")],
    resolved: null,
    state: "open",
    links: [],
    attachments: [],
    missing: [],
    optionsAside: false,
    transfer: true,
    thread: [],
  } as CardBlocker)
  c.state = TaskState.Blocked
  c.onBoard = { ...c.onBoard, canTransfer: false }
  sampleChanged()
  return bid
}

export function sampleWithdraw(id: string): string {
  const c = sampleCard(id)
  if (!c.onBoard)
    throw new Error(
      `coord: invalid input: ${id} is a private card; cancel it instead`
    )
  c.state = TaskState.Canceled
  c.onBoard = { ...c.onBoard, canWithdraw: false }
  const from = sampleState().find((x) => x.sharedTo?.taskId === id)
  if (from) {
    from.state = from.working.length ? TaskState.Working : TaskState.Submitted
    from.sharedTo = { ...from.sharedTo!, withdrawn: true }
  }
  // Other members hide it; here it goes to Done.
  sampleState().splice(sampleState().indexOf(c), 1)
  sampleChanged()
  return from?.id ?? ""
}

function sampleAnswer(
  id: string,
  blockerId: string,
  option: number,
  text: string,
  wake: boolean,
  mentions: Mention[]
): AnswerResult {
  const c = sampleCard(id)
  if (blockerId === NoteItem) return sampleAnswerNote(c, option, text, wake)
  const named = sampleMentions(option >= 0 ? [] : mentions)
  const b = sampleBlocker(c, blockerId)
  const answer = option >= 0 ? b.options[option]?.answer : text.trim()
  if (!answer) throw new Error("coord: invalid input: an answer needs text")
  const now = new Date().toISOString()
  // As BoardService.Answer: a pick on a permission prompt resolves it
  // (card #682); the agent's device hands it to the waiting hook.
  if (b.permission?.answerable && option >= 0) {
    sampleResolve(c, b, answer, option, "", now)
    c.permissionWait = undefined
    c.updatedAt = now
    sampleChanged()
    return AnswerResult({
      resolved: true,
      told: Told({ to: [], mode: "next_turn" }),
      iroriActs: true,
    })
  }
  // As BoardService.Answer: Done on a done check closes the card (#112).
  if (b.doneCheck && option >= 0) {
    sampleResolve(c, b, answer, option, "", now)
    c.state = TaskState.Completed
    c.ready = undefined
    c.updatedAt = now
    sampleChanged()
    return AnswerResult({
      resolved: true,
      told: Told({ to: [], mode: "next_turn" }),
    })
  }
  b.thread = [
    ...b.thread,
    CardThreadItem({
      id: `sample-answer-${b.id}-${b.thread.length}`,
      kind: "comment",
      at: now,
      who: sampleYou,
      to: b.raisedBy,
      text: answer,
      blockerId: b.id,
      mentions: named.length ? named : undefined,
    }),
  ]
  c.thread = sampleThreadLine(
    c,
    now,
    `You answered “${b.question}”`,
    b.id,
    b.question
  )
  // As BoardService.Answer: the answer waits on the raiser, unless nobody
  // is running to act on it.
  if (!b.raisedBy.live && !c.working.some((w) => w.live)) {
    sampleResolve(c, b, answer, option, "", now)
  } else {
    b.state = BlockerState.Answered
    b.sentTo = b.raisedBy
    b.missing = []
    sampleRecount(c)
  }
  c.updatedAt = now
  sampleChanged()
  return AnswerResult({
    resolved: !!b.resolved,
    told: Told({
      to: [b.raisedBy.key, ...c.working.map((w) => w.key)].filter(
        (k, i, all) => all.indexOf(k) === i
      ),
      mode: wake ? "wake" : "next_turn",
    }),
    mentioned: sampleOutcomes(named),
  })
}

// sampleResolveBlocker is BoardService.ResolveBlocker on the sample: the
// person's latest answer stands, with note beside the check.
function sampleResolveBlocker(
  id: string,
  blockerId: string,
  note: string,
  mentions: Mention[]
): AnswerResult {
  const c = sampleCard(id)
  const b = sampleBlocker(c, blockerId)
  const last = [...b.thread].reverse().find((x) => x.who.key === sampleYou.key)
  if (!last && !note.trim())
    throw new Error("coord: invalid input: the answer needs text")
  const now = new Date().toISOString()
  sampleResolve(
    c,
    b,
    last ? last.text : note.trim(),
    last ? b.options.findIndex((o) => o.answer === last.text) : -1,
    last ? note.trim() : "",
    now
  )
  c.updatedAt = now
  sampleChanged()
  return AnswerResult({
    resolved: true,
    told: Told({ to: c.working.map((w) => w.key), mode: "next_turn" }),
    mentioned: sampleOutcomes(sampleMentions(mentions)),
  })
}

function sampleBlocker(c: CardDetail, blockerId: string): CardBlocker {
  const b = c.blockers.find((x) => x.id === blockerId)
  if (!b) throw new Error(`coord: no such blocker "${blockerId}"`)
  if (b.resolved) throw new Error("coord: conflict: that blocker is resolved")
  return b
}

// sampleResolve resolves b as the person, with its line on the card.
function sampleResolve(
  c: CardDetail,
  b: CardBlocker,
  answer: string,
  option: number,
  note: string,
  at: string
) {
  b.resolved = CardResolution({
    answer,
    option,
    note,
    by: sampleYou,
    at,
    closed: false,
  })
  b.state = BlockerState.Resolved
  b.sentTo = null
  b.missing = []
  const line = note
    ? `You resolved “${b.question}”: ${note}`
    : `You answered “${b.question}”: ${answer}`
  c.recent = [{ at, who: sampleYou, text: line }, ...c.recent].slice(0, 8)
  c.thread = sampleThreadLine(c, at, line)
  // Open items first (oldest first), then resolved ones newest first.
  const open = c.blockers.filter((x) => !x.resolved)
  const done = c.blockers.filter((x) => x.resolved && x !== b)
  c.blockers = [...open, b, ...done]
  sampleRecount(c)
  if (!open.length) c.state = TaskState.Working
}

// sampleRecount is cardSummary's counts: answered items wait on an agent,
// waits on other work and another member's questions are waits (card
// #481), the rest of the open ones on the person.
function sampleRecount(c: CardDetail) {
  const open = c.blockers.filter((x) => !x.resolved)
  const asked = open.filter((x) => x.state !== BlockerState.Answered)
  const needs = asked.filter((x) => !x.waitsOn && !x.decider)
  c.openBlockers = needs.length
  c.waiting = open.length - asked.length
  c.waitingOn = asked.length - needs.length || undefined
  // What the first wait is on, and the loop once a wait in it closed.
  const wait = asked.find((x) => x.waitsOn)
  c.waitOn = wait?.waitsOn?.label
  c.waitQuestion = wait?.question
  c.waitTask = wait?.waitsOn?.taskId
  if (!asked.some((x) => x.waitsOn?.taskId)) {
    const ring = c.waitLoop
    c.waitLoop = undefined
    c.waitStuck = undefined
    if (ring?.length)
      for (const o of sampleState())
        if (o.waitLoop?.some((r) => r.id === c.id)) o.waitLoop = undefined
  }
  c.blockerQuestion = needs[0]?.question ?? ""
  c.ready = needs.some((x) => x.doneCheck) || undefined
  // Card #520: a gone wait leaves once its item closes (Stop waiting).
  if (c.goneWait && !open.some((x) => x.id === c.goneWait?.blockerId))
    c.goneWait = undefined
}

// sampleMessage is BoardService.Message on the sample: a held message's
// text and a message's files are made up, as the sample has no inbox.
function sampleMessage(id: string, msgId: string): CardMessage {
  const it = sampleCard(id).thread.find(
    (x) => x.id === msgId && x.kind === "message"
  )
  if (!it)
    throw new Error(`coord: no such task or lease: no message "${msgId}"`)
  return CardMessage({
    text:
      it.text ||
      "Can you check the save format before I bump the version? The old saves need a migration.",
    files: Array.from(
      { length: it.files ?? 0 },
      (_, i) =>
        `capture-${i + 1}.mp4 (2.1 MB, video/mp4) sha256=${"0".repeat(64)}`
    ),
  })
}

// sampleThreadLine is c's thread with the person's line added at the end.
function sampleThreadLine(
  c: CardDetail,
  at: string,
  text: string,
  blockerId?: string,
  blockerGist?: string
): CardThreadItem[] {
  return [
    ...c.thread,
    CardThreadItem({
      id: `sample-line-${c.thread.length}`,
      kind: "event",
      at,
      who: sampleYou,
      text,
      blockerId,
      blockerGist,
    }),
  ]
}

// sampleAnswerNote is BoardService.answerInput on the sample: a comment to
// the holders, and the card back to working.
function sampleAnswerNote(
  c: CardDetail,
  option: number,
  text: string,
  wake: boolean
): AnswerResult {
  if (c.state !== TaskState.InputRequired)
    throw new Error("coord: conflict: the card isn't waiting on your input")
  if (option !== -1 || !text.trim())
    throw new Error("coord: invalid input: answer in your own words")
  const now = new Date().toISOString()
  c.state = TaskState.Working
  c.note = ""
  c.blockers = c.blockers.filter((x) => x.id !== NoteItem)
  c.openBlockers = 0
  c.blockerQuestion = ""
  c.updatedAt = now
  c.recent = [
    { at: now, who: sampleYou, text: `You answered: ${text.trim()}` },
    ...c.recent,
  ].slice(0, 8)
  c.thread = sampleThreadLine(c, now, `You answered: ${text.trim()}`)
  sampleChanged()
  return AnswerResult({
    resolved: false,
    told: Told({
      to: c.working.map((w) => w.key),
      mode: wake ? "wake" : "next_turn",
    }),
  })
}

// EditState is CardThreadItem.edit (card #105): "can" while no agent has
// read the person's comment (Modify takes it back), "editing" while it is
// held back for the person to edit.
export const EditState = {
  Can: "can",
  Editing: "editing",
} as const

// sampleEdit is Modify, Cancel and Save on the sample's comment.
function sampleEdit(
  id: string,
  commentId: string,
  edit: string,
  text?: string
) {
  const c = sampleCard(id)
  c.thread = c.thread.map((it) =>
    it.id === commentId
      ? { ...it, edit, ...(text === undefined ? {} : { text, edited: true }) }
      : it
  )
  sampleChanged()
}

// sampleComment is BoardService.Comment on the sample: the person's
// comment at the end of the thread, heard by the live agents on the card
// (watchers answer only if asked); with none, it is unheard.
function sampleComment(
  id: string,
  text: string,
  wake: boolean,
  mentions: Mention[]
): CommentResult {
  const c = sampleCard(id)
  const t = text.trim()
  if (!t) throw new Error("coord: invalid input: an empty comment")
  const now = new Date().toISOString()
  const answers = c.working.filter((w) => w.live)
  const watching = c.watching.filter((w) => w.live)
  const named = sampleMentions(mentions)
  const unheard =
    !answers.length && !watching.length && !named.some((m) => !m.gone)
  // A mentioned agent watches the card (never holds it).
  for (const m of named)
    if (
      !m.gone &&
      ![...c.working, ...c.watching].some((w) => w.key === m.who.key)
    )
      c.watching = [...c.watching, m.who]
  c.thread = [
    ...c.thread,
    CardThreadItem({
      id: `sample-comment-${c.thread.length}`,
      kind: "comment",
      at: now,
      who: sampleYou,
      text: t,
      unheard,
      mentions: named.length ? named : undefined,
      edit: unheard ? undefined : EditState.Can,
    }),
  ]
  c.updatedAt = now
  sampleChanged()
  return CommentResult({
    told: Told({
      to: [...answers, ...watching].map((w) => w.key),
      mode: unheard ? "" : wake ? "wake" : "next_turn",
    }),
    answers,
    watching,
    mentioned: sampleOutcomes(named),
  })
}

// ---- actions ----------------------------------------------------------------

// Every action acts as the person at this device.
export const cards = {
  // answer answers one blocker: option is an index into blocker.options,
  // or -1 to answer with text. The answer goes to the raiser and the item
  // stays open until it is resolved, unless nobody is running to act on
  // it. wake defaults to true: the raiser is blocked on the person.
  async answer(
    id: string,
    blockerId: string,
    option: number,
    text: string,
    wake = true,
    mentions: Mention[] = [],
    files: File[] = []
  ): Promise<AnswerResult> {
    checkUploads(files)
    if (inWails()) {
      if (!files.length)
        return BoardService.Answer(id, blockerId, option, text, wake, mentions)
      return BoardService.AnswerWithFiles(
        id,
        blockerId,
        option,
        text,
        wake,
        mentions,
        await Promise.all(files.map(uploadOf))
      )
    }
    return sampleAnswer(id, blockerId, option, text, wake, mentions)
  },

  // resolveBlocker is "Mark resolved": the person's answer stands, with
  // note (optional) beside the check. Each of mentions (picked in the
  // note) is woken and watches the card.
  async resolveBlocker(
    id: string,
    blockerId: string,
    note: string,
    mentions: Mention[] = []
  ): Promise<AnswerResult> {
    if (inWails())
      return BoardService.ResolveBlocker(id, blockerId, note, mentions)
    return sampleResolveBlocker(id, blockerId, note, mentions)
  },

  // comment puts the person's comment on card id's thread. Its holders
  // (else its creator) are asked to answer it on the card and its
  // watchers hear it too; wake (the default) wakes whoever answers now
  // instead of at their next turn. Each of mentions (picked in the box)
  // is woken whatever wake says, and watches the card.
  async comment(
    id: string,
    text: string,
    wake: boolean | "stop" = true,
    mentions: Mention[] = [],
    files: File[] = []
  ): Promise<CommentResult> {
    checkUploads(files)
    if (inWails()) {
      // "stop" (card #207): the holders' running turns are cut.
      const tell = wake === "stop" ? "stop" : wake ? "wake" : "next_turn"
      if (!files.length) return BoardService.Comment(id, text, tell, mentions)
      return BoardService.CommentWithFiles(
        id,
        text,
        tell,
        mentions,
        await Promise.all(files.map(uploadOf))
      )
    }
    return sampleComment(id, text, wake !== false, mentions)
  },

  // askAgain is Wake them on an ask nobody took.
  // startAgent is "Start an agent with this" on the person's comment
  // (card #452): card id goes back on the Board with note in its next
  // agent's brief.
  async startAgent(id: string, note: string): Promise<void> {
    if (inWails()) return BoardService.StartAgent(id, note)
    const c = sampleCard(id)
    c.state = TaskState.Submitted
    sampleChanged()
  },

  // handOn is "Hand to an agent" on an item whose agent is gone, on a
  // card nobody holds (card #485): the card goes back on the Board with
  // what was open said on it, and Irori starts an agent for it (or hands
  // it to the next free one). Returns which.
  async handOn(id: string): Promise<string> {
    if (inWails()) return BoardService.HandOn(id)
    const c = sampleCard(id)
    // Its open items close as it goes (card #520: its Needs you row too).
    const at = new Date().toISOString()
    for (const b of c.blockers.filter((x) => !x.resolved)) {
      b.goneAgent = undefined
      sampleResolve(c, b, "Handed to a new agent", -1, "", at)
    }
    c.state = TaskState.Submitted
    sampleChanged()
    return "It is back on the Board: Irori starts an agent for it."
  },

  async askAgain(id: string): Promise<void> {
    if (inWails()) return BoardService.AskAgain(id)
    const c = sampleCard(id)
    if (c.ask) c.ask = CardAsk({ ...c.ask, state: AskState.Open, wake: true })
    sampleChanged()
  },

  // place keeps where the person dropped card id on the Board's canvas
  // (card #93): in coord, so every device shows the same layout.
  async place(id: string, x: number, y: number): Promise<void> {
    const [rx, ry] = [Math.round(x), Math.round(y)]
    if (inWails()) {
      await BoardService.Place(id, rx, ry)
      return
    }
    sampleCard(id).pos = { x: rx, y: ry }
    sampleChanged()
  },

  // modifyComment is Modify on the person's comment no agent read yet
  // (card #105): it is held back from every agent until saveComment or
  // cancelComment.
  async modifyComment(id: string, commentId: string): Promise<void> {
    if (inWails()) {
      await BoardService.CommentModify(id, commentId)
      return
    }
    sampleEdit(id, commentId, EditState.Editing)
  },

  // cancelComment lets a comment held for editing go out as it was.
  async cancelComment(id: string, commentId: string): Promise<void> {
    if (inWails()) {
      await BoardService.CommentModifyCancel(id, commentId)
      return
    }
    sampleEdit(id, commentId, EditState.Can)
  },

  // saveComment sends the held comment's new text to the agents it was
  // held from.
  async saveComment(
    id: string,
    commentId: string,
    text: string
  ): Promise<void> {
    if (inWails()) {
      await BoardService.CommentModifySave(id, commentId, text)
      return
    }
    if (!text.trim()) throw new Error("coord: invalid input: an empty comment")
    sampleEdit(id, commentId, EditState.Can, text.trim())
  },

  // detail is one card's page data (its blockers, for the canvas).
  async detail(id: string): Promise<CardDetail> {
    if (inWails()) return BoardService.Card(id)
    return { ...sampleCard(id) } as CardDetail
  },

  // pin pins (or unpins) a card for the person: the Pinned group, last.
  async pin(id: string, pinned: boolean): Promise<void> {
    if (inWails()) {
      await BoardService.Pin(id, pinned)
      return
    }
    samplePin(id, pinned)
  },

  // setWeight weighs card id for the person (card #963): "light", "normal"
  // or "heavy", with an optional line on why. Theirs: agents and the
  // assistant can't change it after. "" clears it, handing it back to the
  // agents; until one weighs it again it runs as normal.
  async setWeight(id: string, level: string, reason = ""): Promise<void> {
    if (inWails()) {
      await BoardService.SetWeight(id, level, reason)
      return
    }
    sampleSetWeight(id, level, reason)
  },

  // pause pauses card id or resumes it (card #207): its holders are
  // stopped and their tools refused until Resume. Returns who was told.
  async pause(id: string, paused: boolean, note = ""): Promise<string[]> {
    if (inWails()) return (await BoardService.Pause(id, paused, note)) ?? []
    const c = sampleCard(id)
    c.paused = paused
    sampleChanged()
    return []
  },

  // notYet holds card id back from the agents (card #570) until what
  // until says: a card ("#83"), a day ("1 Dec") or a version ("v0.13.0");
  // empty, until the person says.
  async notYet(id: string, until: string, why = ""): Promise<void> {
    if (inWails()) {
      await BoardService.NotYet(id, until, why)
      return
    }
    sampleNotYet(id, until, why)
  },

  // startNow brings held card id back (card #570): it goes out as if
  // just posted.
  async startNow(id: string): Promise<void> {
    if (inWails()) {
      await BoardService.StartNow(id)
      return
    }
    const c = sampleCard(id)
    if (!c.later) throw new Error(`coord: bad state: #${c.short} isn't held`)
    c.later = null
    c.state = TaskState.Submitted
    c.openBlockers = 0
    c.waitingOn = 0
    c.blockers = c.blockers.filter((b) => b.state !== "open")
    sampleChanged()
  },

  // movePin puts pinned card id in front of pinned card before ("": last).
  async movePin(id: string, before: string): Promise<void> {
    if (inWails()) {
      await BoardService.MovePin(id, before)
      return
    }
    sampleMovePin(id, before)
  },

  // closeAsk ends an ask with nothing coming of it.
  async closeAsk(id: string, note = ""): Promise<void> {
    if (inWails()) {
      await BoardService.CloseAsk(id, note)
      return
    }
    sampleCloseAsk(id, note)
  },

  // close ends a card by hand (card #57): done (completed) or canceled,
  // with an optional note. The person may close any card.
  async close(id: string, state: TaskState, note = ""): Promise<void> {
    if (inWails()) {
      await BoardService.Close(id, state, note)
      return
    }
    sampleClose(id, state, note)
  },

  // remove deletes a card (card #57): it leaves the Board on every device;
  // the signed log keeps it.
  async remove(id: string): Promise<void> {
    if (inWails()) {
      await BoardService.Delete(id)
      return
    }
    sampleDelete(id)
  },

  // rename is the person renaming a card or thread by hand (card #779):
  // the board, the Threads list and its page show it at the next read,
  // and an agent's later rename doesn't land.
  async rename(id: string, title: string): Promise<void> {
    if (inWails()) {
      await BoardService.Rename(id, title)
      return
    }
    sampleRename(id, title)
  },

  // undoTitle is the person's Undo on Irori's name for their thread (card
  // #782): the title it had comes back as theirs, and Irori leaves it.
  async undoTitle(id: string): Promise<void> {
    if (inWails()) {
      await BoardService.UndoTitle(id)
      return
    }
    sampleUndoTitle(id)
  },

  // undoWeight is the person's Undo on Irori's weighing of a card (card
  // #965): what it replaced comes back as theirs, and Irori leaves it.
  async undoWeight(id: string): Promise<void> {
    if (inWails()) {
      await BoardService.UndoWeight(id)
      return
    }
    sampleUndoWeight(id)
  },

  // move puts a card on another board in its space (card #563), with its
  // subtasks. It takes the next number there; its old ref still finds it.
  async move(id: string, board: string): Promise<void> {
    if (inWails()) {
      await BoardService.Move(id, board)
      return
    }
    sampleMoveBoard(id, board)
  },

  // clearDoneCount is how many done cards a clear by age would take
  // (card #138): closed more than olderThanSecs ago, 0 for all of them.
  // scope: "" every card, "own" Home's, else one board's (as DoneCardsIn).
  async clearDoneCount(olderThanSecs: number, scope = ""): Promise<number> {
    if (inWails()) return BoardService.ClearDoneCount(olderThanSecs, scope)
    return sampleToClear(olderThanSecs, [], scope).length
  },

  // clearDone takes done cards out of the app's lists (card #138): the
  // ids given, else by age as clearDoneCount. Agents still find them with
  // search and show. Returns how many it cleared.
  async clearDone(
    olderThanSecs: number,
    ids: string[] = [],
    scope = ""
  ): Promise<number> {
    if (inWails()) return BoardService.ClearDone(olderThanSecs, ids, scope)
    const gone = sampleToClear(olderThanSecs, ids, scope)
    for (const id of gone) sampleCleared.add(id)
    sampleChanged()
    return gone.length
  },

  // reopen brings a finished card back into play, with whoever held it.
  async reopen(id: string): Promise<void> {
    if (inWails()) {
      await BoardService.Reopen(id)
      return
    }
    sampleReopen(id)
  },

  // create is New card (card #116): a card on the person's own board,
  // straight from the form. Irori hands it to one free agent, or to
  // assignTo when the person picked one.
  // files go on the new card as its first comment (card #124).
  async create(input: NewCardInput, files: File[] = []): Promise<TaskResult> {
    checkUploads(files)
    if (inWails())
      return BoardService.Create({
        ...input,
        files: await Promise.all(files.map(uploadOf)),
      })
    return sampleCreate(input)
  },

  // askRepos are the repos the agents are in, for the ask box's chip.
  async askRepos(): Promise<AskRepo[]> {
    if (inWails()) return (await BoardService.AskRepos()) ?? []
    return [
      AskRepo({
        repoKey: "remote:github.com/sample-org/example-app",
        label: "example-app",
      }),
      AskRepo({
        repoKey: "remote:github.com/sample-org/irori",
        label: "irori",
      }),
    ]
  },

  // mentionCandidates are the agents the @ picker offers: every device's
  // roster, gone ones included.
  async mentionCandidates(): Promise<MentionCandidate[]> {
    if (inWails()) return (await BoardService.MentionCandidates()) ?? []
    return sampleCandidates()
  },

  // message opens a message on card id's thread in full: the text the
  // card cut or doesn't carry yet, and its files.
  async message(id: string, msgId: string): Promise<CardMessage> {
    if (inWails()) return BoardService.Message(id, msgId)
    return sampleMessage(id, msgId)
  },

  // openURL opens a link (a PR's page, a link in an agent's text or on a
  // blocker) in the person's browser. Only http(s) with a host: anything
  // else is dropped (rich.ts safeHref).
  async openURL(url: string): Promise<void> {
    if (!/^https?:\/\//i.test(url)) return
    const href = safeHref(url)
    if (!href) return
    if (inHost()) return Browser.OpenURL(href)
    window.open(href, "_blank", "noopener")
  },
}

// ---- hooks ------------------------------------------------------------------

// RetryMs is the wait before re-reading after a failed read: there is no
// poll, so a failure has no next event to recover it.
const RetryMs = 4000

// subscribe re-runs fn on every board:changed (or sample change), and on
// devices:changed: a renamed device changes the cards' "on <device>" lines.
// onCardsChanged calls fn on every card change (board:changed,
// devices:changed; the sample's changes in the preview).
export function onCardsChanged(fn: () => void): () => void {
  return subscribe(fn)
}

// cardsById reads cards by id in one call, as the card hooks share
// (readCards): each id's card, or its error. Ask Irori's follower (card
// #785) reads the threads it posted and the cards made from them.
export function cardsById(ids: string[]): Promise<Map<string, CardSlot>> {
  return readCards(ids)
}

function subscribe(fn: () => void): () => void {
  if (!inWails()) {
    sampleListeners.add(fn)
    return () => sampleListeners.delete(fn)
  }
  const offBoard = Events.OnShown(EventBoardChanged, fn)
  const offDevices = Events.OnShown(EventDevicesChanged, fn)
  return () => {
    offBoard()
    offDevices()
  }
}

// useFresh re-reads on every card change, when the window regains focus,
// and once a few seconds after a failed read. With inbox, on every
// inbox:changed too (coalesced to 4 a second on the Go side): the card
// page's "queued for / sent to, not read yet" lines come from the inbox,
// so they change when a delivery does, not when the card does.
function useFresh(
  reload: () => Promise<void>,
  error: string | null,
  inbox = false
) {
  useEffect(() => {
    void reload()
    return subscribe(() => void reload())
  }, [reload])

  useEffect(() => {
    if (!inbox || !inWails()) return
    return Events.OnShown(EventInboxChanged, () => void reload())
  }, [inbox, reload])

  useEffect(() => {
    if (!inWails()) return
    const onFocus = () => void reload()
    window.addEventListener("focus", onFocus)
    return () => window.removeEventListener("focus", onFocus)
  }, [reload])

  useEffect(() => {
    if (!error) return
    const id = window.setTimeout(() => void reload(), RetryMs)
    return () => window.clearTimeout(id)
  }, [error, reload])
}

const updated = (c: { updatedAt: unknown }) =>
  new Date(c.updatedAt as string).getTime()

async function loadCards(): Promise<CardSummary[]> {
  if (!inWails()) {
    const ask = (c: CardDetail) => (openAsk(c) ? 1 : 0)
    const open = (c: CardDetail) => (c.openBlockers > 0 ? 1 : 0)
    const time = updated
    // As BoardService.Cards: the closed ones are on the Done tab.
    const live = sampleState()
      .filter((c) => !over(c) && !sampleThreadIds.has(c.id))
      .sort((a, b) => ask(b) - ask(a) || open(b) - open(a) || time(b) - time(a))
      .map(summary)
    // As BoardService.Cards: pinned first, in their order, each pinned
    // parent followed by its subtasks.
    const pins = live
      .filter((c) => c.pinned)
      .sort((a, b) => (a.pinRank ?? 0) - (b.pinRank ?? 0))
    const pinned: CardSummary[] = []
    for (const p of pins) {
      if (pinned.includes(p)) continue
      pinned.push(p)
      pinned.push(
        ...live
          .filter((k) => k.parent?.id === p.id && !k.pinned)
          .sort((a, b) => parseInt(a.short, 10) - parseInt(b.short, 10))
      )
    }
    return [...pinned, ...families(live.filter((c) => !pinned.includes(c)))]
  }
  return (await BoardService.Cards()) ?? []
}

// families is BoardService.Cards' last step, for the sample: a parent's
// subtasks follow it, and the family sits where its first card sorted.
function families(sorted: CardSummary[]): CardSummary[] {
  const byId = new Map(sorted.map((c) => [c.id, c]))
  const head = (c: CardSummary) =>
    c.parent && byId.has(c.parent.id) ? c.parent.id : c.id
  const out: CardSummary[] = []
  const placed = new Set<string>()
  for (const c of sorted) {
    const h = head(c)
    if (placed.has(h)) continue
    placed.add(h)
    out.push(byId.get(h)!)
    out.push(
      ...sorted
        .filter((k) => k.parent?.id === h)
        .sort((a, b) => parseInt(a.short, 10) - parseInt(b.short, 10))
    )
  }
  return out
}

export type CardsState = {
  cards: CardSummary[]
  loading: boolean
  error: string | null
}

// useCards is the grid: every live card, cards waiting on a decision first,
// with the person's pending changes on them (card #644).
export function useCards(): CardsState {
  const { value, loading, error, reload } = useLatest<CardSummary[]>(
    loadCards,
    [],
    "cards"
  )
  useFresh(reload, error)
  // A read the person sees ends the changes it agrees with: after it is
  // shown, so the old state never paints in between (the change applied
  // to data that already says so changes nothing). Not the empty list
  // before the first read lands, where every card would seem gone.
  useEffect(() => {
    if (!loading && !error) pending.observeSummaries(value)
  }, [value, loading, error])
  const v = useSyncExternalStore(pending.subscribe, pending.version)
  const cards = useMemo(
    () => pending.summaries(value),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- v is when pending changes
    [value, v]
  )
  return { cards, loading, error }
}

// useIdleReport is Home's idle line (card #341): BoardService.IdleReport,
// re-read on every card change and whenever the roster's statuses change
// (an agent going idle or busy changes it without any card changing). The
// sample has no hand-out: its idle agents and no unhanded cards.
export function useIdleReport(
  agents: { status: string; deviceState?: string | null }[]
) {
  // 0 in the app: a load that changed with the count would read beside the
  // running one and drop its answer (card #551).
  const sampleIdle = inWails()
    ? 0
    : agents.filter((a) => a.status === "idle" && a.deviceState !== "offline")
        .length
  const load = useCallback(
    async (): Promise<IdleReport> =>
      inWails() ? BoardService.IdleReport() : sampleIdleReport(sampleIdle),
    [sampleIdle]
  )
  const { value, error, reload } = useLatest<IdleReport>(load, IdleReport())
  useFresh(reload, error)
  const statuses = agents
    .map((a) => `${a.status}/${a.deviceState ?? ""}`)
    .join(",")
  useEffect(() => {
    void reload()
  }, [statuses, reload])
  return value
}

// sampleIdleReport: the sample's idle agents and nothing to hand out, or
// with ?idle=gap two cards nobody was handed (PREVIEW.md).
function sampleIdleReport(idle: number): IdleReport {
  if (new URLSearchParams(window.location.search).get("idle") !== "gap")
    return IdleReport({ idle })
  return IdleReport({
    idle,
    unoffered: [
      UnofferedCard({
        id: "sample-gap-1",
        short: "wind#87",
        title: "Slot 1 rig",
        why: "nobody in this space is free",
      }),
      UnofferedCard({
        id: "sample-gap-2",
        short: "#41",
        title: "Parser",
        why: "no free agent works in irori",
      }),
    ],
  })
}

// DonePageSize is how many closed cards the Done tab loads at a time.
export const DonePageSize = 30

// loadDone is BoardService.DoneCards: limit closed cards, the latest
// change first, after cursor before ("" for the top).
async function loadDone(
  limit: number,
  before: string,
  scope = ""
): Promise<DonePage> {
  if (inWails())
    return scope
      ? BoardService.DoneCardsIn(limit, before, scope)
      : BoardService.DoneCards(limit, before)
  const inScope = (c: { board?: string | null }) =>
    !scope || (scope === "own" ? !c.board : c.board === scope)
  const done = sampleState()
    .filter(
      (c) =>
        over(c) &&
        !sampleThreadIds.has(c.id) &&
        !sampleCleared.has(c.id) &&
        inScope(c)
    )
    .sort((a, b) => updated(b) - updated(a) || (a.id < b.id ? 1 : -1))
  const from = before ? done.findIndex((c) => c.id === before) + 1 : 0
  const page = done.slice(from, from + limit)
  return DonePage({
    cards: page.map(summary),
    next: from + limit < done.length ? page[page.length - 1].id : "",
  })
}

export type DoneState = {
  cards: CardSummary[]
  more: boolean
  loading: boolean
  error: string | null
  loadMore: () => void
  // reload re-reads the cards showing (after a clear, card #138).
  reload: () => void
}

// useDoneCards is the Done tab: closed cards, the latest first, read only
// while active. Show more appends the next page; a card change re-reads
// as many from the top as are showing, so a card that closes moves in.
// scope pages one page's cards on their own (card #161): "own" for Home,
// a board id for its page, "" for every card.
export function useDoneCards(active: boolean, scope = ""): DoneState {
  const [st, setSt] = useState<{
    cards: CardSummary[]
    next: string
    loading: boolean
    error: string | null
  }>({ cards: [], next: "", loading: true, error: null })
  const shown = useRef(DonePageSize)
  const req = useRef(0)

  const reload = useCallback(async () => {
    const n = ++req.current
    try {
      const p = await loadDone(shown.current, "", scope)
      if (n !== req.current) return
      setSt({ cards: p.cards ?? [], next: p.next, loading: false, error: null })
    } catch (err) {
      if (n !== req.current) return
      setSt((s) => ({ ...s, loading: false, error: errText(err) }))
    }
  }, [scope])

  // loadMore raises how many cards the tab shows and reloads them all, so
  // a reload that races it (a board change, focus) loads the raised count
  // too, and the click is never lost.
  const loadMore = useCallback(() => {
    if (!st.next) return
    shown.current = Math.max(shown.current, st.cards.length) + DonePageSize
    void reload()
  }, [st, reload])

  useEffect(() => {
    if (!active) return
    void reload()
    const off = subscribe(() => void reload())
    if (!inWails()) return off
    const onFocus = () => void reload()
    window.addEventListener("focus", onFocus)
    return () => {
      off()
      window.removeEventListener("focus", onFocus)
    }
  }, [active, reload])

  useEffect(() => {
    if (!active || !st.error) return
    const id = window.setTimeout(() => void reload(), RetryMs)
    return () => window.clearTimeout(id)
  }, [active, st.error, reload])

  return {
    cards: st.cards,
    more: !!st.next,
    loading: st.loading,
    error: st.error,
    loadMore,
    reload: () => void reload(),
  }
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

// useCard is one opened card, re-read on every card change. detail is null
// until the card with this id has loaded. Every useCard mounted shares one
// read (cardStore below): a Notifications page of card rows, Needs you
// and the open card cost one board_cards_by_id per change, not one
// board_card each (card #468).
export function useCard(id: string): {
  detail: CardDetail | null
  error: string | null
} {
  const watch = useCallback((fn: () => void) => cardStore.watch(id, fn), [id])
  const read = useCallback(() => cardStore.slot(id), [id])
  const slot = useSyncExternalStore(watch, read)
  return useMemo(
    () => ({
      detail: !id || !slot || slot.detail?.id !== id ? null : slot.detail,
      error: slot?.error ?? null,
    }),
    [id, slot]
  )
}

// What the shared read holds for one card id: its detail or its error.
// A new object only when the card changed, so its rows render only then.
export type CardSlot = {
  detail: CardDetail | null
  error: string | null
}

// readCards is the shared read: each id's card, or why not.
async function readCards(ids: string[]): Promise<Map<string, CardSlot>> {
  const out = new Map<string, CardSlot>()
  if (inWails()) {
    try {
      for (const r of await BoardService.CardsById(ids))
        out.set(r.id, {
          detail: (r.card as CardDetail | null) ?? null,
          error: r.card ? null : r.error || "coord: no such card",
        })
    } catch (err) {
      for (const id of ids) out.set(id, { detail: null, error: errText(err) })
    }
    return out
  }
  // ?slow in the preview: the card never arrives, to see the page while
  // it loads.
  if (new URLSearchParams(window.location.search).has("slow"))
    await new Promise(() => {})
  for (const id of ids) {
    try {
      // A copy, so React sees a new object after a sample answer.
      out.set(id, { detail: { ...sampleCard(id) } as CardDetail, error: null })
    } catch (err) {
      out.set(id, { detail: null, error: errText(err) })
    }
  }
  return out
}

// cardStore keeps the cards the mounted useCard hooks show and reads them
// all in one call: on every card change (board:changed, devices:changed),
// inbox:changed (the open card's "queued for / sent to" lines), focus,
// and a few seconds after a failed read; a newly watched id is read at
// once. One read runs at a time; what comes in meanwhile waits for it,
// then reads again.
export const cardStore = createCardStore(readCards)

export function createCardStore(
  read: (ids: string[]) => Promise<Map<string, CardSlot>>,
  events: (again: () => void) => () => void = cardEvents,
  retryMs = RetryMs,
  pend: Pending = pending
) {
  const slots = new Map<string, CardSlot>()
  // What each card shows with the person's pending changes on it (card
  // #644): one object per read and set of changes, so useCard renders
  // only when either moves.
  const views = new Map<string, { base: CardSlot; v: number; out: CardSlot }>()
  let offPending: (() => void) | null = null
  const view = (id: string): CardSlot | undefined => {
    const base = slots.get(id)
    if (!base?.detail || !pend.touches(id)) return base
    const v = pend.version()
    const had = views.get(id)
    if (had && had.base === base && had.v === v) return had.out
    const out = { ...base, detail: pend.detail(base.detail) }
    views.set(id, { base, v, out })
    return out
  }
  const seen = new Map<string, string>()
  const users = new Map<string, Set<() => void>>()
  // Ids to read next; all: every watched id.
  let want = new Set<string>()
  let all = false
  let running = false
  let retry: ReturnType<typeof setTimeout> | undefined
  const failedIds = new Set<string>()
  let off: (() => void) | null = null

  function request(id?: string) {
    if (id === undefined) all = true
    else want.add(id)
    if (!running) void run()
  }

  async function run() {
    running = true
    // After the effects of one commit: every row mounted with it is in.
    await Promise.resolve()
    try {
      while (all || want.size) {
        const ids = all
          ? [...users.keys()]
          : [...want].filter((id) => users.has(id))
        all = false
        want = new Set()
        if (!ids.length) continue
        const got = await read(ids)
        for (const [id, got1] of got) {
          if (!users.has(id)) continue
          // A failed read keeps the card shown, with the error beside it,
          // as each useCard's own read did.
          const slot = got1.error
            ? { detail: slots.get(id)?.detail ?? null, error: got1.error }
            : got1
          // Real data the person's change agrees with ends it, even when
          // nothing else about the card changed.
          if (got1.detail) pend.observeDetail(got1.detail)
          const json = JSON.stringify(slot)
          if (seen.get(id) === json) continue
          seen.set(id, json)
          slots.set(id, slot)
          for (const fn of users.get(id) ?? []) fn()
          // A new error is tried again once, on its own, a few seconds
          // later; the same error again isn't (a gone card isn't polled).
          if (slot.error && off) {
            failedIds.add(id)
            if (retry === undefined)
              retry = setTimeout(() => {
                retry = undefined
                const ids = [...failedIds]
                failedIds.clear()
                for (const x of ids) request(x)
              }, retryMs)
          }
        }
      }
    } finally {
      running = false
    }
  }

  function listen() {
    const stop = events(() => request())
    // A change shown or taken back renders the cards it touches.
    offPending = pend.subscribe(() => {
      for (const [id, fns] of users)
        if (pend.touches(id) || views.has(id)) {
          if (!pend.touches(id)) views.delete(id)
          for (const fn of fns) fn()
        }
    })
    return () => {
      stop()
      offPending?.()
      offPending = null
      clearTimeout(retry)
      retry = undefined
      failedIds.clear()
    }
  }

  return {
    slot: view,
    watch(id: string, fn: () => void): () => void {
      if (!id) return () => {}
      let set = users.get(id)
      if (!set) {
        set = new Set()
        users.set(id, set)
        request(id)
      }
      set.add(fn)
      if (!off) off = listen()
      return () => {
        set.delete(fn)
        if (set.size) return
        // Unwatched: the next watch of it loads it afresh.
        users.delete(id)
        slots.delete(id)
        seen.delete(id)
        views.delete(id)
        if (!users.size && off) {
          off()
          off = null
        }
      }
    },
  }
}

// cardEvents is when the card hooks read again: every card change, every
// inbox change, and focus.
function cardEvents(again: () => void): () => void {
  const offCards = subscribe(again)
  const offInbox = inWails()
    ? Events.OnShown(EventInboxChanged, again)
    : () => {}
  const onFocus = () => {
    if (inWails()) again()
  }
  window.addEventListener("focus", onFocus)
  return () => {
    offCards()
    offInbox()
    window.removeEventListener("focus", onFocus)
  }
}

// ---- derived ----------------------------------------------------------------

// percent is the card's progress, or null when it has no steps.
export function percent(p: { done: number; total: number }): number | null {
  if (!p.total) return null
  return Math.round((100 * p.done) / p.total)
}

// displayName is who a face or line names: the roster name, else "unknown
// agent". Never a key or a piece of one (card #478).
export function displayName(r: AgentRef): string {
  const n = plainLine(r.name)
  if (!n || n === r.key) return unknownAgent
  return nameOr(n)
}

// busy says an agent holding the card is working on it right now: one of
// its holders is live, on a device that's online, and its roster status is
// "busy". A card that waits on the person (its input, a blocker) or is
// over isn't, whatever its agent is doing elsewhere. The card and its
// current step show a spinner while it is.
export function busy(c: {
  state: string
  openBlockers: number
  working: readonly AgentRef[]
}): boolean {
  return workable(c) && c.working.some(agentBusy)
}

// workable says a card can be worked on now: not over, not waiting on the
// person (its input, a blocker) and not in review (card #191: the work
// waits on its review, so its holder is free and nothing spins).
function workable(c: { state: string; openBlockers: number }): boolean {
  switch (c.state) {
    case TaskState.Completed:
    case TaskState.Canceled:
    case TaskState.Failed:
    case TaskState.InputRequired:
    case TaskState.InReview:
      return false
  }
  return c.openBlockers <= 0
}

// agentBusy says this agent, one of a card's holders, is working right now:
// live, on a device that's online, taken up (not only assigned) and busy in
// the roster. busy is any holder of a card that can be worked on.
export function agentBusy(w: AgentRef): boolean {
  return w.status === "busy" && w.live && !w.offline && !w.assigned
}

// RunningJobStatus is the roster status Claude Code gives a session that
// is between turns with a background job still running (a background Bash
// or agent): idle, but not done (card #431).
export const RunningJobStatus = "shell"

// agentRunningJob says this holder is between turns with a background job
// running: live, online, taken up, and "shell" in the roster.
export function agentRunningJob(w: AgentRef): boolean {
  return w.status === RunningJobStatus && w.live && !w.offline && !w.assigned
}

// runningJobNow is who holds the card and is running a background job
// between turns (card #431), on a card that can be worked on and that no
// holder is busy on (busy wins: it is working now). None otherwise.
export function runningJobNow(c: {
  state: string
  openBlockers: number
  working: readonly AgentRef[]
}): AgentRef[] {
  if (busy(c) || !workable(c)) return []
  const seen = new Set<string>()
  return c.working.filter((w) => {
    if (!agentRunningJob(w) || seen.has(w.key)) return false
    seen.add(w.key)
    return true
  })
}

// withLive is the card with its agents' roster status as of now (card
// #317): a card's AgentRef carries the status of when the card was read,
// the live roster (LiveRoster, card #299) the status of now. Home's Agent
// progress and the card page both read busy through it, so the two never
// disagree and neither waits for the card to be read again. An agent the
// roster doesn't have keeps its status.
export function withLive<C extends { working: readonly AgentRef[] }>(
  c: C,
  live: ReadonlyMap<string, { status: string }>
): C {
  if (!live.size) return c
  let changed = false
  const working = c.working.map((w) => {
    const s = live.get(w.key)?.status
    if (!s || s === w.status || w.status === "person") return w
    changed = true
    return { ...w, status: s }
  })
  return changed ? { ...c, working } : c
}

// workingNow is who is working on the card right now, by busy's rule:
// its taken-up holders busy in the roster, while busy says the card is
// being worked; none otherwise. The card page's working row names them.
export function workingNow(c: {
  state: string
  openBlockers: number
  working: readonly AgentRef[]
}): AgentRef[] {
  if (!busy(c)) return []
  const seen = new Set<string>()
  return c.working.filter((w) => {
    if (!agentBusy(w) || seen.has(w.key)) return false
    seen.add(w.key)
    return true
  })
}

// namesLine joins names for a line: "oak", "oak and elm", "oak, elm and
// ash".
export function namesLine(names: readonly string[]): string {
  return names.length > 1
    ? `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`
    : (names[0] ?? "")
}

// overLabel is a finished card's state word: "Answered" for an ask its
// taker answered, "Closed" for an ask nothing came of (board.md
// "Outcomes"), else Done, Canceled or Failed.
export function overLabel(c: { state: string; ask?: CardAsk | null }): string {
  const kind = c.ask?.state === AskState.Closed ? c.ask.outcome?.kind : ""
  switch (c.state) {
    case TaskState.Completed:
      return kind === "answered" ? "Answered" : "Done"
    case TaskState.Failed:
      return "Failed"
  }
  return kind === "none" ? "Closed" : "Canceled"
}

// over says the card is finished: done, canceled or failed.
export function over(c: { state: string }): boolean {
  return (
    c.state === TaskState.Completed ||
    c.state === TaskState.Canceled ||
    c.state === TaskState.Failed
  )
}

// workLabel is the word beside a card's working faces, by the same rule
// as its spinner: "working" only while busy says so; "offered" or "not
// picked up" before anyone took it up; nothing once the card is over;
// else "on it" (held, but not working on it right now).
// IDLE_UNNAMED_MS is how long a held card's holders may sit idle, with
// nothing named that it waits on, before it stops reading as calm Waiting
// (card #305). Irori nudges the holder when its turn ends; this is what the
// person sees if the nudge didn't help.
export const IDLE_UNNAMED_MS = 10 * 60_000

// idleUnnamed says a working card is held, but every holder is idle (live,
// online, taken up) and has been, like the card itself, for at least
// IDLE_UNNAMED_MS, with no blocker, review, ask, pause or wait saying why:
// "Idle, no reason given". Busy, gone, offline and offered holders are
// other states.
export function idleUnnamed(c: CardSummary, now: number = Date.now()): boolean {
  if (over(c) || c.state !== TaskState.Working) return false
  if (c.openBlockers > 0 || c.waitingOn || c.waiting > 0 || c.paused)
    return false
  if (c.subtaskBlocked || (openAsk(c) && c.ask)) return false
  const held = c.working.filter((w) => !w.assigned)
  if (!held.length) return false
  if (!held.every((w) => w.status === "idle" && w.live && !w.offline))
    return false
  const t = (s: string | null | undefined) => parseTime(s ?? "")?.getTime() ?? 0
  const since = Math.max(t(c.updatedAt), ...held.map((w) => t(w.idleSince)))
  return since > 0 && now - since >= IDLE_UNNAMED_MS
}

export function workLabel(c: {
  state: string
  openBlockers: number
  working: readonly AgentRef[]
}): string {
  // A closed card has nobody working it, whatever its holders were.
  if (over(c) || !c.working.length) return ""
  if (c.working.every((w) => w.assigned))
    return c.working.some((w) => w.waiting) ? "not picked up" : "offered"
  if (busy(c)) return "working"
  return "on it"
}

// cardHash is the window's address for card id's page, "#card/<id>";
// cardFromHash reads one back ("" for the grid).
export function cardHash(id: string): string {
  return `#card/${encodeURIComponent(id)}`
}

// openTarget reads a notification's ref: "task:<id>" opens the card,
// "task:<id>/comment:<commentId>" opens it at that comment (card #177).
// Anything else is null.
export function openTarget(
  ref: string
): { card: string; comment: string } | null {
  if (!ref.startsWith("task:")) return null
  const rest = ref.slice("task:".length)
  const i = rest.indexOf("/comment:")
  const card = i < 0 ? rest : rest.slice(0, i)
  const comment = i < 0 ? "" : rest.slice(i + "/comment:".length)
  return card ? { card, comment } : null
}

export function cardFromHash(hash: string): string {
  const m = /^#card\/(.+)$/.exec(hash)
  if (!m) return ""
  try {
    return decodeURIComponent(m[1])
  } catch {
    return ""
  }
}

// The @ picker's candidates, shared by every box: read the first time a
// box needs them and again on each roster change.
let candidates: MentionCandidate[] = []
let candidatesOff: (() => void) | null = null
const candidateListeners = new Set<() => void>()

function loadCandidates() {
  cards
    .mentionCandidates()
    .then((cs) => {
      candidates = cs
      for (const fn of candidateListeners) fn()
    })
    .catch(() => {})
}

// useMentionCandidates is the picker's rows; active false (a box whose
// popover hasn't opened yet) reads nothing.
export function useMentionCandidates(active = true): MentionCandidate[] {
  const [list, setList] = useState(candidates)
  useEffect(() => {
    if (!active) return
    const fn = () => setList(candidates)
    const first = !candidateListeners.size
    candidateListeners.add(fn)
    if (first) {
      if (inWails())
        candidatesOff = Events.On(EventRosterChanged, loadCandidates)
      loadCandidates()
    } else fn()
    return () => {
      candidateListeners.delete(fn)
      if (!candidateListeners.size) {
        candidatesOff?.()
        candidatesOff = null
      }
    }
  }, [active])
  return list
}

export { MaxUpload, MaxUploadFiles } from "./uploads"
