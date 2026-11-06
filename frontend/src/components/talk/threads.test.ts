import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { AgentRef, ThreadSummary } from "@/gen/models"
import type { ThreadCard } from "@/gen/models"
import {
  openedLine,
  ThreadCards,
  emptyOpened,
  openAfterResolve,
  resolvedShown,
  resolvedToggle,
  stateLine,
} from "./threads"

const threads = [
  { id: "live", state: "open" },
  { id: "done", state: "resolved" },
] as const

describe("the Resolved fold (cards #144, #194)", () => {
  test("a #ref to a resolved thread unfolds the list", () => {
    expect(resolvedShown(false, "done", [...threads])).toBe(true)
    expect(resolvedShown(false, "live", [...threads])).toBe(false)
    expect(resolvedShown(false, "", [...threads])).toBe(false)
  })
  test("folding with a resolved thread open closes it, so the list folds", () => {
    const t = resolvedToggle(false, "done", [...threads])
    expect(t).toEqual({ pick: false, closeOpen: true })
    expect(resolvedShown(t.pick, "", [...threads])).toBe(false)
  })
  test("folding the person's own unfold keeps an open live thread", () => {
    expect(resolvedToggle(true, "live", [...threads])).toEqual({
      pick: false,
      closeOpen: false,
    })
  })
  test("unfolding sets the pick", () => {
    expect(resolvedToggle(false, "", [...threads])).toEqual({
      pick: true,
      closeOpen: false,
    })
    expect(resolvedToggle(false, "live", [...threads])).toEqual({
      pick: true,
      closeOpen: false,
    })
  })
})

describe("resolving a thread (card #194, Stephen #200)", () => {
  test("resolving the open thread closes it, so Resolved stays folded", () => {
    const open = openAfterResolve("done", "done")
    expect(open).toBe("")
    // the thread is resolved now; nothing unfolds the list
    expect(resolvedShown(false, open, [...threads])).toBe(false)
  })
  test("resolving another thread keeps the open one", () => {
    expect(openAfterResolve("live", "done")).toBe("live")
  })
})

describe("an open thread stays inside the column (card #215)", () => {
  // A long card title must truncate, not widen the thread: the chip row
  // may shrink below its content and the title sits in a truncating span.
  test("the card chip row can shrink and its title truncates", () => {
    const html = renderToStaticMarkup(
      createElement(ThreadCards, {
        cards: [
          {
            id: "c",
            short: "3",
            title:
              "Lobby list: pick Steam or our own relay for the public lobbies before the release",
            state: "working",
          } as ThreadCard,
        ],
      })
    )
    expect(html).toMatch(/^<div class="[^"]*\bmin-w-0\b/)
    expect(html).toMatch(/<span class="truncate">Lobby list/)
  })
})

describe("a thread Irori starts an agent for (card #470)", () => {
  const waiting = {
    id: "t",
    short: "27",
    title: "What did we ship?",
    text: "What did we ship?",
    state: "waiting",
    told: [],
    answeredBy: [],
    replies: 0,
    cards: [],
    at: "",
    updatedAt: "",
    private: false,
    agentsNow: true,
  } as unknown as ThreadSummary
  test("says its agent is starting, not to turn on Offer it cards", () => {
    expect(
      stateLine(
        { ...waiting, starting: "Macbook Pro (this one)" },
        "No agent takes offered work"
      )
    ).toBe("starting an agent on Macbook Pro (this one)")
  })
  test("says why one can't start", () => {
    expect(stateLine({ ...waiting, cantStart: "not trusted: /r on mac" })).toBe(
      "can't start: its repo isn't trusted here"
    )
  })
  test("without a spawn it reads as before", () => {
    expect(stateLine(waiting, "")).toBe("Waiting for an agent")
  })
  test("a follow-up no agent can start for says why (card #541)", () => {
    const why =
      "No agent can start: Mac is offline. It starts by itself when it's back."
    expect(
      stateLine({
        ...waiting,
        state: "answered",
        lastBy: "user:x",
        lastText: "And?",
        noAgent: why,
      })
    ).toBe(why)
    expect(stateLine({ ...waiting, noAgent: why })).toBe(why)
  })
})

describe("a thread an agent opened (card #631)", () => {
  const alpha = AgentRef({ key: "dev/a", name: "alpha", status: "idle" })
  const opened = ThreadSummary({
    id: "t1",
    short: "31",
    title: "Which fix?",
    text: "Which fix?",
    state: "answered",
    lastBy: alpha,
    lastText: "Which fix?",
    agentsNow: true,
    openedBy: alpha,
  })
  test("its line is the agent's words, waiting on the person", () => {
    expect(stateLine(opened)).toBe("alpha: Which fix?")
  })
  test("it says who opened it, and that no other agent starts", () => {
    expect(openedLine(alpha)).toBe("alpha opened this thread")
    expect(emptyOpened(alpha)).toBe(
      "No replies yet. alpha hears yours; no other agent is started for it until you write."
    )
  })
})
