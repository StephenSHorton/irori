import { describe, expect, test } from "bun:test"

import { agentsNowFor, fromPost, listed, readAll, talkApi, unreadDms, type Conversation } from "./conversations"
import { TalkConversation } from "./models"

const conv = (p: Partial<Conversation>): Conversation => TalkConversation(p)

describe("Agents answer now, by default (design §3, card #390)", () => {
  test("on when nobody is named or an agent is", () => {
    expect(agentsNowFor([])).toBe(true)
    expect(agentsNowFor([{ kind: "agent" }])).toBe(true)
    expect(agentsNowFor([{ kind: "person" }, { kind: "agent" }])).toBe(true)
  })
  test("off when only people are named", () => {
    expect(agentsNowFor([{ kind: "person" }])).toBe(false)
  })
})

describe("Talk's list (card #390)", () => {
  test("channels by name, DMs newest first, threads left to useThreads", () => {
    const got = listed([
      conv({ convId: "c2", kind: "channel", name: "suzuri" }),
      conv({ convId: "d1", kind: "dm", lastAt: "2026-11-03T10:00:00Z" }),
      conv({ convId: "t", kind: "thread" }),
      conv({ convId: "c1", kind: "channel", name: "irori" }),
      conv({ convId: "d2", kind: "dm", lastAt: "2026-11-03T11:00:00Z" }),
    ])
    expect(got.channels.map((c) => c.convId)).toEqual(["c1", "c2"])
    expect(got.dms.map((c) => c.convId)).toEqual(["d2", "d1"])
  })
  test("Talk's count adds the DMs with something new, not agents' own DMs", () => {
    expect(
      unreadDms([
        conv({ kind: "dm", unread: 2 }),
        conv({ kind: "dm", unread: 0 }),
        conv({ kind: "dm", unread: 3, readOnly: true }),
        conv({ kind: "channel", unread: 5 }),
      ])
    ).toBe(1)
  })
})

describe("Make a card from a message (design §4, card #390)", () => {
  test("the first line is the title; the message and where it was said, the objective", () => {
    const f = fromPost("Tag 0.10.0 once #96 merges\nnot before", "#releases")
    expect(f.title).toBe("Tag 0.10.0 once #96 merges")
    expect(f.objective).toBe("Tag 0.10.0 once #96 merges\nnot before\n\n(From a message in #releases.)")
  })
  test("a long first line is cut to 80", () => {
    const f = fromPost("x".repeat(200), "a DM")
    expect(f.title.length).toBe(80)
    expect(f.title.endsWith("…")).toBe(true)
  })
})

describe("the Archive's Mark all read (card #473)", () => {
  test("reads each unread DM given up to its newest post, on the sample", async () => {
    const before = await talkApi.conversations()
    const one = before.filter((c) => c.convId === "sample-dm-gone-new")
    expect(one[0]?.unread).toBe(1)
    await readAll(one)
    const after = await talkApi.conversations()
    expect(after.find((c) => c.convId === "sample-dm-gone-new")?.unread).toBe(0)
    // Only the ones given are read.
    expect(after.find((c) => c.convId === "sample-dm-heron")?.unread).toBe(
      before.find((c) => c.convId === "sample-dm-heron")?.unread
    )
  })
})
