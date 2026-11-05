import { describe, expect, test } from "bun:test"

import {
  TaskState,
  withLive,
  workingNow,
  type AgentRef,
  type CardSummary,
} from "./cards"
import { readFileSync } from "node:fs"

import {
  asksPerson,
  CRASHED_QUESTION,
  groupRows,
  HELD_QUIET,
  IDLE_LINE,
  loopLine,
  nobodyTook,
  needsOf,
  needsYou,
  prChip,
  rowState,
  spawnNeed,
  stuckLine,
  toPerson,
  WAITING_PERMISSION,
  waitsOnTrust,
} from "./needs"
import { HoldReason, State, type Delivery } from "./talk"

const card = (c: Partial<CardSummary>): CardSummary =>
  ({
    id: "c",
    short: "1",
    title: "A card",
    state: TaskState.Working,
    scope: "mine",
    scopeLabel: "My devices",
    openBlockers: 0,
    blockerQuestion: "",
    waiting: 0,
    working: [],
    watching: [],
    talking: [],
    devices: [],
    updatedAt: "2026-11-02T10:00:00Z",
    progress: { done: 0, total: 0 },
    holder: { state: "nobody", who: { key: "", name: "" }, others: 0 },
    ...c,
  }) as CardSummary

const delivery = (d: Partial<Delivery> & { text?: string }): Delivery =>
  ({
    id: "d",
    messageId: "m",
    to: "user:dev",
    state: State.Queued,
    reminded: false,
    createdAt: "2026-11-02T10:00:00Z",
    updatedAt: "2026-11-02T10:00:00Z",
    message: {
      id: "m",
      from: { key: "dev/agent", name: "otter", kind: "claude" },
      to: { kind: "user", value: "user:dev" },
      text: d.text ?? "Hello",
      hop: 0,
      trust: "agent",
      sentAt: "2026-11-02T10:00:00Z",
    },
    ...d,
  }) as Delivery

// Card #305: a held card whose holder sits idle with nothing named.
describe("idle with no reason given", () => {
  const at = (iso: string) => Date.parse(iso)
  const holder = (w: Record<string, unknown> = {}) => ({
    key: "dev/s1",
    name: "orca",
    status: "idle",
    live: true,
    where: "Macbook Pro (this one)",
    here: true,
    idleSince: "2026-11-02T10:05:00Z",
    ...w,
  })
  const held = (
    c: Partial<CardSummary> = {},
    w: Record<string, unknown> = {}
  ) =>
    card({
      working: [holder(w)] as CardSummary["working"],
      holder: {
        state: "idle",
        who: holder(w),
        others: 0,
      } as CardSummary["holder"],
      ...c,
    })

  test("Working between turns at first, Idle after ten minutes", () => {
    expect(rowState(held(), at("2026-11-02T10:10:00Z"))).toEqual({
      state: "working",
      line: "orca, between turns",
    })
    expect(rowState(held(), at("2026-11-02T10:15:00Z"))).toEqual({
      state: "idle",
      line: `${IDLE_LINE} (orca)`,
    })
  })
  test("counts from the later of the card's change and the holder's idle", () => {
    const c = held({ updatedAt: "2026-11-02T10:12:00Z" })
    expect(rowState(c, at("2026-11-02T10:16:00Z")).state).toBe("working")
    expect(rowState(c, at("2026-11-02T10:22:00Z")).state).toBe("idle")
  })
  test("a named wait, a busy, gone or offline holder is not idle-unnamed", () => {
    const late = at("2026-11-02T11:00:00Z")
    expect(rowState(held({ waitingOn: 1, waitOn: "kestrel" }), late)).toEqual({
      state: "waiting",
      line: "Waiting on kestrel",
    })
    expect(
      rowState(held({ openBlockers: 1, blockerQuestion: "Which?" }), late).state
    ).toBe("needs")
    expect(rowState(held({}, { status: "busy" }), late).state).toBe("working")
    // Card #487: a holder that went away or is offline leaves it Stuck.
    expect(rowState(held({}, { status: "gone", live: false }), late)).toEqual({
      state: "stuck",
      line: "orca went away",
    })
    expect(rowState(held({}, { offline: true }), late)).toEqual({
      state: "stuck",
      line: "orca is on an offline device",
    })
    expect(rowState(held({ state: TaskState.InReview }), late).state).toBe(
      "waiting"
    )
  })
})

