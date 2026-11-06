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
      [
        card("58", [{ key: "thyme", spawned: true }]),
        card("57", [{ key: "basil" }]),
      ]
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
    expect(g.onCards.map((c) => [c.short, c.agents.map((x) => x.key)])).toEqual(
      [
        ["61", ["fern", "clover"]],
        ["66", ["mallow"]],
      ]
    )
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
  test("counts cards starting an agent and cards queued for one", () => {
    const g = rosterGroups(
      [],
      [
        card("64", [], { starting: "Macbook Pro (this one)" }),
        card("67", [], { atCap: "Macbook Pro (this one)", queuePlace: 1 }),
        // Card #450: held on a usage limit.
        card("68", [], { usageLimit: "2026-11-04T23:30:00Z" }),
        card("70", [], { cantStart: "no copy of irori on PC" }),
      ]
    )
    expect([g.starting, g.queued]).toEqual([1, 2])
  })
})

describe("spawnedKeys", () => {
  test("is only the rows Irori started", () => {
    const k = spawnedKeys([
      card("58", [{ key: "thyme", spawned: true }, { key: "basil" }]),
    ])
    expect([...k]).toEqual(["thyme"])
  })
  test("blocked: on a permission prompt, or idle on a blocked card (card #512)", () => {
    const g = rosterGroups(
      [
        a("fern", "waiting"),
        a("clover", "idle"),
        a("mallow", "idle"),
        a("sorrel", "gone"),
        a("thyme"),
        a("basil", "idle"),
      ],
      [
        { ...card("61", [{ key: "clover", spawned: true }]), openBlockers: 1 },
        card("66", [{ key: "mallow" }]),
        { ...card("67", [{ key: "sorrel" }]), openBlockers: 2 },
        // Busy on a blocked card: working, not blocked.
        { ...card("68", [{ key: "thyme" }]), openBlockers: 1 },
        // Ready for your check: not a block.
        { ...card("69", [{ key: "basil" }]), openBlockers: 1, ready: true },
      ]
    )
    expect([...g.blocked].sort()).toEqual(["clover", "fern"])
  })
})
