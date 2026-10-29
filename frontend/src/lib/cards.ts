// Cards gateway (docs/design/v0.4-devices.md §9): the grid, one opened card,
// and the person's answer to a blocker. Like talk.ts, components import from
// here, never from bindings/, and in a plain browser everything runs on the
// sample in sample-cards.ts (answers change it in memory so the buttons can
// be tried).

import { useCallback, useEffect, useMemo } from "react"
import { Browser, Events } from "@wailsio/runtime"

import { BoardService } from "../../bindings/github.com/StephenSHorton/irori"
import {
  AnswerResult,
  CardMessage,
  CardResolution,
  CardSummary,
  CardThreadItem,
  Holding,
  Told,
  type AgentRef,
  type CardActivity,
  type CardBlocker,
  type CardDetail,
  type CardHolder,
  type CardOption,
  type CardPR,
} from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  TaskState,
  type Step,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import { inWails, plainLine } from "./irori"
import { useLatest } from "./talk"
import { NoteItem, sampleCards, sampleYou } from "./sample-cards"

export type {
  AgentRef,
  AnswerResult,
  CardActivity,
  CardBlocker,
  CardDetail,
  CardMessage,
  CardHolder,
  CardOption,
  CardPR,
  CardSummary,
  CardThreadItem,
  Step,
  Told,
}
export { Holding, NoteItem, TaskState }

// EventBoardChanged mirrors the Go constant in board_service.go: it fires on
// every card change.
export const EventBoardChanged = "board:changed"

// MaxAnswer mirrors coord's limit on a blocker answer.
export const MaxAnswer = 500

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
    working: c.working,
    watching: c.watching,
    holder: c.holder,
    updatedAt: c.updatedAt,
  })
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
  const b = c.blockers.find((x) => x.id === blockerId)
  if (!b) throw new Error(`coord: no such blocker "${blockerId}"`)
  if (b.resolved) throw new Error("coord: conflict: that blocker is resolved")
  const answer = option >= 0 ? b.options[option]?.answer : text.trim()
  if (!answer) throw new Error("coord: invalid input: an answer needs text")
  const now = new Date().toISOString()
  b.resolved = new CardResolution({
    answer,
    option,
    by: sampleYou,
    at: now,
    closed: false,
  })
  // Open items first (oldest first), then resolved ones newest first.
  const open = c.blockers.filter((x) => !x.resolved)
  const done = c.blockers.filter((x) => x.resolved && x !== b)
  c.blockers = [...open, b, ...done]
  c.openBlockers = open.length
  c.blockerQuestion = open[0]?.question ?? ""
  if (!open.length) c.state = TaskState.Working
  c.updatedAt = now
  c.recent = [
    { at: now, who: sampleYou, text: `You answered: ${answer}` },
    ...c.recent,
  ].slice(0, 8)
  c.thread = sampleThreadLine(c, now, `You answered: ${answer}`)
  sampleChanged()
  return new AnswerResult({
    card: c,
    told: new Told({ to: [b.raisedBy.key], mode: wake ? "wake" : "next_turn" }),
  })
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
  text: string
): CardThreadItem[] {
  return [
    ...c.thread,
    new CardThreadItem({
      id: `sample-answer-${c.thread.length}`,
      kind: "event",
      at,
      who: sampleYou,
      text,
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

// ---- actions ----------------------------------------------------------------

// Every action acts as the person at this device.
export const cards = {
  // answer resolves one blocker: option is an index into blocker.options,
  // or -1 to answer with text. wake defaults to true: the raiser is blocked
  // on the person.
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

  // message opens a message on card id's thread in full: the text the
  // card cut or doesn't carry yet, and its files.
  async message(id: string, msgId: string): Promise<CardMessage> {
    if (inWails()) return BoardService.Message(id, msgId)
    return sampleMessage(id, msgId)
  },

  // openURL opens a PR's page in the person's browser.
  async openURL(url: string): Promise<void> {
    if (!/^https:\/\//.test(url)) return
    if (inWails()) return Browser.OpenURL(url)
    window.open(url, "_blank", "noopener")
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
// and once a few seconds after a failed read.
function useFresh(reload: () => Promise<void>, error: string | null) {
  useEffect(() => {
    void reload()
    return subscribe(() => void reload())
  }, [reload])

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

async function loadCards(): Promise<CardSummary[]> {
  if (!inWails()) {
    const open = (c: CardDetail) => (c.openBlockers > 0 ? 1 : 0)
    const time = (c: CardDetail) => new Date(c.updatedAt as string).getTime()
    return [...sampleState()]
      .sort((a, b) => open(b) - open(a) || time(b) - time(a))
      .map(summary)
  }
  return (await BoardService.Cards()) ?? []
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

// useCard is one opened card, re-read on every card change. detail is null
// until the card with this id has loaded.
export function useCard(id: string): {
  detail: CardDetail | null
  error: string | null
} {
  const load = useCallback(async (): Promise<CardDetail | null> => {
    if (!id) return null
    if (inWails()) return BoardService.Card(id)
    // A copy, so React sees a new object after a sample answer.
    return { ...sampleCard(id) } as CardDetail
  }, [id])
  const { value, loading, error, reload } = useLatest<CardDetail | null>(
    load,
    null
  )
  useFresh(reload, error)
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
