import { afterAll, beforeAll, describe, expect, test } from "bun:test"

import { TalkCanvasItem } from "../gen/models"
import { splitReply } from "./assistant"
import {
  applySettle,
  boardChoices,
  cardText,
  cardable,
  defaultChoice,
  finishedSections,
  isCarded,
  markCarded,
  parseCard,
  parseSettle,
  refOfTask,
  settleCandidates,
  shownCards,
  shownSettle,
  suggestedChoice,
  summaryOf,
  takeDrafts,
} from "./huddle-divide"

function item(p: Partial<TalkCanvasItem> & { itemId: string }) {
  return TalkCanvasItem({ kind: "task", status: "open", atMs: 1, ...p })
}

const card = (c: object) => `<<cards>>\n${JSON.stringify(c)}\n<</cards>>`

describe("takeDrafts", () => {
  test("takes every block out and keeps the words around them", () => {
    const raw = [
      "Here is how I would split it.",
      "",
      card({ title: "Renew cert", objective: "Before the 14th", items: ["a"] }),
      card({ title: "Check status", items: ["b", "c"], board: "Release" }),
      '<<settle>>{"settle":["q1"],"drop":["q2"]}<</settle>>',
    ].join("\n")
    const d = takeDrafts(raw)
    expect(d.text).toBe("Here is how I would split it.")
    expect(d.cards.map((c) => c.title)).toEqual(["Renew cert", "Check status"])
    expect(d.cards[1]).toEqual({
      title: "Check status",
      objective: "",
      items: ["b", "c"],
      board: "Release",
    })
    expect(d.settle).toEqual({ settle: ["q1"], drop: ["q2"] })
    expect(d.partial).toBe(false)
  })

  test("a block still being written is cut and reported", () => {
    const raw = `Splitting.\n\n${card({ title: "One", items: ["a"] })}\n<<cards>>\n{"title":"Tw`
    const d = takeDrafts(raw)
    expect(d.cards).toHaveLength(1)
    expect(d.partial).toBe(true)
    expect(d.text).toBe("Splitting.")
    // The opening marker's first characters never flash.
    expect(takeDrafts("Splitting.\n\n<<car").text).toBe("Splitting.")
    expect(takeDrafts("Splitting.\n\n<<set").text).toBe("Splitting.")
  })

  test("a block that is not valid is dropped, the words stay", () => {
    const d = takeDrafts(
      'Hm.\n<<cards>>\nnot json\n<</cards>>\n<<cards>>\n{"title":""}\n<</cards>>\n<<settle>>[]<</settle>>'
    )
    expect(d.cards).toEqual([])
    expect(d.settle).toBeNull()
    expect(d.text).toBe("Hm.")
  })

  test("an item named both ways is settled or dropped by neither", () => {
    expect(parseSettle('{"settle":["a","b"],"drop":["b","c"]}')).toEqual({
      settle: ["a"],
      drop: ["c"],
    })
    expect(parseSettle('{"settle":["a"],"drop":["a"]}')).toBeNull()
  })

  test("titles and objectives are clipped, ids deduplicated", () => {
    const c = parseCard(
      JSON.stringify({
        title: "x".repeat(500),
        objective: "y".repeat(9000),
        items: ["a", "a", 7, " b "],
      })
    )
    expect(c?.title.length).toBe(200)
    expect(c?.objective.length).toBe(4000)
    expect(c?.items).toEqual(["a", "b"])
  })

  test("at most twelve cards", () => {
    const raw = Array.from({ length: 20 }, (_, i) =>
      card({ title: `c${i}`, items: [] })
    ).join("\n")
    expect(takeDrafts(raw).cards).toHaveLength(12)
  })
})

