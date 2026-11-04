import { describe, expect, test } from "bun:test"

import { AgentRef, CardBlocker, CardThreadItem } from "./models"
import {
  answerable,
  blockerChip,
  heardLine,
  isRevision,
  BlockerState,
  blockerTone,
  needsPerson,
  opensInFull,
  threadLabel,
  ThreadPage,
  visible,
  workingLine,
  StoppedGrace,
  toolPhrase,
  ponderWord,
  PonderWords,
  startAgentItem,
} from "./thread"

const otter = AgentRef({
  key: "dev/otter-session",
  name: "sample-otter",
  status: "busy",
  live: true,
})
const wren = AgentRef({
  key: "mac/wren-session",
  name: "mac/wren-session",
  status: "gone",
})

describe("threadLabel", () => {
  test("a comment names its author and has no arrow or badges", () => {
    const l = threadLabel(
      CardThreadItem({ kind: "comment", who: otter, text: "hi" })
    )
    expect(l).toEqual({
      author: "sample-otter",
      device: "",
      badges: [],
      waiting: [],
    })
  })

  test("a message gets the arrow, its device and its badges", () => {
    const l = threadLabel(
      CardThreadItem({
        kind: "message",
        who: otter,
        to: wren,
        device: "MacBook",
        gone: true,
        held: true,
        files: 2,
      })
    )
    // A key the roster no longer names shows as its session part.
    expect(l.arrow).toBe("wren-session")
    expect(l.device).toBe("MacBook")
    expect(l.badges).toEqual([
      { label: "recipient gone", tone: "warning" },
      { label: "held", tone: "slate" },
      { label: "2 files", tone: "slate" },
    ])
  })

  test("a blocker answer names its item, except under that item", () => {
    const it = CardThreadItem({
      kind: "comment",
      who: otter,
      to: wren,
      blockerId: "b2",
    })
    expect(threadLabel(it).badges).toEqual([{ label: "on b2", tone: "slate" }])
    expect(threadLabel(it, true).badges).toEqual([])
    // Its line in the card's thread is an event: no badge.
    expect(
      threadLabel(
        CardThreadItem({ kind: "event", who: otter, blockerId: "b2" })
      ).badges
    ).toEqual([])
  })

  test("one file is singular", () => {
    const l = threadLabel(
      CardThreadItem({ kind: "message", who: otter, to: wren, files: 1 })
    )
    expect(l.badges).toEqual([{ label: "1 file", tone: "slate" }])
  })
})

describe("visible", () => {
  const items = Array.from({ length: ThreadPage + 7 }, (_, i) => i)

  test("keeps the newest page", () => {
    const v = visible(items, false)
    expect(v.length).toBe(ThreadPage)
    expect(v[0]).toBe(7)
    expect(v[v.length - 1]).toBe(ThreadPage + 6)
  })

  test("shows everything when asked, or when it fits", () => {
    expect(visible(items, true)).toBe(items)
    expect(visible([1, 2], false)).toEqual([1, 2])
    expect(visible(items, false, 3)).toEqual([
      ThreadPage + 4,
      ThreadPage + 5,
      ThreadPage + 6,
    ])
  })
})

describe("opensInFull", () => {
  const msg = (over: Partial<CardThreadItem>) =>
    CardThreadItem({
      id: "msg_1",
      kind: "message",
      who: otter,
      text: "hi",
      ...over,
    })

  test("a whole message without files has nothing more", () => {
    expect(opensInFull(msg({}))).toBe(false)
  })

  test("cut, held or with files, it opens in full", () => {
    expect(opensInFull(msg({ text: "xxx… (full: msg:msg_1)" }))).toBe(true)
    expect(opensInFull(msg({ text: "", held: true }))).toBe(true)
    expect(opensInFull(msg({ files: 2 }))).toBe(true)
  })

  test("a comment never does, nor another message's pointer", () => {
    expect(
      opensInFull(
        CardThreadItem({
          id: "c1",
          kind: "comment",
          who: otter,
          held: true,
        })
      )
    ).toBe(false)
    expect(opensInFull(msg({ text: "see (full: msg:msg_2)" }))).toBe(false)
  })
})

