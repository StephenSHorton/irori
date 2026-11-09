import { describe, expect, test } from "bun:test"

import { TalkAssistant, TalkCanvasItem } from "../gen/models"
import {
  Placer,
  askAvailable,
  buildCanvas,
  canSettle,
  canvasKey,
  reconcile,
  rosterRows,
  shouldRefit,
  statusWord,
  type Point,
} from "./huddle-canvas"

const item = (p: Partial<ReturnType<typeof TalkCanvasItem>>) =>
  TalkCanvasItem({ itemId: "i", kind: "note", byName: "you", ...p })
const assist = (p: Partial<ReturnType<typeof TalkAssistant>>) =>
  TalkAssistant({ name: "you", mine: true, state: "connected", ...p })

const t = (
  canvas: ReturnType<typeof TalkCanvasItem>[],
  assistants = [assist({})]
) => ({
  assistants,
  canvas,
})

describe("huddle canvas graph", () => {
  test("people head the canvas and items group under who asked", () => {
    const g = buildCanvas(
      new Placer(),
      t(
        [
          item({ itemId: "q", kind: "question", byName: "Spencer", atMs: 1 }),
          item({
            itemId: "a",
            kind: "answer",
            reply: "q",
            assistant: true,
            label: "Juno · Stephen's assistant",
            byName: "you",
            askerName: "Spencer",
            atMs: 2,
          }),
        ],
        [assist({}), assist({ name: "Spencer", mine: false })]
      ),
      ["you", "Spencer"]
    )
    const heads = g.nodes.filter((n) => n.type === "head")
    expect(heads.map((n) => n.id)).toEqual(["head:You", "head:Spencer"])
    const col = (id: string) => g.nodes.find((n) => n.id === id)?.pos.x
    // The answer asked by Spencer sits in Spencer's column, under his head.
    expect(col("a")).toBe(col("head:Spencer"))
    expect(col("q")).toBe(col("head:Spencer"))
    expect(g.edges.map((e) => [e.source, e.target])).toEqual([["q", "a"]])
  })

  test("a new item goes to the end of its column and the rest stay put", () => {
    const placer = new Placer()
    const a = item({ itemId: "a", atMs: 1 })
    const b = item({ itemId: "b", atMs: 2 })
    const first = buildCanvas(placer, t([a, b]), ["you"])
    const spot = (g: typeof first, id: string) =>
      g.nodes.find((n) => n.id === id)?.pos
    const before = { a: spot(first, "a"), b: spot(first, "b") }
    // An item with an earlier time arrives late, and a new person speaks.
    const late = item({ itemId: "z", atMs: 0 })
    const other = item({ itemId: "c", byName: "Aiko", atMs: 3 })
    const next = buildCanvas(placer, t([a, b, late, other]), ["you"])
    expect(spot(next, "a")).toEqual(before.a)
    expect(spot(next, "b")).toEqual(before.b)
    expect(spot(next, "z")!.y).toBeGreaterThan(spot(next, "b")!.y)
    expect(spot(next, "c")!.x).toBeGreaterThan(spot(next, "a")!.x)
    expect(next.columns).toBe(2)
  })

  test("an item's saved spot beats the layout", () => {
    const g = buildCanvas(
      new Placer(),
      t([item({ itemId: "a", x: 900, y: 40 })]),
      ["you"]
    )
    expect(g.nodes.find((n) => n.id === "a")?.pos).toEqual({ x: 900, y: 40 })
  })

  test("the key follows the items, not the array they came in", () => {
    const items = [item({ itemId: "a", text: "x" })]
    const k = canvasKey(t(items), ["you"])
    expect(canvasKey(t(items.map((i) => ({ ...i }))), ["you"])).toBe(k)
    expect(canvasKey(t([item({ itemId: "a", text: "y" })]), ["you"])).not.toBe(
      k
    )
    expect(
      canvasKey(t([item({ itemId: "a", text: "x", x: 1, y: 2 })]), ["you"])
    ).not.toBe(k)
  })
})

describe("a drag while items arrive", () => {
  type N = { id: string; position: Point; data?: string }
  const node = (id: string, x: number, y: number, data = ""): N => ({
    id,
    position: { x, y },
    data,
  })

  test("the dragged node and the viewport are unchanged", () => {
    const prev = [node("a", 500, 500), node("b", 0, 200)]
    // The graph still has a at its saved spot, and a new item c appended.
    const next = [
      node("a", 0, 112, "new text"),
      node("b", 0, 200),
      node("c", 0, 372),
    ]
    const out = reconcile(prev, next, new Set(["a"]), new Map())
    expect(out.find((n) => n.id === "a")?.position).toEqual({ x: 500, y: 500 })
    expect(out.find((n) => n.id === "a")?.data).toBe("new text")
    expect(out.map((n) => n.id)).toEqual(["a", "b", "c"])
    // Items landing in a column never refit the view; neither does a drag.
    expect(shouldRefit({ touched: true, columns: 2, fitted: 1 })).toBe(false)
    expect(shouldRefit({ touched: false, columns: 1, fitted: 1 })).toBe(false)
    expect(shouldRefit({ touched: false, columns: 2, fitted: 1 })).toBe(true)
  })

  test("a dropped node keeps its spot until the saved one comes back", () => {
    const pending = new Map([["a", { x: 500, y: 500 }]])
    const stale = reconcile(
      [node("a", 500, 500)],
      [node("a", 0, 112)],
      new Set(),
      pending
    )
    expect(stale[0].position).toEqual({ x: 500, y: 500 })
    expect(pending.size).toBe(1)
    const saved = reconcile(stale, [node("a", 500, 500)], new Set(), pending)
    expect(saved[0].position).toEqual({ x: 500, y: 500 })
    expect(pending.size).toBe(0)
  })
})

