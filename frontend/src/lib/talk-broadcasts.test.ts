// Card #486: one agent's message to many agents was one identical row per
// agent in Agents talking, and every row's subtitle named the section.
// A broadcast is one row now, rows say their last line and when, and the
// section lists the newest, capped.

import { describe, expect, test } from "bun:test"

import type { Conversation, Member } from "@/lib/conversations"
import { TalkConversation } from "@/lib/models"
import {
  agentRows,
  agentSectionRows,
  agentsCap,
  broadcastTo,
  broadcastWindow,
  shownAtOnce,
  type AgentRow,
} from "./talk-sections"

const g = globalThis as unknown as { window?: unknown }
g.window ??= {
  matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
  addEventListener() {},
  removeEventListener() {},
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
}
const { lastLine } = await import("@/components/talk/talk-page")

const t0 = Date.parse("2026-11-01T18:31:54Z")
const iso = (ms: number) => new Date(ms).toISOString()
const agent = (key: string, name = key): Member => ({ kind: "agent", key, name })
const owl = agent("dev/owl", "owl")

// by's copy to `to`, sent `sec` seconds after t0 (newer copies later).
const copy = (to: string, sec: number, text = "WIND DOWN NOW", by = owl): Conversation =>
  TalkConversation({
    convId: `dm:${by.key}:${to}`,
    kind: "dm",
    name: `${by.name} ↔ ${to}`,
    members: [by, agent(`dev/${to}`, to)],
    readOnly: true,
    lastAt: iso(t0 + sec * 1000),
    lastText: text,
    lastBy: by.key,
  })

const newestFirst = (cs: Conversation[]) => [...cs].sort((a, b) => (a.lastAt < b.lastAt ? 1 : -1))
const shape = (rows: AgentRow[]) =>
  rows.map((r) => (r.kind === "dm" ? r.c.convId : `${r.b.by} → ${r.b.convs.length}`))

describe("Agents talking: broadcasts (card #486)", () => {
  test("one message sent to 16 agents is one row, where its newest copy was", () => {
    const names = Array.from({ length: 16 }, (_, i) => `a${i}`)
    const other = copy("zed", 100, "Rebased onto yours.", agent("dev/heron", "heron"))
    const rows = agentRows(newestFirst([...names.map((n, i) => copy(n, i * 2)), other]))
    expect(shape(rows)).toEqual(["dm:dev/heron:zed", "dev/owl → 16"])
    const b = rows[1].kind === "broadcast" ? rows[1].b : null
    expect(b?.text).toBe("WIND DOWN NOW")
    expect(b?.lastAt).toBe(iso(t0 + 30_000))
    expect(broadcastTo(b!).map((m) => m.name)).toEqual([...names].reverse())
  })

  test("a DM someone answered keeps its own row; the rest stay one", () => {
    const answered = { ...copy("a2", 4), lastText: "Done, going idle.", lastBy: "dev/a2", lastAt: iso(t0 + 60_000) }
    const rows = agentRows(newestFirst([copy("a0", 0), copy("a1", 2), answered]))
    expect(shape(rows)).toEqual([answered.convId, "dev/owl → 2"])
  })

  test("the same text much later, or from someone else, is not the same broadcast", () => {
    const later = copy("a2", broadcastWindow / 1000 + 60)
    const fromHeron = copy("a3", 1, "WIND DOWN NOW", agent("dev/heron", "heron"))
    const rows = agentRows(newestFirst([copy("a0", 0), copy("a1", 2), later, fromHeron]))
    expect(shape(rows)).toEqual([later.convId, "dev/owl → 2", fromHeron.convId])
  })

  test("a broadcast keeps its id while copies come in and recipients answer", () => {
    const before = agentRows(newestFirst([copy("a0", 0), copy("a1", 2), copy("a2", 4)]))
    const answered = { ...copy("a2", 4), lastText: "Done.", lastBy: "dev/a2", lastAt: iso(t0 + 60_000) }
    const after = agentRows(newestFirst([copy("a0", 0), copy("a1", 2), answered, copy("a3", 6)]))
    const ids = (rows: AgentRow[]) => rows.flatMap((r) => (r.kind === "broadcast" ? [r.b.id] : []))
    expect(ids(after)).toEqual(ids(before))
  })

  test("two long messages that only start alike are two rows", () => {
    const head = "Stephen asked the Mac agents to test and finish. ".repeat(8)
    const rows = agentRows(newestFirst([copy("a0", 0, `${head}take #470`), copy("a1", 2, `${head}take #471`)]))
    expect(rows.every((r) => r.kind === "dm")).toBe(true)
  })

  test("a lone copy, the person's own DMs and rows without a last line stay DMs", () => {
    const mine = { ...copy("a1", 1), readOnly: false }
    const blank = { ...copy("a2", 2), lastText: "" }
    const rows = agentRows(newestFirst([copy("a0", 0), mine, blank]))
    expect(rows.every((r) => r.kind === "dm")).toBe(true)
  })
})