describe("blocker states", () => {
  // Each state: its tone, whether it waits on the person, and whether the
  // person can still answer it.
  const cases: [string, string, boolean, boolean][] = [
    [BlockerState.Open, "amber", true, true],
    [BlockerState.Answered, "grey", false, true],
    [BlockerState.Replied, "amber", true, true],
    [BlockerState.Resolved, "green", false, false],
    [BlockerState.Closed, "closed", false, false],
  ]
  for (const [state, tone, needs, open] of cases)
    test(state, () => {
      expect(blockerTone(state)).toBe(tone as ReturnType<typeof blockerTone>)
      expect(needsPerson(state)).toBe(needs)
      expect(answerable(state)).toBe(open)
    })

  test("an item from an older app (no state) is open", () => {
    expect(blockerTone("")).toBe("amber")
    expect(needsPerson("")).toBe(true)
  })
})

describe("blockerChip", () => {
  const item = (over: Partial<CardBlocker>) =>
    CardBlocker({ id: "b1", raisedBy: otter, ...over })

  test("an open done check is ready for you; answered, it went back", () => {
    expect(blockerChip(item({ doneCheck: true, state: "open" }))).toEqual({
      label: "Ready for you",
      tone: "success",
    })
    expect(
      blockerChip(item({ doneCheck: true, state: "answered", sentTo: otter }))
        ?.label
    ).toBe("Sent to sample-otter")
  })

  test("answered says who it went to, and when they have ended", () => {
    expect(blockerChip(item({ state: "answered", sentTo: otter }))).toEqual({
      label: "Sent to sample-otter",
      tone: "slate",
    })
    expect(blockerChip(item({ state: "answered", sentTo: wren }))).toEqual({
      label: "Sent to wren-session · session ended",
      tone: "slate",
    })
    // Without sentTo, the raiser.
    expect(blockerChip(item({ state: "answered" }))?.label).toBe(
      "Sent to sample-otter"
    )
  })

  test("replied is back with the person; the rest have none", () => {
    expect(blockerChip(item({ state: "replied" }))).toEqual({
      label: "Replied",
      tone: "warning",
    })
    for (const state of ["open", "resolved", "closed", ""])
      expect(blockerChip(item({ state }))).toBeNull()
  })
})

describe("revisions", () => {
  const rev = CardThreadItem({
    id: "d:3",
    kind: "event",
    who: otter,
    text: "sample-otter revised blocker b1: no options now, a plain question",
    blockerId: "b1",
    revised: true,
  })
  const reply = CardThreadItem({
    id: "c2",
    kind: "comment",
    who: otter,
    text: "Wintun is signed.",
    blockerId: "b1",
  })

  test("a revision is an event marked revised", () => {
    expect(isRevision(rev)).toBe(true)
    expect(isRevision(reply)).toBe(false)
    expect(isRevision(CardThreadItem({ ...rev, kind: "comment" }))).toBe(false)
  })

  test("an open item revised under the person's reply says so", () => {
    const item = (state: string, thread: CardThreadItem[]) =>
      CardBlocker({ id: "b1", raisedBy: otter, state, thread })
    expect(blockerChip(item("open", [reply, rev]))).toEqual({
      label: "Revised",
      tone: "warning",
    })
    expect(blockerChip(item("", [rev]))?.label).toBe("Revised")
    expect(blockerChip(item("open", [reply]))).toBeNull()
    // Once the person answers the revised item, it is sent again.
    expect(blockerChip(item("answered", [reply, rev]))?.label).toBe(
      "Sent to sample-otter"
    )
  })
})

describe("heardLine", () => {
  test("nobody around", () => {
    expect(heardLine([], [], "")).toBe("Nobody has heard this yet.")
  })

  test("the holder is woken; a watcher hears at its next turn", () => {
    expect(heardLine([otter], [wren], "wake")).toBe(
      "sample-otter is woken and will answer here. wren-session hears it at its next turn, watching: they answer only if asked."
    )
  })

  test("at the next turn, several", () => {
    expect(heardLine([otter, wren], [], "next_turn")).toBe(
      "sample-otter and wren-session hear it at their next turn and will answer here."
    )
  })

  test("watchers alone carry the wake", () => {
    expect(heardLine([], [otter], "wake")).toBe(
      "sample-otter is woken, watching."
    )
  })
})

describe("unheard", () => {
  test("the person's comment nobody heard has the badge", () => {
    const l = threadLabel(
      CardThreadItem({ kind: "comment", who: otter, unheard: true })
    )
    expect(l.badges).toEqual([
      { label: "nobody has heard this yet", tone: "warning" },
    ])
  })
})

describe("pending", () => {
  test("the person's comment says who hasn't heard it yet, under it", () => {
    const lines = [
      "sample-heron is busy: it hears this when its current turn ends",
      "sent to sample-crow, not read yet",
      "sample-wren read it",
    ]
    const l = threadLabel(
      CardThreadItem({ kind: "comment", who: otter, pending: lines })
    )
    expect(l.waiting).toEqual(lines)
    expect(l.badges).toEqual([])
  })
})

