import { describe, expect, test } from "bun:test"

import { agentCounts, freeIn, idleLine, whyWaiting } from "./agent-counts"

// An agent that takes offered work (card #453) unless extra says otherwise.
const a = (
  status: string,
  extra: { deviceState?: string; repoKey?: string; offersCards?: boolean; spawned?: boolean } = {}
) => ({
  status,
  offersCards: true,
  ...extra,
})

describe("agentCounts (card #172)", () => {
  test("shell counts as working, not free; gone isn't counted", () => {
    const c = agentCounts([
      a("busy"),
      a("shell"),
      a("shell"),
      a("idle"),
      a("unresponsive"),
      a("idle", { deviceState: "offline" }),
      a("gone"),
    ])
    expect(c).toEqual({
      total: 6,
      working: 3,
      running: 2,
      free: 1,
      ownWork: 0,
      silent: 1,
      offline: 1,
    })
  })

  test("an idle terminal agent without Offer it cards isn't free (card #453)", () => {
    const agents = [
      a("idle", { offersCards: false }),
      a("idle", { offersCards: true, spawned: true }),
      a("idle"),
    ]
    const c = agentCounts(agents)
    expect(c.free).toBe(1)
    expect(c.ownWork).toBe(2)
    expect(freeIn(agents)).toBe(1)
    expect(freeIn([a("idle", { offersCards: false })])).toBe(0)
  })

  test("free matches who a thread goes to: idle on an online device", () => {
    const agents = [a("idle"), a("idle", { deviceState: "offline" }), a("shell"), a("unresponsive")]
    expect(agentCounts(agents).free).toBe(freeIn(agents))
    expect(freeIn(agents)).toBe(1)
  })
})

describe("whyWaiting (card #172)", () => {
  test("everyone working, some on their own commands", () => {
    expect(whyWaiting([a("busy"), a("shell"), a("shell")])).toBe(
      "Nobody is free: 3 working (2 waiting on a command they started). The first agent with Offer it cards on to go idle answers it."
    )
  })
  test("idle but not answering", () => {
    expect(whyWaiting([a("busy"), a("unresponsive"), a("unresponsive")])).toBe(
      "Nobody is free: 1 working, 2 idle but not answering. The first agent with Offer it cards on to go idle answers it."
    )
  })
  test("on an offline device", () => {
    expect(whyWaiting([a("idle", { deviceState: "offline" })])).toBe(
      "Nobody is free: 1 on an offline device. The first agent with Offer it cards on to go idle answers it."
    )
  })
  test("idle, but none in the repo", () => {
    expect(whyWaiting([a("idle", { repoKey: "mlg" }), a("idle", { repoKey: "mlg" })], "irori")).toBe(
      "2 agents are idle, but none works in irori."
    )
  })
  test("someone free: no reason", () => {
    expect(whyWaiting([a("idle"), a("busy")])).toBe("")
    expect(whyWaiting([a("idle", { repoKey: "irori" })], "irori")).toBe("")
  })
  test("nobody takes offered work (card #453)", () => {
    expect(whyWaiting([a("idle", { offersCards: false }), a("busy", { spawned: true })])).toBe(
      "No agent takes offered work: Irori starts one on boards with spawning on, or turn on Offer it cards for an agent."
    )
  })
  test("idle agents not taking offers while a taker works", () => {
    expect(whyWaiting([a("busy"), a("idle", { offersCards: false })])).toBe(
      "Nobody is free: 1 working, 1 idle, not taking offered work. The first agent with Offer it cards on to go idle answers it."
    )
  })
  test("no agents", () => {
    expect(whyWaiting([a("gone")])).toBe("No agents are running.")
  })
})

describe("idleLine (card #341)", () => {
  const u = (short: string, why: string) => ({ id: short, short, title: short, why })
  test("idle agents and nothing to hand out: a work shortage", () => {
    expect(idleLine({ idle: 20, unoffered: [] })).toBe("20 agents idle, no open work.")
    expect(idleLine({ idle: 1, unoffered: [] })).toBe("1 agent idle, no open work.")
  })
  test("nobody idle and nothing waiting says nothing", () => {
    expect(idleLine({ idle: 0, unoffered: [] })).toBe("")
  })
  test("idle agents and cards nobody was handed: a dispatch gap, with why", () => {
    expect(
      idleLine({
        idle: 20,
        unoffered: [
          u("wind#87", "nobody on this board is free"),
          u("wind#88", "nobody on this board is free"),
          u("#12", "no free agent works in irori"),
          u("#13", "every free agent passed it"),
        ],
      })
    ).toBe(
      "20 agents idle; 4 cards nobody took: wind#87 (nobody on this board is free), wind#88 (nobody on this board is free), #12 (no free agent works in irori), and 1 more."
    )
  })
  test("cards waiting with nobody idle", () => {
    expect(idleLine({ idle: 0, unoffered: [u("#5", "no agent is free")] })).toBe(
      "No agent is free; 1 card nobody took: #5 (no agent is free)."
    )
  })
  // Card #487: an idle agent and cards nobody took are not "no open work",
  // whether Irori handed them out already (offered, assigned, an ask told
  // three agents) or not yet (with why).
  test("cards Home files as nobody took them count, with the report's why", () => {
    const took = [
      { id: "c25", short: "#25" },
      { id: "c6", short: "#6" },
      { id: "c14", short: "#14" },
    ]
    expect(idleLine({ idle: 1, unoffered: [] }, took)).toBe(
      "1 agent idle; 3 cards nobody took: #25, #6, #14."
    )
    expect(
      idleLine({ idle: 1, unoffered: [{ ...u("#25", "every free agent passed it"), id: "c25" }] }, took)
    ).toBe("1 agent idle; 3 cards nobody took: #25 (every free agent passed it), #6, #14.")
    // A report card Home doesn't list still counts, once.
    expect(idleLine({ idle: 2, unoffered: [u("wind#9", "nobody on this board is free")] }, took.slice(0, 1))).toBe(
      "2 agents idle; 2 cards nobody took: #25, wind#9 (nobody on this board is free)."
    )
  })
})
