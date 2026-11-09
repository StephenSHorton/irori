import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test"

import {
  buildLines,
  forgetAsking,
  isAsking,
  letItAnswer,
  toastsFor,
  type Channel,
  type TalkCard,
} from "./assistant-talk-follow"
import type { Member, Post } from "./conversations"
import { AssistantService } from "./host"
import { replyLine } from "./talk-reply"

const card: TalkCard = {
  space: "s1",
  card: "MINE",
  conv: "ch_oa_s1",
  owner: "Sam",
}
const MINE = "agent:MINE"
const JUNO = "agent:JUNO"

const members: Member[] = [
  { kind: "person", key: "user", name: "you" },
  { kind: "person", key: "person:eve", name: "Eve" },
  { kind: "agent", key: MINE, name: "Sam's assistant", assistant: true },
  {
    kind: "agent",
    key: JUNO,
    name: "Juno · Eve's assistant",
    owner: "Eve",
    assistant: true,
  },
  { kind: "agent", key: "agent:otter", name: "sample-otter", owner: "Eve" },
]

let n = 0
const post = (by: string, extra: Partial<Post> = {}): Post =>
  ({
    postId: `p${n++}`,
    convId: card.conv,
    by,
    text: "t",
    mentions: [],
    agentsNow: true,
    attachments: [],
    at: new Date(1000 + n * 1000).toISOString(),
    ...extra,
  }) as Post

const channel = (posts: Post[]): Channel => ({
  card,
  spaceName: "mlg",
  conv: { convId: card.conv, members } as never,
  posts,
})

describe("buildLines", () => {
  test("an answer to my question is an answer line", () => {
    const q = post("user", { text: "Who reviews?" })
    const a = post(JUNO, { replyTo: q.postId, text: "Eve can." })
    const out = buildLines([channel([q, a])])
    expect(out).toHaveLength(1)
    expect(out[0].from).toMatchObject({
      kind: "answer",
      name: "Juno · Eve's assistant",
      question: "Who reviews?",
      spaceName: "mlg",
      postId: q.postId,
    })
    expect(out[0].text).toBe("Eve can.")
    // Someone else's answer to someone else's question isn't mine.
    const other = post("person:eve", { text: "hm" })
    const b = post(JUNO, { replyTo: other.postId })
    expect(buildLines([channel([other, b])])).toEqual([])
  })
  test("a waiting line is a needs line until my assistant really answers", () => {
    const q = post("person:eve", { text: "What's next?" })
    const w = post(MINE, { replyTo: q.postId, waiting: "ask" })
    let out = buildLines([channel([q, w])])
    expect(out.map((e) => e.from?.kind)).toEqual(["needs"])
    expect(out[0].from).toMatchObject({ waiting: "ask", name: "Eve" })
    // Let it answer posts the answer: the needs line goes, an asked line comes.
    const a = post(MINE, { replyTo: q.postId, text: "On mlg, Sam holds #14." })
    out = buildLines([channel([q, w, a])])
    expect(out.map((e) => e.from?.kind)).toEqual(["asked"])
    expect(out[0].text).toBe("On mlg, Sam holds #14.")
  })
  test("an owner waiting line carries its kind", () => {
    const q = post("agent:otter", { text: "Can Sam review?" })
    const w = post(MINE, { replyTo: q.postId, waiting: "owner" })
    const [l] = buildLines([channel([q, w])])
    expect(l.from).toMatchObject({
      kind: "needs",
      waiting: "owner",
      name: "sample-otter, Eve's agent",
    })
  })
  test("lines are plain text, clipped to 1,500 characters", () => {
    const q = post("user")
    const a = post(JUNO, { replyTo: q.postId, text: "x".repeat(4000) })
    const [l] = buildLines([channel([q, a])])
    expect([...l.text].length).toBe(1501)
  })
})

describe("toasts", () => {
  test("a new needs line rings once per read; old ones don't", () => {
    const q = post("person:eve", { text: "What's next?" })
    const w = post(MINE, { replyTo: q.postId, waiting: "ask" })
    const lines = buildLines([channel([q, w])])
    expect(toastsFor(lines, 0, [card])).toEqual([
      {
        conv: card.conv,
        title: "Sam's assistant",
        body: "Eve asked: What's next?",
      },
    ])
    expect(toastsFor(lines, lines[0].at!, [card])).toEqual([])
  })
  test("an answer rings only for my assistant's own question", () => {
    const typed = post("user")
    const a = post(JUNO, { replyTo: typed.postId })
    const mine = post(MINE)
    const b = post(JUNO, { replyTo: mine.postId })
    const lines = buildLines([channel([typed, a, mine, b])])
    expect(lines.map((l) => l.from?.kind)).toEqual(["answer", "answer"])
    const out = toastsFor(lines, 0, [card])
    expect(out).toHaveLength(1)
    expect(out[0].title).toBe("Sam's assistant")
  })
})

describe("Let it answer", () => {
  let req: ReturnType<typeof spyOn>
  beforeEach(() => {
    forgetAsking()
    req = spyOn(AssistantService, "AnswerRequest").mockResolvedValue("r1")
  })
  afterEach(() => req.mockRestore())

  test("a second tap while asking does nothing", async () => {
    const f = { postId: "p", space: "s1" }
    expect(await letItAnswer(f)).toBe("")
    expect(isAsking("p")).toBe(true)
    expect(await letItAnswer(f)).toBe("")
    expect(req).toHaveBeenCalledTimes(1)
    expect(req).toHaveBeenCalledWith("p", "s1", "let")
    // Lapsed, it can be tapped again.
    expect(await letItAnswer(f, Date.now() + 60_000)).toBe("")
    expect(req).toHaveBeenCalledTimes(2)
  })
  test("a failure says why and lets the tap be made again", async () => {
    req.mockRejectedValueOnce(new Error("no assistant here"))
    const f = { postId: "q", space: "s1" }
    expect(await letItAnswer(f)).toBe("no assistant here")
    expect(isAsking("q")).toBe(false)
  })
})

describe("the Talk reply line", () => {
  const posts = [{ postId: "q1", by: "person:eve" }]
  test("answered, and waiting", () => {
    expect(replyLine({ by: JUNO, replyTo: "q1" }, posts, members, "mlg")).toBe(
      "Answered from what is on mlg · replying to Eve"
    )
    expect(
      replyLine(
        { by: MINE, replyTo: "q1", waiting: "ask" },
        posts,
        [
          ...members.filter((m) => m.key !== MINE),
          {
            kind: "agent",
            key: MINE,
            name: "Sam's assistant",
            assistant: true,
          },
        ],
        "mlg"
      )
    ).toBe("Waiting on Sam · replying to Eve")
  })
  test("nothing for an ordinary agent or a post that replies to nothing", () => {
    expect(
      replyLine({ by: "agent:otter", replyTo: "q1" }, posts, members, "mlg")
    ).toBe("")
    expect(replyLine({ by: JUNO }, posts, members, "mlg")).toBe("")
  })
})