describe("workingLine", () => {
  const oak = AgentRef({
    key: "mac/oak",
    name: "oak",
    status: "idle",
    live: true,
  })
  const elm = AgentRef({
    key: "mac/elm",
    name: "elm",
    status: "idle",
    live: true,
  })
  const you = AgentRef({
    key: "user:me",
    name: "You",
    status: "person",
    live: true,
  })
  const t0 = new Date("2026-11-03T05:00:00Z")
  const at = (s: number) => new Date(t0.getTime() + s * 1000)
  const comment = CardThreadItem({
    id: "c1",
    kind: "comment",
    who: you,
    text: "how's it going?",
    at: at(0).toISOString(),
    pending: ["oak read it"],
    heardBy: ["mac/oak"],
  })
  const roster = (m: Record<string, [string, number]>) =>
    new Map(
      Object.entries(m).map(([k, [status, s]]) => [
        k,
        { status, updatedAt: at(s).toISOString() },
      ])
    )

  test("a holder in a turn: working, by the live roster not the card's snapshot", () => {
    expect(
      workingLine([oak], [comment], roster({ "mac/oak": ["busy", 1] }), at(2))
    ).toEqual({
      text: "oak is working…",
      tone: "working",
      who: "oak",
      verb: "is",
      doing: "",
    })
  })

  test("two busy agents are named together", () => {
    const l = workingLine(
      [oak, elm],
      [],
      roster({ "mac/oak": ["busy", 1], "mac/elm": ["busy", 1] }),
      at(2)
    )
    expect(l?.text).toBe("oak and elm are working…")
  })

  test("idle after a turn that followed the comment, nothing on the thread: stopped", () => {
    expect(
      workingLine([oak], [comment], roster({ "mac/oak": ["idle", 20] }), at(30))
    ).toEqual({
      text: "oak stopped without replying",
      tone: "stopped",
    })
  })

  test("a reply after the comment clears it", () => {
    const reply = CardThreadItem({
      id: "r1",
      kind: "comment",
      who: oak,
      text: "done",
      at: at(10).toISOString(),
    })
    expect(
      workingLine(
        [oak],
        [comment, reply],
        roster({ "mac/oak": ["idle", 20] }),
        at(30)
      )
    ).toBeNull()
  })

  test("not stopped while the idle moment is shorter than the grace", () => {
    const now = new Date(at(20).getTime() + StoppedGrace - 1)
    expect(
      workingLine([oak], [comment], roster({ "mac/oak": ["idle", 20] }), now)
    ).toBeNull()
  })

  test("not stopped when it went idle before the comment (no turn since)", () => {
    expect(
      workingLine([oak], [comment], roster({ "mac/oak": ["idle", -5] }), at(30))
    ).toBeNull()
  })

  test("not stopped while it hasn't read the comment yet", () => {
    const waiting = { ...comment, pending: ["queued for oak"], heardBy: [] }
    expect(
      workingLine([oak], [waiting], roster({ "mac/oak": ["idle", 20] }), at(30))
    ).toBeNull()
  })

  test("nothing once the card is over, or for an agent that is gone or offline", () => {
    expect(
      workingLine(
        [oak],
        [comment],
        roster({ "mac/oak": ["busy", 1] }),
        at(2),
        true
      )
    ).toBeNull()
    const gone = { ...oak, live: false }
    const away = { ...oak, offline: true }
    expect(
      workingLine(
        [gone, away],
        [comment],
        roster({ "mac/oak": ["busy", 1] }),
        at(2)
      )
    ).toBeNull()
  })

  test("an ask handed to an agent counts while it decides", () => {
    const handed = { ...oak, assigned: true }
    expect(
      workingLine(
        [handed],
        [comment],
        roster({ "mac/oak": ["busy", 1] }),
        at(2)
      )?.tone
    ).toBe("working")
  })
})

