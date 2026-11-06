import { describe, expect, test } from "bun:test"

import {
  CardBlocker,
  CardDetail,
  CardSummary,
  Notification,
  NotificationAsk,
  TaskState,
} from "./models"
import { createPending, type Patch } from "./pending"

// fakeTimers is a hand-driven clock for the settle cap.
function fakeTimers() {
  let now = 0
  let next = 1
  const due = new Map<number, { at: number; fn: () => void }>()
  return {
    timers: {
      set: (fn: () => void, ms: number) => {
        const id = next++
        due.set(id, { at: now + ms, fn })
        return id as unknown as ReturnType<typeof setTimeout>
      },
      clear: (t: ReturnType<typeof setTimeout>) => {
        due.delete(t as unknown as number)
      },
    },
    advance(ms: number) {
      now += ms
      for (const [id, t] of [...due])
        if (t.at <= now) {
          due.delete(id)
          t.fn()
        }
    },
    count: () => due.size,
  }
}

const card = (p: Partial<CardDetail> = {}) =>
  CardDetail({ id: "c1", state: TaskState.Working, ...p })
const summary = (p: Partial<CardSummary> = {}) =>
  CardSummary({ id: "c1", state: TaskState.Working, ...p })

// A close of card c1: every part, agreeing once the card is over.
const isOver = (c: { state: string }) => c.state === TaskState.Completed
const closing = (key = "close:c1"): Patch => ({
  key,
  cardId: "c1",
  detail: (d) => ({ ...d, state: TaskState.Completed }),
  detailAgrees: isOver,
  summary: (c) => ({ ...c, state: TaskState.Completed }),
  summaryAgrees: isOver,
})

const asked = (noteKey = "n1"): Patch => ({
  key: "ask:a1",
  noteKey,
  notification: (n) => ({ ...n, read: true }),
  notificationAgrees: (n) => n.read,
})

describe("pending changes (card #644)", () => {
  test("add shows the patch at once on every part", () => {
    const p = createPending()
    let calls = 0
    p.subscribe(() => calls++)
    const v0 = p.version()
    p.add(closing())
    expect(calls).toBe(1)
    expect(p.version()).toBe(v0 + 1)
    expect(p.has("close:c1")).toBe(true)
    expect(p.touches("c1")).toBe(true)
    expect(p.touches("c2")).toBe(false)
    expect(p.detail(card()).state).toBe(TaskState.Completed)
    expect(p.detail(card({ id: "c2" })).state).toBe(TaskState.Working)
    const [a, b] = p.summaries([summary(), summary({ id: "c2" })])
    expect(a.state).toBe(TaskState.Completed)
    expect(b.state).toBe(TaskState.Working)
  })

  test("with nothing pending the lists come back as they are", () => {
    const p = createPending()
    const list = [summary()]
    const notes = [Notification({ key: "n1" })]
    expect(p.summaries(list)).toBe(list)
    expect(p.notifications(notes)).toBe(notes)
  })

  test("a notification patch touches only its notification", () => {
    const p = createPending()
    p.add(asked())
    const [n1, n2] = p.notifications([
      Notification({ key: "n1" }),
      Notification({ key: "n2" }),
    ])
    expect(n1.read).toBe(true)
    expect(n2.read).toBe(false)
    expect(p.touches("c1")).toBe(false)
  })

  test("undo takes the change back", () => {
    const p = createPending()
    const t = p.add(closing())
    p.undo("close:c1", t)
    expect(p.has("close:c1")).toBe(false)
    expect(p.detail(card()).state).toBe(TaskState.Working)
  })

  test("saved keeps it showing until a read agrees", () => {
    const clock = fakeTimers()
    const p = createPending(1000, clock.timers)
    const t = p.add(closing())
    p.saved("close:c1", t)
    clock.advance(999)
    expect(p.has("close:c1")).toBe(true)
    expect(p.detail(card()).state).toBe(TaskState.Completed)
  })

  test("a saved change goes at the settle cap when no read agrees", () => {
    const clock = fakeTimers()
    const p = createPending(1000, clock.timers)
    let calls = 0
    p.subscribe(() => calls++)
    const t = p.add(closing())
    // Not yet saved: no cap runs.
    clock.advance(5000)
    expect(p.has("close:c1")).toBe(true)
    p.saved("close:c1", t)
    expect(clock.count()).toBe(1)
    clock.advance(1000)
    expect(p.has("close:c1")).toBe(false)
    expect(calls).toBe(2)
    expect(p.detail(card()).state).toBe(TaskState.Working)
  })

  test("a stale token does nothing", () => {
    const clock = fakeTimers()
    const p = createPending(1000, clock.timers)
    const first = p.add(closing())
    const second = p.add(closing())
    expect(second).not.toBe(first)
    // The first change's save and failure come after it was replaced.
    p.saved("close:c1", first)
    expect(clock.count()).toBe(0)
    p.undo("close:c1", first)
    expect(p.has("close:c1")).toBe(true)
    // A replaced change's cap does not drop its successor.
    p.saved("close:c1", second)
    const third = p.add(closing())
    expect(clock.count()).toBe(0)
    clock.advance(1000)
    expect(p.has("close:c1")).toBe(true)
    p.undo("close:c1", third)
    expect(p.has("close:c1")).toBe(false)
  })

  test("unsubscribe stops the calls", () => {
    const p = createPending()
    let calls = 0
    const off = p.subscribe(() => calls++)
    off()
    p.add(closing())
    expect(calls).toBe(0)
  })
})