describe("splitReply", () => {
  test("takes the blocks out before the go line, so one inside never fires", () => {
    const raw = [
      "Open it for you.",
      "<<go card #8>>",
      card({ title: "Hidden", objective: "<<go card #9>>", items: [] }),
    ].join("\n")
    const s = splitReply(raw)
    expect(s.go).toEqual({ kind: "card", ref: "#8" })
    expect(s.cards.map((c) => c.title)).toEqual(["Hidden"])
    expect(s.text).toBe("Open it for you.")
    // A go line only inside a block is not a go.
    const inside = splitReply(
      card({ title: "T", objective: "<<go card #9>>", items: [] })
    )
    expect(inside.go).toBeNull()
  })

  test("a thread draft and card drafts live side by side", () => {
    const raw = `Done.\n\n<<thread>>\nAsk about #8\n<</thread>>\n${card({ title: "T", items: [] })}`
    const s = splitReply(raw)
    expect(s.draft).toBe("Ask about #8")
    expect(s.cards).toHaveLength(1)
  })

  test("a plain reply is untouched", () => {
    const s = splitReply("Three agents are working.")
    expect(s.cards).toEqual([])
    expect(s.settle).toBeNull()
    expect(s.text).toBe("Three agents are working.")
  })
})

const items = [
  item({ itemId: "q1", kind: "question", text: "Which cert?", atMs: 1 }),
  item({
    itemId: "q2",
    kind: "question",
    text: "Old endpoint?",
    atMs: 2,
    status: "settled",
  }),
  item({ itemId: "t1", kind: "task", text: "Renew", atMs: 3 }),
  item({
    itemId: "t2",
    kind: "task",
    text: "Docs",
    atMs: 4,
    status: "carded",
    card: "#12",
    cardByName: "Addam",
  }),
  item({
    itemId: "d1",
    kind: "decision",
    text: "Sign on PC",
    atMs: 5,
    status: "settled",
  }),
  item({
    itemId: "n1",
    kind: "note",
    text: "Takes nine min",
    atMs: 6,
    status: "",
  }),
  item({
    itemId: "x1",
    kind: "task",
    text: "Never mind",
    atMs: 7,
    status: "dropped",
  }),
  item({
    itemId: "s1",
    kind: "summary",
    text: "Sign on the PC.",
    status: "",
    atMs: 8,
    refs: ["q1", "d1", "gone", "s1"],
  }),
]

describe("the finished canvas", () => {
  test("groups by kind, open questions and tasks first, open before closed", () => {
    const s = finishedSections(items)
    expect(s.map((x) => x.group)).toEqual([
      "question",
      "task",
      "decision",
      "note",
    ])
    expect(s[0].title).toBe("Open questions")
    expect(s[0].items.map((i) => i.itemId)).toEqual(["q1", "q2"])
    // Open, then carded, then dropped.
    expect(s[1].items.map((i) => i.itemId)).toEqual(["t1", "t2", "x1"])
    // The summary sits on top, not among them.
    expect(s.flatMap((x) => x.items).some((i) => i.kind === "summary")).toBe(
      false
    )
    expect(summaryOf(items)?.itemId).toBe("s1")
  })

  test("Settle these is what the summary's refs judge, still open", () => {
    // d1 is settled already, `gone` is not on the canvas, s1 is a summary.
    expect(settleCandidates(items).map((i) => i.itemId)).toEqual(["q1"])
    expect(settleCandidates(items.filter((i) => i.kind !== "summary"))).toEqual(
      []
    )
  })

  test("carded is the status or a card ref", () => {
    expect(isCarded(items[3])).toBe(true)
    expect(isCarded(items[2])).toBe(false)
    expect(cardable(items).map((i) => i.itemId)).toEqual([
      "q1",
      "q2",
      "t1",
      "d1",
      "n1",
    ])
  })
})

