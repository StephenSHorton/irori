import { describe, expect, test } from "bun:test"

import {
  createFollower,
  facts,
  finished,
  news,
  parseSeen,
  quiet,
  workThread,
  type Fact,
  type SeenStore,
  type WorkCard,
  type WorkThread,
} from "./assistant-follow"

const thread = (over: Partial<WorkThread> = {}): WorkThread => ({
  id: "t1",
  ref: "#20",
  taker: "",
  cantStart: "",
  closed: false,
  outcome: "",
  asks: [],
  cards: [],
  ...over,
})

const card = (over: Partial<WorkCard> = {}): WorkCard => ({
  id: "c1",
  ref: "#21",
  title: "Fix the typo on Home",
  state: "submitted",
  working: [],
  asks: [],
  ...over,
})

const keys = (fs: Fact[]) => fs.map((f) => f.key)

describe("Ask Irori follows the work it posted (card #785)", () => {
  test("a thread nobody took yet says nothing", () => {
    expect(facts(thread())).toEqual([])
  })

  test("the life of a small fix, in order", () => {
    const t = thread({
      taker: "otter",
      cards: [
        card({
          state: "working",
          working: ["otter"],
          asks: [
            {
              id: "b1",
              question: "Merge PR #9?",
              by: "otter",
              doneCheck: false,
              permission: false,
            },
          ],
        }),
      ],
    })
    const fs = facts(t)
    expect(keys(fs)).toEqual(["taken", "made:c1", "start:c1", "ask:c1:b1"])
    expect(fs[0].text).toBe("**otter** took your thread #20.")
    expect(fs[1].text).toBe("Your thread #20 became #21.")
    expect(fs[2].text).toBe("otter started on #21.")
    expect(fs[3].text).toBe("#21 needs you: Merge PR #9?")
    expect(fs[3].act).toEqual({ kind: "answer", card: "c1", blocker: "b1" })
    // Only what waits on the person is said (card #1000).
    expect(fs.filter((f) => !f.silent).map((f) => f.key)).toEqual(["ask:c1:b1"])
  })

  test("a done check is ready, with Done to tap; a permission has no act", () => {
    const fs = facts(
      thread({
        cards: [
          card({
            state: "in_review",
            asks: [
              {
                id: "d1",
                question: "Done?",
                by: "otter",
                doneCheck: true,
                permission: false,
              },
              {
                id: "p1",
                question: "Run `git push`?",
                by: "",
                doneCheck: false,
                permission: true,
              },
            ],
          }),
        ],
      })
    )
    const ready = fs.find((f) => f.key === "ready:c1:d1")
    expect(ready?.text).toBe("#21 is ready: otter says it's done.")
    expect(ready?.act?.kind).toBe("answer")
    const perm = fs.find((f) => f.key === "perm:c1:p1")
    expect(perm?.act).toBeUndefined()
  })

  test("a card over says how it ended, and its old asks are not said", () => {
    const asks = [
      {
        id: "b1",
        question: "?",
        by: "",
        doneCheck: false,
        permission: false,
      },
    ]
    expect(
      keys(facts(thread({ cards: [card({ state: "completed", asks })] })))
    ).toEqual(["made:c1", "done:c1"])
    const failed = facts(thread({ cards: [card({ state: "failed" })] }))
    expect(failed[1].text).toBe("#21 failed.")
    const canceled = facts(thread({ cards: [card({ state: "canceled" })] }))
    expect(canceled[1].text).toBe("#21 was canceled.")
  })

  test("a thread resolved with no card made says so; with cards it doesn't", () => {
    expect(keys(facts(thread({ closed: true })))).toEqual(["closed"])
    expect(keys(facts(thread({ closed: true, cards: [card()] })))).toEqual([
      "made:c1",
    ])
  })

  test("no agent could start: said until one takes it", () => {
    const t = thread({ cantStart: "No agent can start on this device." })
    expect(keys(facts(t))).toEqual(["cant"])
    expect(keys(facts({ ...t, taker: "otter" }))).toEqual(["taken"])
  })

  test("a long question is cut, the card keeps it whole", () => {
    const question = "word ".repeat(200)
    const f = facts(
      thread({
        asks: [
          { id: "b1", question, by: "", doneCheck: false, permission: false },
        ],
      })
    )[0]
    expect(f.text.length).toBeLessThan(320)
    expect(f.text.endsWith("…")).toBe(true)
  })

  test("finished: resolved and every card over", () => {
    expect(finished(thread())).toBe(false)
    expect(finished(thread({ closed: true }))).toBe(true)
    expect(
      finished(thread({ closed: true, cards: [card({ state: "working" })] }))
    ).toBe(false)
    expect(
      finished(thread({ closed: true, cards: [card({ state: "completed" })] }))
    ).toBe(true)
  })
})

