import { describe, expect, test } from "bun:test"

import {
  CardBlocker,
  CardDetail,
  CardOption,
  CardResolution,
  CardSummary,
  Notification,
  NotificationAsk,
  TaskState,
} from "./models"
import {
  answerKey,
  answerPatch,
  askKey,
  askPatch,
  closeKey,
  closePatch,
  createInstant,
  isDone,
  reasonOf,
} from "./optimistic"
import { createPending } from "./pending"

// deferred is a send the test settles by hand.
function deferred<T = void>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const tick = () => new Promise((r) => setTimeout(r, 0))

const closed = (d: { state: string }) => d.state === TaskState.Completed

function setup() {
  const pending = createPending()
  const toasts: [string, string][] = []
  const instant = createInstant(pending, () => (title, reason) => {
    toasts.push([title, reason])
  })
  return { pending, toasts, instant }
}

const card = (p: Partial<CardDetail> = {}) =>
  CardDetail({ id: "c1", state: TaskState.Working, ...p })
const summary = (p: Partial<CardSummary> = {}) =>
  CardSummary({ id: "c1", state: TaskState.Working, ...p })

describe("instant actions (card #644)", () => {
  test("applies the patch at once, before the save ends", () => {
    const { pending, instant } = setup()
    const send = deferred()
    expect(
      instant({
        key: closeKey("c1"),
        patch: closePatch("c1", TaskState.Completed),
        send: () => send.promise,
        failed: "Not closed",
      })
    ).toBe(true)
    expect(pending.detail(card()).state).toBe(TaskState.Completed)
  })

  test("saved keeps the change until real data agrees", async () => {
    const { pending, toasts, instant } = setup()
    const send = deferred<string>()
    let saved: string | undefined
    instant({
      key: closeKey("c1"),
      patch: closePatch("c1", TaskState.Completed),
      send: () => send.promise,
      failed: "Not closed",
      onSaved: (r) => (saved = r),
    })
    send.resolve("ok")
    await tick()
    expect(saved).toBe("ok")
    expect(toasts).toEqual([])
    expect(pending.detail(card()).state).toBe(TaskState.Completed)
    pending.observeDetail(card({ state: TaskState.Completed }))
    pending.observeSummaries([summary({ state: TaskState.Completed })])
    expect(pending.has(closeKey("c1"))).toBe(false)
  })

  test("a failure undoes the change, toasts the host's reason and calls onFailed", async () => {
    const { pending, toasts, instant } = setup()
    const send = deferred()
    let failed: string | undefined
    let savedCalled = false
    instant({
      key: answerKey("c1", "b1"),
      patch: closePatch("c1", TaskState.Completed),
      send: () => send.promise,
      failed: "Your answer wasn't saved",
      onSaved: () => (savedCalled = true),
      onFailed: (r) => (failed = r),
    })
    send.reject(new Error("already answered by civet"))
    await tick()
    expect(pending.has(answerKey("c1", "b1"))).toBe(false)
    expect(pending.detail(card()).state).toBe(TaskState.Working)
    expect(toasts).toEqual([
      ["Your answer wasn't saved", "already answered by civet"],
    ])
    expect(failed).toBe("already answered by civet")
    expect(savedCalled).toBe(false)
  })

  test("a send that throws at once fails the same way", async () => {
    const { pending, toasts, instant } = setup()
    instant({
      key: "k",
      patch: closePatch("c1", TaskState.Completed),
      send: () => {
        throw "no host"
      },
      failed: "Failed",
    })
    await tick()
    expect(pending.has("k")).toBe(false)
    expect(toasts).toEqual([["Failed", "no host"]])
  })

  test("one action per key: false while in flight, true after", async () => {
    const { instant } = setup()
    const first = deferred()
    let sends = 0
    const act = (p: Promise<unknown>) =>
      instant({
        key: askKey("a1"),
        send: () => {
          sends++
          return p
        },
        failed: "Failed",
      })
    expect(act(first.promise)).toBe(true)
    expect(act(Promise.resolve())).toBe(false)
    expect(sends).toBe(1)
    // Another key is not held up.
    expect(
      instant({ key: askKey("a2"), send: async () => {}, failed: "x" })
    ).toBe(true)
    first.resolve()
    await tick()
    expect(act(Promise.reject(new Error("no")))).toBe(true)
    await tick()
    // After a failure the key is free again too.
    expect(act(Promise.resolve())).toBe(true)
    expect(sends).toBe(3)
  })

  test("an action without a patch changes nothing pending", async () => {
    const { pending, instant } = setup()
    const before = pending.version()
    instant({ key: "k", send: async () => {}, failed: "x" })
    await tick()
    expect(pending.version()).toBe(before)
    expect(pending.has("k")).toBe(false)
  })

  test("reasonOf reads errors, message objects and strings", () => {
    expect(reasonOf(new Error("e"))).toBe("e")
    expect(reasonOf({ message: "m" })).toBe("m")
    expect(reasonOf("s")).toBe("s")
  })

  test("keys are the same wherever a thing is answered", () => {
    expect(answerKey("c1", "b1")).toBe("answer:c1/b1")
    expect(closeKey("c1")).toBe("close:c1")
    expect(askKey("a1")).toBe("ask:a1")
  })
})