// Card #431: a holder between turns with a background job running is
// Claude Code's "shell": it reads as running a job, not "not working".
describe("a holder running a background job", () => {
  const holder = (w: Record<string, unknown> = {}) => ({
    key: "dev/s1",
    name: "wonton",
    status: "shell",
    live: true,
    where: "Macbook Pro (this one)",
    here: true,
    idleSince: "2026-11-02T10:05:00Z",
    ...w,
  })
  const held = (
    c: Partial<CardSummary> = {},
    w: Record<string, unknown> = {}
  ) =>
    card({
      working: [holder(w)] as CardSummary["working"],
      holder: {
        state: "idle",
        who: holder(w),
        others: 0,
      } as CardSummary["holder"],
      ...c,
    })
  const late = Date.parse("2026-11-02T11:00:00Z")

  test("shell: running a job, even long after the turn ended", () => {
    expect(rowState(held(), late)).toEqual({
      state: "working",
      line: "wonton running a job",
      spin: true,
    })
  })
  test("idle: between turns, still on it", () => {
    expect(
      rowState(held({}, { status: "idle" }), Date.parse("2026-11-02T10:06:00Z"))
    ).toEqual({
      state: "working",
      line: "wonton, between turns",
    })
  })
  test("a card waiting on the person, or in review, says that instead", () => {
    expect(
      rowState(held({ openBlockers: 1, blockerQuestion: "Which?" }), late).state
    ).toBe("needs")
    expect(rowState(held({ state: TaskState.InReview }), late)).toEqual({
      state: "waiting",
      line: "In review",
    })
  })
  test("only taken-up, live, online holders count", () => {
    expect(rowState(held({}, { offline: true }), late).line).not.toContain(
      "background"
    )
    expect(
      rowState(held({}, { live: false, status: "gone" }), late).line
    ).not.toContain("background")
  })
})

describe("rowState", () => {
  test("the four states", () => {
    expect(
      rowState(card({ openBlockers: 1, blockerQuestion: "Steam or relay?" }))
    ).toEqual({
      state: "needs",
      line: "Steam or relay?",
    })
    expect(rowState(card({ ready: true, openBlockers: 1 })).state).toBe("needs")
    expect(rowState(card({ state: TaskState.Completed })).state).toBe("done")
    expect(rowState(card({ state: TaskState.InReview })).line).toBe("In review")
    expect(rowState(card({})).state).toBe("stuck")
  })
  test("Agent progress and the card page agree, by the live roster (card #317)", () => {
    const oak = {
      key: "d/oak",
      name: "oak",
      status: "idle",
      live: true,
      where: "Macbook Pro (this one)",
      here: true,
    } as AgentRef
    const c = card({
      working: [oak],
      holder: {
        state: "one",
        who: { key: "d/oak", name: "oak" },
        others: 0,
      } as CardSummary["holder"],
    })
    const live = new Map([["d/oak", { status: "busy" }]])
    for (const [m, working] of [
      [new Map(), false],
      [live, true],
    ] as const) {
      const now = withLive(c, m)
      expect(rowState(now).state === "working").toBe(working)
      expect(workingNow(now).length > 0).toBe(working)
    }
    // A blocker: Needs you on Home, and no working row on the page.
    const blocked = withLive({ ...c, openBlockers: 1 }, live)
    expect(rowState(blocked).state).toBe("needs")
    expect(workingNow(blocked)).toEqual([])
  })
  test("a closed card never needs you", () => {
    expect(
      needsYou(card({ state: TaskState.Completed, openBlockers: 1 }))
    ).toBe(false)
  })
})

