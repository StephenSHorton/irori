import { describe, expect, test } from "bun:test"

import { AgentRef } from "../gen/models"
import {
  addMention,
  agentOnRows,
  rowName,
  boardRows,
  cardRefs,
  localRef,
  refText,
  cardTokenAt,
  filterCards,
  insertCardRef,
  askingLine,
  composing,
  exactUnique,
  filterCandidates,
  hasMention,
  insertMention,
  keepMentions,
  MaxMentions,
  menuKey,
  moveHighlight,
  notDelivered,
  settle,
  settleRange,
  splitMentions,
  tokenAt,
  type MentionCandidate,
} from "./mentions"

function cand(
  key: string,
  name: string,
  opts: { status?: string; device?: string } = {}
): MentionCandidate {
  return {
    who: AgentRef({
      key,
      name,
      status: opts.status ?? "idle",
      live: opts.status !== "gone",
      where: opts.device ?? "Macbook Pro (this one)",
      here: !opts.device,
      device: opts.device,
    }),
    cwd: "",
    repo: "",
  }
}

const oak = cand("agent:oak", "oak")
const oakMac = cand("agent:oak-mac", "oak", { device: "Mac" })
const oaktree = cand("agent:oaktree", "oak tree")
const fern = cand("agent:fern", "fern")
const cedar = cand("agent:cedar", "cedar", { status: "gone" })
const boak = cand("agent:boak", "boak")
const all = [fern, cedar, oakMac, boak, oaktree, oak]
const names = all.map((c) => c.who.name)
const keys = (cs: MentionCandidate[]) => cs.map((c) => c.who.key)

describe("tokenAt", () => {
  test("@ at the start, after whitespace or ( starts a token", () => {
    expect(tokenAt("@oa", 3)).toEqual({ start: 0, query: "oa" })
    expect(tokenAt("hi @", 4)).toEqual({ start: 3, query: "" })
    expect(tokenAt("hi\n@f", 5)).toEqual({ start: 3, query: "f" })
    expect(tokenAt("(@f", 3)).toEqual({ start: 1, query: "f" })
  })

  test("a@b is not a token", () => {
    expect(tokenAt("mail a@b", 8)).toBeNull()
  })

  test("the query runs to the caret", () => {
    expect(tokenAt("@fern and more", 3)).toEqual({ start: 0, query: "fe" })
  })

  test("a space ends it, unless a name still holds it", () => {
    expect(tokenAt("@oak hello", 10, names)).toBeNull()
    expect(tokenAt("@oak ", 5, ["oak"])).toBeNull()
    expect(tokenAt("@oak t", 6, names)).toEqual({ start: 0, query: "oak t" })
    expect(tokenAt("@oak\n", 5, names)).toBeNull()
  })
})

describe("filterCandidates", () => {
  test("names holding the query, ignoring case", () => {
    expect(keys(filterCandidates(all, "OAK"))).toEqual([
      "agent:oak",
      "agent:oaktree",
      "agent:oak-mac",
      "agent:boak",
    ])
  })

  test("live first, this device's before others', gone last", () => {
    expect(keys(filterCandidates(all, ""))).toEqual([
      "agent:boak",
      "agent:fern",
      "agent:oak",
      "agent:oaktree",
      "agent:oak-mac",
      "agent:cedar",
    ])
  })

  test("at most 20 rows", () => {
    const many = Array.from({ length: 30 }, (_, i) => cand(`k${i}`, `a${i}`))
    expect(filterCandidates(many, "a")).toHaveLength(20)
  })
})

describe("moveHighlight", () => {
  test("Tab enters at the first row, then steps and wraps", () => {
    let i = -1
    const seen: number[] = []
    for (let n = 0; n < 4; n++) seen.push((i = moveHighlight(i, 3, 1)))
    expect(seen).toEqual([0, 1, 2, 0])
  })

  test("Shift+Tab goes back, wrapping; from none it is the last row", () => {
    expect(moveHighlight(-1, 3, -1)).toBe(2)
    expect(moveHighlight(0, 3, -1)).toBe(2)
    expect(moveHighlight(2, 3, -1)).toBe(1)
  })

  test("no rows, no highlight", () => {
    expect(moveHighlight(0, 0, 1)).toBe(-1)
  })
})