describe("what the chat has said, once (card #785)", () => {
  test("a thread it just posted says each new fact once", () => {
    let seen = { said: [] as string[] }
    const ask = (id: string) => ({
      id,
      question: "?",
      by: "",
      doneCheck: false,
      permission: false,
    })
    const t1 = thread({ asks: [ask("b1")] })
    const a = news(seen, t1)
    expect(keys(a.say)).toEqual(["ask:t1:b1"])
    seen = a.seen as { said: string[] }
    expect(news(seen, t1).say).toEqual([])
    const t2 = thread({ asks: [ask("b1"), ask("b2")] })
    expect(keys(news(seen, t2).say)).toEqual(["ask:t1:b2"])
  })

  test("a thread from before (said unknown) only remembers at first", () => {
    const t = thread({
      taker: "otter",
      cards: [card({ state: "working", working: ["otter"] })],
    })
    const a = news({ said: null }, t)
    expect(a.say).toEqual([])
    expect(a.seen.said).toEqual(["taken", "made:c1", "start:c1"])
    expect(news(undefined, t).say).toEqual([])
  })

  test("over once finished", () => {
    const t = thread({ closed: true, cards: [card({ state: "completed" })] })
    expect(news({ said: [] }, t).seen.done).toBe(true)
    expect(news({ said: [] }, thread()).seen.done).toBeUndefined()
  })

  test("parseSeen keeps only well-formed entries", () => {
    expect(parseSeen(null)).toEqual({})
    expect(parseSeen("nope")).toEqual({})
    expect(parseSeen("[1]")).toEqual({})
    expect(
      parseSeen(
        JSON.stringify({
          a: { said: ["taken", 3], done: true },
          b: { said: "x" },
          c: 5,
        })
      )
    ).toEqual({ a: { said: ["taken"], done: true }, b: { said: null } })
  })
})

describe("the follower (card #785)", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0))

  function harness(ids: string[], store: SeenStore = {}) {
    let work = new Map<string, WorkThread | null>()
    let wake: () => void = () => {}
    const said: { id: string; keys: string[]; gone: boolean }[] = []
    let reads = 0
    let saved: SeenStore = store
    const f = createFollower({
      ids: () => ids,
      read: async (want) => {
        reads++
        return new Map([...work].filter(([id]) => want.includes(id)))
      },
      listen: (fn) => {
        wake = fn
        return () => {}
      },
      load: () => store,
      save: (s) => {
        saved = JSON.parse(JSON.stringify(s))
      },
      say: (id, lines, w) => said.push({ id, keys: keys(lines), gone: !w }),
      delayMs: 0,
      quietMs: 10_000,
    })
    return {
      f,
      said,
      set: (id: string, w: WorkThread | null) => {
        work = new Map(work).set(id, w)
      },
      wake: () => wake(),
      reads: () => reads,
      saved: () => saved,
    }
  }

  test("a posted thread is followed from nothing said, then each change", async () => {
    const h = harness(["t1"])
    await flush()
    h.set("t1", thread({ taker: "otter" }))
    h.f.follow("t1")
    await flush()
    expect(h.said.at(-1)?.keys).toEqual([])
    h.set("t1", thread({ taker: "otter", cards: [card()] }))
    h.wake()
    await new Promise((r) => setTimeout(r, 5))
    // Said nothing, but remembered: made:c1 keeps the card followed.
    expect(h.said.at(-1)?.keys).toEqual([])
    expect(h.saved().t1.said).toEqual(["taken", "made:c1"])
  })

  test("a finished thread is read no more; a gone one neither", async () => {
    const h = harness(["t1", "t2"], { t1: { said: [] }, t2: { said: [] } })
    h.set("t1", thread({ closed: true }))
    h.set("t2", null)
    h.wake()
    await new Promise((r) => setTimeout(r, 5))
    expect(h.said.find((s) => s.id === "t1")?.keys).toEqual([])
    expect(h.said.find((s) => s.id === "t2")?.gone).toBe(true)
    const n = h.reads()
    h.wake()
    await new Promise((r) => setTimeout(r, 5))
    expect(h.reads()).toBe(n)
  })

  test("threads that left the chat are forgotten", async () => {
    const h = harness([], { old: { said: ["taken"] } })
    await flush()
    expect(Object.keys(h.saved())).toEqual([])
    expect(h.reads()).toBe(0)
  })
})

