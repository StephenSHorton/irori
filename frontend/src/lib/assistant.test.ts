import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test"

import {
  ask,
  assistantState,
  capHistory,
  capKeeping,
  failedLine,
  foldEvent,
  freshReply,
  historyFor,
  HistoryCap,
  liftOver,
  newChat,
  postDraft,
  postsByItself,
  replaced,
  parseHistory,
  replyText,
  sampleAnswer,
  serializeHistory,
  settled,
  stopAsking,
  splitDraft,
  DraftClose,
  DraftOpen,
  DraftOpenCheck,
  type AssistantEvent,
  type ChatEntry,
  type Reply,
} from "./assistant"
import { threads } from "./cards"
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

describe("Ask Irori's drafted thread (card #783)", () => {
  const draft = (body: string) => `${DraftOpen}\n${body}\n${DraftClose}`
  test("a reply with no draft is left as it is", () => {
    expect(splitDraft("Two cards wait on you.")).toEqual({
      text: "Two cards wait on you.",
      draft: "",
      partial: false,
      check: false,
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
    })
    expect(splitDraft(`Drafting.\n${DraftOpen}\nFix the lob`)).toEqual({
      text: "Drafting.",
      draft: "Fix the lob",
      partial: true,
      check: false,
    })
    expect(splitDraft(`${DraftOpen}\nFix the lobby.\n<</thr`)).toEqual({
      text: "",
      draft: "Fix the lobby.",
      partial: true,
      check: false,
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
      text: `${text}\n\n(This draft was posted as thread #9.)`,
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
        text: "**otter** took your thread mlg#5.",
        update: "task:1",
      },
      { role: "person", text: "Use Steam.", to: "mlg#5" },
    ]
    const h = historyFor(entries)
    expect(h[0].text).toEndWith("(This draft was posted as thread mlg#5.)")
    expect(h[1]).toEqual({
      role: "assistant",
      text: "(The chat's own lines on the work it follows:)\n- **otter** took your thread mlg#5.",
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
      {
        role: "assistant",
        text: "otter replied on #20: Which page?",
        update: "task:1",
        act: { kind: "reply", thread: "task:1", to: "otter" },
      },
      { role: "person", text: "Home.", to: "#20" },
    ]
    expect(parseHistory(serializeHistory(entries))).toEqual(entries)
    const bad = parseHistory(
      JSON.stringify([
        { role: "assistant", text: "x", update: "t", act: { kind: "answer" } },
        { role: "person", text: "y", update: "t", to: 3 },
      ])
    )
    expect(bad).toEqual([
      { role: "assistant", text: "x", update: "t" },
      { role: "person", text: "y" },
    ])
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

describe("the Office Assistant posts what it drafts (card #988)", () => {
  const whole = `Posting it.\n${DraftOpen}\nFix the lobby.\n${DraftClose}`
  const held = `Waiting for you.\n${DraftOpenCheck}\nFix the lobby.\n${DraftClose}`
  const ended = (text: string, end: Reply["end"] = "done"): Reply => ({
    ...freshReply,
    text,
    end,
  })
  const started = (short: string) =>
    ({ task: { id: `task:${short}`, short } }) as never
  const lastDraft = () => {
    const { entries } = assistantState()
    return entries.findLast((e) => e.role === "assistant" && !e.note)
  }
  const replied = async () => {
    while (assistantState().reply || assistantState().autoPosting)
      await Bun.sleep(10)
  }
  let start: ReturnType<typeof spyOn>

  beforeEach(() => {
    // ask() checks for the app through window; here it is the preview.
    Object.assign(globalThis, { window: { location: { search: "" } } })
    newChat()
    start = spyOn(threads, "start").mockResolvedValue(started("20"))
  })
  afterEach(() => {
    start.mockRestore()
    delete (globalThis as { window?: unknown }).window
  })

  test("a check-first thread is known by its opening line, whole or partial", () => {
    expect(splitDraft(held)).toEqual({
      text: "Waiting for you.",
      draft: "Fix the lobby.",
      partial: false,
      check: true,
    })
    expect(splitDraft(`${DraftOpenCheck}\nFix the`).check).toBe(true)
    expect(splitDraft(`${DraftOpenCheck}\nFix the`).partial).toBe(true)
  })
  test("half of either opening line never shows", () => {
    for (const tail of ["<<thread>", "<<thread ", "<<thread chec"])
      expect(splitDraft(`Writing.\n${tail}`).text).toBe("Writing.")
  })
  test("only a finished, whole thread nobody asked to check posts itself", () => {
    expect(postsByItself(ended(whole))).toBe(true)
    expect(postsByItself(ended(held))).toBe(false)
    expect(postsByItself(ended("Two cards wait on you."))).toBe(false)
    expect(postsByItself(ended(`${DraftOpen}\nFix the`))).toBe(false)
    expect(postsByItself(ended(`${DraftOpen}\n${DraftClose}`))).toBe(false)
    expect(postsByItself(ended(whole, "error"))).toBe(false)
    expect(postsByItself(ended(whole, ""))).toBe(false)
  })
  test("markers only quoted, or laid out oddly, wait for a tap", () => {
    const quoted = `A thread goes between ${DraftOpen} and ${DraftClose}.`
    expect(splitDraft(quoted).draft).toBe("and")
    expect(postsByItself(ended(quoted))).toBe(false)
    expect(postsByItself(ended(`${quoted}\n\n${whole}`))).toBe(false)
    expect(postsByItself(ended(`${DraftOpen}Fix.${DraftClose}`))).toBe(false)
    expect(postsByItself(ended(`${whole}\nAnd more words.`))).toBe(false)
    expect(postsByItself(ended(`${whole}\n\n<<go card #8>>`))).toBe(true)
  })
  test("a newer thread retires the check-first one before it", () => {
    const es: ChatEntry[] = [
      { role: "assistant", text: held },
      { role: "person", text: "Good, mention #8 too" },
      { role: "assistant", text: held },
      { role: "assistant", text: "Ok.", note: true },
    ]
    expect(replaced(es, 0)).toBe(true)
    expect(replaced(es, 2)).toBe(false)
    expect(
      replaced([{ ...es[0], posted: { id: "t", short: "1" } }, es[2]], 0)
    ).toBe(false)
    expect(replaced([{ role: "assistant", text: whole }, es[2]], 0)).toBe(false)
  })
  test("asking for work posts one thread with no tap, and a tap adds none", async () => {
    await ask("Can you fix the lobby?")
    await replied()
    expect(start).toHaveBeenCalledTimes(1)
    expect(lastDraft()?.posted?.short).toBe("20")
    await postDraft(assistantState().entries.indexOf(lastDraft()!))
    expect(start).toHaveBeenCalledTimes(1)
  })
  test("a tap while it posts by itself is the same post", async () => {
    let done = (_: unknown) => {}
    start.mockReturnValue(new Promise((r) => (done = r)))
    await ask("Can you fix the lobby?")
    while (!assistantState().autoPosting) await Bun.sleep(10)
    const tap = postDraft(assistantState().entries.length - 1)
    done(started("21"))
    expect((await tap).short).toBe("21")
    await replied()
    expect(start).toHaveBeenCalledTimes(1)
  })
  test("asked to check first, only the tap posts", async () => {
    await ask("Can you fix the lobby? Check with me first.")
    await replied()
    expect(start).not.toHaveBeenCalled()
    expect(lastDraft()?.posted).toBeUndefined()
    await postDraft(assistantState().entries.length - 1)
    expect(start).toHaveBeenCalledTimes(1)
    expect(lastDraft()?.posted?.short).toBe("20")
  })
  test("a plain question posts nothing", async () => {
    await ask("What's blocked?")
    await replied()
    expect(start).not.toHaveBeenCalled()
  })
  test("a thread Stopped while being written posts nothing", async () => {
    await ask("Can you fix the lobby?")
    while (!assistantState().reply?.text.includes(DraftOpen)) await Bun.sleep(5)
    stopAsking()
    await Bun.sleep(30)
    expect(start).not.toHaveBeenCalled()
  })
  test("a failed post says so, and a tap retries it", async () => {
    start.mockRejectedValueOnce(new Error("The server is away."))
    await ask("Can you fix the lobby?")
    await replied()
    const { entries, autoFailed } = assistantState()
    expect(autoFailed.get(lastDraft()!)).toBe("The server is away.")
    expect(entries[entries.length - 1].note).toBeUndefined()
    expect(lastDraft()?.posted).toBeUndefined()
    await postDraft(entries.indexOf(lastDraft()!))
    expect(start).toHaveBeenCalledTimes(2)
    expect(lastDraft()?.posted?.short).toBe("20")
  })
  test("restored history never posts, even an unposted whole thread", async () => {
    const saved = serializeHistory([
      { role: "person", text: "Can you fix the lobby?" },
      { role: "assistant", text: whole },
    ])
    const was = globalThis.localStorage
    globalThis.localStorage = {
      getItem: () => saved,
      setItem: () => {},
      removeItem: () => {},
    } as unknown as Storage
    try {
      const fresh = await import("./assistant?restored")
      expect(fresh.assistantState().entries.length).toBe(2)
      await Bun.sleep(30)
      expect(start).not.toHaveBeenCalled()
      expect(fresh.assistantState().entries[1].posted).toBeUndefined()
    } finally {
      globalThis.localStorage = was
    }
  })
})