describe("needsOf", () => {
  test("cards and held messages, newest first, each once; an agent's message is a DM (card #390)", () => {
    const cs = [
      card({ id: "a", openBlockers: 1, updatedAt: "2026-11-02T10:00:00Z" }),
      card({
        id: "b",
        ready: true,
        openBlockers: 1,
        updatedAt: "2026-11-02T10:05:00Z",
      }),
      card({ id: "c" }),
    ]
    const held = delivery({
      id: "h",
      to: "",
      repoKey: "root:5a3c",
      state: State.Held,
      hold: HoldReason.HoldUnowned,
      createdAt: "2026-11-02T10:02:00Z",
    })
    held.message.from = { key: "user:dev", name: "you", kind: "user" }
    const got = needsOf(cs, [held, held]).map((n) => `${n.kind}:${n.id}`)
    expect(got).toEqual(["check:b", "held:h", "decision:a"])
  })
  test("holds between agents and Pause all's are one line, last and uncounted (card #488)", () => {
    const cs = [
      card({ id: "a", openBlockers: 1, updatedAt: "2026-11-02T10:00:00Z" }),
    ]
    const hop = delivery({
      id: "hop",
      to: "dev/heron",
      state: State.Held,
      hold: HoldReason.HoldHopLimit,
      createdAt: "2026-11-02T10:09:00Z",
    })
    const old = delivery({
      id: "old",
      to: "dev/heron",
      state: State.Held,
      hold: HoldReason.HoldBypassReceiver,
    })
    const listed = delivery({
      id: "allow",
      to: "dev/heron",
      state: State.Held,
      hold: HoldReason.HoldNotAllowed,
    })
    const paused = delivery({
      id: "p",
      to: "dev/heron",
      state: State.Held,
      hold: HoldReason.HoldPaused,
    })
    // An agent's message to a repo nobody is in is between agents too.
    const agentRepo = delivery({
      id: "ar",
      to: "",
      repoKey: "root:5a3c",
      state: State.Held,
      hold: HoldReason.HoldUnowned,
    })
    const mine = delivery({
      id: "mine",
      to: "",
      repoKey: "root:5a3c",
      state: State.Held,
      hold: HoldReason.HoldUnowned,
    })
    mine.message.from = { key: "user:dev", name: "you", kind: "user" }
    // Your own message held by your Pause all is quiet too.
    const minePaused = delivery({
      id: "mp",
      to: "dev/heron",
      state: State.Held,
      hold: HoldReason.HoldPaused,
    })
    minePaused.message.from = { key: "user:dev", name: "you", kind: "user" }
    const person = delivery({
      id: "pal",
      to: "dev/heron",
      state: State.Held,
      hold: HoldReason.HoldCrossPerson,
    })
    person.message.from = { key: "board:b1/p2", name: "Sam", kind: "board" }
    const got = needsOf(cs, [
      hop,
      old,
      listed,
      paused,
      agentRepo,
      mine,
      minePaused,
      person,
    ])
    expect(got.map((n) => `${n.kind}:${n.id}`)).toEqual([
      "decision:a",
      "held:mine",
      "held:pal",
      `heldQuiet:${HELD_QUIET}`,
    ])
    const quiet = got.at(-1)
    expect(
      quiet?.kind === "heldQuiet" && quiet.deliveries.map((d) => d.id)
    ).toEqual(["hop", "old", "allow", "p", "ar", "mp"])
    expect(asksPerson(hop)).toBe(false)
    // An agent's message to you, held by your allow list, is yours to decide.
    const toYou = delivery({
      id: "ty",
      to: "user:dev",
      state: State.Held,
      hold: HoldReason.HoldNotAllowed,
    })
    expect(asksPerson(toYou)).toBe(true)
    expect(asksPerson({ ...toYou, hold: HoldReason.HoldPaused })).toBe(false)
    expect(asksPerson(mine)).toBe(true)
    expect(asksPerson({ ...mine, state: State.Queued })).toBe(false)
    expect(needsOf([], [hop]).map((n) => n.kind)).toEqual(["heldQuiet"])
  })
  test("toPerson: open messages to a user key only", () => {
    expect(toPerson(delivery({}))).toBe(true)
    expect(toPerson(delivery({ state: State.Held }))).toBe(false)
    expect(toPerson(delivery({ to: "dev/agent" }))).toBe(false)
  })
})