describe("a thread closed as a card is that card (card #785)", () => {
  test("its agent's card outcome makes it the first card, same #", () => {
    const t = thread({
      taker: "otter",
      cards: [
        card({ id: "t1", ref: "#20", state: "working", working: ["wren"] }),
      ],
    })
    const fs = facts(t)
    expect(keys(fs)).toEqual(["taken", "made:t1", "start:t1"])
    expect(fs[1].text).toBe("Your thread #20 is now a card.")
    expect(finished(t)).toBe(false)
    const done = thread({
      closed: true,
      cards: [card({ id: "t1", ref: "#20", state: "completed" })],
    })
    expect(keys(facts(done))).toEqual(["made:t1", "done:t1"])
    expect(finished(done)).toBe(true)
  })
  test("answered says so", () => {
    expect(facts(thread({ closed: true, outcome: "answered" }))[0].text).toBe(
      "#20 was answered."
    )
  })
})

describe("review fixes (card #785)", () => {
  const ask = (round = "") => ({
    id: "b1",
    question: "Steam or relay?",
    by: "otter",
    doneCheck: false,
    permission: false,
    round,
  })
  test("a question back with the person a second round is said again", () => {
    const one = facts(thread({ cards: [card({ asks: [ask()] })] }))
    const two = facts(thread({ cards: [card({ asks: [ask("it9")] })] }))
    expect(one.at(-1)?.key).toBe("ask:c1:b1")
    expect(two.at(-1)?.key).toBe("ask:c1:b1:it9")
  })
  test("a card the thread changed is said once, not followed", () => {
    const t = thread({
      closed: true,
      cards: [card({ state: "working", changed: true, asks: [ask()] })],
    })
    const fs = facts(t)
    expect(keys(fs)).toEqual(["made:c1"])
    expect(fs[0].text).toBe("Your thread #20 changed #21.")
    expect(finished(t)).toBe(true)
  })
  test("quiet: answered, its cards over, nothing on the person", () => {
    expect(
      quiet(thread({ answered: true, cards: [card({ state: "completed" })] }))
    ).toBe(true)
    // Answered with no card yet: its agent may make one next.
    expect(quiet(thread({ answered: true }))).toBe(false)
    expect(quiet(thread())).toBe(false)
    expect(
      quiet(thread({ answered: true, cards: [card({ state: "working" })] }))
    ).toBe(false)
  })

  // A thread card as the Board gives it, as far as workThread reads it.
  const detail = (over: Record<string, unknown> = {}) =>
    ({
      id: "t1",
      short: "20",
      tag: "",
      title: "Fix the typo",
      state: "submitted",
      working: [],
      blockers: [],
      thread: [],
      ask: {
        state: "taken",
        taker: null,
        told: 1,
        owner: "",
        wake: true,
        outcomeRef: "",
      },
      ...over,
    }) as unknown as Parameters<typeof workThread>[0]
  test("cards already said stay when the thread's row is gone", () => {
    const made = detail({
      id: "c1",
      short: "21",
      state: "working",
      ask: null,
      fromThread: { id: "t1", short: "20", tag: "", title: "" },
    })
    const t = workThread(
      detail({
        state: "completed",
        ask: {
          state: "closed",
          told: 1,
          owner: "",
          wake: true,
          outcomeRef: "",
        },
      }),
      undefined,
      new Map([["c1", made]]),
      ["c1"]
    )
    expect(t.cards.map((c) => c.id)).toEqual(["c1"])
    expect(finished(t)).toBe(false)
    expect(keys(facts(t))).not.toContain("closed")
  })
})

