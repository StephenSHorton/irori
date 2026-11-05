import { describe, expect, test } from "bun:test"

import { AgentRef, CardAsk, CardWait, Step } from "./models"
import { TaskState } from "./models"
import {
  refStatus,
  askLine,
  askStarting,
  busy,
  namesLine,
  withLive,
  workingNow,
  sampleCardRef,
  cardFromHash,
  cardHash,
  openTarget,
  deviceChips,
  deviceVar,
  holderLineShows,
  Holding,
  nobodyOn,
  openAsk,
  overLabel,
  peopleRows,
  workLabel,
} from "./cards"

const agent = (status: string, more: Partial<AgentRef> = {}) =>
  AgentRef({
    key: "d/s",
    name: "a",
    status,
    live: status !== "gone",
    where: "Macbook Pro (this one)",
    here: true,
    ...more,
  })

const card = (state: TaskState, working: AgentRef[], openBlockers = 0) => ({
  state,
  openBlockers,
  working,
})

describe("busy", () => {
  test("a busy holder on a working card spins", () => {
    expect(busy(card(TaskState.Working, [agent("busy")]))).toBe(true)
  })
  test("idle, gone, offline or only assigned holders don't", () => {
    expect(busy(card(TaskState.Working, [agent("idle")]))).toBe(false)
    expect(busy(card(TaskState.Working, [agent("gone")]))).toBe(false)
    expect(
      busy(card(TaskState.Working, [agent("busy", { offline: true })]))
    ).toBe(false)
    expect(
      busy(card(TaskState.Working, [agent("busy", { assigned: true })]))
    ).toBe(false)
    expect(busy(card(TaskState.Working, []))).toBe(false)
  })
  test("waiting on the person or done doesn't", () => {
    expect(busy(card(TaskState.InputRequired, [agent("busy")]))).toBe(false)
    expect(busy(card(TaskState.Blocked, [agent("busy")], 1))).toBe(false)
    expect(busy(card(TaskState.Completed, [agent("busy")]))).toBe(false)
    expect(busy(card(TaskState.Canceled, [agent("busy")]))).toBe(false)
  })
  test("in review doesn't: its holder is free for other work (card #191)", () => {
    expect(busy(card(TaskState.InReview, [agent("busy")]))).toBe(false)
  })
  test("one busy holder among idle ones is enough", () => {
    expect(busy(card(TaskState.Working, [agent("idle"), agent("busy")]))).toBe(
      true
    )
  })
})

describe("withLive and workingNow (card #317)", () => {
  const live = (m: Record<string, string>) =>
    new Map(Object.entries(m).map(([k, status]) => [k, { status }]))
  test("the live roster's status wins over the card's snapshot", () => {
    const c = card(TaskState.Working, [agent("idle")])
    expect(busy(c)).toBe(false)
    expect(busy(withLive(c, live({ "d/s": "busy" })))).toBe(true)
    const was = card(TaskState.Working, [agent("busy")])
    expect(busy(withLive(was, live({ "d/s": "idle" })))).toBe(false)
  })
  test("an agent the roster doesn't have keeps its status; nothing new when nothing changed", () => {
    const c = card(TaskState.Working, [agent("busy")])
    expect(withLive(c, live({ "other/x": "idle" }))).toBe(c)
    expect(withLive(c, new Map())).toBe(c)
  })
  test("workingNow names the busy taken-up holders, once each, by busy's rule", () => {
    const oak = agent("busy", { key: "d/oak", name: "oak" })
    const elm = agent("idle", { key: "d/elm", name: "elm" })
    const offered = agent("busy", { key: "d/ash", name: "ash", assigned: true })
    expect(
      workingNow(card(TaskState.Working, [oak, elm, offered, oak])).map(
        (w) => w.name
      )
    ).toEqual(["oak"])
    // Same rule as Agent progress: a card that waits on the person shows nobody working.
    expect(workingNow(card(TaskState.Working, [oak], 1))).toEqual([])
    expect(workingNow(card(TaskState.InReview, [oak]))).toEqual([])
    expect(workingNow(card(TaskState.Completed, [oak]))).toEqual([])
  })
  test("namesLine", () => {
    expect(namesLine([])).toBe("")
    expect(namesLine(["oak"])).toBe("oak")
    expect(namesLine(["oak", "elm"])).toBe("oak and elm")
    expect(namesLine(["oak", "elm", "ash"])).toBe("oak, elm and ash")
  })
})