// Card #347: "Waiting" that is really the person's says so and sits under
// Needs you. Card #471: a PR never does; nobody but agents merges it, and
// its card shows a small chip instead. Today's board: #330 and #334 in
// review on PR #96, #329 waiting on #334, #290's question on an external
// wait.
describe("what waits on the person (cards #347, #471)", () => {
  const pr = {
    repo: "StephenSHorton/suzuri",
    number: 96,
    url: "https://github.com/StephenSHorton/suzuri/pull/96",
  }
  test("a card in review on its PR waits, it doesn't need the person", () => {
    expect(rowState(card({ state: TaskState.InReview, waitPr: pr }))).toEqual({
      state: "waiting",
      line: "In review",
    })
  })
  test("a wait whose chain ends at the person's answer names it", () => {
    const c = card({
      state: TaskState.Blocked,
      waitingOn: 1,
      waitOn: "#290",
      waitYou: "your answer on #290",
    })
    expect(rowState(c)).toEqual({
      state: "needs",
      line: "Waits on your answer on #290",
    })
    // A chain that ends at an agent or a PR stays Waiting on #N.
    expect(rowState({ ...c, waitOn: "#334", waitYou: undefined })).toEqual({
      state: "waiting",
      line: "Waiting on #334",
    })
  })
  test("Needs you has no PR rows, only the decision", () => {
    const cs = [
      card({
        id: "330",
        state: TaskState.InReview,
        waitPr: pr,
        updatedAt: "2026-11-03T17:12:00Z",
      }),
      card({
        id: "334",
        state: TaskState.InReview,
        waitPr: pr,
        updatedAt: "2026-11-03T18:36:00Z",
      }),
      card({
        id: "290",
        state: TaskState.Blocked,
        openBlockers: 1,
        updatedAt: "2026-11-03T15:14:00Z",
      }),
      card({
        id: "329",
        state: TaskState.Blocked,
        waitingOn: 1,
        waitOn: "#334",
      }),
    ]
    expect(needsOf(cs, []).map((n) => `${n.kind}:${n.id}`)).toEqual([
      "decision:290",
    ])
  })
  test("the PR chip names the PR, links it and says who it waits on", () => {
    expect(prChip(card({ state: TaskState.InReview, waitPr: pr }))).toEqual({
      text: "waits on PR #96",
      title:
        "Waits on PR #96 in StephenSHorton/suzuri: an agent merges it once its checks pass. Opens https://github.com/StephenSHorton/suzuri/pull/96",
      url: pr.url,
    })
    expect(
      prChip(
        card({ state: TaskState.InReview, waitPr: { ...pr, approval: true } })
      )?.title
    ).toContain("another member's agents approve it")
    expect(prChip(card({ state: TaskState.InReview }))).toBeNull()
    expect(prChip(card({ state: TaskState.Completed, waitPr: pr }))).toBeNull()
  })
})

