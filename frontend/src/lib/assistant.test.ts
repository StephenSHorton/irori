import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test"

import {
  ask,
  assistantState,
  capHistory,
  capKeeping,
  mergeLines,
  quietKey,
  setTalkLines,
  failedLine,
  foldEvent,
  freshReply,
  HistoryCap,
  liftOver,
  newChat,
  postDraft,
  parseHistory,
  replyText,
  sampleAnswer,
  sampleFiles,
  serializeHistory,
  settled,
  spaceOfTag,
  splitDraft,
  splitReply,
  unreadableNote,
  DraftClose,
  DraftOpen,
  entriesFrom,
  cursorAfter,
  refresh,
  stopTurn,
  setAssistantOpen,
  onGo,
  type AssistantEvent,
  type AssistantFile,
  type ChatEntry,
  type Reply,
} from "./assistant"
import type { BoardSummary } from "./boards"
import { threads } from "./cards"
import { AssistantService } from "./host"
import * as irori from "./irori"
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
      { role: "assistant", text: "Partly", cutOff: true },
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
      check: false,
      space: "",
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
      check: false,
      space: "",
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
      check: false,
      space: "",
    })
    expect(splitDraft(`Drafting.\n${DraftOpen}\nFix the lob`)).toEqual({
      text: "Drafting.",
      draft: "Fix the lob",
      partial: true,
      check: false,
      space: "",
    })
    expect(splitDraft(`${DraftOpen}\nFix the lobby.\n<</thr`)).toEqual({
      text: "",
      draft: "Fix the lobby.",
      partial: true,
      check: false,
      space: "",
    })
  })
  test("a near-miss closing line still closes the draft", () => {
    for (const close of [
      "</</thread>>",
      "</thread>>",
      "<</thread>",
      "<</ thread>>",
      "<< /thread >>",
      "<</THREAD>>",
      "</thread>",
      "<</</thread>>",
    ])
      expect([
        close,
        splitDraft(
          `Posting it.\n${DraftOpen}\nFix the lobby.\n${close}\n<<go card #8>>`
        ),
      ]).toEqual([
        close,
        {
          text: "Posting it.\n\n<<go card #8>>",
          draft: "Fix the lobby.",
          partial: false,
          check: false,
          space: "",
        },
      ])
    // The opening lines never close one.
    expect(splitDraft("A <<thread>> and <<thread check>>").partial).toBe(true)
  })
  test("a thread for a shared space waits for a tap (card #1198)", () => {
    for (const open of ["<<thread in:MLG>>", "<<thread check in:mlg>>"])
      expect(
        splitDraft(`Posting it in mlg.\n${open}\nFix the lobby.\n${DraftClose}`)
      ).toEqual({
        text: "Posting it in mlg.",
        draft: "Fix the lobby.",
        partial: false,
        check: true,
        space: "mlg",
      })
    // Not a tag: no draft.
    expect(splitDraft(`<<thread in:#5 x>>\nFix it.\n${DraftClose}`).draft).toBe(
      ""
    )
    // Being written, the opening line never shows.
    expect(splitDraft("Posting it.\n<<thread in:ml").text).toBe("Posting it.")
    // Its space is found by any of the space's board tags.
    const space = {
      boardId: "sp1",
      name: "mlg farming",
      tag: "mlg",
      boards: [
        { boardId: "sp1", name: "", tag: "mlg" },
        { boardId: "b2", name: "Art", tag: "art" },
      ],
    } as unknown as BoardSummary
    expect(spaceOfTag("ART", [space])).toEqual({
      id: "sp1",
      name: "mlg farming",
    })
    expect(spaceOfTag("nope", [space])).toBeNull()
  })
  test("a reply that ended without closing its draft has the whole draft", () => {
    const raw = `Posting it.\n${DraftOpen}\nFix the lobby.\nAsk on #8.`
    expect(splitDraft(raw, true)).toEqual({
      text: "Posting it.",
      draft: "Fix the lobby.\nAsk on #8.",
      partial: false,
      check: false,
      space: "",
    })
    // Still being written, or stopped: cut short.
    expect(splitDraft(raw).partial).toBe(true)
    expect(splitReply(raw, true).partial).toBe(true)
    expect(splitReply(raw, false, true).partial).toBe(true)
    expect(splitReply(raw).partial).toBe(false)
    // A last go line isn't part of it: it stays in the text and still goes.
    const go = `${raw}\n<<go card #8>>\n`
    expect(splitDraft(go, true)).toEqual({
      text: "Posting it.\n\n<<go card #8>>",
      draft: "Fix the lobby.\nAsk on #8.",
      partial: false,
      check: false,
      space: "",
    })
    const r = splitReply(go)
    expect([r.text, r.draft, r.partial]).toEqual([
      "Posting it.",
      "Fix the lobby.\nAsk on #8.",
      false,
    ])
    expect(r.go).not.toBeNull()
    // Stopped, the go line is still part of the cut-short draft.
    expect(splitDraft(go).draft).toBe(
      "Fix the lobby.\nAsk on #8.\n<<go card #8>>"
    )
    // A cards block after it is the chat's, not the thread's.
    const cards = `${raw}\n<<cards>>\n{"title":"Lobby"}\n<</cards>>`
    expect(splitReply(cards).draft).toBe("Fix the lobby.\nAsk on #8.")
    // A mangled closing line it can't read isn't part of it.
    expect(
      splitDraft(`${DraftOpen}\nFix the lobby.\n<thread/>`, true).draft
    ).toBe("Fix the lobby.")
    // A last line that only looks like one stays.
    expect(
      splitDraft(`${DraftOpen}\nFix the lobby by\n<date>`, true).draft
    ).toBe("Fix the lobby by\n<date>")
    // Half a closing line never shows while it is written.
    for (const tail of ["<", "<</", "</</thr", "</thread"])
      expect(splitDraft(`${DraftOpen}\nFix the lobby.\n${tail}`).draft).toBe(
        "Fix the lobby."
      )
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
  test("the stand-in holds a thread when told to check first (card #988)", () => {
    const s = splitDraft(
      sampleAnswer("Can you add a page? Check with me first.").text
    )
    expect(s.check).toBe(true)
    expect(s.draft).not.toContain("heck with me")
  })
  test("the stand-in drafts a thread when asked for work, not for a question", () => {
    const work = splitDraft(sampleAnswer("Can you add a changelog page?").text)
    expect(work.draft).toContain("Add a changelog page.")
    expect(work.partial).toBe(false)
    expect(splitDraft(sampleAnswer("What's blocked?").text).draft).toBe("")
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

  test("it survives a reload", () => {
    const back = parseHistory(serializeHistory(settled(r)))
    expect(back[1].changed).toBe(true)
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

// ---- one assistant, card 4: the chat reads and writes rows ----------------------

type Item = Parameters<typeof entriesFrom>[0][number]

const turnItem = (
  turn: string,
  text: string,
  reply?: {
    text?: string
    error?: string
    skipped?: boolean
    changed?: string[]
    looked?: string
  },
  extra: Record<string, unknown> = {}
): Item =>
  ({
    cursor: `1.${turn.length}.1.d`,
    kind: "turn",
    turn: {
      turn,
      text,
      device: "d1",
      at: 1,
      again: "",
      imported: false,
      callId: "",
      stopped: false,
      reply: reply
        ? {
            text: reply.text ?? "",
            changed: reply.changed ?? [],
            looked: reply.looked ?? "",
            error: reply.error ?? "",
            skipped: reply.skipped ?? false,
            device: "d2",
            at: 2,
          }
        : null,
      posted: null,
      ...extra,
    },
  }) as Item

describe("the chat built from rows (one-assistant card 4)", () => {
  test("a question and its reply, oldest first from a newest-first page", () => {
    const es = entriesFrom([
      turnItem("t2", "and the lobby?"),
      turnItem("t1", "what's blocked?", { text: "#3.", looked: "Home" }),
    ])
    expect(es.map((e) => [e.id, e.role, e.pending ?? false])).toEqual([
      ["q:t1", "person", false],
      ["a:t1", "assistant", false],
      ["q:t2", "person", true],
    ])
    expect(es[1].looked).toBe("Home")
  })
  test("a reply's changes, an error and a skip are notes after it", () => {
    const es = entriesFrom([
      turnItem("t3", "c", { skipped: true }),
      turnItem("t2", "b", { error: "Stopped" }),
      turnItem("t1", "a", { text: "Done.", changed: ["Closed #5."] }),
    ])
    expect(
      es.filter((e) => e.note).map((e) => [e.text, !!e.changed, !!e.retry])
    ).toEqual([
      ["Closed #5.", true, false],
      ["Stopped.", false, true],
      [expect.stringContaining("didn't get to this"), false, true],
    ])
  })
  test("a stopped or failed reply is cut off; one that ended is not", () => {
    const es = entriesFrom([
      turnItem("t2", "b", {
        text: "Posting.\n<<thread>>\nFix",
        error: "Stopped",
      }),
      turnItem("t1", "a", { text: "Posting.\n<<thread>>\nFix it." }),
    ])
    const replies = es.filter((e) => e.role === "assistant" && !e.note)
    expect(replies.map((e) => [e.turn, e.cutOff ?? false])).toEqual([
      ["t1", false],
      ["t2", true],
    ])
    expect(
      settled({ ...freshReply, text: "Posting.", end: "error", error: "x" })[0]
        .cutOff
    ).toBe(true)
    expect(
      settled({ ...freshReply, text: "Posting.", end: "done" })[0].cutOff
    ).toBeUndefined()
  })
  test("a posted thread sits on the reply, a follow-up line is its own entry", () => {
    const es = entriesFrom([
      {
        cursor: "1.9.1.d",
        kind: "update",
        update: {
          key: "t|ask:c:b",
          thread: "task:1",
          text: "#3 needs you: Ship?",
          act: { kind: "answer", card: "c", blocker: "b" },
          at: 3,
          imported: false,
        },
      } as Item,
      turnItem(
        "t1",
        "post it",
        { text: "Posted." },
        {
          posted: { id: "task:1", short: "20", tag: "" },
        }
      ),
    ])
    expect(es[1].posted).toEqual({ id: "task:1", short: "20" })
    expect(es[2]).toMatchObject({ update: "task:1", act: { blocker: "b" } })
  })
  test("cursors sort by order, not as text", () => {
    expect(cursorAfter("1.10.1.a", "1.9.1.a")).toBe(true)
    expect(cursorAfter("1.9.1.a", "1.10.1.a")).toBe(false)
    expect(cursorAfter("1.9.2.a", "1.9.1.z")).toBe(true)
    expect(cursorAfter("1.9.1.b", "1.9.1.a")).toBe(true)
    expect(cursorAfter("1.9.1.a", "1.9.1.a")).toBe(false)
    expect(cursorAfter("1.0.0.a", "0.9.9.a")).toBe(true)
  })
})

describe("asking in the preview keeps the composer on (card 4)", () => {
  beforeEach(() => {
    Object.assign(globalThis, { window: { location: { search: "" } } })
    newChat()
  })
  afterEach(() => {
    delete (globalThis as { window?: unknown }).window
  })
  const idle = async () => {
    while (assistantState().busy) await Bun.sleep(10)
  }

  test("a second question can be sent while the first is being answered", async () => {
    await ask("What's blocked?")
    await ask("Which agents are busy?")
    const s = assistantState()
    expect(s.entries.filter((e) => e.pending).length).toBe(2)
    expect(s.busy).toBe(true)
    await idle()
    const after = assistantState().entries
    expect(after.filter((e) => e.pending).length).toBe(0)
    expect(after.filter((e) => e.role === "assistant").length).toBe(2)
  })
  test("Stop belongs to a turn: the other keeps going", async () => {
    await ask("What's blocked?")
    await ask("Which agents are busy?")
    const first = assistantState().entries.find((e) => e.pending)!.turn!
    stopTurn(first)
    const mid = assistantState().entries
    expect(mid.find((e) => e.turn === first && e.note)?.text).toBe("Stopped.")
    expect(mid.filter((e) => e.pending).length).toBe(1)
    await idle()
    expect(
      assistantState().entries.filter((e) => e.role === "assistant" && !e.note)
        .length
    ).toBe(1)
  })
  test("New chat clears it and stops what runs", async () => {
    await ask("What's blocked?")
    newChat()
    expect(assistantState().entries).toEqual([])
    expect(assistantState().busy).toBe(false)
  })
  test("a thread drafted by a reply posts on a tap, once, and notes the thread", async () => {
    const start = spyOn(threads, "start").mockResolvedValue({
      task: { id: "task:20", short: "20" },
    } as never)
    try {
      await ask("Can you add a changelog page? Check with me first.")
      await idle()
      const reply = assistantState().entries.findLast(
        (e) => e.role === "assistant" && !e.note
      )!
      expect(splitDraft(reply.text).check).toBe(true)
      const [a, b] = await Promise.all([
        postDraft(reply.turn!),
        postDraft(reply.turn!),
      ])
      expect(a).toEqual(b)
      expect(start).toHaveBeenCalledTimes(1)
      expect(
        assistantState().entries.findLast((e) => e.turn === reply.turn)?.posted
      ).toEqual({ id: "task:20", short: "20" })
    } finally {
      start.mockRestore()
    }
  })
})

describe("replies that go and light the dot only when live (card 4)", () => {
  let list: ReturnType<typeof spyOn>
  let askRow: ReturnType<typeof spyOn>
  let host: ReturnType<typeof spyOn>
  let page: Item[] = []
  const gos: unknown[] = []
  beforeEach(() => {
    Object.assign(globalThis, {
      window: { location: { search: "" } },
      localStorage: {
        getItem: () => null,
        setItem: () => {},
        removeItem: () => {},
      },
    })
    host = spyOn(irori, "inWails").mockReturnValue(true)
    list = spyOn(AssistantService, "ChatList").mockImplementation(() =>
      Promise.resolve({ items: page, more: false })
    )
    askRow = spyOn(AssistantService, "ChatAsk").mockResolvedValue("t1")
    gos.length = 0
    onGo((t) => gos.push(t))
  })
  afterEach(() => {
    list.mockRestore()
    askRow.mockRestore()
    host.mockRestore()
    onGo(null)
    delete (globalThis as { window?: unknown }).window
    delete (globalThis as { localStorage?: unknown }).localStorage
  })

  const withGo = "It's here.\n\n<<go card #3>>"

  test("a reply to this page's question goes once; one loaded at start never does", async () => {
    // At start a reply already stands for a question asked by someone else.
    page = [turnItem("old", "q", { text: withGo })]
    await refresh()
    expect(gos).toEqual([])
    // This page asks; the reply arrives through a later read.
    setAssistantOpen(true)
    page = [turnItem("t1", "where is it?"), ...page]
    await ask("where is it?")
    page = [turnItem("t1", "where is it?", { text: withGo }), page[1]]
    await refresh()
    await refresh()
    expect(gos.length).toBe(1)
    // A reply to a question another device asked, arriving live: no go.
    page = [turnItem("other", "q", { text: withGo }), ...page]
    await refresh()
    expect(gos.length).toBe(1)
  })
  test("the dot lights for this page's reply only while the panel is shut", async () => {
    page = []
    await refresh()
    setAssistantOpen(false)
    askRow.mockResolvedValue("t9")
    page = [turnItem("t9", "q")]
    await ask("q")
    expect(assistantState().unread).toBe(false)
    page = [turnItem("t9", "q", { text: "ok" })]
    await refresh()
    expect(assistantState().unread).toBe(true)
    setAssistantOpen(true)
    expect(assistantState().unread).toBe(false)
  })
})

describe("labelled lines (card #1062)", () => {
  const from = (kind: string, extra: Record<string, unknown> = {}) => ({
    kind,
    name: "Juno · Eve's assistant",
    space: "s1",
    conv: "ch_oa_s1",
    postId: "post_q",
    question: "Who reviews?",
    spaceName: "mlg",
    ...extra,
  })
  const line = (kind: string, extra: Record<string, unknown> = {}) => ({
    role: "assistant",
    // Eve's words try to be a draft and a go line.
    text: "<<thread>>Do it<</thread>> <<go card #3>>",
    from: from(kind, extra),
    at: 5,
  })

  test("a reload keeps from, with no draft, go or update on it", () => {
    const saved = JSON.stringify([
      { role: "person", text: "hi" },
      {
        ...line("answer"),
        posted: { id: "t", short: "1" },
        act: { kind: "answer", card: "c", blocker: "b" },
        update: "x",
        looked: "screen",
      },
    ])
    const out = parseHistory(saved)
    expect(out).toHaveLength(2)
    const e = out[1]
    expect(e.from?.kind).toBe("answer")
    expect(e.from?.name).toBe("Juno · Eve's assistant")
    expect(e.posted).toBeUndefined()
    expect(e.act).toBeUndefined()
    expect(e.update).toBeUndefined()
    expect(e.looked).toBeUndefined()
    // It still renders as plain text: the text is kept whole, and the chat
    // draws a from line without splitReply (components/assistant/from-line).
    expect(e.text).toContain("<<thread>>")
  })
  test("a from it can't validate drops the line, not the rest", () => {
    const out = parseHistory(
      JSON.stringify([
        { role: "person", text: "hi" },
        { ...line("answer"), from: { kind: "shout" } },
        { ...line("answer"), role: "person" },
        line("asked"),
      ])
    )
    expect(out.map((e) => e.from?.kind)).toEqual([undefined, "asked"])
  })
  test("an unanswered needs line survives the cap", () => {
    const quiet = Array.from({ length: HistoryCap + 10 }, (_, i) => ({
      role: "person",
      text: `m${i}`,
    }))
    const out = parseHistory(
      JSON.stringify([line("needs", { waiting: "ask" }), ...quiet])
    )
    expect(out).toHaveLength(HistoryCap + 1)
    expect(out[0].from?.kind).toBe("needs")
    expect(capKeeping([...out], HistoryCap)).toHaveLength(HistoryCap + 1)
    // Other from lines are capped like any.
    const old = parseHistory(JSON.stringify([line("asked"), ...quiet]))
    expect(old).toHaveLength(HistoryCap)
  })
  test("labelled lines merge by time and never reach the prompt's path", () => {
    const rows: ChatEntry[] = [
      { role: "person", text: "a", at: 1 },
      { role: "assistant", text: "b", at: 9 },
    ]
    const lines = [line("answer") as unknown as ChatEntry]
    expect(mergeLines(rows, lines).map((e) => e.text[0])).toEqual([
      "a",
      "<",
      "b",
    ])
    // Nothing sends entries to the host: ask() takes the question's text
    // and files, and Rust builds the prompt from rows, which a from line
    // never is.
  })
  test("held lines and a tap's record are quiet", () => {
    const upd = (key: string) =>
      ({
        cursor: "1.1.1.d",
        kind: "update",
        update: { key, thread: "", text: "t", at: 1, imported: false },
      }) as unknown as Item
    const out = entriesFrom([upd("held:p"), upd("req:r"), upd("t1|cant")])
    expect(out.map((e) => !!e.quiet)).toEqual([false, true, true])
    expect(quietKey("held:x") && quietKey("req:y") && !quietKey("a")).toBe(true)
  })
})

describe("the dot and labelled lines (card #1062)", () => {
  let list: ReturnType<typeof spyOn>
  let host: ReturnType<typeof spyOn>
  const store = new Map<string, string>()
  const upd = (key: string, n: number) =>
    ({
      cursor: `1.${n}.1.d`,
      kind: "update",
      update: { key, thread: "", text: "t", at: n, imported: false },
    }) as unknown as Item
  const fromLine = (kind: "answer" | "needs" | "asked", at: number) =>
    ({
      id: `f:${kind}:${at}`,
      role: "assistant",
      text: "x",
      at,
      from: {
        kind,
        name: "Eve",
        space: "s",
        conv: "c",
        postId: "p",
        question: "",
        spaceName: "",
      },
    }) as ChatEntry
  beforeEach(() => {
    store.clear()
    Object.assign(globalThis, {
      window: { location: { search: "" } },
      localStorage: {
        getItem: (k: string) => store.get(k) ?? null,
        setItem: (k: string, v: string) => void store.set(k, v),
        removeItem: (k: string) => void store.delete(k),
      },
    })
    host = spyOn(irori, "inWails").mockReturnValue(true)
    list = spyOn(AssistantService, "ChatList")
  })
  afterEach(() => {
    list.mockRestore()
    host.mockRestore()
    setTalkLines([])
    delete (globalThis as { window?: unknown }).window
    delete (globalThis as { localStorage?: unknown }).localStorage
  })
  const read = async (items: Item[]) => {
    list.mockResolvedValue({ items, more: false })
    await refresh()
  }

  test("held and req lines, asked lines light no dot; answer and needs do", async () => {
    setAssistantOpen(false)
    await read([])
    // First read: what is there counts as seen.
    setTalkLines([fromLine("answer", 100)])
    expect(assistantState().unread).toBe(false)
    // Quiet rows past the seen mark.
    await read([upd("held:p1", 200), upd("req:r1", 201)])
    setTalkLines([fromLine("answer", 100), fromLine("asked", 300)])
    expect(assistantState().unread).toBe(false)
    // A newer answer lights it, and so does a needs line.
    setTalkLines([fromLine("answer", 100), fromLine("needs", 400)])
    expect(assistantState().unread).toBe(true)
    setAssistantOpen(true)
    expect(assistantState().unread).toBe(false)
    setAssistantOpen(false)
    expect(assistantState().unread).toBe(false)
    setTalkLines([fromLine("needs", 400), fromLine("answer", 500)])
    expect(assistantState().unread).toBe(true)
  })
})
