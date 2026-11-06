import { describe, expect, test } from "bun:test"

import { sameSection, sectionFromHash, sectionHash } from "./nav"

describe("Talk lives in the address (card #390)", () => {
  test("#talk is the list, #talk/<id> a conversation open", () => {
    expect(sectionFromHash("#talk")).toEqual({ kind: "talk", id: "" })
    expect(sectionFromHash("#talk/01ABC")).toEqual({
      kind: "talk",
      id: "01ABC",
    })
  })
  test("the hash round-trips", () => {
    for (const s of [
      { kind: "talk", id: "" },
      { kind: "talk", id: "01M6 x/y" },
      { kind: "board", id: "b1" },
      { kind: "boards" },
      { kind: "profile" },
      { kind: "home" },
    ] as const)
      expect(sectionFromHash(sectionHash(s))).toEqual(s)
  })
  test("another conversation is another place, so it adds history", () => {
    expect(
      sameSection({ kind: "talk", id: "a" }, { kind: "talk", id: "a" })
    ).toBe(true)
    expect(
      sameSection({ kind: "talk", id: "a" }, { kind: "talk", id: "b" })
    ).toBe(false)
    expect(sameSection({ kind: "talk", id: "" }, { kind: "home" })).toBe(false)
  })
})

describe("Notes lives in the address (card #413)", () => {
  test("#notes is Notes and round-trips; an old board tab opens it (card #616)", () => {
    expect(sectionFromHash("#notes")).toEqual({ kind: "notes" })
    expect(sectionFromHash("#notes/mlg")).toEqual({ kind: "notes" })
    expect(sectionHash({ kind: "notes" })).toBe("#notes")
    expect(sameSection({ kind: "notes" }, { kind: "notes" })).toBe(true)
  })
})

describe("merge desks live in the address (card #417)", () => {
  test("#desk is the first desk, #desk/<repoKey> one repo's", () => {
    expect(sectionFromHash("#desk")).toEqual({ kind: "desk", repo: "" })
    const s = { kind: "desk", repo: "remote:github.com/you/game-repo" } as const
    expect(sectionFromHash(sectionHash(s))).toEqual(s)
    expect(sameSection(s, { kind: "desk", repo: "" })).toBe(false)
  })
})