describe("an agent's wait on a PR's merge is the person's (card #509)", () => {
  const pr = {
    repo: "StephenSHorton/irori",
    number: 41,
    url: "https://github.com/StephenSHorton/irori/pull/41",
  }
  const merge = card({
    id: "454",
    state: TaskState.Blocked,
    waitingOn: 1,
    waitKind: "external",
    waitOn: pr.url,
    mergePr: pr,
    updatedAt: "2026-11-05T06:23:00Z",
  })
  test("its row needs the person and names the PR", () => {
    expect(rowState(merge)).toEqual({ state: "needs", line: "Merge PR #41" })
  })
  test("it is a merge row in Needs you, beside the decisions", () => {
    const cs = [
      merge,
      card({
        id: "290",
        state: TaskState.Blocked,
        openBlockers: 1,
        updatedAt: "2026-11-05T05:00:00Z",
      }),
      // A wait on CI, or on a PR in review, stays off it.
      card({
        id: "500",
        state: TaskState.Blocked,
        waitingOn: 1,
        waitKind: "external",
        waitOn: "CI on the release",
      }),
      card({ id: "330", state: TaskState.InReview, waitPr: pr }),
    ]
    const ns = needsOf(cs, [])
    expect(ns.map((n) => `${n.kind}:${n.id}`)).toEqual([
      "merge:454",
      "decision:290",
    ])
    expect(ns[0].kind === "merge" && ns[0].pr).toEqual(pr)
  })
  test("a card with a decision too is one decision row; a finished one none", () => {
    expect(
      needsOf([{ ...merge, openBlockers: 1 }], []).map((n) => n.kind)
    ).toEqual(["decision"])
    expect(
      needsOf([{ ...merge, state: TaskState.Completed }], []).map((n) => n.kind)
    ).toEqual([])
  })
})

describe("waits nothing will end (card #481)", () => {
  const ring = [
    { id: "t392", short: "392", title: "Talk 4" },
    { id: "t396", short: "396", title: "Talk 8" },
    { id: "t394", short: "394", title: "Talk 6" },
  ]
  test("three cards that wait on each other are one Needs you item", () => {
    const cs = ring.map((r, i) =>
      card({
        id: r.id,
        short: r.short,
        state: TaskState.Blocked,
        waitingOn: 1,
        waitOn: `#${ring[(i + 1) % 3].short}`,
        waitLoop: ring,
        updatedAt: `2026-11-03T1${i}:00:00Z`,
      })
    )
    const got = needsOf(cs, [])
    expect(got.map((n) => `${n.kind}:${n.id}`)).toEqual(["loop:t392,t396,t394"])
    const n = got[0]
    expect(n.kind === "loop" && n.cards.map((c) => c.id)).toEqual([
      "t392",
      "t396",
      "t394",
    ])
    expect(n.at).toBe(Date.parse("2026-11-03T12:00:00Z"))
    expect(loopLine(ring)).toBe("#392, #396 and #394 wait on each other")
    expect(rowState(cs[0])).toEqual({
      state: "needs",
      line: "#392, #396 and #394 wait on each other",
    })
  })
  test("a wait on a card is no decision", () => {
    const c = card({
      id: "t394",
      state: TaskState.Blocked,
      openBlockers: 0,
      waitingOn: 1,
      waitOn: "#392",
    })
    expect(needsOf([c], [])).toEqual([])
  })
  test("cards waiting on one nobody is on are one item", () => {
    const end = { id: "t34", short: "34", title: "Vendor" }
    const cs = ["12", "13"].map((s) =>
      card({
        id: `t${s}`,
        short: s,
        state: TaskState.Blocked,
        waitingOn: 1,
        waitOn: "#34",
        waitStuck: end,
      })
    )
    const got = needsOf(cs, [])
    expect(got.map((n) => `${n.kind}:${n.id}`)).toEqual(["stuck:t34"])
    const n = got[0]
    expect(n.kind === "stuck" && stuckLine(n.end, n.cards)).toBe(
      "#12 and #13 wait on #34, which nobody is on"
    )
    expect(rowState(cs[0]).line).toBe("Waits on #34, which nobody is on")
  })
  test("a done card leaves the loop item", () => {
    const c = card({ id: "t392", state: TaskState.Completed, waitLoop: ring })
    expect(needsOf([c], [])).toEqual([])
  })
})