describe("workLabel", () => {
  test("says working only when the card spins", () => {
    expect(workLabel(card(TaskState.Working, [agent("busy")]))).toBe("working")
    expect(workLabel(card(TaskState.Working, [agent("idle")]))).toBe("on it")
    expect(workLabel(card(TaskState.Blocked, [agent("busy")], 1))).toBe("on it")
    expect(workLabel(card(TaskState.InputRequired, [agent("busy")]))).toBe(
      "on it"
    )
  })
  test("a finished card says nothing", () => {
    expect(workLabel(card(TaskState.Completed, [agent("busy")]))).toBe("")
  })
  test("assigned holders say so", () => {
    expect(
      workLabel(card(TaskState.Working, [agent("idle", { assigned: true })]))
    ).toBe("offered")
    expect(
      workLabel(
        card(TaskState.Working, [
          agent("idle", { assigned: true, waiting: true }),
        ])
      )
    ).toBe("not picked up")
    expect(workLabel(card(TaskState.Working, []))).toBe("")
  })
})

describe("card hash", () => {
  test("round-trips an id", () => {
    expect(cardFromHash(cardHash("01M62QZZYANGS87A9MNTP2M54J"))).toBe(
      "01M62QZZYANGS87A9MNTP2M54J"
    )
    expect(cardFromHash(cardHash("a b/c"))).toBe("a b/c")
  })
  test("anything else is the grid", () => {
    expect(cardFromHash("")).toBe("")
    expect(cardFromHash("#")).toBe("")
    expect(cardFromHash("#card/")).toBe("")
    expect(cardFromHash("#card/%E0%A4%A")).toBe("")
  })
})

describe("asks", () => {
  const ask = (more: Partial<CardAsk>) =>
    CardAsk({
      state: "open",
      told: 3,
      owner: "MacBook",
      wake: false,
      taker: null,
      outcome: null,
      ...more,
    })
  test("the tile's line says where an ask stands", () => {
    expect(askLine(ask({}))).toBe("Told 3 agents; nobody took it yet")
    expect(askLine(ask({ told: 1 }))).toBe("Told 1 agent; nobody took it yet")
    expect(askLine(ask({ told: 0 }))).toBe("Nobody was free to hear it")
    expect(askLine(ask({ state: "waiting" }))).toBe(
      "Waiting for MacBook to hand it out"
    )
    expect(askLine(ask({ state: "unclaimed" }))).toBe("Nobody took this yet")
    expect(
      askLine(
        ask({ state: "taken", taker: agent("idle", { name: "sample-heron" }) })
      )
    ).toBe("sample-heron took it")
    expect(
      askLine(
        ask({ state: "handed", taker: agent("idle", { name: "sample-heron" }) })
      )
    ).toBe("Handed to sample-heron")
  })
  // Card #518: while Irori starts the ask's agent the tile says so, not
  // that nobody took it.
  test("an ask whose agent is starting says where", () => {
    const pc = "Macbook Pro (this one)"
    for (const state of ["unclaimed", "open"]) {
      expect(askStarting(ask({ state }), pc)).toBe(true)
      expect(askLine(ask({ state }), pc)).toBe(`Starting an agent on ${pc}`)
    }
    expect(askLine(ask({ state: "unclaimed" }), "")).toBe(
      "Nobody took this yet"
    )
    for (const state of ["waiting", "taken", "handed", "closed"])
      expect(askStarting(ask({ state }), pc)).toBe(false)
    expect(
      askLine(
        ask({ state: "taken", taker: agent("idle", { name: "sample-heron" }) }),
        pc
      )
    ).toBe("sample-heron took it")
  })
  test("only an ask without an outcome is open", () => {
    expect(openAsk({ ask: ask({}) })).toBe(true)
    expect(
      openAsk({
        ask: ask({ state: "closed", outcome: { kind: "none", note: "x" } }),
      })
    ).toBe(false)
    expect(openAsk({ ask: null })).toBe(false)
    expect(openAsk({})).toBe(false)
  })
  test("a closed ask reads by its outcome (card #28)", () => {
    const closed = (kind: string) =>
      ask({ state: "closed", outcome: { kind, note: "x" } })
    expect(
      overLabel({ state: TaskState.Completed, ask: closed("answered") })
    ).toBe("Answered")
    expect(overLabel({ state: TaskState.Canceled, ask: closed("none") })).toBe(
      "Closed"
    )
    expect(overLabel({ state: TaskState.Completed, ask: closed("edit") })).toBe(
      "Done"
    )
    expect(overLabel({ state: TaskState.Completed })).toBe("Done")
    expect(overLabel({ state: TaskState.Canceled })).toBe("Canceled")
    expect(overLabel({ state: TaskState.Failed })).toBe("Failed")
    expect(askLine(closed("answered"))).toBe("Answered: x")
    expect(askLine(closed("none"))).toBe("Closed: x")
  })
})