describe("words", () => {
  test("status reads by kind", () => {
    expect(statusWord({ kind: "question", status: "open", card: "" }, 1)).toBe(
      "Open · 1 answer"
    )
    expect(
      statusWord({ kind: "question", status: "settled", card: "" }, 1)
    ).toBe("Answered")
    expect(statusWord({ kind: "decision", status: "open", card: "" }, 0)).toBe(
      "Proposed"
    )
    expect(statusWord({ kind: "task", status: "open", card: "" }, 0)).toBe(
      "Proposed, not a card"
    )
    expect(statusWord({ kind: "note", status: "", card: "" }, 0)).toBe("")
    expect(canSettle({ kind: "note", status: "" })).toBe(false)
    expect(canSettle({ kind: "task", status: "open" })).toBe(true)
    expect(canSettle({ kind: "task", status: "dropped" })).toBe(false)
  })

  const base = { convKind: "channel", over: false, lines: [] }
  const rows = (
    assistants: ReturnType<typeof TalkAssistant>[],
    canvas: ReturnType<typeof TalkCanvasItem>[] = [],
    o: Partial<{ live: boolean; agentsOff: boolean }> = {},
    extra: Partial<typeof base> & { lines?: never[] } = {}
  ) =>
    rosterRows(
      { ...base, ...extra, assistants, canvas },
      ["you", "Spencer", "Aiko"],
      { live: true, agentsOff: false, ...o }
    )

  test("the roster says connected, taking notes, reconnecting, not connected", () => {
    const r = rows(
      [
        assist({}),
        assist({ name: "Spencer", mine: false, state: "reconnecting" }),
        assist({
          name: "Aiko",
          mine: false,
          state: "not connected",
          why: "needs a computer that's on",
        }),
      ],
      [item({ itemId: "n", assistant: true, byName: "you", label: "Juno" })]
    )
    expect(r.map((x) => x.words)).toEqual([
      "Taking notes",
      "Reconnecting",
      "Not connected: needs a computer that's on",
    ])
    expect(r[0].state).toBe("notes")
    expect(rows([assist({})], [], {}, {})[0].words).toBe("Connected")
  })

  test("the cap, a space not ready and an older Irori are said", () => {
    const capped = rows(
      [assist({})],
      [
        item({
          itemId: "c",
          assistant: true,
          byName: "you",
          text: "Juno has done its looks for this huddle.",
        }),
      ]
    )
    expect(capped[0].notes).toContain("Its looks for this huddle are used up.")
    const off = rows([assist({ name: "Spencer", mine: false })], [], {
      agentsOff: true,
    })
    expect(off.find((x) => x.name === "Spencer")?.notes).toContain(
      "Spencer's lines reach assistants only at the end."
    )
    const dm = rosterRows(
      {
        convKind: "dm",
        over: false,
        assistants: [assist({ name: "Spencer", mine: false })],
        canvas: [],
        lines: [
          { name: "Spencer", mine: false, agents: false, atMs: 1, text: "hi" },
        ],
      },
      ["you", "Spencer"],
      { live: true, agentsOff: false }
    )
    expect(dm.find((x) => x.name === "Spencer")?.notes).toContain(
      "Spencer's lines aren't heard by assistants (older Irori)."
    )
  })

  test("a huddle that is over shows no assistant state", () => {
    const r = rows([assist({})], [], { live: false })
    expect(r.every((x) => x.words === "")).toBe(true)
  })

  test("Ask shows only for a live huddle and the viewer's connected assistant", () => {
    const ok = { over: false, assistants: [assist({})] }
    expect(askAvailable(ok, true)).toBe(true)
    expect(askAvailable(ok, false)).toBe(false)
    expect(askAvailable({ ...ok, over: true }, true)).toBe(false)
    expect(
      askAvailable(
        { over: false, assistants: [assist({ state: "not connected" })] },
        true
      )
    ).toBe(false)
    expect(
      askAvailable(
        { over: false, assistants: [assist({ name: "Spencer", mine: false })] },
        true
      )
    ).toBe(false)
    expect(askAvailable({ over: false, assistants: [] }, true)).toBe(false)
  })
})