describe("trust (card #449)", () => {
  test("a card whose agent can't start for trust is a Trust row", () => {
    const t = card({
      id: "t",
      cantStart: "not trusted: /r/api on dev1",
      state: TaskState.Submitted,
    })
    const other = card({
      id: "o",
      cantStart: "no copy of x on Desktop Computer",
    })
    expect(waitsOnTrust(t)).toBe(true)
    expect(waitsOnTrust(other)).toBe(false)
    expect(needsOf([t, other], []).map((n) => `${n.kind}:${n.id}`)).toEqual([
      "trust:t",
    ])
  })
  test("a card already waiting on the person stays one row", () => {
    const t = card({
      id: "t",
      cantStart: "not trusted: /r on d",
      openBlockers: 1,
    })
    expect(needsOf([t], []).map((n) => n.kind)).toEqual(["decision"])
  })
  test("a closed card has no trust row", () => {
    const t = card({
      id: "t",
      cantStart: "not trusted: /r on d",
      state: TaskState.Completed,
    })
    expect(needsOf([t], [])).toEqual([])
  })
})

describe("spawned agents' own decisions (card #456, sa-7)", () => {
  // The words come from the Rust sweep; read them there so a change on
  // either side fails here.
  const rust = (f: string) =>
    readFileSync(new URL(`../../../crates/${f}`, import.meta.url), "utf8")
  test("the constants match the Rust ones", () => {
    expect(rust("core/src/service/spawn_waiting.rs")).toContain(
      `pub const WAITING_PERMISSION: &str = "${WAITING_PERMISSION}";`
    )
    expect(rust("coord/src/board/spawn.rs")).toContain(
      `pub const CRASHED_QUESTION: &str = "${CRASHED_QUESTION}";`
    )
  })
  test("a permission wait, a crash, anything else", () => {
    const q = (blockerQuestion: string) =>
      card({ openBlockers: 1, blockerQuestion })
    expect(
      spawnNeed(q("Waiting for your permission: thyme is stopped on a prompt"))
    ).toBe("permission")
    expect(spawnNeed(q(CRASHED_QUESTION))).toBe("crashed")
    expect(spawnNeed(q("Ship today?"))).toBeNull()
  })
})

