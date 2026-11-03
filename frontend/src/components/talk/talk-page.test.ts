import { describe, expect, test } from "bun:test"

import type { ThreadSummary } from "@/lib/cards"
import { needsYou, talkMatches } from "./talk-page"

const t = (p: Partial<ThreadSummary>): ThreadSummary =>
  ({
    id: "x",
    short: "12",
    title: "Why is the Mac build slow?",
    text: "It takes ten minutes since Tuesday",
    state: "waiting",
    told: [],
    answeredBy: [],
    replies: 0,
    cards: [],
    at: "",
    updatedAt: "",
    ...p,
  }) as ThreadSummary

describe("Talk's list search (card #390)", () => {
  test("matches the title, the question, the #ref and the newest reply", () => {
    expect(talkMatches(t({}), "mac build")).toBe(true)
    expect(talkMatches(t({}), "TUESDAY")).toBe(true)
    expect(talkMatches(t({}), "#12")).toBe(true)
    expect(talkMatches(t({ lastText: "it was the linker" }), "linker")).toBe(true)
    expect(talkMatches(t({}), "lobby")).toBe(false)
  })
  test("an empty search shows everything", () => {
    expect(talkMatches(t({}), "  ")).toBe(true)
  })
})

describe("Talk's count in the sidebar (card #390)", () => {
  test("counts only threads an agent answered, waiting on the person", () => {
    expect(needsYou({ state: "answered" })).toBe(true)
    for (const state of ["waiting", "handed", "resolved"])
      expect(needsYou({ state })).toBe(false)
  })
})
