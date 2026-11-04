import { describe, expect, test } from "bun:test"

import { TaskState, withLive, workingNow, type AgentRef, type CardSummary } from "./cards"
import { IDLE_LINE, needsOf, needsYou, prChip, rowState, toPerson, waitsOnTrust } from "./needs"
import { State, type Delivery } from "./talk"

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
    where: "This PC",
    idleSince: "2026-11-02T10:05:00Z",
    ...w,
  })
  const held = (c: Partial<CardSummary> = {}, w: Record<string, unknown> = {}) =>
    card({
      working: [holder(w)] as CardSummary["working"],
      holder: { state: "idle", who: holder(w), others: 0 } as CardSummary["holder"],
      ...c,
    })

  test("calm Waiting at first, Idle after ten minutes", () => {
    expect(rowState(held(), at("2026-11-02T10:10:00Z")).state).toBe("waiting")
    expect(rowState(held(), at("2026-11-02T10:15:00Z"))).toEqual({
      state: "idle",
      line: `${IDLE_LINE} (orca)`,
    })
  })
  test("counts from the later of the card's change and the holder's idle", () => {
    const c = held({ updatedAt: "2026-11-02T10:12:00Z" })
    expect(rowState(c, at("2026-11-02T10:16:00Z")).state).toBe("waiting")
    expect(rowState(c, at("2026-11-02T10:22:00Z")).state).toBe("idle")
  })
  test("a named wait, a busy, gone or offline holder is not idle-unnamed", () => {
    const late = at("2026-11-02T11:00:00Z")
    expect(rowState(held({ waitingOn: 1, waitOn: "kestrel" }), late)).toEqual({
      state: "waiting",
      line: "Waiting on kestrel",
    })
    expect(rowState(held({ openBlockers: 1, blockerQuestion: "Which?" }), late).state).toBe("needs")
    expect(rowState(held({}, { status: "busy" }), late).state).toBe("working")
    expect(rowState(held({}, { status: "gone", live: false }), late).state).toBe("waiting")
    expect(rowState(held({}, { offline: true }), late).state).toBe("waiting")
    expect(rowState(held({ state: TaskState.InReview }), late).state).toBe("waiting")
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
    where: "This PC",
    idleSince: "2026-11-02T10:05:00Z",
    ...w,
  })
  const held = (c: Partial<CardSummary> = {}, w: Record<string, unknown> = {}) =>
    card({
      working: [holder(w)] as CardSummary["working"],
      holder: { state: "idle", who: holder(w), others: 0 } as CardSummary["holder"],
      ...c,
    })
  const late = Date.parse("2026-11-02T11:00:00Z")

  test("shell: running a background job, even long after the turn ended", () => {
    expect(rowState(held(), late)).toEqual({
      state: "working",
      line: "wonton running a background job",
    })
  })
  test("idle: still not working on it now", () => {
    expect(rowState(held({}, { status: "idle" }), Date.parse("2026-11-02T10:06:00Z"))).toEqual({
      state: "waiting",
      line: "wonton, not working on it now",
    })
  })
  test("a card waiting on the person, or in review, says that instead", () => {
    expect(rowState(held({ openBlockers: 1, blockerQuestion: "Which?" }), late).state).toBe("needs")
    expect(rowState(held({ state: TaskState.InReview }), late)).toEqual({
      state: "waiting",
      line: "In review",
    })
  })
  test("only taken-up, live, online holders count", () => {
    expect(rowState(held({}, { offline: true }), late).line).not.toContain("background")
    expect(rowState(held({}, { live: false, status: "gone" }), late).line).not.toContain("background")
  })
})

