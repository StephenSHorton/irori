import { describe, expect, test } from "bun:test"

import { rosterGroups, spawnedKeys } from "./roster-groups"

const a = (key: string, status = "busy") => ({ key, status })
const card = (
  short: string,
  working: { key: string; spawned?: boolean }[],
  more: { starting?: string; atCap?: string } = {}
) => ({
  id: `c${short}`,
  short,
  title: `card ${short}`,
  working: working.map((w) => ({ status: "busy", ...w })),
  ...more,
})

describe("rosterGroups", () => {
  test("splits terminal agents from the ones started for a card", () => {
    const g = rosterGroups(
      [a("juniper", "idle"), a("thyme"), a("basil")],
      [card("58", [{ key: "thyme", spawned: true }]), card("57", [{ key: "basil" }])]
    )
    expect(g.yours.map((x) => x.key)).toEqual(["juniper", "basil"])
    expect(g.onCards).toEqual([
      { cardId: "c58", short: "58", title: "card 58", agents: [a("thyme")] },
    ])
    expect(g.spawned).toBe(1)
  })
  test("several on one card sit together under it", () => {
    const g = rosterGroups(
      [a("fern"), a("clover"), a("mallow")],
      [
        card("61", [
          { key: "fern", spawned: true },
          { key: "clover", spawned: true },
        ]),
        card("66", [{ key: "mallow", spawned: true }]),
      ]
    )
    expect(g.onCards.map((c) => [c.short, c.agents.map((x) => x.key)])).toEqual([
      ["61", ["fern", "clover"]],
      ["66", ["mallow"]],
    ])
    expect(g.yours).toEqual([])
  })
  test("an ended agent is in neither group", () => {
    const g = rosterGroups(
      [a("sorrel", "gone"), a("juniper", "idle")],
      [card("61", [{ key: "sorrel", spawned: true }])]
    )
    expect(g.onCards).toEqual([])
    expect(g.yours.map((x) => x.key)).toEqual(["juniper"])
    expect(g.spawned).toBe(0)
  })
  test("counts cards starting an agent and cards at the cap", () => {
    const g = rosterGroups(
      [],
      [card("64", [], { starting: "Macbook Pro (this one)" }), card("67", [], { atCap: "Macbook Pro (this one)" })]
    )
    expect([g.starting, g.atCap]).toEqual([1, 1])
  })
})

describe("spawnedKeys", () => {
  test("is only the rows Irori started", () => {
    const k = spawnedKeys([card("58", [{ key: "thyme", spawned: true }, { key: "basil" }])])
    expect([...k]).toEqual(["thyme"])
  })
})
