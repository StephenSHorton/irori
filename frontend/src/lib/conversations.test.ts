import { describe, expect, test } from "bun:test"

import {
  agentsNowFor,
  dmGone,
  dmPerson,
  fromPost,
  listed,
  readAll,
  replyingLine,
  talkApi,
  talkWaiting,
  waitingText,
  type Conversation,
} from "./conversations"
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
          conv({
            kind: "dm",
            unread: 3,
            readOnly: true,
            members: [agent("heron"), agent("otter")],
          }),
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
    const ended = conv({
      kind: "dm",
      unread: 1,
      members: [you, agent("ended")],
    })
    expect(dmGone(gone, live)).toBe(true)
    expect(dmGone(ended, live)).toBe(true)
    expect(talkWaiting([gone, ended], live)).toBe(0)
    // A DM with a person only is never gone.
    expect(dmGone(conv({ kind: "dm", members: [you] }), live)).toBe(false)
    // Back on the roster, it counts again.
    expect(
      talkWaiting([gone], new Map([["tulip", { status: "working" }]]))
    ).toBe(1)
  })
})

describe("Make a card from a message (design §4, card #390)", () => {
  test("the first line is the title; the message and where it was said, the objective", () => {
    const f = fromPost("Tag 0.10.0 once #96 merges\nnot before", "#releases")
    expect(f.title).toBe("Tag 0.10.0 once #96 merges")
    expect(f.objective).toBe(
      "Tag 0.10.0 once #96 merges\nnot before\n\n(From a message in #releases.)"
    )
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
    expect(
      cs.some(
        (c) =>
          c.spaceId === "sample-board-mlg" && c.convId === "sample-ch-netcode"
      )
    ).toBe(true)
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

describe("Start a thread on a channel message (card #454)", () => {
  test("the thread starts and the channel gets its one-line pointer", async () => {
    const before = await talkApi.posts("sample-ch-suzuri")
    const [about, n] = [before[0], before.length]
    await talkApi.threadFrom(
      "sample-ch-suzuri",
      about.postId,
      "Why is that?\n\n> quoted\n(you, in #suzuri)"
    )
    const after = await talkApi.posts("sample-ch-suzuri")
    expect(after.length).toBe(n + 1)
    const pointer = after[after.length - 1]
    expect(pointer.threadFrom).toBe(about.postId)
    expect(pointer.text).toMatch(/^#\d+$/)
    expect(pointer.agentsNow).toBe(false)
  })
})

describe("a DM with another person (card #538)", () => {
  const m = (kind: string, key: string, name = "") => ({
    kind,
    key,
    name,
    status: "",
    owner: "",
  })
  test("dmPerson is the other person, never you or an agent", () => {
    const bob = conv({
      kind: "dm",
      members: [
        m("person", "user:dev1", "you"),
        m("person", "person:bob", "Bob"),
      ],
    })
    expect(dmPerson(bob)?.key).toBe("person:bob")
    const agent = conv({
      kind: "dm",
      members: [m("person", "user", "you"), m("agent", "api:a1", "A1")],
    })
    expect(dmPerson(agent)).toBeUndefined()
    // A shared board's sender isn't a person DM's other side.
    const sender = conv({
      kind: "dm",
      members: [
        m("person", "user", "you"),
        m("person", "board:b1/person:sam", "Sam"),
      ],
    })
    expect(dmPerson(sender)).toBeUndefined()
    expect(
      dmPerson(conv({ kind: "channel", members: bob.members }))
    ).toBeUndefined()
  })
  test("its waiting line names them", () => {
    expect(waitingText({ waiting: "them" }, "Bob")).toBe(
      "Waiting for Bob's Irori"
    )
    expect(waitingText({ waiting: "update" }, "Bob")).toBe(
      "Bob's Irori needs updating"
    )
    expect(waitingText({}, "Bob")).toBe("")
  })
  test("dmGone never archives it", () => {
    const bob = conv({
      kind: "dm",
      members: [m("person", "user", "you"), m("person", "person:bob", "Bob")],
    })
    expect(dmGone(bob, new Map())).toBe(false)
  })
})

describe("replyingLine (card #645)", () => {
  const now = Date.parse("2026-11-06T12:00:00Z")
  const ago = (s: number) => new Date(now - s * 1000).toISOString()
  const a = (name: string, s = 5, key = `k-${name}`) => ({
    key,
    name,
    since: ago(s),
  })

  test("nobody: no row", () => {
    expect(replyingLine([], now)).toBeNull()
  })

  test("one agent is replying", () => {
    expect(replyingLine([a("jasmine")], now)?.text).toBe("jasmine is replying…")
  })

  test("several share one row", () => {
    expect(replyingLine([a("jasmine"), a("kestrel")], now)).toEqual({
      who: "jasmine and kestrel",
      verb: "are",
      text: "jasmine and kestrel are replying…",
    })
    expect(replyingLine([a("a"), a("b"), a("c")], now)?.who).toBe("a, b and c")
  })

  test("past 30 minutes an agent drops off", () => {
    expect(
      replyingLine([a("jasmine", 31 * 60), a("kestrel", 60)], now)?.text
    ).toBe("kestrel is replying…")
    expect(replyingLine([a("jasmine", 31 * 60)], now)).toBeNull()
  })

  test("never a key, never twice", () => {
    const key =
      "3dc30c94c6a74af3c59d39f4ca02d6f1/7ff86a62-6cc4-4916-be2d-602034fc281a"
    expect(replyingLine([a(key), a(" ")], now)).toBeNull()
    expect(
      replyingLine([a("oak", 5, "k1"), a("oak", 5, "k1")], now)?.text
    ).toBe("oak is replying…")
  })
})