describe("only what waits on the person is said (card #1000)", () => {
  const ask = {
    id: "b1",
    question: "Steam or relay?",
    by: "otter",
    doneCheck: false,
    permission: false,
  }
  const flush = () => new Promise((r) => setTimeout(r, 5))

  test("taken, made, started, done and closed are remembered, never said", () => {
    const t = thread({
      taker: "otter",
      closed: true,
      cards: [
        card({ id: "c1", state: "working", working: ["otter"] }),
        card({ id: "c2", state: "completed" }),
        card({ id: "c3", state: "failed" }),
        card({ id: "c4", state: "canceled" }),
        card({ id: "c5", changed: true }),
      ],
    })
    const n = news({ said: [] }, t)
    expect(n.say).toEqual([])
    expect(n.seen.said).toEqual(keys(facts(t)))
    expect(n.seen.said).toContain("made:c1")
    expect(news({ said: [] }, thread({ closed: true })).say).toEqual([])
  })

  test("needs you, ready, permission and no agent are said", () => {
    const t = thread({
      cantStart: "No agent can start.",
      cards: [
        card({
          asks: [
            ask,
            { ...ask, id: "d1", doneCheck: true },
            { ...ask, id: "p1", permission: true },
          ],
        }),
      ],
    })
    expect(keys(news({ said: [] }, t).say)).toEqual([
      "cant",
      "ask:c1:b1",
      "ready:c1:d1",
      "perm:c1:p1",
    ])
  })

  test("a card whose thread row has left the list still says it needs you", async () => {
    // The first read saw the card made: silent, but its key is kept.
    const store: SeenStore = {
      t1: { said: ["taken", "made:c1"] },
    }
    const said: string[][] = []
    const wants: (readonly string[] | undefined)[] = []
    const f = createFollower({
      ids: () => ["t1"],
      read: async (_ids, known) => {
        wants.push(known.get("t1"))
        const detail = (over: Record<string, unknown>) =>
          ({
            id: "t1",
            short: "20",
            tag: "",
            title: "",
            state: "completed",
            working: [],
            blockers: [],
            thread: [],
            ...over,
          }) as unknown as Parameters<typeof workThread>[0]
        const made = detail({
          id: "c1",
          short: "21",
          state: "working",
          fromThread: { id: "t1", short: "20", tag: "", title: "" },
          blockers: [
            {
              id: "b1",
              question: "Steam or relay?",
              state: "open",
              resolved: false,
            },
          ],
        })
        const closed = detail({
          ask: {
            state: "closed",
            told: 1,
            owner: "",
            wake: true,
            outcomeRef: "",
          },
        })
        // No row: the thread list has dropped it.
        return new Map([
          [
            "t1",
            workThread(closed, undefined, new Map([["c1", made]]), [
              ...(known.get("t1") ?? []),
            ]),
          ],
        ])
      },
      listen: () => () => {},
      load: () => store,
      save: () => {},
      say: (_id, lines) => said.push(keys(lines)),
      delayMs: 0,
    })
    await flush()
    f.stop()
    expect(wants[0]).toEqual(["c1"])
    expect(said[0]).toEqual(["ask:c1:b1"])
  })
})

describe("the follower reads quiet work less often (card #785)", () => {
  test("a quiet thread waits on card changes; a poke reads now", async () => {
    let reads = 0
    let wake: () => void = () => {}
    const quietWork = thread({
      answered: true,
      cards: [card({ state: "completed" })],
    })
    const f = createFollower({
      ids: () => ["t1"],
      read: async () => {
        reads++
        return new Map([["t1", quietWork]])
      },
      listen: (fn) => {
        wake = fn
        return () => {}
      },
      load: () => ({ t1: { said: [] } }),
      save: () => {},
      say: () => {},
      delayMs: 0,
      quietMs: 10_000,
    })
    await new Promise((r) => setTimeout(r, 5))
    expect(reads).toBe(1)
    wake()
    await new Promise((r) => setTimeout(r, 5))
    expect(reads).toBe(1)
    f.poke()
    await new Promise((r) => setTimeout(r, 5))
    expect(reads).toBe(2)
    f.stop()
  })
})

describe("the follower's minimum gap (card #1136)", () => {
  test("a burst of card changes reads once per gap, not once per change", async () => {
    let wake: () => void = () => {}
    const starts: number[] = []
    const f = createFollower({
      ids: () => ["t1"],
      read: async () => {
        starts.push(Date.now())
        return new Map([["t1", thread({ taker: "otter" })]])
      },
      listen: (fn) => {
        wake = fn
        return () => {}
      },
      load: () => ({ t1: { said: [] } }),
      save: () => {},
      say: () => {},
      delayMs: 0,
      quietMs: 0,
      minGapMs: 100,
    })
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
    await wait(20)
    expect(starts.length).toBe(1)
    // Twenty changes inside the gap: one more read, once the gap is over.
    for (let i = 0; i < 20; i++) {
      wake()
      await wait(2)
    }
    expect(starts.length).toBe(1)
    await wait(150)
    expect(starts.length).toBe(2)
    expect(starts[1] - starts[0]).toBeGreaterThanOrEqual(95)
    f.stop()
  })
})
