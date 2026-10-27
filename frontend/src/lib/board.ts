// Board gateway (docs/design/board.md): the lanes, one card's comments, and
// the person's actions. Like talk.ts, components import from here, never
// from bindings/, and in a plain browser everything runs on the sample in
// sample-board.ts (actions change it in memory so the buttons can be tried).

import { useCallback, useEffect, useMemo } from "react"
import { Browser, Events } from "@wailsio/runtime"

import { BoardService } from "../../bindings/github.com/StephenSHorton/irori"
import {
  CommentResult,
  NewCard,
  TaskDetail,
  TaskResult,
  Told,
} from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  BlockerKind,
  BoardView,
  LinkKind,
  TaskState,
  type BlockedBy,
  type Card,
  type Comment,
  type LeaseCard,
  type Link,
  type Task,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import { inWails } from "./irori"
import { useLatest } from "./talk"
import { tellTo } from "./tell"
import { sampleBoard, sampleLink, type SampleBoard } from "./sample-board"

export type {
  BlockedBy,
  Card,
  Comment,
  CommentResult,
  LeaseCard,
  Link,
  NewCard,
  Task,
  TaskDetail,
  TaskResult,
  Told,
}
export { BlockerKind, BoardView, LinkKind, TaskState }

// EventBoardChanged mirrors the Go constant in board_service.go.
export const EventBoardChanged = "board:changed"

// Limits mirror coord (board.go, coord.go).
export const MaxTitle = 200
export const MaxComment = 4000
export const MaxReason = 500

// Tell is when the agent hears a person's comment (decision 18).
export type Tell = "next_turn" | "wake" | "none"

// ---- sample mode ------------------------------------------------------------

let sample: SampleBoard | null = null
const sampleListeners = new Set<() => void>()

function sampleChanged() {
  for (const fn of sampleListeners) fn()
}

function onSample(fn: () => void): () => void {
  sampleListeners.add(fn)
  return () => sampleListeners.delete(fn)
}

function sampleState(): SampleBoard {
  sample ??= sampleBoard(new Date())
  return sample
}

function sampleTask(id: string): Task {
  const t = sampleState().tasks.find((x) => x.id === id)
  if (!t) throw new Error(`coord: no such task or lease: task "${id}"`)
  return t
}

function sampleUpdate(id: string, fn: (t: Task) => Partial<Task>): Task {
  const s = sampleState()
  const old = sampleTask(id)
  const t = { ...old, ...fn(old), updatedAt: new Date().toISOString() } as Task
  s.tasks = s.tasks.map((x) => (x.id === id ? t : x))
  sampleChanged()
  return t
}

function terminal(s: TaskState): boolean {
  return (
    s === TaskState.Completed ||
    s === TaskState.Failed ||
    s === TaskState.Canceled
  )
}

// sampleView groups the sample's tasks the way coord's Board does.
function sampleView(repoKey: string): BoardView {
  const s = sampleState()
  const you = s.you
  const v = new BoardView()
  const newest = (a: Task, b: Task) =>
    new Date(b.updatedAt as string).getTime() -
    new Date(a.updatedAt as string).getTime()
  const tasks = s.tasks
    .filter((t) => !repoKey || !t.repoKey || t.repoKey === repoKey)
    .sort(newest)
  for (const t of tasks) {
    const cs = s.comments.filter((c) => c.taskId === t.id)
    const card = {
      ...t,
      comments: cs.length,
      lastComment: cs[cs.length - 1],
    } as Card
    if (terminal(t.state)) v.done.push(card)
    else if (
      t.state === TaskState.InputRequired ||
      (t.state === TaskState.Blocked &&
        t.blockedBy?.kind === BlockerKind.BlockedOnPerson &&
        (!t.blockedBy.ref || t.blockedBy.ref === you)) ||
      (t.holders ?? []).includes(you)
    )
      v.needsYou.push(card)
    else if (t.state === TaskState.Blocked) v.blocked.push(card)
    else if ((t.holders ?? []).length > 0) v.working.push(card)
    else v.open.push(card)
  }
  v.doneTotal = v.done.length
  v.leases = s.leases.map((l) => ({
    ...l,
    blocks: s.tasks
      .filter(
        (t) =>
          t.state === TaskState.Blocked &&
          t.blockedBy?.kind === BlockerKind.BlockedOnLease &&
          t.blockedBy.ref === l.resource
      )
      .map((t) => t.id),
  }))
  return v
}