describe("what a draft will do", () => {
  test("a card leaves out items another person already carded", () => {
    const shown = shownCards(
      [
        { title: "A", objective: "", items: ["t1", "t2"], board: "" },
        { title: "B", objective: "", items: ["t2"], board: "" },
        { title: "C", objective: "", items: [], board: "" },
      ],
      items
    )
    expect(shown.map((c) => c.title)).toEqual(["A", "C"])
    expect(shown[0].ready.map((i) => i.itemId)).toEqual(["t1"])
    expect(shown[0].skipped).toEqual(["t2"])
  })

  test("the settle list leaves out summaries, carded items and no-ops", () => {
    const s = shownSettle(
      { settle: ["q1", "q2", "t2", "s1", "nope"], drop: ["x1", "n1"] },
      items
    )
    expect(s.settle.map((i) => i.itemId)).toEqual(["q1"])
    expect(s.drop.map((i) => i.itemId)).toEqual(["n1"])
    expect(shownSettle(null, items)).toEqual({ settle: [], drop: [] })
  })

  test("the card's text is the assistant's words, the items and the huddle", () => {
    const [c] = shownCards(
      [{ title: "T", objective: "Do it.", items: ["t1", "n1"], board: "" }],
      items
    )
    expect(cardText(c, "Release call")).toBe(
      'Do it.\n\nFrom the canvas:\n- Renew\n- Takes nine min\n\nFrom the huddle "Release call".'
    )
  })

  test("a card's ref carries its board's tag", () => {
    expect(refOfTask({ id: "task:x", short: "12", tag: "mlg" })).toBe("mlg#12")
    expect(refOfTask({ id: "task:x", short: "12" })).toBe("#12")
    expect(refOfTask({ id: "task:x" })).toBe("task:x")
  })
})

describe("which board", () => {
  const personal = [{ boardId: "pb", name: "Main", tag: "" }]
  const spaces = [
    {
      boardId: "sp1",
      name: "Suzuri",
      tag: "su",
      boards: [
        { boardId: "sp1", name: "Release", tag: "su" },
        { boardId: "sp1b", name: "Ideas", tag: "id" },
      ],
    },
    { boardId: "sp2", name: "Dent", tag: "dn", boards: [] },
  ]
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const choices = boardChoices(personal, spaces as any)

  test("Personal first, then each open board of each space", () => {
    expect(choices.map((c) => c.label)).toEqual([
      "Personal",
      "Suzuri › Release",
      "Suzuri › Ideas",
      "Dent",
    ])
    expect(choices[1]).toMatchObject({ board: "sp1", spaceBoard: "sp1" })
    expect(choices[0]).toMatchObject({ board: "", spaceBoard: "pb" })
  })

  test("a channel's cards go to its space, else Personal", () => {
    expect(defaultChoice(choices, "sp2")?.label).toBe("Dent")
    expect(defaultChoice(choices, undefined)?.label).toBe("Personal")
    expect(defaultChoice(choices, "unknown")?.label).toBe("Personal")
  })

  test("the assistant's suggestion is matched by name", () => {
    expect(suggestedChoice(choices, "ideas")?.label).toBe("Suzuri › Ideas")
    expect(suggestedChoice(choices, "Dent")?.label).toBe("Dent")
    expect(suggestedChoice(choices, "Nowhere")).toBeUndefined()
    expect(suggestedChoice(choices, "")).toBeUndefined()
  })
})

describe("the writes", () => {
  // The preview's stand-in needs a window; this one is not the app's.
  const g = globalThis as { window?: unknown }
  const had = g.window
  beforeAll(() => {
    g.window ??= {}
  })
  afterAll(() => {
    g.window = had
  })

  test("in the preview, marking and settling land on the canvas", async () => {
    // Outside the app the writes go to the preview's stand-in.
    const call = "sample-call-2"
    const m = await markCarded(call, "#77", ["t1", "t2"])
    expect(m.marked).toEqual(["t1", "t2"])
    expect(m.failed).toEqual([])
    const { sampleCanvas } = await import("./sample-huddle")
    const t1 = sampleCanvas(call, Date.now()).find((i) => i.itemId === "t1")
    expect(t1).toMatchObject({ status: "carded", card: "#77", cardMine: true })
    const r = await applySettle(call, {
      settle: sampleCanvas(call, Date.now()).filter((i) => i.itemId === "q1"),
      drop: sampleCanvas(call, Date.now()).filter((i) => i.itemId === "q2"),
    })
    expect(r.set).toBe(2)
    const after = sampleCanvas(call, Date.now())
    expect(after.find((i) => i.itemId === "q1")?.status).toBe("settled")
    expect(after.find((i) => i.itemId === "q2")?.status).toBe("dropped")
    // An item that isn't there fails alone; the others still go.
    const bad = await markCarded(call, "#78", ["nope", "n1"])
    expect(bad.marked).toEqual(["n1"])
    expect(bad.failed.map((f) => f.id)).toEqual(["nope"])
  })
})