describe("real data agreeing (card #644)", () => {
  test("observeDetail drops the detail part that agrees, keeps the summary", () => {
    const p = createPending()
    p.add(closing())
    p.observeDetail(card({ state: TaskState.Completed }))
    expect(p.touches("c1")).toBe(false)
    expect(p.has("close:c1")).toBe(true)
    expect(p.detail(card()).state).toBe(TaskState.Working)
    expect(p.summaries([summary()])[0].state).toBe(TaskState.Completed)
  })

  test("observeDetail keeps a part that doesn't agree, and ignores other cards", () => {
    const p = createPending()
    p.add(closing())
    let calls = 0
    p.subscribe(() => calls++)
    p.observeDetail(card())
    p.observeDetail(card({ id: "c2", state: TaskState.Completed }))
    expect(calls).toBe(0)
    expect(p.touches("c1")).toBe(true)
    expect(p.detail(card()).state).toBe(TaskState.Completed)
  })

  test("a part with no agrees test is never dropped by a read", () => {
    const p = createPending()
    p.add({ ...closing(), detailAgrees: undefined })
    p.observeDetail(card({ state: TaskState.Completed }))
    expect(p.touches("c1")).toBe(true)
  })

  test("the change goes once every part agrees", () => {
    const p = createPending()
    p.add(closing())
    p.observeDetail(card({ state: TaskState.Completed }))
    p.observeSummaries([summary({ state: TaskState.Completed })])
    expect(p.has("close:c1")).toBe(false)
  })

  test("observeSummaries drops parts that agree and keeps ones that don't", () => {
    const p = createPending()
    p.add(closing())
    p.add({ ...closing("close:c2"), cardId: "c2" })
    p.observeSummaries([
      summary({ state: TaskState.Completed }),
      summary({ id: "c2" }),
    ])
    const [a, b] = p.summaries([summary(), summary({ id: "c2" })])
    expect(a.state).toBe(TaskState.Working)
    expect(b.state).toBe(TaskState.Completed)
    // c1's detail part still shows.
    expect(p.touches("c1")).toBe(true)
  })

  test("a card missing from the list agrees (it left the live list)", () => {
    const p = createPending()
    p.add(closing())
    let calls = 0
    p.subscribe(() => calls++)
    p.observeSummaries([summary({ id: "c2" })])
    expect(calls).toBe(1)
    expect(p.summaries([summary()])[0].state).toBe(TaskState.Working)
    p.observeDetail(card({ state: TaskState.Completed }))
    expect(p.has("close:c1")).toBe(false)
  })

  test("observeNotifications drops parts that agree and keeps ones that don't", () => {
    const p = createPending()
    p.add(asked())
    p.observeNotifications([Notification({ key: "n1" })])
    expect(p.has("ask:a1")).toBe(true)
    expect(p.notifications([Notification({ key: "n1" })])[0].read).toBe(true)
    p.observeNotifications([Notification({ key: "n1", read: true })])
    expect(p.has("ask:a1")).toBe(false)
  })

  test("a notification missing from the list agrees", () => {
    const p = createPending()
    p.add(asked())
    p.observeNotifications([Notification({ key: "n2" })])
    expect(p.has("ask:a1")).toBe(false)
  })

  test("agreement clears the settle cap's timer", () => {
    const clock = fakeTimers()
    const p = createPending(1000, clock.timers)
    const t = p.add(asked())
    p.saved("ask:a1", t)
    expect(clock.count()).toBe(1)
    p.observeNotifications([
      Notification({ key: "n1", read: true, ask: NotificationAsk() }),
    ])
    expect(clock.count()).toBe(0)
  })

  test("a part without its target never shows", () => {
    const p = createPending()
    // A detail with no cardId and a notification with no noteKey.
    p.add({
      key: "k",
      detail: (d) => ({ ...d, state: TaskState.Completed }),
      notification: (n) => ({ ...n, read: true }),
    })
    expect(p.touches("")).toBe(false)
    expect(p.detail(card({ id: "" })).state).toBe(TaskState.Working)
    expect(p.notifications([Notification()])[0].read).toBe(false)
    expect(
      p.detail(card({ blockers: [CardBlocker({ id: "b1" })] })).blockers
    ).toHaveLength(1)
  })
})