describe("menuKey (the Enter guard)", () => {
  const k = (key: string, shiftKey = false, isComposing = false) => ({
    key,
    shiftKey,
    isComposing,
  })

  test("open with rows: Tab, arrows, Enter and Esc are the picker's", () => {
    expect(menuKey(k("Tab"), true, 2)).toBe("next")
    expect(menuKey(k("Tab", true), true, 2)).toBe("prev")
    expect(menuKey(k("ArrowDown"), true, 2)).toBe("next")
    expect(menuKey(k("ArrowUp"), true, 2)).toBe("prev")
    expect(menuKey(k("Enter"), true, 2)).toBe("pick")
    expect(menuKey(k("Escape"), true, 2)).toBe("close")
  })

  test("closed, Enter is the site's (it sends)", () => {
    expect(menuKey(k("Enter"), false, 2)).toBeNull()
    expect(menuKey(k("Tab"), false, 2)).toBeNull()
  })

  test("no matches: Enter is held (no send), Tab moves focus; Esc closes", () => {
    expect(menuKey(k("Enter"), true, 0)).toBe("hold")
    expect(menuKey(k("Enter", true), true, 0)).toBeNull()
    expect(menuKey(k("Tab"), true, 0)).toBeNull()
    expect(menuKey(k("Escape"), true, 0)).toBe("close")
  })

  test("Shift+Enter stays a new line; an IME gets every key", () => {
    expect(menuKey(k("Enter", true), true, 2)).toBeNull()
    expect(menuKey(k("Enter", false, true), true, 2)).toBeNull()
    expect(menuKey(k("Escape", false, true), true, 2)).toBeNull()
  })
})

describe("insertMention", () => {
  test("puts @name and a space in place of the query", () => {
    expect(
      insertMention("hi @oa", { start: 3, query: "oa" }, 6, "oak")
    ).toEqual({
      text: "hi @oak ",
      caret: 8,
    })
  })

  test("keeps the text after the caret, without doubling a space", () => {
    expect(
      insertMention("@f there", { start: 0, query: "f" }, 2, "fern")
    ).toEqual({
      text: "@fern there",
      caret: 6,
    })
  })

  test("replaces the rest of the word the caret is in", () => {
    // Caret after "@f" in "@foak hello": the old name goes.
    expect(
      insertMention("@foak hello", { start: 0, query: "f" }, 2, "fern")
    ).toEqual({ text: "@fern hello", caret: 6 })
    // Caret after "@oa" in "@oak".
    expect(insertMention("@oak", { start: 0, query: "oa" }, 3, "oak")).toEqual({
      text: "@oak ",
      caret: 5,
    })
    // Punctuation after the word stays.
    expect(
      insertMention("@fx, hi", { start: 0, query: "f" }, 2, "fern")
    ).toEqual({
      text: "@fern , hi",
      caret: 6,
    })
  })
})

describe("IME", () => {
  test("keyCode 229 counts as composing (WKWebView's commit Enter)", () => {
    expect(
      composing({ nativeEvent: { isComposing: false, keyCode: 229 } })
    ).toBe(true)
    expect(composing({ nativeEvent: { isComposing: true, keyCode: 13 } })).toBe(
      true
    )
    expect(
      composing({ nativeEvent: { isComposing: false, keyCode: 13 } })
    ).toBe(false)
  })
})

describe("punctuation after a name", () => {
  test('settle and the filter read "@oak," as oak', () => {
    expect(settle("@fern,", 6, [], all)).toEqual([
      { key: "agent:fern", name: "fern" },
    ])
    expect(keys(filterCandidates(all, "fern."))).toEqual(["agent:fern"])
  })
})

describe("same-named mentions", () => {
  const ms = [
    { key: "agent:oak", name: "oak" },
    { key: "agent:oak-mac", name: "oak" },
  ]

  test("one @oak left keeps one key, the first added", () => {
    expect(keepMentions(ms, "@oak hi")).toEqual([ms[0]])
    expect(keepMentions(ms, "@oak and @oak")).toBe(ms)
    expect(keepMentions(ms, "hi")).toEqual([])
  })

  test("the k-th @oak chip is the k-th oak", () => {
    const spans = splitMentions("@oak and @oak", ms)
    const got = spans.flatMap((x) =>
      x.kind === "mention" ? [x.mention.key] : []
    )
    expect(got).toEqual(["agent:oak", "agent:oak-mac"])
  })
})

