import { describe, expect, test } from "bun:test"

import {
  boardsFor,
  go,
  openSettings,
  sameSection,
  sectionFromHash,
  sectionHash,
  settingsBack,
  settingsSectionIds,
} from "./nav"

describe("Talk lives in the address (card #390)", () => {
  test("#huddle/<callId> is a huddle's page, and sits apart from Talk", () => {
    expect(sectionFromHash("#huddle/call%201")).toEqual({
      kind: "huddle",
      id: "call 1",
    })
    expect(sectionHash({ kind: "huddle", id: "call 1" })).toBe(
      "#huddle/call%201"
    )
    expect(
      sameSection({ kind: "huddle", id: "a" }, { kind: "huddle", id: "b" })
    ).toBe(false)
    expect(
      sameSection({ kind: "huddle", id: "a" }, { kind: "talk", id: "a" })
    ).toBe(false)
  })

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

describe("Settings lives in the address (card #1007)", () => {
  test("#settings is the page, #settings/<section> one section", () => {
    expect(sectionFromHash("#settings")).toEqual({ kind: "settings", tab: "" })
    expect(sectionFromHash("#settings/agents")).toEqual({
      kind: "settings",
      tab: "agents",
    })
  })
  test("every section round-trips", () => {
    for (const tab of ["", ...settingsSectionIds] as const) {
      const s = { kind: "settings", tab } as const
      expect(sectionFromHash(sectionHash(s))).toEqual(s)
    }
  })
  test("a section that's gone still opens Settings", () => {
    expect(sectionFromHash("#settings/maintenance")).toEqual({
      kind: "settings",
      tab: "",
    })
  })
  test("another section is another place, so it adds history", () => {
    expect(
      sameSection(
        { kind: "settings", tab: "agents" },
        { kind: "settings", tab: "agents" }
      )
    ).toBe(true)
    expect(
      sameSection(
        { kind: "settings", tab: "agents" },
        { kind: "settings", tab: "app" }
      )
    ).toBe(false)
  })
  test("Agents comes first", () => {
    expect(settingsSectionIds[0]).toBe("agents")
  })
})

// A window with just enough history for go, replace and back: a stack of
// [hash, state] and where it stands.
function withHistory(start: string, run: (stack: () => string[]) => void) {
  const entries: { hash: string; state: unknown }[] = [
    { hash: start, state: null },
  ]
  let at = 0
  const loc = {
    pathname: "/",
    search: "",
    get hash() {
      return entries[at].hash
    },
  }
  const hashOf = (url: string) => url.slice(url.indexOf("#"))
  const fake = {
    location: loc,
    history: {
      get state() {
        return entries[at].state
      },
      pushState(state: unknown, _t: string, url: string) {
        entries.splice(at + 1)
        entries.push({ hash: hashOf(url), state })
        at++
      },
      replaceState(state: unknown, _t: string, url: string) {
        entries[at] = { hash: hashOf(url), state }
      },
      back() {
        if (at > 0) at--
      },
    },
    dispatchEvent: () => true,
  }
  const was = globalThis.window
  const wasEvent = globalThis.PopStateEvent
  globalThis.window = fake as unknown as Window & typeof globalThis
  globalThis.PopStateEvent = class {} as unknown as typeof PopStateEvent
  try {
    run(() => entries.slice(0, at + 1).map((e) => e.hash))
  } finally {
    globalThis.window = was
    globalThis.PopStateEvent = wasEvent
  }
}

describe("Settings' Back and the gear (card #1007)", () => {
  test("a section opened from the list goes back to it, not forward", () => {
    withHistory("#home", (stack) => {
      openSettings()
      go({ kind: "settings", tab: "agents" }, { fromList: true })
      settingsBack()
      expect(stack()).toEqual(["#home", "#settings"])
    })
  })
  test("a section reached straight goes to the list in its place", () => {
    withHistory("#home", (stack) => {
      openSettings("devices")
      settingsBack()
      expect(stack()).toEqual(["#home", "#settings"])
    })
  })
  test("the gear on a section keeps it, with no new entry", () => {
    withHistory("#settings/devices", (stack) => {
      openSettings()
      expect(stack()).toEqual(["#settings/devices"])
    })
  })
  test("the gear from elsewhere opens Settings, and a named section moves", () => {
    withHistory("#home", (stack) => {
      openSettings()
      openSettings("app")
      expect(stack()).toEqual(["#home", "#settings", "#settings/app"])
    })
  })
})