// The done check's options, as the coord raises them.
const doneCheck = CardBlocker({
  id: "b1",
  doneCheck: true,
  state: "open",
  options: [CardOption({ answer: "Done" }), CardOption({ answer: "Not done" })],
})
const pick = CardBlocker({
  id: "b2",
  state: "open",
  options: [CardOption({ answer: "Yes" }), CardOption({ answer: "No" })],
})

// show runs a patch's detail and summary parts on c1.
function show(
  patch: ReturnType<typeof answerPatch>,
  d: CardDetail,
  s: CardSummary
) {
  const p = createPending()
  p.add({ ...patch, key: "k" })
  return { p, d: p.detail(d), s: p.summaries([s])[0] }
}

describe("answerPatch (card #644)", () => {
  test("isDone only for an option whose answer is Done on a done check", () => {
    expect(isDone(doneCheck, 0)).toBe(true)
    expect(isDone(doneCheck, 1)).toBe(false)
    expect(isDone(doneCheck, -1)).toBe(false)
    expect(isDone(doneCheck, 5)).toBe(false)
    expect(isDone({ ...doneCheck, doneCheck: false }, 0)).toBe(false)
    // A done check whose first option isn't "Done" doesn't close.
    const odd = {
      ...doneCheck,
      options: [CardOption({ answer: "Ship it" })],
    }
    expect(isDone(odd, 0)).toBe(false)
  })

  test("a pick marks the item answered and lowers the open count once", () => {
    const { d, s } = show(
      answerPatch("c1", pick, 0),
      card({ blockers: [pick, CardBlocker({ id: "b9", state: "open" })] }),
      summary({ openBlockers: 2, ready: true })
    )
    expect(d.state).toBe(TaskState.Working)
    expect(d.blockers[0].state).toBe("answered")
    expect(d.blockers[0].missing).toEqual([])
    expect(d.blockers[1].state).toBe("open")
    expect(s.openBlockers).toBe(1)
    // Not a done check: ready stays.
    expect(s.ready).toBe(true)
  })

  test("a read that already counts the answer isn't lowered twice", () => {
    const patch = answerPatch("c1", pick, 0)
    const p = createPending()
    p.add({ ...patch, key: "k" })
    expect(p.summaries([summary({ openBlockers: 2 })])[0].openBlockers).toBe(1)
    // The host's read: the answer already counted.
    const real = summary({ openBlockers: 1 })
    expect(p.summaries([real])[0].openBlockers).toBe(1)
    expect(patch.summaryAgrees!(real)).toBe(true)
    expect(patch.summaryAgrees!(summary({ openBlockers: 2 }))).toBe(false)
  })

  // The card page answers while the grid isn't mounted; the grid's first
  // read already counts the answer. With the count at the click (the
  // callers pass the card's page data), that read agrees and shows as is.
  test("a fresh read seen before any patched read shows the real count", () => {
    const p = createPending()
    p.add({ ...answerPatch("c1", pick, 0, 2), key: "k" })
    const fresh = [summary({ openBlockers: 1 })]
    p.observeSummaries(fresh)
    expect(p.summaries(fresh)[0].openBlockers).toBe(1)
  })

  test("adding to an answered item leaves the open count alone", () => {
    const answered = { ...pick, state: "answered" }
    const patch = answerPatch("c1", answered, -1, 1)
    expect(patch.summary).toBeUndefined()
    const p = createPending()
    p.add({ ...patch, key: "k" })
    // Another item still needs the person: the card stays in Needs you.
    expect(p.summaries([summary({ openBlockers: 1 })])[0].openBlockers).toBe(1)
  })

  test("the pick agrees once the item is shut, or gone", () => {
    const patch = answerPatch("c1", pick, 0)
    expect(patch.detailAgrees!(card({ blockers: [pick] }))).toBe(false)
    expect(
      patch.detailAgrees!(card({ blockers: [{ ...pick, state: "replied" }] }))
    ).toBe(false)
    expect(
      patch.detailAgrees!(card({ blockers: [{ ...pick, state: "answered" }] }))
    ).toBe(true)
    expect(
      patch.detailAgrees!(
        card({ blockers: [{ ...pick, resolved: CardResolution() }] })
      )
    ).toBe(true)
    expect(patch.detailAgrees!(card())).toBe(true)
  })

  test("a pick on an item already shut changes nothing", () => {
    const shut = { ...pick, state: "answered", missing: ["x"] }
    const before = card({ blockers: [shut] })
    const { d } = show(answerPatch("c1", pick, 1), before, summary())
    expect(d).toBe(before)
  })

  test("own words are an answer, not a close", () => {
    const { d, s } = show(
      answerPatch("c1", doneCheck, -1),
      card({ blockers: [doneCheck], ready: true }),
      summary({ openBlockers: 1, ready: true })
    )
    expect(d.state).toBe(TaskState.Working)
    expect(d.blockers[0].state).toBe("answered")
    // A done check answered: the card is no longer ready.
    expect(d.ready).toBeUndefined()
    expect(s.ready).toBeUndefined()
    expect(s.openBlockers).toBe(0)
  })

  test("Done on a done check closes the card and marks it done by you", () => {
    const patch = answerPatch("c1", doneCheck, 0)
    const { d, s } = show(
      patch,
      card({ blockers: [doneCheck, pick], ready: true }),
      summary({ openBlockers: 2, ready: true })
    )
    expect(d.state).toBe(TaskState.Completed)
    expect(d.ready).toBeUndefined()
    expect(d.blockers[0].state).toBe("resolved")
    expect(d.blockers[0].resolved?.answer).toBe("Done")
    expect(d.blockers[0].resolved?.option).toBe(0)
    expect(d.blockers[0].resolved?.by.name).toBe("You")
    expect(d.blockers[1]).toBe(pick)
    expect(s.state).toBe(TaskState.Completed)
    expect(s.ready).toBeUndefined()
    expect(patch.detailAgrees!(card())).toBe(false)
    expect(patch.detailAgrees!(card({ state: TaskState.Completed }))).toBe(true)
    expect(patch.summaryAgrees!(summary({ state: TaskState.Canceled }))).toBe(
      true
    )
  })

  test("Done leaves a card that is already over as it is", () => {
    const real = CardResolution({ answer: "Done", closed: true })
    const before = card({
      state: TaskState.Canceled,
      blockers: [{ ...doneCheck, resolved: real }],
    })
    const { d } = show(answerPatch("c1", doneCheck, 0), before, summary())
    expect(d.state).toBe(TaskState.Canceled)
    expect(d.blockers[0].resolved).toBe(real)
  })

  test("Not done goes back to the agent, the card stays open", () => {
    const { d, s } = show(
      answerPatch("c1", doneCheck, 1),
      card({ blockers: [doneCheck], ready: true }),
      summary({ openBlockers: 1, ready: true })
    )
    expect(d.state).toBe(TaskState.Working)
    expect(d.blockers[0].state).toBe("answered")
    expect(d.blockers[0].resolved).toBeNull()
    expect(d.ready).toBeUndefined()
    expect(s.state).toBe(TaskState.Working)
    expect(s.openBlockers).toBe(0)
  })

  test("the note item sets the card back to working and drops the question", () => {
    const note = CardBlocker({ id: "note", state: "open" })
    const patch = answerPatch("c1", note, -1)
    const { d, s } = show(
      patch,
      card({ state: TaskState.InputRequired, blockers: [note, pick] }),
      summary({ state: TaskState.InputRequired })
    )
    expect(d.state).toBe(TaskState.Working)
    expect(d.blockers.map((b) => b.id)).toEqual(["b2"])
    expect(s.state).toBe(TaskState.Working)
    expect(patch.detailAgrees!(card({ state: TaskState.InputRequired }))).toBe(
      false
    )
    expect(patch.detailAgrees!(card({ state: TaskState.Submitted }))).toBe(true)
    expect(patch.summaryAgrees!(summary())).toBe(true)
  })

  test("the note item leaves a card not waiting on it as it is", () => {
    const note = CardBlocker({ id: "note" })
    const { d, s } = show(
      answerPatch("c1", note, 0),
      card({ state: TaskState.Completed }),
      summary({ state: TaskState.Completed })
    )
    expect(d.state).toBe(TaskState.Completed)
    expect(s.state).toBe(TaskState.Completed)
  })
})

