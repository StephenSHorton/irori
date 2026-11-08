import { describe, expect, test } from "bun:test"

import {
  capHistory,
  failedLine,
  foldEvent,
  freshReply,
  historyFor,
  HistoryCap,
  liftOver,
  parseHistory,
  replyText,
  sampleAnswer,
  serializeHistory,
  settled,
  type AssistantEvent,
  type ChatEntry,
  type Reply,
} from "./assistant"
import { boldRuns, richBlocks } from "./rich"

const ev = (kind: string, text = ""): AssistantEvent => ({ kind, text })
const fold = (...evs: AssistantEvent[]): Reply =>
  evs.reduce(foldEvent, freshReply)

describe("Ask Irori's reply (card #783)", () => {
  test("deltas append, in order", () => {
    const r = fold(ev("delta", "Two cards "), ev("delta", "wait on you."))
    expect(r.text).toBe("Two cards wait on you.")
    expect(r.end).toBe("")
  })
  test("a tool event replaces the status line and isn't reply text", () => {
    const r = fold(
      ev("tool", "Searching for “renaming”"),
      ev("delta", "It's "),
      ev("tool", "Opening #12")
    )
    expect(r.status).toBe("Opening #12")
    expect(r.text).toBe("It's ")
  })
  test("done ends it and clears the status", () => {
    const r = fold(
      ev("tool", "Reading the Board"),
      ev("delta", "ok"),
      ev("done")
    )
    expect(r).toEqual({ text: "ok", status: "", end: "done", error: "" })
    expect(settled(r)).toEqual([{ role: "assistant", text: "ok" }])
  })
  test("an error ends it as a plain line, after any text it wrote", () => {
    const r = fold(
      ev("delta", "Partly"),
      ev("error", "The assistant needs Claude Code signed in on this computer.")
    )
    expect(r.end).toBe("error")
    expect(settled(r)).toEqual([
      { role: "assistant", text: "Partly" },
      {
        role: "assistant",
        text: "The assistant needs Claude Code signed in on this computer.",
        note: true,
      },
    ])
    expect(settled(fold(ev("error", "  ")))).toEqual([
      { role: "assistant", text: failedLine, note: true },
    ])
  })
  test("once ended, late events change nothing (a stopped run's Stopped.)", () => {
    const stopped: Reply = {
      ...fold(ev("delta", "Half")),
      end: "error",
      error: "Stopped.",
    }
    const after = [
      ev("delta", " more"),
      ev("error", "Stopped."),
      ev("done"),
    ].reduce(foldEvent, stopped)
    expect(after).toBe(stopped)
    expect(settled(after).map((e) => e.text)).toEqual(["Half", "Stopped."])
  })
  test("an unknown kind is ignored", () => {
    expect(fold(ev("delta", "a"), ev("ping", "x")).text).toBe("a")
  })
})

describe("Ask Irori's history (card #783)", () => {
  const many = (n: number): ChatEntry[] =>
    Array.from({ length: n }, (_, i) => ({
      role: i % 2 ? "assistant" : "person",
      text: `m${i}`,
    }))
  test("keeps the last HistoryCap messages", () => {
    const c = capHistory(many(HistoryCap + 7))
    expect(c.length).toBe(HistoryCap)
    expect(c[0].text).toBe("m7")
    expect(c[c.length - 1].text).toBe(`m${HistoryCap + 6}`)
  })
  test("notes stay in the chat but aren't sent", () => {
    const h = historyFor([
      { role: "person", text: "What's blocked?" },
      { role: "assistant", text: "Stopped.", note: true },
      { role: "assistant", text: "#3" },
    ])
    expect(h).toEqual([
      { role: "person", text: "What's blocked?" },
      { role: "assistant", text: "#3" },
    ])
  })
  test("saves and reads back, dropping what isn't a message", () => {
    const es: ChatEntry[] = [
      { role: "person", text: "hi" },
      { role: "assistant", text: "No answer.", note: true },
    ]
    expect(parseHistory(serializeHistory(es))).toEqual(es)
    expect(parseHistory(null)).toEqual([])
    expect(parseHistory("{nope")).toEqual([])
    expect(parseHistory('{"role":"person"}')).toEqual([])
    expect(
      parseHistory(
        JSON.stringify([
          { role: "system", text: "x" },
          { role: "person", text: 3 },
          { role: "person", text: "  " },
          null,
          { role: "assistant", text: "kept", note: "yes" },
        ])
      )
    ).toEqual([{ role: "assistant", text: "kept" }])
    expect(JSON.parse(serializeHistory(many(80))).length).toBe(HistoryCap)
  })
})

describe("Ask Irori's reply text (card #783)", () => {
  test("**bold** runs, and an unclosed ** stays", () => {
    expect(boldRuns("a **b** c")).toEqual([
      { text: "a ", bold: false },
      { text: "b", bold: true },
      { text: " c", bold: false },
    ])
    expect(boldRuns("**#12** waits")).toEqual([
      { text: "#12", bold: true },
      { text: " waits", bold: false },
    ])
    expect(boldRuns("2 ** 3 and **x")).toEqual([
      { text: "2 ** 3 and **x", bold: false },
    ])
    expect(boldRuns("no stars")).toEqual([{ text: "no stars", bold: false }])
  })
  test("headings become bold lines; lists and paragraphs keep their shape", () => {
    const t = replyText("## Blocked\n\n- #1 waits\n* #3 too\n\nThat's all.")
    expect(t.startsWith("**Blocked**\n")).toBe(true)
    const blocks = richBlocks(t)
    expect(blocks.map((b) => b.kind)).toEqual(["para", "list", "para"])
    const list = blocks[1]
    expect(list.kind === "list" && list.items.length).toBe(2)
  })
  test("a #12 ref isn't taken for a heading", () => {
    expect(replyText("#12 is done")).toBe("#12 is done")
  })
})

describe("Ask Irori's preview stand-in (card #783)", () => {
  test("answers by the question's words, citing sample cards", () => {
    expect(sampleAnswer("What's blocked?").text).toContain("#1")
    expect(sampleAnswer("What are my agents doing?").tools[0]).toBe(
      "Reading the agents"
    )
    expect(sampleAnswer("Find the card about release notes").tools[0]).toBe(
      "Searching for “release notes”"
    )
  })
})

describe("Ask Irori keeps clear of the bars (card #783)", () => {
  const vh = { height: 844 }
  const col = { left: 310, right: 390 }
  test("sits above the tabs and a composer in its column", () => {
    const tabs = { left: 0, right: 390, top: 788, bottom: 844 }
    const composer = { left: 0, right: 390, top: 670, bottom: 788 }
    expect(liftOver([tabs], vh, col)).toBe(56)
    expect(liftOver([tabs, composer], vh, col)).toBe(174)
  })
  test("ignores bars outside its column, hidden or off screen", () => {
    const leftPane = { left: 0, right: 300, top: 600, bottom: 844 }
    const hidden = { left: 0, right: 0, top: 0, bottom: 0 }
    const below = { left: 0, right: 390, top: 900, bottom: 950 }
    expect(liftOver([leftPane, hidden, below], vh, col)).toBe(0)
  })
})