describe("settleRange (paste, undo)", () => {
  test("records each exact, unique live name in the range", () => {
    expect(settleRange("@fern please review", 0, 19, [], all)).toEqual([
      { key: "agent:fern", name: "fern" },
    ])
    // Two oaks: ambiguous, not recorded; a gone one neither.
    expect(settleRange("@oak @cedar", 0, 11, [], all)).toEqual([])
    // Outside the range is left alone.
    expect(settleRange("@fern and @boak", 6, 15, [], all)).toEqual([
      { key: "agent:boak", name: "boak" },
    ])
    // A longer name wins.
    expect(settleRange("@oak tree hi", 0, 12, [], [oaktree, fern])).toEqual([
      { key: "agent:oaktree", name: "oak tree" },
    ])
  })
})

describe("token tracking on edit", () => {
  const ms = [
    { key: "agent:oak", name: "oak" },
    { key: "agent:fern", name: "fern" },
  ]

  test("removing @name drops its key", () => {
    expect(keepMentions(ms, "@oak and @fern")).toBe(ms)
    expect(keepMentions(ms, "@oak and ")).toEqual([ms[0]])
    expect(keepMentions(ms, "@oa and @fern")).toEqual([ms[1]])
    expect(keepMentions(ms, "")).toEqual([])
  })

  test("the name must stand whole, case as inserted", () => {
    expect(hasMention("@oakx", "oak")).toBe(false)
    expect(hasMention("@Oak", "oak")).toBe(false)
    expect(hasMention("(@oak).", "oak")).toBe(true)
    expect(hasMention("x@oak", "oak")).toBe(false)
    expect(hasMention("@oak tree", "oak tree")).toBe(true)
  })

  test("one entry per key, at most ten", () => {
    expect(addMention(ms, { key: "agent:oak", name: "oak" })).toBe(ms)
    let many: { key: string; name: string }[] = []
    for (let i = 0; i < 12; i++)
      many = addMention(many, { key: `k${i}`, name: `n${i}` })
    expect(many).toHaveLength(MaxMentions)
  })
})

describe("exact names", () => {
  test("one live agent named exactly that, ignoring case", () => {
    expect(exactUnique("FERN", all)?.who.key).toBe("agent:fern")
    // two live oaks: ambiguous
    expect(exactUnique("oak", all)).toBeNull()
    // gone agents never match by name
    expect(exactUnique("cedar", all)).toBeNull()
    expect(exactUnique("", all)).toBeNull()
  })

  test("settle records the exact name at the caret, as typed", () => {
    expect(settle("hi @Fern", 8, [], all)).toEqual([
      { key: "agent:fern", name: "Fern" },
    ])
    expect(settle("hi @fer", 7, [], all)).toEqual([])
    const ms = [{ key: "agent:fern", name: "fern" }]
    expect(settle("@fern", 5, ms, all)).toBe(ms)
  })
})

describe("splitMentions", () => {
  const ms = [
    { name: "oak", id: 1 },
    { name: "oak tree", id: 2 },
  ]

  test("cuts around each named @name, longest first", () => {
    expect(splitMentions("ask @oak tree and @oak.", ms)).toEqual([
      { kind: "text", text: "ask " },
      { kind: "mention", text: "@oak tree", mention: ms[1] },
      { kind: "text", text: " and " },
      { kind: "mention", text: "@oak", mention: ms[0] },
      { kind: "text", text: "." },
    ])
  })

  test("an @word no mention names stays text", () => {
    expect(splitMentions("hi @fern, a@oak", ms)).toEqual([
      { kind: "text", text: "hi @fern, a@oak" },
    ])
    expect(splitMentions("plain", [])).toEqual([
      { kind: "text", text: "plain" },
    ])
  })
})