describe("gone holders (card #16)", () => {
  test("a card with steps shows its holder line unless someone is busy on it", () => {
    expect(holderLineShows({ holder: { state: Holding.HoldActive } })).toBe(
      false
    )
    expect(holderLineShows({ holder: { state: Holding.HoldPerson } })).toBe(
      false
    )
    for (const s of [
      Holding.HoldIdle,
      Holding.HoldGone,
      Holding.HoldOffline,
      Holding.HoldNobody,
      Holding.HoldWaiting,
    ])
      expect(holderLineShows({ holder: { state: s } })).toBe(true)
  })
  test("nobody is on a card whose holder went away or was taken off", () => {
    expect(nobodyOn({ holder: { state: Holding.HoldGone } })).toBe(true)
    expect(
      nobodyOn({
        holder: { state: Holding.HoldNobody },
        returned: agent("gone"),
      })
    ).toBe(true)
    expect(nobodyOn({ holder: { state: Holding.HoldNobody } })).toBe(false)
    expect(nobodyOn({ holder: { state: Holding.HoldIdle } })).toBe(false)
  })
})

describe("device colours (card #22)", () => {
  const pc = agent("busy", {
    deviceId: "pc",
    where: "Macbook Pro (this one)",
    color: "teal",
  })
  const mac = agent("idle", {
    key: "mac/s",
    deviceId: "mac",
    where: "Desktop Computer",
    here: false,
    color: "orange",
  })
  test("a chip per device the cards' agents run on, once there are two", () => {
    expect(deviceChips([{ working: [pc] }, { working: [pc] }])).toEqual([])
    expect(deviceChips([{ working: [pc] }, { working: [mac, pc] }])).toEqual([
      { id: "mac", name: "Desktop Computer", color: "orange" },
      { id: "pc", name: "Macbook Pro (this one)", color: "teal" },
    ])
  })
  test("faces with no colour (one device) give no chips", () => {
    expect(
      deviceChips([
        { working: [agent("busy", { deviceId: "pc" })] },
        { working: [agent("idle", { deviceId: "mac" })] },
      ])
    ).toEqual([])
  })
  test("a colour is a CSS variable", () => {
    expect(deviceVar("teal")).toBe("var(--device-teal)")
    expect(deviceVar("")).toBe("")
  })
})

describe("closed cards (Done tab)", () => {
  test("no worker label or spinner on a closed card, whatever its holders", () => {
    for (const st of [
      TaskState.Completed,
      TaskState.Canceled,
      TaskState.Failed,
    ]) {
      expect(workLabel(card(st, [agent("busy")]))).toBe("")
      expect(workLabel(card(st, [agent("idle", { assigned: true })]))).toBe("")
      expect(
        workLabel(card(st, [agent("idle", { assigned: true, waiting: true })]))
      ).toBe("")
      expect(busy(card(st, [agent("busy")]))).toBe(false)
    }
  })
})

describe("sampleCardRef: done cards and threads (card #144)", () => {
  // The sample reads ?empty from the address; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }
  test("a done card", () => {
    expect(sampleCardRef("#5")).toMatchObject({
      id: "sample-card-5",
      thread: false,
      done: true,
    })
  })
  test("a resolved thread and an open one", () => {
    expect(sampleCardRef("#26")).toMatchObject({
      id: "sample-thread-4",
      thread: true,
      done: true,
    })
    expect(sampleCardRef("#21")).toMatchObject({
      id: "sample-thread-2",
      thread: true,
      done: false,
    })
    // #20 is a card and a thread in the sample: ambiguous, so text.
    expect(sampleCardRef("#20")).toBeNull()
  })
  test("a board card by its tag; nothing for an unknown ref", () => {
    expect(sampleCardRef("mlg#1")?.id).toBeTruthy()
    expect(sampleCardRef("#9999")).toBeNull()
    expect(sampleCardRef("nonsense")).toBeNull()
  })
})

describe("openTarget", () => {
  test("a card, and a card at a comment (card #177)", () => {
    expect(openTarget("task:t1")).toEqual({ card: "t1", comment: "" })
    expect(openTarget("task:t1/comment:c9")).toEqual({
      card: "t1",
      comment: "c9",
    })
  })
  test("anything else is null", () => {
    expect(openTarget("msg:x")).toBeNull()
    expect(openTarget("task:")).toBeNull()
    expect(openTarget("")).toBeNull()
  })
})

