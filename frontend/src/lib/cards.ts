// Cards gateway (docs/design/v0.4-devices.md §9): the grid, one opened card,
// and the person's answer to a blocker. Like talk.ts, components import from
// here, never from bindings/, and in a plain browser everything runs on the
// sample in sample-cards.ts (answers change it in memory so the buttons can
// be tried).

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Browser, Events } from "@wailsio/runtime"

import { BoardService } from "../../bindings/github.com/StephenSHorton/irori"
import {
  AnswerResult,
  AskRepo,
  CardAsk,
  CardMessage,
  CardResolution,
  CardSummary,
  CardThreadItem,
  CommentResult,
  DonePage,
  Holding,
  Told,
  TaskResult,
  UploadFile,
  type AgentRef,
  type CardActivity,
  type CardBlocker,
  type CardDetail,
  type CardHolder,
  type CardOption,
  type CardPR,
  type CardRef,
  type CardSubtask,
} from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  LinkKind,
  TaskState,
  type Link,
  type Step,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import { inWails, plainLine } from "./irori"
import { safeHref } from "./rich"
import { EventInboxChanged, useLatest } from "./talk"
import { NoteItem, sampleCards, sampleYou } from "./sample-cards"

export type {
  AgentRef,
  AnswerResult,
  AskRepo,
  CardAsk,
  CardActivity,
  CardBlocker,
  CardDetail,
  CardMessage,
  CardHolder,
  CardOption,
  CardPR,
  CardRef,
  CardSubtask,
  CardSummary,
  CardThreadItem,
  CommentResult,
  Link,
  Step,
  Told,
}
export { Holding, LinkKind, NoteItem, TaskState }

// EventBoardChanged mirrors the Go constant in board_service.go: it fires on
// every card change.
export const EventBoardChanged = "board:changed"

// MaxAnswer mirrors coord's limit on a blocker answer.
export const MaxAnswer = 500

// MaxComment mirrors coord's limit on a comment.
export const MaxComment = 4000

// BlockerState mirrors the Go constants on CardBlocker.state: open (no
// answer yet), answered (the person answered; it waits on the raiser),
// replied (an agent replied to the answer; back with the person),
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
  Closed: "closed",
} as const

// openAsk says the card is an ask still finding its taker or its outcome:
// the person's prompt to the room, before it became a card.
// holderLineShows says whether a card with steps also shows its holder
// line under the bar (card #16): whenever nobody is busy on it, so a card
// whose agent is idle, offline or gone doesn't look like work in progress.
// A card without steps always shows it in place of the bar.
export function holderLineShows(c: { holder: { state: string } }): boolean {
  return c.holder.state !== Holding.HoldActive && c.holder.state !== Holding.HoldPerson
}