describe("lines", () => {
  test("not delivered names each miss", () => {
    expect(
      notDelivered([
        { name: "oak", state: "woke" },
        { name: "fern", state: "gone" },
        { name: "cedr", state: "unknown" },
      ])
    ).toBe("Not delivered: fern is gone; no agent named cedr.")
    expect(notDelivered([{ name: "oak", state: "woke" }])).toBe("")
    expect(notDelivered(undefined)).toBe("")
  })

  test("asking line", () => {
    expect(askingLine([{ key: "a", name: "oak" }])).toBe("Asking @oak only")
    expect(
      askingLine([
        { key: "a", name: "oak" },
        { key: "b", name: "fern" },
        { key: "c", name: "elm" },
      ])
    ).toBe("Asking @oak, @fern and @elm only")
  })
})

test("@oak inside a recorded @oak tree doesn't keep oak", () => {
  const ms = [
    { key: "agent:oak", name: "oak" },
    { key: "agent:oaktree", name: "oak tree" },
  ]
  expect(keepMentions(ms, "@oak tree hi")).toEqual([ms[1]])
  expect(keepMentions(ms, "@oak tree and @oak")).toBe(ms)
})

describe("card mentions (card #114)", () => {
  test("cardRefs finds whole-word refs only", () => {
    const r = cardRefs("see #12, (mlg#3) not a.#4 or #5a; #67")
    expect(r.map((x) => [x.tag, x.n])).toEqual([
      [null, "12"],
      ["mlg", "3"],
      [null, "67"],
    ])
  })
  test("cardTokenAt opens on # at a word start", () => {
    expect(cardTokenAt("hi #1", 5)).toEqual({ start: 3, query: "1" })
    expect(cardTokenAt("a#1", 3)).toBeNull()
    expect(cardTokenAt("#subtask lay", 12, ["Subtask layout"])).toEqual({
      start: 0,
      query: "subtask lay",
    })
    expect(cardTokenAt("#sub lay", 8, ["Other"])).toBeNull()
  })
  test("filterCards ranks number matches first", () => {
    const cs = [
      { id: "a", ref: "#21", short: "21", title: "Two" },
      { id: "b", ref: "#3", short: "3", title: "Has 2 in title" },
      { id: "c", ref: "#2", short: "2", title: "Second" },
    ]
    expect(filterCards(cs, "2").map((c) => c.id)).toEqual(["a", "c", "b"])
    expect(filterCards(cs, "sec").map((c) => c.id)).toEqual(["c"])
  })
  test("insertCardRef replaces the token", () => {
    expect(
      insertCardRef("see #su", { start: 4, query: "su" }, 7, "#12")
    ).toEqual({
      text: "see #12 ",
      caret: 8,
    })
  })
})

describe("localRef (card #144)", () => {
  const cards = [
    { id: "c12", short: "12", title: "Live card" },
    { id: "m1", short: "1", title: "MLG card", tag: "mlg" },
    { id: "x7", short: "7", title: "One" },
    { id: "y7", short: "7", title: "Two", tag: "web" },
  ]
  const threads = [
    { id: "t136", short: "136", title: "A thread" },
    { id: "t12", short: "12", title: "Same number" },
  ]
  const ref = (s: string) => cardRefs(s)[0]

  test("a bare ref never picks a space's thread (#558)", () => {
    const onSpace = [{ id: "s9", short: "9", title: "x", spaceId: "b-mlg" }]
    expect(localRef(ref("#9"), cards, onSpace)).toBeNull()
  })

  test("your own thread wins over a tagged card's number (#982)", () => {
    const live = [{ id: "m5", short: "5", title: "MLG five", tag: "mlg" }]
    const own = [{ id: "t5", short: "5", title: "Thread five" }]
    expect(localRef(ref("#5"), live, own)?.id).toBe("t5")
    expect(localRef(ref("mlg#5"), live, own)?.id).toBe("m5")
    const priv = [{ ...own[0], spaceId: "b-mlg", private: true }]
    expect(localRef(ref("#5"), live, priv)?.id).toBe("t5")
  })
  test("an untagged live card still wins (#982)", () => {
    const live = [{ id: "c5", short: "5", title: "Mine" }]
    const own = [{ id: "t5", short: "5", title: "Thread five" }]
    expect(localRef(ref("#5"), live, own)?.id).toBe("c5")
  })
  test("a ref then a space closes the token when no title given holds it (#982)", () => {
    expect(cardTokenAt("see #12 ", 8, [])).toBeNull()
  })

  test("a live card first", () => {
    expect(localRef(ref("see #12"), cards, threads)).toEqual({
      kind: "card",
      id: "c12",
      title: "Live card",
    })
  })
  test("a thread when no live card has it", () => {
    expect(localRef(ref("mentioned this in #136"), cards, threads)).toEqual({
      kind: "thread",
      id: "t136",
      title: "A thread",
    })
  })
  test("a tagged ref never names a thread", () => {
    expect(localRef(ref("mlg#136"), cards, threads)).toBeNull()
    expect(localRef(ref("mlg#1"), cards, threads)?.id).toBe("m1")
  })
  test("two live cards, or none: left to the lookup", () => {
    expect(localRef(ref("#7"), cards, threads)).toBeNull()
    expect(localRef(ref("#99"), cards, threads)).toBeNull()
  })
  test("a bare ref only a shared board's live card has: left to the lookup (card #343)", () => {
    expect(localRef(ref("#1"), cards, threads)).toBeNull()
  })
  test("refText is what the backend reads", () => {
    expect(refText(ref("in #136"))).toBe("#136")
    expect(refText(ref("mlg#12"))).toBe("mlg#12")
  })
})