function sampleNotice(t: Task, wake: boolean): Told {
  const to = tellTo(t.holders, t.createdBy)
  return new Told({
    to,
    mode: to.length ? (wake ? "wake" : "next_turn") : "",
  })
}

// ---- actions ----------------------------------------------------------------

// Every action acts as the person at this device.
export const board = {
  async create(card: NewCard): Promise<TaskResult> {
    if (inWails()) return BoardService.Create(card)
    const s = sampleState()
    const now = new Date().toISOString()
    const blocked = card.blockReason.trim() !== ""
    const to = card.assignTo.trim()
    const t = {
      id: `sample-task-${s.tasks.length + 1}-${Date.now() % 1000}`,
      title: card.title.trim(),
      objective: card.objective.trim(),
      doneCriteria: card.doneCriteria.trim() || undefined,
      repoKey: card.repoKey || undefined,
      state: blocked
        ? TaskState.Blocked
        : to
          ? TaskState.Working
          : TaskState.Submitted,
      holder: to || undefined,
      holders: to ? [to] : undefined,
      assigned: to ? [to] : undefined,
      shared: card.shared,
      createdBy: s.you,
      createdAt: now,
      updatedAt: now,
      reason: blocked ? card.blockReason.trim() : undefined,
      blockedBy:
        blocked && card.blockedOn
          ? { kind: card.blockedOn, ref: card.blockedRef || undefined }
          : undefined,
      links: (card.links ?? []).filter((l) => l.trim()).map(sampleLink),
    } as Task
    if (!t.title) throw new Error("coord: invalid input: a task needs a title")
    s.tasks = [t, ...s.tasks]
    sampleChanged()
    return new TaskResult({
      task: t,
      told: new Told({ to: to ? [to] : [], mode: to ? "next_turn" : "" }),
    })
  },

  async comment(id: string, text: string, tell: Tell): Promise<CommentResult> {
    if (inWails()) return BoardService.Comment(id, text, tell)
    const s = sampleState()
    const t = sampleTask(id)
    if (!text.trim()) throw new Error("coord: invalid input: empty comment")
    const c = {
      id: `sample-comment-${s.comments.length + 1}`,
      taskId: id,
      by: s.you,
      text: text.trim(),
      wake: tell === "wake",
      at: new Date().toISOString(),
    } as Comment
    s.comments = [...s.comments, c]
    sampleUpdate(id, () => ({}))
    const told =
      tell === "none" ? new Told({ to: [] }) : sampleNotice(t, tell === "wake")
    return new CommentResult({ comment: c, told })
  },

  async block(
    id: string,
    reason: string,
    on: BlockerKind,
    ref: string
  ): Promise<Task> {
    if (inWails()) return BoardService.Block(id, reason, on, ref)
    if (!reason.trim())
      throw new Error("coord: invalid input: say what blocks it")
    return sampleUpdate(id, () => ({
      state: TaskState.Blocked,
      reason: reason.trim(),
      blockedBy: on ? { kind: on, ref: ref || undefined } : undefined,
    }))
  },

  async unblock(id: string, note: string): Promise<TaskResult> {
    if (inWails()) return BoardService.Unblock(id, note)
    const t = sampleUpdate(id, (t) => ({
      state:
        (t.holders ?? []).length > (t.assigned ?? []).length
          ? TaskState.Working
          : TaskState.Submitted,
      reason: undefined,
      blockedBy: undefined,
    }))
    return new TaskResult({ task: t, told: sampleNotice(t, false) })
  },

  async assign(id: string, to: string): Promise<TaskResult> {
    if (inWails()) return BoardService.Assign(id, to)
    const t = sampleUpdate(id, (t) => ({
      holder: t.shared ? (t.holder ?? to) : to,
      holders: t.shared ? [...(t.holders ?? []), to] : [to],
      assigned: [...(t.assigned ?? []), to],
      state: t.state === TaskState.Submitted ? TaskState.Working : t.state,
    }))
    return new TaskResult({
      task: t,
      told: new Told({ to: [to], mode: "next_turn" }),
    })
  },

  async setDone(id: string, canceled: boolean): Promise<Task> {
    if (inWails()) return BoardService.SetDone(id, canceled)
    return sampleUpdate(id, () => ({
      state: canceled ? TaskState.Canceled : TaskState.Completed,
      reason: undefined,
      blockedBy: undefined,
    }))
  },

  async addLink(id: string, link: string): Promise<Task> {
    if (inWails()) return BoardService.AddLink(id, link)
    return sampleUpdate(id, (t) => ({
      links: [...(t.links ?? []), sampleLink(link)],
    }))
  },

  async removeLink(id: string, ref: string): Promise<Task> {
    if (inWails()) return BoardService.RemoveLink(id, ref)
    return sampleUpdate(id, (t) => ({
      links: (t.links ?? []).filter((l) => l.ref !== ref),
    }))
  },

  async releaseLease(resource: string): Promise<void> {
    if (inWails()) return BoardService.ReleaseLease(resource)
    const s = sampleState()
    s.leases = s.leases.filter((l) => l.resource !== resource)
    sampleChanged()
  },

  // openLink opens a URL link in the person's browser. Paths and commits
  // aren't opened: the card only names them.
  async openLink(l: Link): Promise<void> {
    if (l.kind !== LinkKind.LinkURL) return
    if (inWails()) return Browser.OpenURL(l.ref)
    window.open(l.ref, "_blank", "noopener")
  },
}

