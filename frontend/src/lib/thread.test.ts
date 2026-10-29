import { describe, expect, test } from "bun:test"

import {
  AgentRef,
  CardThreadItem,
} from "../../bindings/github.com/StephenSHorton/irori/models"
import { opensInFull, threadLabel, ThreadPage, visible } from "./thread"

const otter = new AgentRef({
  key: "dev/otter-session",
  name: "sample-otter",
  status: "busy",
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
