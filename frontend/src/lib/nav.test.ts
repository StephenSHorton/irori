import { describe, expect, test } from "bun:test"

import { boardsFor, sameSection, sectionFromHash, sectionHash } from "./nav"

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

describe("Boards shows only the picked space's (card #689)", () => {
  test("#boards names no space, and round-trips", () => {
    expect(sectionFromHash("#boards")).toEqual({ kind: "boards" })
    expect(sectionHash({ kind: "boards" })).toBe("#boards")
  })
  test("the Boards tab goes to the picked space's page", () => {
    expect(boardsFor("")).toEqual({ kind: "board", id: "personal" })
    expect(boardsFor("personal")).toEqual({ kind: "board", id: "personal" })
    expect(boardsFor("b1")).toEqual({ kind: "board", id: "b1" })
  })
  test("a space's page and Personal's still open from their links", () => {
    expect(sectionFromHash("#board/b1")).toEqual({ kind: "board", id: "b1" })
    expect(sectionFromHash("#board/personal")).toEqual({
      kind: "board",
      id: "personal",
    })
    expect(sectionFromHash("#board/b1/b2")).toEqual({
      kind: "board",
      id: "b1",
      board: "b2",
    })
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

describe("Storage lives in the address (card #728)", () => {
  test("#storage is the overview, #storage/<kind> one kind's page", () => {
    expect(sectionFromHash("#storage")).toEqual({ kind: "storage", tab: "" })
    expect(sectionFromHash("#storage/files")).toEqual({
      kind: "storage",
      tab: "files",
    })
  })
  test("the hash round-trips", () => {
    for (const s of [
      { kind: "storage", tab: "" },
      { kind: "storage", tab: "builds" },
    ] as const)
      expect(sectionFromHash(sectionHash(s))).toEqual(s)
  })
  test("another kind is another place, so it adds history", () => {
    expect(
      sameSection({ kind: "storage", tab: "" }, { kind: "storage", tab: "" })
    ).toBe(true)
    expect(
      sameSection(
        { kind: "storage", tab: "" },
        { kind: "storage", tab: "files" }
      )
    ).toBe(false)
  })
})