// Card #487: Home groups its rows by what the card really waits on, and
// the idle line counts the ones nobody took.
describe("Home's groups (card #487)", () => {
  const heron = {
    key: "d/heron",
    name: "heron",
    status: "idle",
    live: true,
    where: "This PC",
  }
  const ask = (a: Record<string, unknown>) =>
    card({
      state: TaskState.Submitted,
      ask: { state: "open", told: 3, ...a } as CardSummary["ask"],
    })
  const offered = (waiting: boolean) =>
    card({
      state: TaskState.Submitted,
      working: [
        { ...heron, assigned: true, waiting },
      ] as CardSummary["working"],
      holder: {
        state: waiting ? "waiting" : "assigned",
        who: heron,
        others: 0,
      } as CardSummary["holder"],
    })

  test("nobody took it: Stuck, and the idle line counts it", () => {
    for (const [c, line] of [
      [card({ state: TaskState.Submitted }), "Nobody has it yet"],
      [offered(true), "Not picked up"],
      [offered(false), "Offered, not taken up yet"],
      [ask({}), "Told 3 agents; nobody took it yet"],
      [ask({ state: "unclaimed" }), "Nobody took this yet"],
    ] as const) {
      expect(rowState(c)).toEqual({ state: "stuck", line })
      expect(nobodyTook(c)).toBe(true)
    }
  })
  test("someone on it, or starting: Working, not counted", () => {
    const taken = ask({ state: "taken", taker: heron })
    expect(rowState(taken)).toEqual({ state: "working", line: "heron took it" })
    expect(nobodyTook(taken)).toBe(false)
    const starting = card({ state: TaskState.Submitted, starting: "This PC" })
    expect(rowState(starting)).toEqual({
      state: "working",
      line: "Starting an agent on This PC",
    })
    expect(nobodyTook(starting)).toBe(false)
  })
  test("a wait with a name: Waiting; a wait on a gone agent: Stuck", () => {
    expect(rowState(card({ waiting: 1 }))).toEqual({
      state: "waiting",
      line: "Your answer is with the agent",
    })
    const lease = card({
      state: TaskState.Blocked,
      waitingOn: 1,
      waitKind: "lease",
      waitOn:
        "ww/dolphin-slot-1 ww/dolphin-slot-2 ww/dolphin-slot-3 ww/dolphin-slot-4",
    })
    expect(rowState(lease)).toEqual({
      state: "waiting",
      line: "Waiting for a free Dolphin slot",
    })
    const gone = card({
      state: TaskState.Blocked,
      waitingOn: 1,
      waitKind: "agent",
      waitOn: "kestrel",
      waitGone: true,
    })
    expect(rowState(gone)).toEqual({
      state: "stuck",
      line: "Waiting on kestrel, who is gone",
    })
    expect(nobodyTook(gone)).toBe(false)
  })
  test("a holder that went away is Stuck but someone took it", () => {
    const c = card({
      returned: { ...heron, status: "gone", live: false } as AgentRef,
    })
    expect(rowState(c)).toEqual({
      state: "stuck",
      line: "Nobody on it: heron went away",
    })
    expect(nobodyTook(c)).toBe(false)
  })
  test("one holder still there carries it; a gone co-holder doesn't make it Stuck", () => {
    const gone = {
      ...heron,
      key: "d/a",
      name: "a",
      status: "gone",
      live: false,
    }
    const c = card({
      working: [gone, heron] as CardSummary["working"],
      holder: { state: "gone", who: gone, others: 1 } as CardSummary["holder"],
    })
    expect(rowState(c, Date.parse("2026-11-02T10:01:00Z"))).toEqual({
      state: "working",
      line: "heron, between turns",
    })
  })
  test("a step owner carries it: not nobody", () => {
    const c = card({
      working: [{ ...heron, steps: ["write tests"] }] as CardSummary["working"],
      holder: {
        state: "nobody",
        who: { key: "", name: "" },
        others: 0,
      } as CardSummary["holder"],
    })
    expect(rowState(c, Date.parse("2026-11-02T10:01:00Z")).state).toBe(
      "working"
    )
    expect(nobodyTook(c)).toBe(false)
  })
  test("a parent nobody holds waits on its subtasks, not counted", () => {
    const c = card({ subtasks: 2 })
    expect(rowState(c)).toEqual({
      state: "waiting",
      line: "Waiting on its subtasks",
    })
    expect(nobodyTook(c)).toBe(false)
  })
  test("an ask whose taker went away is Stuck, but someone took it", () => {
    const oak = { ...heron, key: "d/oak", name: "oak", offline: true }
    const c = card({
      ask: { state: "taken", told: 2, taker: oak } as CardSummary["ask"],
      working: [oak] as CardSummary["working"],
      holder: {
        state: "offline",
        who: oak,
        others: 0,
      } as CardSummary["holder"],
    })
    expect(rowState(c)).toEqual({
      state: "stuck",
      line: "oak is on an offline device",
    })
    expect(nobodyTook(c)).toBe(false)
  })
  test("groups come in order, each in the order given, empty ones left out", () => {
    const rows = [
      { id: "a", s: "waiting" },
      { id: "b", s: "stuck" },
      { id: "c", s: "working" },
      { id: "d", s: "stuck" },
      { id: "e", s: "needs" },
    ] as const
    expect(
      groupRows(rows, (r) => r.s).map((g) => [g.state, g.rows.map((r) => r.id)])
    ).toEqual([
      ["needs", ["e"]],
      ["stuck", ["b", "d"]],
      ["working", ["c"]],
      ["waiting", ["a"]],
    ])
  })
})