describe("working line on a card page (card #317)", () => {
  const oak = AgentRef({ key: "mac/oak", name: "oak", status: "idle", live: true })
  const elm = AgentRef({ key: "mac/elm", name: "elm", status: "idle", live: true })
  const busyRoster = new Map([
    ["mac/oak", { status: "busy", updatedAt: "" }],
    ["mac/elm", { status: "busy", updatedAt: "" }],
  ])
  test("names who the card's rule gives, not every busy agent on it", () => {
    const l = workingLine([oak, elm], [], busyRoster, new Date(), false, [elm])
    expect(l).toMatchObject({ tone: "working", who: "elm", verb: "is" })
  })
  test("nobody working by the card's rule: no working line, though the roster says busy", () => {
    expect(workingLine([oak], [], busyRoster, new Date(), false, [])).toBeNull()
  })
  // Card #431: the card's rule gives who runs a background job between turns.
  test("a holder running a background job: says so, not stopped or nothing", () => {
    const shell = new Map([["mac/oak", { status: "shell", updatedAt: "" }]])
    expect(workingLine([oak], [], shell, new Date(), false, [], [oak])).toEqual({
      text: "oak is running a job…",
      tone: "working",
      who: "oak",
      verb: "is",
      doing: "running a job",
    })
    expect(workingLine([oak], [], shell, new Date(), false, [], [])).toBeNull()
  })
  // Card #489: how long one job has run, from when its status began.
  test("a job says how long it has run", () => {
    const now = new Date("2026-11-03T05:12:00Z")
    const since = (at: string) =>
      new Map([["mac/oak", { status: "shell", statusSince: at }]])
    expect(
      workingLine([oak], [], since("2026-11-03T05:00:00Z"), now, false, [], [oak])?.text
    ).toBe("oak is running a job · 12 min…")
    expect(
      workingLine([oak], [], since("2026-11-03T05:11:30Z"), now, false, [], [oak])?.text
    ).toBe("oak is running a job…")
  })
})

describe("working line for a background job (card #431)", () => {
  const oak = AgentRef({ key: "mac/oak", name: "oak", status: "idle", live: true })
  const you = AgentRef({ key: "user:me", name: "You", status: "person", live: true })
  const t0 = new Date("2026-11-03T05:00:00Z")
  const comment = CardThreadItem({
    id: "c1",
    kind: "comment",
    who: you,
    text: "how's it going?",
    at: t0.toISOString(),
    heardBy: ["mac/oak"],
  })
  const later = new Date(t0.getTime() + 60_000)
  const roster = (status: string) =>
    new Map([["mac/oak", { status, updatedAt: new Date(t0.getTime() + 5_000).toISOString() }]])
  test("shell after the person's comment: running a job, not stopped", () => {
    expect(workingLine([oak], [comment], roster("shell"), later)?.text).toBe(
      "oak is running a job…"
    )
  })
  test("idle after the person's comment: stopped without replying, as before", () => {
    expect(workingLine([oak], [comment], roster("idle"), later)).toEqual({
      text: "oak stopped without replying",
      tone: "stopped",
    })
  })
})

describe("ponderWord", () => {
  test("changes word each tick, in turn, and wraps", () => {
    expect(ponderWord(0)).toBe("working")
    expect(ponderWord(1)).not.toBe(ponderWord(0))
    expect(ponderWord(PonderWords.length)).toBe(ponderWord(0))
    expect(ponderWord(-1)).toBe(PonderWords[PonderWords.length - 1])
  })
  test("Reduce Motion's still word is the plain one", () => {
    expect(ponderWord(0)).toBe(PonderWords[0])
  })
})

describe("toolPhrase", () => {
  test("names what common tools do", () => {
    expect(toolPhrase("")).toBe("")
    expect(toolPhrase("Bash")).toBe("running a command")
    expect(toolPhrase("Edit")).toBe("editing files")
    expect(toolPhrase("mcp__irori__board_comment")).toBe("on the Board")
    expect(toolPhrase("mcp__claude-in-chrome__navigate")).toBe(
      "using the browser"
    )
    expect(toolPhrase("mcp__blender__look")).toBe("using blender")
    expect(toolPhrase("Frobnicate")).toBe("using Frobnicate")
  })

  test("the working line adds it for one busy agent", () => {
    const oak = AgentRef({
      key: "mac/oak",
      name: "oak",
      status: "idle",
      live: true,
    })
    const live = new Map([
      ["mac/oak", { status: "busy", updatedAt: "", lastTool: "Grep" }],
    ])
    expect(workingLine([oak], [], live, new Date())?.text).toBe(
      "oak is searching files…"
    )
  })
})

describe("Start an agent with this (card #452)", () => {
  const it_ = (id: string, kind: string, status: string, blockerId?: string) => ({
    id,
    kind,
    who: { status },
    blockerId,
  })
  test("the person's latest comment offers it, never an answer or an agent's", () => {
    expect(
      startAgentItem([
        it_("c1", "comment", "person"),
        it_("c2", "comment", "idle"),
        it_("c3", "comment", "person", "b1"),
        it_("e1", "event", "person"),
      ])
    ).toBe("c1")
    expect(startAgentItem([it_("c2", "comment", "idle")])).toBe("")
  })
})
