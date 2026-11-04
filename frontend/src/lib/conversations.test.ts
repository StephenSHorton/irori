import { describe, expect, test } from "bun:test"

import { agentsNowFor, dmGone, fromPost, listed, readAll, talkApi, talkWaiting, type Conversation } from "./conversations"
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
  const you = { kind: "person", key: "person:me", name: "you" }
  const agent = (key: string) => ({ kind: "agent", key, name: key })
  const live = new Map([
    ["heron", { status: "idle" }],
    ["ended", { status: "gone" }],
  ])

  test("Talk's count: your DMs with something new and channels that mention you (card #484)", () => {
    expect(
      talkWaiting(
        [
          conv({ kind: "dm", unread: 2, members: [you, agent("heron")] }),
          conv({ kind: "dm", unread: 0, members: [you, agent("heron")] }),
          // Agents' own DMs, read only.
          conv({ kind: "dm", unread: 3, readOnly: true, members: [agent("heron"), agent("otter")] }),
          // A channel with something new counts only when it mentions you.
          conv({ kind: "channel", unread: 5 }),
          conv({ kind: "channel", unread: 2, mentions: 1 }),
        ],
        live
      )
    ).toBe(2)
  })

  test("a DM with an agent that has gone leaves Talk's count, unread or not", () => {
    const gone = conv({ kind: "dm", unread: 1, members: [you, agent("tulip")] })
    const ended = conv({ kind: "dm", unread: 1, members: [you, agent("ended")] })
    expect(dmGone(gone, live)).toBe(true)
    expect(dmGone(ended, live)).toBe(true)
    expect(talkWaiting([gone, ended], live)).toBe(0)
    // A DM with a person only is never gone.
    expect(dmGone(conv({ kind: "dm", members: [you] }), live)).toBe(false)
    // Back on the roster, it counts again.
    expect(talkWaiting([gone], new Map([["tulip", { status: "working" }]]))).toBe(1)
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

describe("channels in a space (card #461)", () => {
  // The sample runs outside the host; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("the sample makes a channel in the space it's given, and in Personal with none", async () => {
    const inMlg = await talkApi.createChannel("lobby", null, "sample-board-mlg")
    const mine = await talkApi.createChannel("lobby", null)
    expect(inMlg).not.toBe(mine)
    const cs = await talkApi.conversations()
    expect(cs.find((c) => c.convId === inMlg)?.spaceId).toBe("sample-board-mlg")
    expect(cs.find((c) => c.convId === mine)?.spaceId).toBeUndefined()
    expect(cs.some((c) => c.spaceId === "sample-board-mlg" && c.convId === "sample-ch-netcode")).toBe(true)
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
