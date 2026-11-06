import { describe, expect, test } from "bun:test"

import {
  agentsNowFor,
  dmGone,
  dmPerson,
  fromPost,
  listed,
  postLine,
  readAll,
  talkApi,
  talkWaiting,
  waitingText,
  type Conversation,
} from "./conversations"
import { TalkConversation, TalkPost, type FileRef } from "./models"

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
  test("a message of files alone takes its first file's name (card #643)", () => {
    const f = fromPost("", "#suzuri", [ref("shot.png"), ref("log.txt")])
    expect(f.title).toBe("shot.png")
    expect(f.objective).toBe(
      "Files: shot.png, log.txt\n\n(From a message in #suzuri.)"
    )
  })
})

const ref = (name: string): FileRef => ({
  sha256: "a".repeat(64),
  name,
  size: 10,
  device: "d",
})

describe("files on a post (card #643)", () => {
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }
  const file = (name: string, type = "image/png") =>
    new File([new Uint8Array(1234)], name, { type })

  test("the sample keeps each file's name and size, with its own sha", async () => {
    const id = await talkApi.post(
      "sample-ch-releases",
      "the installer",
      true,
      null,
      [],
      [file("a.png"), file("b.png")]
    )
    const p = (await talkApi.posts("sample-ch-releases")).find(
      (x) => x.postId === id
    )
    expect(p?.attachments.map((f) => [f.name, f.size, f.mime])).toEqual([
      ["a.png", 1234, "image/png"],
      ["b.png", 1234, "image/png"],
    ])
    const [a, b] = p?.attachments ?? []
    expect(a.sha256).toMatch(/^[0-9a-f]{64}$/)
    expect(a.sha256).not.toBe(b.sha256)
  })

  test("files alone make a post, and the list shows the file's name", async () => {
    const id = await talkApi.post(
      "sample-ch-irori",
      "  ",
      true,
      null,
      [],
      [file("shot.png")]
    )
    const p = (await talkApi.posts("sample-ch-irori")).find(
      (x) => x.postId === id
    )
    expect(p?.attachments[0]?.name).toBe("shot.png")
    const c = (await talkApi.conversations()).find(
      (x) => x.convId === "sample-ch-irori"
    )
    expect(c?.lastText).toBe("📎 shot.png")
  })

  test("no text and no files is refused", async () => {
    await expect(talkApi.post("sample-ch-irori", " ", true)).rejects.toThrow()
  })

  test("postLine: the text, else the first file and how many more", () => {
    expect(postLine(undefined)).toBe("")
    expect(postLine(TalkPost({ text: " a\n b " }))).toBe("a b")
    expect(
      postLine(TalkPost({ attachments: [ref("x.png"), ref("y.png")] }))
    ).toBe("📎 x.png +1")
  })

  test("the sample's suzuri channel and heron DM each have a picture", async () => {
    for (const id of ["sample-ch-suzuri", "sample-dm-heron"]) {
      const ps = await talkApi.posts(id)
      expect(
        ps.some((p) => p.attachments.some((f) => f.mime === "image/png"))
      ).toBe(true)
    }
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
