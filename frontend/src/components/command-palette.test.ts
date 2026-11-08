import { describe, expect, test } from "bun:test"

import { AgentCard, AgentRef, BoardSummary, CardSummary } from "@/lib/models"
import { buildItems, isRef, paletteOpenKey, results } from "./command-palette"

const noop = () => {}
const items = buildItems({
  commands: [
    { id: "nav.home", title: "Dashboard", group: "Go to", run: noop },
    {
      id: "settings.open",
      title: "Settings",
      group: "Go to",
      keywords: ["preferences"],
      run: noop,
    },
    { id: "card.new", title: "New card", run: noop },
  ],
  cards: [
    CardSummary({ id: "a", short: "212", title: "Command palette for Irori" }),
    CardSummary({
      id: "b",
      short: "51",
      tag: "wind",
      title: "Boss relay on the Mac rig",
    }),
    CardSummary({ id: "c", short: "2120", title: "Something else" }),
  ],
  boards: [
    BoardSummary({ boardId: "w", name: "Wind Waker co-op", tag: "wind" }),
  ],
  agents: [],
  onCard: noop,
  onBoard: noop,
  onAgent: noop,
})

describe("results", () => {
  test("a ref puts its card first", () => {
    expect(results(items, "#212", [])[0].key).toBe("card:a")
    expect(results(items, "wind#51", [])[0].key).toBe("card:b")
  })
  test("a title word, a board name and settings", () => {
    expect(results(items, "palette", [])[0].key).toBe("card:a")
    expect(results(items, "waker", [])[0].key).toBe("board:w")
    expect(results(items, "settings", [])[0].key).toBe("command:settings.open")
  })
  test("empty: recents first, then commands only", () => {
    const got = results(items, "", ["card:b", "command:card.new", "card:gone"])
    expect(got.slice(0, 2).map((i) => [i.key, i.group])).toEqual([
      ["card:b", "Recent"],
      ["command:card.new", "Recent"],
    ])
    expect(got.slice(2).map((i) => i.key)).toEqual([
      "command:nav.home",
      "command:settings.open",
    ])
  })
  test("groups stay together in order", () => {
    const groups = results(items, "o", []).map((i) => i.group)
    const seen: string[] = []
    for (const g of groups) if (seen.at(-1) !== g) seen.push(g)
    expect(new Set(seen).size).toBe(seen.length)
  })
})

describe("agents on cards (card #456)", () => {
  const opened: string[] = []
  const rows = buildItems({
    commands: [],
    cards: [
      CardSummary({
        id: "c58",
        short: "58",
        title: "Lantern flicker on the dock",
        working: [
          AgentRef({
            key: "pc/thyme",
            name: "thyme",
            status: "busy",
            spawned: true,
          }),
        ],
      }),
    ],
    boards: [],
    agents: [
      AgentCard({ key: "pc/thyme", name: "thyme", status: "busy" }),
      AgentCard({ key: "pc/juniper", name: "juniper", status: "idle" }),
    ],
    onCard: (id) => opened.push(id),
    onBoard: noop,
    onAgent: noop,
  })
  test("a spawned agent sits under its card and opens it", () => {
    const t = rows.find((i) => i.key === "agent:pc/thyme")!
    expect([t.group, t.hint]).toEqual([
      "Agents on cards",
      "on #58 Lantern flicker on the dock",
    ])
    t.run()
    expect(opened).toEqual(["c58"])
  })
  test("a terminal agent stays in Agents", () => {
    expect(rows.find((i) => i.title === "juniper")?.group).toBe("Agents")
  })
  test("its card's ref finds it", () => {
    expect(results(rows, "lantern", []).map((i) => i.key)).toContain(
      "agent:pc/thyme"
    )
  })
})

describe("keys", () => {
  const k = (key: string, o: Partial<KeyboardEvent> = {}) => ({
    key,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...o,
  })
  test("Cmd on a Mac, Ctrl elsewhere", () => {
    expect(paletteOpenKey(k("k", { metaKey: true }), true)).toBe(true)
    expect(paletteOpenKey(k("p", { metaKey: true }), true)).toBe(true)
    expect(paletteOpenKey(k("p", { ctrlKey: true }), true)).toBe(false)
    expect(paletteOpenKey(k("K", { ctrlKey: true }), false)).toBe(true)
    expect(
      paletteOpenKey(k("p", { ctrlKey: true, shiftKey: true }), false)
    ).toBe(false)
    expect(paletteOpenKey(k("j", { ctrlKey: true }), false)).toBe(false)
  })
  test("isRef", () => {
    expect(isRef("#212")).toBe(true)
    expect(isRef("wind#51")).toBe(true)
    expect(isRef("palette")).toBe(false)
  })
})