describe("rowState", () => {
  test("the four states", () => {
    expect(rowState(card({ openBlockers: 1, blockerQuestion: "Steam or relay?" }))).toEqual({
      state: "needs",
      line: "Steam or relay?",
    })
    expect(rowState(card({ ready: true, openBlockers: 1 })).state).toBe("needs")
    expect(rowState(card({ state: TaskState.Completed })).state).toBe("done")
    expect(rowState(card({ state: TaskState.InReview })).line).toBe("In review")
    expect(rowState(card({})).state).toBe("waiting")
  })
  test("Agent progress and the card page agree, by the live roster (card #317)", () => {
    const oak = { key: "d/oak", name: "oak", status: "idle", live: true, where: "This PC" } as AgentRef
    const c = card({
      working: [oak],
      holder: { state: "one", who: { key: "d/oak", name: "oak" }, others: 0 } as CardSummary["holder"],
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
    expect(needsYou(card({ state: TaskState.Completed, openBlockers: 1 }))).toBe(false)
  })
})

describe("needsOf", () => {
  test("cards and held messages, newest first, each once; an agent's message is a DM (card #390)", () => {
    const cs = [
      card({ id: "a", openBlockers: 1, updatedAt: "2026-11-02T10:00:00Z" }),
      card({ id: "b", ready: true, openBlockers: 1, updatedAt: "2026-11-02T10:05:00Z" }),
      card({ id: "c" }),
    ]
    const held = delivery({ id: "h", to: "dev/agent2", state: State.Held, createdAt: "2026-11-02T10:02:00Z" })
    const got = needsOf(cs, [held, held]).map((n) => `${n.kind}:${n.id}`)
    expect(got).toEqual(["check:b", "held:h", "decision:a"])
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
  const pr = { repo: "StephenSHorton/suzuri", number: 96, url: "https://github.com/StephenSHorton/suzuri/pull/96" }
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
    expect(rowState(c)).toEqual({ state: "needs", line: "Waits on your answer on #290" })
    // A chain that ends at an agent or a PR stays Waiting on #N.
    expect(rowState({ ...c, waitOn: "#334", waitYou: undefined })).toEqual({
      state: "waiting",
      line: "Waiting on #334",
    })
  })
  test("Needs you has no PR rows, only the decision", () => {
    const cs = [
      card({ id: "330", state: TaskState.InReview, waitPr: pr, updatedAt: "2026-11-03T17:12:00Z" }),
      card({ id: "334", state: TaskState.InReview, waitPr: pr, updatedAt: "2026-11-03T18:36:00Z" }),
      card({ id: "290", state: TaskState.Blocked, openBlockers: 1, updatedAt: "2026-11-03T15:14:00Z" }),
      card({ id: "329", state: TaskState.Blocked, waitingOn: 1, waitOn: "#334" }),
    ]
    expect(needsOf(cs, []).map((n) => `${n.kind}:${n.id}`)).toEqual(["decision:290"])
  })
  test("the PR chip names the PR, links it and says who it waits on", () => {
    expect(prChip(card({ state: TaskState.InReview, waitPr: pr }))).toEqual({
      text: "waits on PR #96",
      title: "Waits on PR #96 in StephenSHorton/suzuri: an agent merges it once its checks pass. Opens https://github.com/StephenSHorton/suzuri/pull/96",
      url: pr.url,
    })
    expect(prChip(card({ state: TaskState.InReview, waitPr: { ...pr, approval: true } }))?.title).toContain(
      "another member's agents approve it"
    )
    expect(prChip(card({ state: TaskState.InReview }))).toBeNull()
    expect(prChip(card({ state: TaskState.Completed, waitPr: pr }))).toBeNull()
  })
})

describe("trust (card #449)", () => {
  test("a card whose agent can't start for trust is a Trust row", () => {
    const t = card({ id: "t", cantStart: "not trusted: /r/api on dev1", state: TaskState.Submitted })
    const other = card({ id: "o", cantStart: "no checkout of remote:x on dev1" })
    expect(waitsOnTrust(t)).toBe(true)
    expect(waitsOnTrust(other)).toBe(false)
    expect(needsOf([t, other], []).map((n) => `${n.kind}:${n.id}`)).toEqual(["trust:t"])
  })
  test("a card already waiting on the person stays one row", () => {
    const t = card({ id: "t", cantStart: "not trusted: /r on d", openBlockers: 1 })
    expect(needsOf([t], []).map((n) => n.kind)).toEqual(["decision"])
  })
  test("a closed card has no trust row", () => {
    const t = card({ id: "t", cantStart: "not trusted: /r on d", state: TaskState.Completed })
    expect(needsOf([t], [])).toEqual([])
  })
})