// Card #343: a #ref's chip says what its card's page pill would.
describe("refStatus", () => {
  const c = (state: string, blocked = false, more = {}) =>
    refStatus({ thread: false, done: false, state, blocked, ...more })
  test("each state reads as its word and colour", () => {
    expect(c(TaskState.Submitted)).toMatchObject({
      label: "Open",
      tone: "slate",
    })
    expect(c(TaskState.Working)).toMatchObject({
      label: "Working",
      tone: "info",
    })
    expect(c(TaskState.Working, false, { working: false }).label).toBe("Open")
    expect(c(TaskState.Working, true)).toMatchObject({
      label: "Blocked",
      tone: "destructive",
    })
    expect(c(TaskState.InputRequired).label).toBe("Blocked")
    expect(c(TaskState.InReview)).toMatchObject({
      label: "In review",
      tone: "warning",
    })
    expect(c(TaskState.Completed)).toMatchObject({
      label: "Done",
      tone: "success",
    })
    expect(c(TaskState.Failed).label).toBe("Failed")
    expect(c(TaskState.Canceled).label).toBe("Canceled")
  })
  test("done but for your word reads Ready, not blocked (card #112)", () => {
    expect(c(TaskState.Working, false, { ready: true })).toMatchObject({
      label: "Ready",
      tone: "success",
    })
  })
  test("done wins over a blocker left open", () => {
    expect(c(TaskState.Completed, true).label).toBe("Done")
  })
  test("steps only when it has some", () => {
    expect(
      c(TaskState.Working, false, { progress: { done: 2, total: 5 } }).progress
    ).toEqual({ done: 2, total: 5 })
    expect(
      c(TaskState.Working, false, { progress: { done: 0, total: 0 } }).progress
    ).toBeNull()
  })
  test("a thread is open or resolved", () => {
    const t = (done: boolean) =>
      refStatus({ thread: true, done, state: "working", blocked: false })
    expect(t(false).label).toBe("Open thread")
    expect(t(true).label).toBe("Resolved")
  })
})

describe("peopleRows (card #490)", () => {
  const otter = agent("idle", { key: "d/otter", name: "otter" })
  const heron = agent("idle", { key: "d/heron", name: "heron" })
  const wren = agent("idle", { key: "d/wren", name: "wren" })
  const steps = [
    Step({ id: "s1", text: "Plan" }),
    Step({ id: "s3", text: "Sync" }),
  ]
  const wait = (id: string, who: AgentRef, text: string, onThis = false) =>
    CardWait({ id, who, text, onThis })

  test("a holder that also waits is one row with its wait", () => {
    const rows = peopleRows({
      working: [otter],
      watching: [],
      waits: [
        wait("w1", agent("idle", { key: "d/otter" }), "branch main moves"),
      ],
      steps,
    })
    expect(rows).toHaveLength(1)
    expect(rows[0].role).toBe("working")
    expect(rows[0].who.name).toBe("otter")
    expect(rows[0].waits.map((w) => w.text)).toEqual(["branch main moves"])
  })
  test("a watcher waiting on this card stays one watching row", () => {
    const rows = peopleRows({
      working: [],
      watching: [wren],
      waits: [wait("w1", wren, "until done", true), wait("w2", wren, "#3")],
      steps,
    })
    expect(rows.map((r) => [r.who.key, r.role, r.waits.length])).toEqual([
      ["d/wren", "watching", 2],
    ])
  })
  test("an agent that only waits gets a waiting row, after working and watching", () => {
    const rows = peopleRows({
      working: [otter],
      watching: [heron],
      waits: [wait("w1", wren, "until done", true)],
      steps,
    })
    expect(rows.map((r) => r.role)).toEqual(["working", "watching", "waiting"])
  })
  test("step owners show step titles, never ids", () => {
    const rows = peopleRows({
      working: [otter, AgentRef({ ...heron, steps: ["s3", "s9"] })],
      watching: [],
      steps,
    })
    expect(rows[1].stepTitles).toEqual(["Sync", "a step"])
  })
  test("the same agent twice in working merges into one row", () => {
    const rows = peopleRows({
      working: [
        AgentRef({ ...otter, assigned: true }),
        AgentRef({ ...otter, steps: ["s1"] }),
      ],
      watching: [otter],
      steps,
    })
    expect(rows).toHaveLength(1)
    expect(rows[0].who.assigned).toBe(true)
    expect(rows[0].stepTitles).toEqual(["Plan"])
  })
})
