import { describe, expect, test } from "bun:test"

import {
  capHistory,
  capKeeping,
  failedLine,
  foldEvent,
  freshReply,
  historyFor,
  HistoryCap,
  liftOver,
  parseHistory,
  replyText,
  sampleAnswer,
  sampleFiles,
  serializeHistory,
  settled,
  splitDraft,
  unreadableNote,
  DraftClose,
  DraftOpen,
  type AssistantEvent,
  type AssistantFile,
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
    expect(r).toEqual({
      text: "ok",
      status: "",
      end: "done",
      error: "",
      looked: "",
    })
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

describe("Ask Irori's drafted thread (card #783)", () => {
  const draft = (body: string) => `${DraftOpen}\n${body}\n${DraftClose}`
  test("a reply with no draft is left as it is", () => {
    expect(splitDraft("Two cards wait on you.")).toEqual({
      text: "Two cards wait on you.",
      draft: "",
      partial: false,
    })
  })
  test("a whole draft comes out of the text", () => {
    const s = splitDraft(
      `Drafted it, naming #8.\n\n${draft("Add a changelog page. #8 has the notes.")}`
    )
    expect(s).toEqual({
      text: "Drafted it, naming #8.",
      draft: "Add a changelog page. #8 has the notes.",
      partial: false,
    })
  })
  test("words after the draft stay in the text", () => {
    const s = splitDraft(
      `Here it is.\n${draft("Fix the lobby.")}\nTap Post thread.`
    )
    expect(s.text).toBe("Here it is.\n\nTap Post thread.")
    expect(s.draft).toBe("Fix the lobby.")
  })
  test("a draft being written is partial, and half a marker never shows", () => {
    expect(splitDraft("Drafting.\n<<thr")).toEqual({
      text: "Drafting.",
      draft: "",
      partial: false,
    })
    expect(splitDraft(`Drafting.\n${DraftOpen}\nFix the lob`)).toEqual({
      text: "Drafting.",
      draft: "Fix the lob",
      partial: true,
    })
    expect(splitDraft(`${DraftOpen}\nFix the lobby.\n<</thr`)).toEqual({
      text: "",
      draft: "Fix the lobby.",
      partial: true,
    })
  })
  test("the history keeps the reply whole, draft and all", () => {
    const text = `Drafted it.\n${draft("Fix the lobby.")}`
    const entries: ChatEntry[] = [
      { role: "person", text: "Can you fix the lobby?" },
      { role: "assistant", text, posted: { id: "task:1", short: "9" } },
    ]
    expect(historyFor(entries)[1]).toEqual({
      role: "assistant",
      text: `${text}\n\n(The person posted this draft as thread #9.)`,
    })
  })
  test("the history says where a posted thread and a reply went (card #785)", () => {
    const entries: ChatEntry[] = [
      {
        role: "assistant",
        text: draft("A"),
        posted: { id: "task:1", short: "5", tag: "mlg" },
      },
      {
        role: "assistant",
        text: "No agent has started on mlg#5 yet.",
        update: "task:1",
      },
      { role: "person", text: "Use Steam.", to: "mlg#5" },
    ]
    const h = historyFor(entries)
    expect(h[0].text).toEndWith(
      "(The person posted this draft as thread mlg#5.)"
    )
    expect(h[1]).toEqual({
      role: "assistant",
      text: "(The chat's own lines on the work it follows:)\n- No agent has started on mlg#5 yet.",
    })
    expect(h[2]).toEqual({
      role: "person",
      text: "(My reply on mlg#5) Use Steam.",
    })
  })
  test("updates and replies are kept across reloads, bad acts dropped (card #785)", () => {
    const entries: ChatEntry[] = [
      {
        role: "assistant",
        text: "#21 needs you: Merge?",
        update: "task:1",
        act: { kind: "answer", card: "task:2", blocker: "b1" },
      },
      { role: "person", text: "Home.", to: "#20" },
    ]
    expect(parseHistory(serializeHistory(entries))).toEqual(entries)
    const bad = parseHistory(
      JSON.stringify([
        {
          role: "assistant",
          text: "#21 is waiting for your permission: Run it?",
          update: "t",
          act: { kind: "answer" },
        },
        { role: "person", text: "y", update: "t", to: 3 },
      ])
    )
    expect(bad).toEqual([
      {
        role: "assistant",
        text: "#21 is waiting for your permission: Run it?",
        update: "t",
      },
      { role: "person", text: "y" },
    ])
  })
  test("a saved chat loses the work-following lines (card #1000)", () => {
    const line = (text: string, act?: unknown) => ({
      role: "assistant",
      text,
      update: "task:1",
      ...(act ? { act } : {}),
    })
    const answer = { kind: "answer", card: "task:2", blocker: "b1" }
    const kept = [
      { role: "person", text: "Is it moving?" },
      { role: "person", text: "Home.", to: "#20" },
      { role: "assistant", text: "It is." },
      // A line the chat added itself, not about followed work (#987).
      { role: "assistant", text: "Closed #9.", note: true },
      line("#21 needs you: Merge?", answer),
      line("#21 is ready: otter says it's done.", answer),
      line("mlg#21 is waiting for your permission: Run it?"),
      line("No agent has started on #20 yet. None can start."),
    ]
    const gone = [
      line("**otter** took your thread #20."),
      line("otter replied on #20: Which page?", {
        kind: "reply",
        thread: "task:1",
        to: "otter",
      }),
      // A reply quoting a kept line is still a reply.
      line("otter replied on #20: #21 is waiting for your permission: x", {
        kind: "reply",
        thread: "task:1",
        to: "otter",
      }),
      line("Your thread #20 became #21."),
      line("Your thread #20 is now a card."),
      line("Your thread #20 changed #21."),
      line("otter started on #21."),
      line("#21 is done."),
      line("#21 failed."),
      line("#21 was canceled."),
      line("#20 was answered."),
      line("#20 was resolved."),
    ]
    expect(parseHistory(JSON.stringify([...gone, ...kept]))).toEqual(
      kept as ChatEntry[]
    )
  })
  test("a posted draft's thread is kept across reloads, a bad one dropped", () => {
    const saved = serializeHistory([
      {
        role: "assistant",
        text: draft("A"),
        posted: { id: "task:1", short: "9" },
      },
      { role: "assistant", text: draft("B") },
    ])
    const back = parseHistory(saved)
    expect(back[0].posted).toEqual({ id: "task:1", short: "9" })
    expect(back[1].posted).toBeUndefined()
    const bad = parseHistory(
      JSON.stringify([
        { role: "assistant", text: "x", posted: { id: 3 } },
        { role: "person", text: "y", posted: { id: "task:2", short: "" } },
      ])
    )
    expect(bad.map((e) => e.posted)).toEqual([undefined, undefined])
  })
  test("the stand-in drafts a thread when asked for work, not for a question", () => {
    const work = splitDraft(sampleAnswer("Can you add a changelog page?").text)
    expect(work.draft).toContain("Add a changelog page.")
    expect(work.partial).toBe(false)
    expect(splitDraft(sampleAnswer("What's blocked?").text).draft).toBe("")
  })
})

describe("the cap keeps what the chat still follows (card #785)", () => {
  test("a followed thread's posted message stays; others go oldest first", () => {
    const posted: ChatEntry = {
      role: "assistant",
      text: "A draft",
      posted: { id: "t1", short: "1" },
    }
    const lines: ChatEntry[] = Array.from({ length: 5 }, (_, i) => ({
      role: "assistant",
      text: `line ${i}`,
      update: "t1",
    }))
    const all = [{ role: "person", text: "q" } as ChatEntry, posted, ...lines]
    const kept = capKeeping(all, new Set(["t1"]), 4)
    expect(kept).toEqual([posted, lines[2], lines[3], lines[4]])
    expect(capKeeping(all, new Set(), 4)).toEqual(lines.slice(1))
  })
})

describe("Looked at (card #1009)", () => {
  test("a reply that read the screen keeps the text it read", () => {
    const r = { ...freshReply, text: "It needs a call.", looked: "Page: Home" }
    expect(settled({ ...r, end: "done" })).toEqual([
      { role: "assistant", text: "It needs a call.", looked: "Page: Home" },
    ])
  })
  test("saved history keeps it", () => {
    const e = [
      { role: "assistant", text: "ok", looked: "Page: Home\nIn view: #1 “A”" },
    ] as const
    expect(parseHistory(serializeHistory(e))).toEqual([...e])
  })
  test("the sample reads the screen for a question about where you are", () => {
    const a = sampleAnswer(
      "what does this card need?",
      "Page: Home\nIn view: #1 “A”"
    )
    expect(a.tools).toEqual(["Looking at your screen: Home, #1 “A”"])
    expect(sampleAnswer("who is working?", "Page: Home").tools).not.toContain(
      "Looking at your screen: Home"
    )
  })
})

describe("what the chat changed (card #987)", () => {
  const r = fold(
    ev("delta", "Done."),
    ev("changed", "Resolved thread #983 “Mic button”."),
    ev("changed", "  "),
    ev("done")
  )

  test("each change is its own line after the reply", () => {
    expect(settled(r)).toEqual([
      { role: "assistant", text: "Done." },
      {
        role: "assistant",
        text: "Resolved thread #983 “Mic button”.",
        note: true,
        changed: true,
      },
    ])
  })

  test("it survives a reload and isn't sent back as history", () => {
    const back = parseHistory(serializeHistory(settled(r)))
    expect(back[1].changed).toBe(true)
    expect(historyFor(back).map((t) => t.text)).toEqual(["Done."])
  })
})

describe("the chat's files (card #999)", () => {
  const ref = (c: string, name: string, can_read?: boolean): AssistantFile => ({
    sha256: c.repeat(64),
    name,
    size: 12,
    mime: "image/png",
    device: "d1",
    ...(can_read === undefined ? {} : { can_read }),
  })
  test("files come back from saved history, as references", () => {
    const es: ChatEntry[] = [
      { role: "person", text: "look", files: [ref("a", "a.png", true)] },
      { role: "assistant", text: "A cat." },
    ]
    expect(parseHistory(serializeHistory(es))).toEqual(es)
  })
  test("a message of files alone stays; one of neither doesn't", () => {
    const es: ChatEntry[] = [
      { role: "person", text: "", files: [ref("a", "a.png")] },
    ]
    expect(parseHistory(serializeHistory(es))).toEqual(es)
    expect(
      parseHistory(JSON.stringify([{ role: "person", text: "", files: [] }]))
    ).toEqual([])
  })
  test("a malformed file is dropped, its message stays", () => {
    const good = ref("b", "b.png")
    const bad = [
      null,
      "x",
      { ...good, sha256: "../../etc/passwd" },
      { ...good, sha256: "B".repeat(64) },
      { ...good, name: "" },
      { ...good, name: 3 },
      { ...good, size: "12" },
      { ...good, size: -1 },
      { ...good, device: undefined },
    ]
    expect(
      parseHistory(
        JSON.stringify([
          { role: "person", text: "hi", files: [...bad, good, good] },
          { role: "person", text: "still here", files: "nope" },
          { role: "person", text: "", files: bad },
        ])
      )
    ).toEqual([
      { role: "person", text: "hi", files: [good] },
      { role: "person", text: "still here" },
    ])
  })
  test("only the person's messages carry files", () => {
    expect(
      parseHistory(
        JSON.stringify([
          { role: "assistant", text: "x", files: [ref("c", "c.png")] },
          { role: "assistant", text: "", files: [ref("c", "c.png")] },
        ])
      )
    ).toEqual([{ role: "assistant", text: "x" }])
  })
  test("a wrong kind of mime or can_read is left out, the file kept", () => {
    expect(
      parseHistory(
        JSON.stringify([
          {
            role: "person",
            text: "x",
            files: [{ ...ref("d", "d.png"), mime: 3, can_read: "no" }],
          },
        ])
      )[0].files
    ).toEqual([
      { sha256: "d".repeat(64), name: "d.png", size: 12, device: "d1" },
    ])
  })
  test("historyFor passes files, and a turn of files alone", () => {
    const files = [ref("a", "a.png")]
    expect(
      historyFor([
        { role: "person", text: "", files },
        { role: "assistant", text: "Can't read it.", note: true },
        { role: "person", text: "and?" },
      ])
    ).toEqual([
      { role: "person", text: "", files },
      { role: "person", text: "and?" },
    ])
  })
  test("the note names what can't be read, and only that", () => {
    expect(
      unreadableNote([
        ref("a", "a.png", true),
        ref("b", "report.docx", false),
        ref("c", "old.zip", false),
        ref("d", "older.png"),
      ])
    ).toBe(
      "Can't read report.docx, old.zip: the assistant reads pictures, PDFs and text files."
    )
    expect(unreadableNote([ref("a", "a.png", true)])).toBe("")
  })
  test("the preview's stand-in never claims to have read a file", () => {
    const { text } = sampleFiles("", [ref("a", "a.png", true)])
    expect(text).toContain("nothing read a.png")
    expect(sampleFiles("", [ref("b", "b.docx", false)]).text).not.toContain(
      "b.docx"
    )
  })
})