describe("boardRows", () => {
  const row = (key: string, board?: string) => ({
    who: AgentRef({ key, name: key, status: "idle", live: true, where: "" }),
    cwd: "",
    repo: "",
    board,
  })
  const rows = [
    row("d/me"),
    row("person:blake", "b1"),
    row("agent:c1", "b1"),
    row("agent:c2", "b2"),
  ]
  test("your own board offers only your agents", () => {
    expect(boardRows(rows, "").map((r) => r.who.key)).toEqual(["d/me"])
  })
  test("a shared board adds its people and their agents (card #177)", () => {
    expect(boardRows(rows, "b1").map((r) => r.who.key)).toEqual([
      "d/me",
      "person:blake",
      "agent:c1",
    ])
  })
})

describe("the agent on #N (card #452, spawned-agents §4)", () => {
  const fern = cand("dev/fern", "fern")
  const clover = cand("dev/clover", "clover")
  const card = (
    id: string,
    short: string,
    who: MentionCandidate["who"],
    state = "active",
    tag?: string
  ) => ({
    id,
    short,
    tag,
    holder: { who, state },
  })
  const cards = [
    card("c57", "57", clover.who),
    card("c61", "61", fern.who),
    card(
      "c62",
      "62",
      AgentRef({ key: "user:me", name: "You", status: "person" }),
      "person"
    ),
    card(
      "c63",
      "63",
      AgentRef({ key: "dev/gone", name: "sorrel", status: "gone" }),
      "gone"
    ),
    card("c70", "4", fern.who, "active", "wind"),
  ]

  test("one row per card a live agent holds; this card first, then newest", () => {
    const rows = agentOnRows(cards, [fern, clover], "c61")
    expect(rows.map((r) => r.card)).toEqual(["#61", "#57", "wind#4"])
    expect(rows.map(rowName)).toEqual([
      "the agent on #61",
      "the agent on #57",
      "the agent on wind#4",
    ])
    expect(rows[0].who.key).toBe("dev/fern")
  })

  test("with nothing typed the card rows lead; typing a name finds the name's row", () => {
    const all = [fern, clover, ...agentOnRows(cards, [fern, clover], "c61")]
    expect(rowName(filterCandidates(all, "")[0])).toBe("the agent on #61")
    expect(filterCandidates(all, "fer").map(rowName)).toEqual(["fern"])
  })

  test("#N, N and the words find a card row", () => {
    const all = [fern, clover, ...agentOnRows(cards, [fern, clover])]
    expect(filterCandidates(all, "#57").map(rowName)).toEqual([
      "the agent on #57",
    ])
    expect(filterCandidates(all, "57").map(rowName)).toEqual([
      "the agent on #57",
    ])
    expect(filterCandidates(all, "the agent on #6").map(rowName)).toEqual([
      "the agent on #61",
    ])
    expect(tokenAt("ask @the agent on #6", 20, all.map(rowName))).toEqual({
      start: 4,
      query: "the agent on #6",
    })
  })
})