// nobodyOn says the card has nobody live on it: its holder went away, or
// Irori took that holder off and it is open again (card #16).
export function nobodyOn(c: {
  holder: { state: string }
  returned?: AgentRef | null
}): boolean {
  return !!c.returned || c.holder.state === Holding.HoldGone
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

// askLine is an open ask's line in place of the holder line: how many
// heard it, who it waits on, or who took it.
export function askLine(a: CardAsk): string {
  switch (a.state) {
    case AskState.Taken:
      return `${a.taker ? displayName(a.taker) : "An agent"} took it`
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
  return `${a.told} ${a.told === 1 ? "agent" : "agents"} heard it`
}

// ---- sample mode ------------------------------------------------------------

let sample: CardDetail[] | null = null
const sampleListeners = new Set<() => void>()

function sampleState(): CardDetail[] {
  sample ??= sampleCards(new Date())
  return sample
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
  return new CardSummary({
    id: c.id,
    short: c.short,
    shortWas: c.shortWas,
    title: c.title,
    state: c.state,
    repoKey: c.repoKey,
    scope: c.scope,
    scopeLabel: c.scopeLabel,
    progress: c.progress,
    openBlockers: c.openBlockers,
    blockerQuestion: c.blockerQuestion,
    waiting: c.waiting,
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
  })
}

// MaxUpload and MaxUploadFiles mirror board_service.go's MaxUpload and
// inbox.MaxAttachments: files added in the window cross the webview bridge
// in memory.
export const MaxUpload = 32 << 20
export const MaxUploadFiles = 10

// uploadOf is a File as BoardService takes it: its name and base64 bytes
// (Go's []byte in JSON).
async function uploadOf(f: File): Promise<UploadFile> {
  const bytes = new Uint8Array(await f.arrayBuffer())
  let bin = ""
  for (let i = 0; i < bytes.length; i += 0x8000)
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return new UploadFile({ name: f.name || "pasted.png", data: btoa(bin) })
}

// sampleAsk is BoardService.Ask on the sample: a new ask first in the
// grid, heard by the sample's idle agents.
function sampleAsk(text: string, repoKey: string, wake: boolean): TaskResult {
  const t = text.trim()
  if (!t) throw new Error("coord: invalid input: an empty comment")
  const now = new Date().toISOString()
  const cs = sampleState()
  const top = Math.max(0, ...cs.map((c) => parseInt(c.short, 10) || 0))
  const heard = ["sample-heron"]
  const first = t.split(/\r?\n/)[0].replace(/\s+/g, " ")
  const c = {
    ...cs[0],
    id: `sample-ask-${Date.now()}`,
    short: String(top + 1),
    shortWas: "",
    title: first.length > 80 ? first.slice(0, 79) + "…" : first,
    goal: t,
    doneWhen: "",
    note: "",
    state: TaskState.Submitted,
    repoKey,
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
    holder: cs[0].holder,
    recent: [],
    thread: [
      new CardThreadItem({ id: "sample-ask-post", kind: "event", at: now, who: sampleYou, text: "You asked the room" }),
    ],
    threadEarlier: 0,
    historyTrimmed: false,
    updatedAt: now,
    ask: new CardAsk({ state: AskState.Open, told: heard.length, owner: "This PC", wake, taker: null, outcome: null }),
  } as CardDetail
  sample = [c, ...cs]
  sampleChanged()
  return new TaskResult({ task: { id: c.id } as TaskResult["task"], told: new Told({ to: heard, mode: wake ? "wake" : "next_turn" }) })
}

// sampleCloseAsk is BoardService.CloseAsk on the sample: nothing came of
// it, so it leaves the grid.
function sampleCloseAsk(id: string, note: string) {
  const c = sampleCard(id)
  c.state = TaskState.Canceled
  c.ask = new CardAsk({
    ...c.ask,
    state: AskState.Closed,
    outcome: { kind: "none", note: note.trim() || "closed by the person" },
  })
  sample = sampleState().filter((x) => x.id !== id)
  sampleChanged()
}

// sampleReopen is BoardService.Reopen on the sample: working again when
// someone holds it, else open.
function sampleReopen(id: string) {
  const c = sampleCard(id)
  c.state = c.working.length ? TaskState.Working : TaskState.Submitted
  sampleChanged()
}

function sampleAnswer(
  id: string,
  blockerId: string,
  option: number,
  text: string,
  wake: boolean
): AnswerResult {
  const c = sampleCard(id)
  if (blockerId === NoteItem) return sampleAnswerNote(c, option, text, wake)
  const b = sampleBlocker(c, blockerId)
  const answer = option >= 0 ? b.options[option]?.answer : text.trim()
  if (!answer) throw new Error("coord: invalid input: an answer needs text")
  const now = new Date().toISOString()
  b.thread = [
    ...b.thread,
    new CardThreadItem({
      id: `sample-answer-${b.id}-${b.thread.length}`,
      kind: "comment",
      at: now,
      who: sampleYou,
      to: b.raisedBy,
      text: answer,
      blockerId: b.id,
    }),
  ]
  c.thread = sampleThreadLine(c, now, `You answered blocker ${b.id}`, b.id)
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
  return new AnswerResult({
    card: c,
    told: new Told({
      to: [b.raisedBy.key, ...c.working.map((w) => w.key)].filter(
        (k, i, all) => all.indexOf(k) === i
      ),
      mode: wake ? "wake" : "next_turn",
    }),
  })
}

// sampleResolveBlocker is BoardService.ResolveBlocker on the sample: the
// person's latest answer stands, with note beside the check.
function sampleResolveBlocker(
  id: string,
  blockerId: string,
  note: string
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
  return new AnswerResult({
    card: c,
    told: new Told({ to: c.working.map((w) => w.key), mode: "next_turn" }),
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
  b.resolved = new CardResolution({
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
// the rest of the open ones on the person.
function sampleRecount(c: CardDetail) {
  const open = c.blockers.filter((x) => !x.resolved)
  const needs = open.filter((x) => x.state !== BlockerState.Answered)
  c.openBlockers = needs.length
  c.waiting = open.length - needs.length
  c.blockerQuestion = needs[0]?.question ?? ""
}

// sampleMessage is BoardService.Message on the sample: a held message's
// text and a message's files are made up, as the sample has no inbox.
function sampleMessage(id: string, msgId: string): CardMessage {
  const it = sampleCard(id).thread.find(
    (x) => x.id === msgId && x.kind === "message"
  )
  if (!it)
    throw new Error(`coord: no such task or lease: no message "${msgId}"`)
  return new CardMessage({
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
  blockerId?: string
): CardThreadItem[] {
  return [
    ...c.thread,
    new CardThreadItem({
      id: `sample-line-${c.thread.length}`,
      kind: "event",
      at,
      who: sampleYou,
      text,
      blockerId,
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
  return new AnswerResult({
    card: c,
    told: new Told({
      to: c.working.map((w) => w.key),
      mode: wake ? "wake" : "next_turn",
    }),
  })
}

// sampleComment is BoardService.Comment on the sample: the person's
// comment at the end of the thread, heard by the live agents on the card
// (watchers answer only if asked); with none, it is unheard.
function sampleComment(id: string, text: string, wake: boolean): CommentResult {
  const c = sampleCard(id)
  const t = text.trim()
  if (!t) throw new Error("coord: invalid input: an empty comment")
  const now = new Date().toISOString()
  const answers = c.working.filter((w) => w.live)
  const watching = c.watching.filter((w) => w.live)
  const unheard = !answers.length && !watching.length
  c.thread = [
    ...c.thread,
    new CardThreadItem({
      id: `sample-comment-${c.thread.length}`,
      kind: "comment",
      at: now,
      who: sampleYou,
      text: t,
      unheard,
    }),
  ]
  c.updatedAt = now
  sampleChanged()
  return new CommentResult({
    told: new Told({
      to: [...answers, ...watching].map((w) => w.key),
      mode: unheard ? "" : wake ? "wake" : "next_turn",
    }),
    answers,
    watching,
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
    wake = true
  ): Promise<AnswerResult> {
    if (inWails()) return BoardService.Answer(id, blockerId, option, text, wake)
    return sampleAnswer(id, blockerId, option, text, wake)
  },

  // resolveBlocker is "Mark resolved": the person's answer stands, with
  // note (optional) beside the check.
  async resolveBlocker(
    id: string,
    blockerId: string,
    note: string
  ): Promise<AnswerResult> {
    if (inWails()) return BoardService.ResolveBlocker(id, blockerId, note)
    return sampleResolveBlocker(id, blockerId, note)
  },

  // comment puts the person's comment on card id's thread. Its holders
  // (else its creator) are asked to answer it on the card and its
  // watchers hear it too; wake (the default) wakes whoever answers now
  // instead of at their next turn.
  async comment(id: string, text: string, wake = true): Promise<CommentResult> {
    if (inWails())
      return BoardService.Comment(id, text, wake ? "wake" : "next_turn")
    return sampleComment(id, text, wake)
  },

  // ask posts the person's prompt to the room: every free agent hears it
  // (in repoKey's repo when one is named), at its next turn or, with wake,
  // now. One of them takes it.
  // files (picked, dropped or pasted) go on the ask's card, and the agent
  // that takes it fetches them (card #25).
  async ask(
    text: string,
    repoKey = "",
    wake = false,
    files: File[] = []
  ): Promise<TaskResult> {
    if (files.length > MaxUploadFiles)
      throw new Error(`at most ${MaxUploadFiles} files`)
    const big = files.find((f) => f.size > MaxUpload)
    if (big)
      throw new Error(
        `${big.name} is over ${MaxUpload >> 20} MiB; ask an agent to attach it from disk`
      )
    if (inWails()) {
      if (!files.length) return BoardService.Ask(text, repoKey, wake)
      return BoardService.AskWithFiles(
        text,
        repoKey,
        wake,
        await Promise.all(files.map(uploadOf))
      )
    }
    return sampleAsk(text, repoKey, wake)
  },

  // askAgain is Wake them on an ask nobody took.
  async askAgain(id: string): Promise<void> {
    if (inWails()) return BoardService.AskAgain(id)
    const c = sampleCard(id)
    if (c.ask) c.ask = new CardAsk({ ...c.ask, state: AskState.Open, wake: true })
    sampleChanged()
  },

  // closeAsk ends an ask with nothing coming of it.
  async closeAsk(id: string, note = ""): Promise<void> {
    if (inWails()) {
      await BoardService.CloseAsk(id, note)
      return
    }
    sampleCloseAsk(id, note)
  },

  // reopen brings a finished card back into play, with whoever held it.
  async reopen(id: string): Promise<void> {
    if (inWails()) {
      await BoardService.Reopen(id)
      return
    }
    sampleReopen(id)
  },

  // askRepos are the repos the agents are in, for the ask box's chip.
  async askRepos(): Promise<AskRepo[]> {
    if (inWails()) return (await BoardService.AskRepos()) ?? []
    return [
      new AskRepo({ repoKey: "remote:github.com/sample-org/example-app", label: "example-app" }),
      new AskRepo({ repoKey: "remote:github.com/sample-org/irori", label: "irori" }),
    ]
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
    if (inWails()) return Browser.OpenURL(href)
    window.open(href, "_blank", "noopener")
  },
}

// ---- hooks ------------------------------------------------------------------

// RetryMs is the wait before re-reading after a failed read: there is no
// poll, so a failure has no next event to recover it.
const RetryMs = 4000

// subscribe re-runs fn on every board:changed (or sample change).
function subscribe(fn: () => void): () => void {
  if (!inWails()) {
    sampleListeners.add(fn)
    return () => sampleListeners.delete(fn)
  }
  return Events.On(EventBoardChanged, fn)
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
    return Events.On(EventInboxChanged, () => void reload())
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
    return families(
      sampleState()
        .filter((c) => !over(c))
        .sort((a, b) => ask(b) - ask(a) || open(b) - open(a) || time(b) - time(a))
        .map(summary)
    )
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

// useCards is the grid: every live card, cards waiting on a decision first.
export function useCards(): CardsState {
  const { value, loading, error, reload } = useLatest<CardSummary[]>(
    loadCards,
    []
  )
  useFresh(reload, error)
  return { cards: value, loading, error }
}

// DonePageSize is how many closed cards the Done tab loads at a time.
export const DonePageSize = 30

// loadDone is BoardService.DoneCards: limit closed cards, the latest
// change first, after cursor before ("" for the top).
async function loadDone(limit: number, before: string): Promise<DonePage> {
  if (inWails()) return BoardService.DoneCards(limit, before)
  const done = sampleState()
    .filter(over)
    .sort((a, b) => updated(b) - updated(a) || (a.id < b.id ? 1 : -1))
  const from = before ? done.findIndex((c) => c.id === before) + 1 : 0
  const page = done.slice(from, from + limit)
  return new DonePage({
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
}

// useDoneCards is the Done tab: closed cards, the latest first, read only
// while active. Show more appends the next page; a card change re-reads
// as many from the top as are showing, so a card that closes moves in.
export function useDoneCards(active: boolean): DoneState {
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
      const p = await loadDone(shown.current, "")
      if (n !== req.current) return
      setSt({ cards: p.cards ?? [], next: p.next, loading: false, error: null })
    } catch (err) {
      if (n !== req.current) return
      setSt((s) => ({ ...s, loading: false, error: errText(err) }))
    }
  }, [])

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
  }
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

// useCard is one opened card, re-read on every card change. detail is null
// until the card with this id has loaded.
export function useCard(id: string): {
  detail: CardDetail | null
  error: string | null
} {
  const load = useCallback(async (): Promise<CardDetail | null> => {
    if (!id) return null
    if (inWails()) return BoardService.Card(id)
    // ?slow in the preview: the card never arrives, to see the page while
    // it loads.
    if (new URLSearchParams(window.location.search).has("slow"))
      await new Promise(() => {})
    // A copy, so React sees a new object after a sample answer.
    return { ...sampleCard(id) } as CardDetail
  }, [id])
  const { value, loading, error, reload } = useLatest<CardDetail | null>(
    load,
    null
  )
  useFresh(reload, error, true)
  const detail = useMemo(
    () => (!id || loading || value?.id !== id ? null : value),
    [id, loading, value]
  )
  return { detail, error }
}

// ---- derived ----------------------------------------------------------------

// percent is the card's progress, or null when it has no steps.
export function percent(p: { done: number; total: number }): number | null {
  if (!p.total) return null
  return Math.round((100 * p.done) / p.total)
}

// initials is a face's two letters: "sample-otter" -> "so", "Grok" -> "gr".
export function initials(name: string): string {
  const words = name
    .replace(/^.*\//, "")
    .split(/[\s\-_.:]+/)
    .filter((w) => /[a-z0-9]/i.test(w))
  if (words.length >= 2) return (words[0][0] + words[1][0]).toLowerCase()
  return (words[0] ?? "?").slice(0, 2).toLowerCase()
}

// displayName is who a face or line names: the roster name, else (for an
// agent the roster no longer has) the session part of its key.
export function displayName(r: AgentRef): string {
  const n = plainLine(r.name)
  if (!n || n === r.key) return r.key.split("/").pop() || r.key
  return n
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
  switch (c.state) {
    case TaskState.Completed:
    case TaskState.Canceled:
    case TaskState.Failed:
    case TaskState.InputRequired:
      return false
  }
  if (c.openBlockers > 0) return false
  return c.working.some(agentBusy)
}

// agentBusy says this agent, one of a card's holders, is working right now:
// live, on a device that's online, taken up (not only assigned) and busy in
// the roster. busy is any holder of a card that can be worked on.
export function agentBusy(w: AgentRef): boolean {
  return w.status === "busy" && w.live && !w.offline && !w.assigned
}

// overLabel is a finished card's state word: "Answered" for an ask its
// taker answered, "Closed" for an ask nothing came of (board.md
// "Outcomes"), else Done, Canceled or Failed.
export function overLabel(c: {
  state: string
  ask?: CardAsk | null
}): string {
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
// as its spinner: "working" only while busy says so; "assigned" or "not
// picked up" before anyone took it up; nothing once the card is over;
// else "on it" (held, but not working on it right now).
export function workLabel(c: {
  state: string
  openBlockers: number
  working: readonly AgentRef[]
}): string {
  // A closed card has nobody working it, whatever its holders were.
  if (over(c) || !c.working.length) return ""
  if (c.working.every((w) => w.assigned))
    return c.working.some((w) => w.waiting) ? "not picked up" : "assigned"
  if (busy(c)) return "working"
  return "on it"
}

// cardHash is the window's address for card id's page, "#card/<id>";
// cardFromHash reads one back ("" for the grid).
export function cardHash(id: string): string {
  return `#card/${encodeURIComponent(id)}`
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