describe("Agents talking: rows and the cap (card #486)", () => {
  const many = (n: number) =>
    agentRows(Array.from({ length: n }, (_, i) => copy(`x${i}`, -i, `msg ${i}`)))
  const id = (r: AgentRow) => (r.kind === "dm" ? r.c.convId : r.b.id)

  test("unfolded lists the newest few; Show more lists up to the cap and says how many are past it", () => {
    const rows = many(80)
    const some = agentSectionRows(rows, false, "", false)
    expect(some.rows.length).toBe(shownAtOnce)
    expect(some.hidden).toBe(agentsCap - shownAtOnce)
    const all = agentSectionRows(rows, false, "", true)
    expect(all.rows.length).toBe(agentsCap)
    expect(all.hidden).toBe(0)
    expect(all.capped).toBe(80 - agentsCap)
  })

  test("the open one shows wherever it is, folded or past the cap; search lists everything", () => {
    const rows = many(80)
    const deep = id(rows[70])
    expect(agentSectionRows(rows, true, deep, false).rows.map(id)).toEqual([deep])
    expect(agentSectionRows(rows, true, "", false).rows).toEqual([])
    const some = agentSectionRows(rows, false, deep, false)
    expect(some.rows.map(id).at(-1)).toBe(deep)
    expect(some.hidden).toBe(agentsCap - shownAtOnce)
    const all = agentSectionRows(rows, false, deep, true)
    expect(all.rows.map(id).at(-1)).toBe(deep)
    expect(all.capped).toBe(80 - agentsCap - 1)
    expect(agentSectionRows(rows, false, "", false, true).rows.length).toBe(80)
  })

  test("a broadcast counts as one row, and opening one of its DMs keeps it listed when folded", () => {
    const rows = agentRows(newestFirst([copy("a0", 0), copy("a1", 2)]))
    expect(rows.length).toBe(1)
    expect(agentSectionRows(rows, true, "dm:dev/owl:a0", false).rows.length).toBe(1)
  })

  test("a row's subtitle is who said what, and when", () => {
    const now = new Date(t0 + 5 * 60_000)
    const c = copy("a0", 0, "Rebased   #412\nonto yours.")
    expect(lastLine(c, now)).toEqual({ line: "owl: Rebased #412 onto yours.", when: "5m ago" })
    const you: Member = { kind: "person", key: "person:me", name: "you" }
    const mine = TalkConversation({
      convId: "d",
      kind: "dm",
      name: "heron",
      members: [you, agent("dev/heron", "heron")],
      lastAt: iso(t0),
      lastText: "Is #470 on main?",
      lastBy: you.key,
    })
    expect(lastLine(mine, now)).toEqual({ line: "You: Is #470 on main?", when: "5m ago" })
    expect(lastLine({ ...mine, lastText: "", lastAt: "" }, now)).toEqual({ line: "", when: "" })
  })
})