// ---- hooks ------------------------------------------------------------------

export type BoardState = {
  view: BoardView
  loading: boolean
  error: string | null
  reload: () => Promise<void>
}

// BoardRetryMs is the wait before re-reading after a failed Board read.
const BoardRetryMs = 4000

const emptyView = new BoardView()

async function loadBoard(): Promise<BoardView> {
  if (!inWails()) return sampleView("")
  return BoardService.Board("", 0)
}

// subscribe re-runs fn on every board:changed (or sample change).
function subscribe(fn: () => void): () => void {
  if (!inWails()) return onSample(fn)
  return Events.On(EventBoardChanged, fn)
}

// useBoard loads every lane (the panel filters by repo and kind itself). It
// re-reads on board:changed (every task, comment or lease change, lease
// expiry too), and when the tab opens or the window regains focus.
export function useBoard(active: boolean): BoardState {
  const { value, loading, error, reload } = useLatest<BoardView>(
    loadBoard,
    emptyView
  )

  useEffect(() => {
    void reload()
    return subscribe(() => void reload())
  }, [reload])

  useEffect(() => {
    if (!active || !inWails()) return
    void reload()
    const onFocus = () => void reload()
    window.addEventListener("focus", onFocus)
    return () => window.removeEventListener("focus", onFocus)
  }, [active, reload])

  // A failed read has no next event to recover it (there is no poll), so
  // retry once a few seconds later while the tab is open.
  useEffect(() => {
    if (!active || !error) return
    const id = window.setTimeout(() => void reload(), BoardRetryMs)
    return () => window.clearTimeout(id)
  }, [active, error, reload])

  return { view: value, loading, error, reload }
}

const noDetail = new TaskDetail()

// useTaskDetail reads one card's task and comments, re-read on board:changed.
export function useTaskDetail(id: string): {
  detail: TaskDetail | null
  error: string | null
} {
  const load = useCallback(async (): Promise<TaskDetail> => {
    if (!id) return noDetail
    if (inWails()) return BoardService.Task(id)
    const s = sampleState()
    return new TaskDetail({
      task: sampleTask(id),
      comments: s.comments.filter((c) => c.taskId === id),
    })
  }, [id])
  const { value, loading, error, reload } = useLatest<TaskDetail>(
    load,
    noDetail
  )
  useEffect(() => {
    void reload()
    return subscribe(() => void reload())
  }, [reload])
  const detail = useMemo(
    () => (!id || loading || value.task.id !== id ? null : value),
    [id, loading, value]
  )
  return { detail, error }
}

// ---- derived ----------------------------------------------------------------

// isUser is a person's key.
export function isUser(key: string): boolean {
  return key.startsWith("user:")
}

// tellTargets are the agents a comment on t reaches (coord's recipients()
// rule, see tell.ts): a card held only by people reaches no agent.
export function tellTargets(t: Task): string[] {
  return tellTo(t.holders, t.createdBy)
}

// isTerminal is true for finished task states.
export function isTerminal(s: TaskState): boolean {
  return terminal(s)
}

// waitsOnPerson is a task blocked on a person (the person, at this device).
export function waitsOnPerson(t: Task): boolean {
  return (
    t.state === TaskState.Blocked &&
    t.blockedBy?.kind === BlockerKind.BlockedOnPerson
  )
}