describe("closePatch (card #644)", () => {
  test("Done and Cancel close the card, agreeing once it is over", () => {
    for (const state of [TaskState.Completed, TaskState.Canceled]) {
      const patch = closePatch("c1", state)
      const { d, s } = show(
        patch,
        card({ ready: true }),
        summary({ ready: true })
      )
      expect(d.state).toBe(state)
      expect(d.ready).toBeUndefined()
      expect(s.state).toBe(state)
      expect(patch.detailAgrees!(card())).toBe(false)
      expect(patch.detailAgrees!(card({ state: TaskState.Failed }))).toBe(true)
      expect(patch.summaryAgrees!(summary({ state }))).toBe(true)
    }
  })

  test("a card already over keeps its state", () => {
    const { d } = show(
      closePatch("c1", TaskState.Canceled),
      card({ state: TaskState.Completed }),
      summary()
    )
    expect(closed(d)).toBe(true)
  })
})

describe("askPatch (card #644)", () => {
  const open = Notification({
    key: "n1",
    ask: NotificationAsk({ open: true, actions: ["Yes", "No"] }),
  })

  test("a pick shuts the ask with the choice at once", () => {
    const p = createPending()
    p.add({ ...askPatch("n1", "Yes"), key: askKey("a1") })
    const [n] = p.notifications([open])
    expect(n.ask?.open).toBe(false)
    expect(n.ask?.answered).toBe(true)
    expect(n.ask?.choice).toBe("Yes")
    expect(n.ask?.actions).toEqual(["Yes", "No"])
  })

  test("an ask already shut, or no ask, is left as it is", () => {
    const patch = askPatch("n1", "Yes")
    const shut = Notification({
      key: "n1",
      ask: NotificationAsk({ choice: "No", answered: true }),
    })
    expect(patch.notification!(shut)).toBe(shut)
    const plain = Notification({ key: "n1" })
    expect(patch.notification!(plain)).toBe(plain)
  })

  test("it agrees once the ask is shut", () => {
    const patch = askPatch("n1", "Yes")
    expect(patch.notificationAgrees!(open)).toBe(false)
    expect(
      patch.notificationAgrees!(
        Notification({ key: "n1", ask: NotificationAsk() })
      )
    ).toBe(true)
    expect(patch.cardId).toBeUndefined()
  })
})
