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
  isWait,
  opensInFull,
  threadLabel,
  ThreadPage,
  visible,
  workingLine,
  StoppedGrace,
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
    expect(l.arrow).toBe("unknown agent")
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
      blockerGist: "Ship today, or wait…",
    })
    // Card #490: the question, never the id.
    expect(threadLabel(it).badges).toEqual([
      { label: "on “Ship today, or wait…”", tone: "slate" },
    ])
    expect(
      threadLabel(
        CardThreadItem({ kind: "comment", who: otter, blockerId: "b2" })
      ).badges
    ).toEqual([{ label: "on a blocker", tone: "slate" }])
    expect(threadLabel(it, true).badges).toEqual([])
    // Its line in the card's thread is an event: no badge.
    expect(
      threadLabel(
        CardThreadItem({ kind: "event", who: otter, blockerId: "b2" })
      ).badges
    ).toEqual([])
  })

  test("an unheard comment says an agent is starting for it (card #739)", () => {
    const it = CardThreadItem({ kind: "comment", who: otter, unheard: true })
    expect(threadLabel(it).badges).toEqual([
      { label: "nobody has heard this yet", tone: "warning" },
    ])
    expect(threadLabel(it, false, true).badges).toEqual([
      { label: "an agent is starting to answer", tone: "slate" },
    ])
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

  test("an open wait on other work is a wait, a closed one isn't (card #481)", () => {
    const waitsOn = { kind: "task" as const, label: "#392", taskId: "t392" }
    expect(isWait(CardBlocker({ state: "open", waitsOn }))).toBe(true)
    expect(isWait(CardBlocker({ state: "open" }))).toBe(false)
    const closed = CardBlocker({
      state: "closed",
      waitsOn,
      resolved: {
        answer: "task completed",
        option: -1,
        by: AgentRef(),
        at: "",
        closed: true,
        note: "",
      },
    })
    expect(isWait(closed)).toBe(false)
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
      label: "Sent to unknown agent · session ended",
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
      "sample-otter is woken and will answer here. unknown agent hears it at its next turn, watching: they answer only if asked."
    )
  })

  test("at the next turn, several", () => {
    expect(heardLine([otter, wren], [], "next_turn")).toBe(
      "sample-otter and unknown agent hear it at their next turn and will answer here."
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

  test("read it, in a turn, nothing written since: replying, by the live roster not the card's snapshot", () => {
    expect(
      workingLine([oak], [comment], roster({ "mac/oak": ["busy", 1] }), at(2))
    ).toEqual({
      text: "oak is replying…",
      tone: "replying",
      who: "oak",
      verb: "is",
    })
  })

  test("in a turn but it hasn't read the comment: nothing", () => {
    const unread = { ...comment, pending: ["queued for oak"], heardBy: [] }
    expect(
      workingLine([oak], [unread], roster({ "mac/oak": ["busy", 1] }), at(2))
    ).toBeNull()
  })

  test("in a turn with no comment from the person: nothing", () => {
    expect(
      workingLine([oak], [], roster({ "mac/oak": ["busy", 1] }), at(2))
    ).toBeNull()
  })

  test("a step tick isn't an answer: still replying", () => {
    const tick = CardThreadItem({
      id: "e1",
      kind: "event",
      who: oak,
      text: "ticked a step",
      at: at(5).toISOString(),
    })
    expect(
      workingLine(
        [oak],
        [comment, tick],
        roster({ "mac/oak": ["busy", 1] }),
        at(6)
      )?.text
    ).toBe("oak is replying…")
  })

  test.each(["comment", "message"])("a %s it wrote since ends it", (kind) => {
    const reply = CardThreadItem({
      id: "r1",
      kind,
      who: oak,
      text: "on it",
      at: at(5).toISOString(),
    })
    expect(
      workingLine(
        [oak],
        [comment, reply],
        roster({ "mac/oak": ["busy", 1] }),
        at(6)
      )
    ).toBeNull()
  })

  test("an answer on a blocker, an event here, ends it", () => {
    const answer = CardThreadItem({
      id: "e2",
      kind: "event",
      who: oak,
      text: "replied on a blocker",
      blockerId: "b1",
      at: at(5).toISOString(),
    })
    expect(
      workingLine(
        [oak],
        [comment, answer],
        roster({ "mac/oak": ["busy", 1] }),
        at(6)
      )
    ).toBeNull()
  })

  test("a message from the person doesn't replace their latest comment", () => {
    const message = CardThreadItem({
      id: "m1",
      kind: "message",
      who: you,
      text: "also this",
      at: at(5).toISOString(),
    })
    expect(
      workingLine(
        [oak],
        [comment, message],
        roster({ "mac/oak": ["busy", 1] }),
        at(6)
      )?.text
    ).toBe("oak is replying…")
  })

  test("another agent's reply doesn't end it", () => {
    const reply = CardThreadItem({
      id: "r1",
      kind: "comment",
      who: elm,
      text: "not me",
      at: at(5).toISOString(),
    })
    expect(
      workingLine(
        [oak, elm],
        [comment, reply],
        roster({ "mac/oak": ["busy", 1], "mac/elm": ["busy", 1] }),
        at(6)
      )?.text
    ).toBe("oak is replying…")
  })

  test("the 30-minute cap, from the comment", () => {
    const busy = roster({ "mac/oak": ["busy", 1] })
    expect(workingLine([oak], [comment], busy, at(30 * 60))?.tone).toBe(
      "replying"
    )
    expect(workingLine([oak], [comment], busy, at(30 * 60 + 1))).toBeNull()
  })

  test("a background job between turns isn't replying, and isn't stopped", () => {
    expect(
      workingLine([oak], [comment], roster({ "mac/oak": ["shell", 5] }), at(60))
    ).toBeNull()
  })

  test("two busy agents are named together", () => {
    const l = workingLine(
      [oak, elm],
      [{ ...comment, heardBy: ["mac/oak", "mac/elm"] }],
      roster({ "mac/oak": ["busy", 1], "mac/elm": ["busy", 1] }),
      at(2)
    )
    expect(l?.text).toBe("oak and elm are replying…")
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
    ).toBe("replying")
  })
})

describe("Start an agent with this (card #452)", () => {
  const it_ = (
    id: string,
    kind: string,
    status: string,
    blockerId?: string
  ) => ({
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
