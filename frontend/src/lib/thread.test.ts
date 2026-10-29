import { describe, expect, test } from "bun:test"

import {
  AgentRef,
  CardBlocker,
  CardThreadItem,
} from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  answerable,
  blockerChip,
  BlockerState,
  blockerTone,
  needsPerson,
  opensInFull,
  threadLabel,
  ThreadPage,
  visible,
} from "./thread"

const otter = new AgentRef({
  key: "dev/otter-session",
  name: "sample-otter",
  status: "busy",
  live: true,
})
const wren = new AgentRef({
  key: "mac/wren-session",
  name: "mac/wren-session",
  status: "gone",
})

describe("threadLabel", () => {
  test("a comment names its author and has no arrow or badges", () => {
    const l = threadLabel(
      new CardThreadItem({ kind: "comment", who: otter, text: "hi" })
    )
    expect(l).toEqual({ author: "sample-otter", device: "", badges: [] })
  })

  test("a message gets the arrow, its device and its badges", () => {
    const l = threadLabel(
      new CardThreadItem({
        kind: "message",
        who: otter,
        to: wren,
        device: "MacBook",
        gone: true,
        held: true,
        files: 2,
      })
    )
    // A key the roster no longer names shows as its session part.
    expect(l.arrow).toBe("wren-session")
    expect(l.device).toBe("MacBook")
    expect(l.badges).toEqual([
      { label: "recipient gone", tone: "warning" },
      { label: "held", tone: "slate" },
      { label: "2 files", tone: "slate" },
    ])
  })

  test("a blocker answer names its item, except under that item", () => {
    const it = new CardThreadItem({
      kind: "comment",
      who: otter,
      to: wren,
      blockerId: "b2",
    })
    expect(threadLabel(it).badges).toEqual([{ label: "on b2", tone: "slate" }])
    expect(threadLabel(it, true).badges).toEqual([])
    // Its line in the card's thread is an event: no badge.
    expect(
      threadLabel(
        new CardThreadItem({ kind: "event", who: otter, blockerId: "b2" })
      ).badges
    ).toEqual([])
  })

  test("one file is singular", () => {
    const l = threadLabel(
      new CardThreadItem({ kind: "message", who: otter, to: wren, files: 1 })
    )
    expect(l.badges).toEqual([{ label: "1 file", tone: "slate" }])
  })
})

describe("visible", () => {
  const items = Array.from({ length: ThreadPage + 7 }, (_, i) => i)

  test("keeps the newest page", () => {
    const v = visible(items, false)
    expect(v.length).toBe(ThreadPage)
    expect(v[0]).toBe(7)
    expect(v[v.length - 1]).toBe(ThreadPage + 6)
  })

  test("shows everything when asked, or when it fits", () => {
    expect(visible(items, true)).toBe(items)
    expect(visible([1, 2], false)).toEqual([1, 2])
    expect(visible(items, false, 3)).toEqual([
      ThreadPage + 4,
      ThreadPage + 5,
      ThreadPage + 6,
    ])
  })
})

describe("opensInFull", () => {
  const msg = (over: Partial<CardThreadItem>) =>
    new CardThreadItem({
      id: "msg_1",
      kind: "message",
      who: otter,
      text: "hi",
      ...over,
    })

  test("a whole message without files has nothing more", () => {
    expect(opensInFull(msg({}))).toBe(false)
  })

  test("cut, held or with files, it opens in full", () => {
    expect(opensInFull(msg({ text: "xxx… (full: msg:msg_1)" }))).toBe(true)
    expect(opensInFull(msg({ text: "", held: true }))).toBe(true)
    expect(opensInFull(msg({ files: 2 }))).toBe(true)
  })

  test("a comment never does, nor another message's pointer", () => {
    expect(
      opensInFull(
        new CardThreadItem({
          id: "c1",
          kind: "comment",
          who: otter,
          held: true,
        })
      )
    ).toBe(false)
    expect(opensInFull(msg({ text: "see (full: msg:msg_2)" }))).toBe(false)
  })
})

describe("blocker states", () => {
  // Each state: its tone, whether it waits on the person, and whether the
  // person can still answer it.
  const cases: [string, string, boolean, boolean][] = [
    [BlockerState.Open, "amber", true, true],
    [BlockerState.Answered, "grey", false, true],
    [BlockerState.Replied, "amber", true, true],
    [BlockerState.Resolved, "green", false, false],
    [BlockerState.Closed, "closed", false, false],
  ]
  for (const [state, tone, needs, open] of cases)
    test(state, () => {
      expect(blockerTone(state)).toBe(tone as ReturnType<typeof blockerTone>)
      expect(needsPerson(state)).toBe(needs)
      expect(answerable(state)).toBe(open)
    })

  test("an item from an older app (no state) is open", () => {
    expect(blockerTone("")).toBe("amber")
    expect(needsPerson("")).toBe(true)
  })
})

describe("blockerChip", () => {
  const item = (over: Partial<CardBlocker>) =>
    new CardBlocker({ id: "b1", raisedBy: otter, ...over })

  test("answered says who it went to, and when they have ended", () => {
    expect(blockerChip(item({ state: "answered", sentTo: otter }))).toEqual({
      label: "Sent to sample-otter",
      tone: "slate",
    })
    expect(blockerChip(item({ state: "answered", sentTo: wren }))).toEqual({
      label: "Sent to wren-session · session ended",
      tone: "slate",
    })
    // Without sentTo, the raiser.
    expect(blockerChip(item({ state: "answered" }))?.label).toBe(
      "Sent to sample-otter"
    )
  })

  test("replied is back with the person; the rest have none", () => {
    expect(blockerChip(item({ state: "replied" }))).toEqual({
      label: "Replied",
      tone: "warning",
    })
    for (const state of ["open", "resolved", "closed", ""])
      expect(blockerChip(item({ state }))).toBeNull()
  })
})
