import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import type { ThreadSummary } from "@/lib/cards"
import { needsYou, talkMatches, ThreadsSection } from "./talk-page"

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
    expect(talkMatches(t({ lastText: "it was the linker" }), "linker")).toBe(
      true
    )
    expect(talkMatches(t({}), "lobby")).toBe(false)
  })
  test("an empty search shows everything", () => {
    expect(talkMatches(t({}), "  ")).toBe(true)
  })
})

describe("Talk's count in the sidebar (card #390, #719)", () => {
  test("counts threads with an agent reply the person hasn't read", () => {
    expect(needsYou({ unread: true })).toBe(true)
    expect(needsYou({ unread: false })).toBe(false)
  })
  test("and threads no agent can start for", () => {
    expect(needsYou({ unread: false, noAgent: "No agent can start" })).toBe(
      true
    )
  })
})

import { channelName, convMatches } from "./talk-page"
import { agentStatus } from "@/lib/agent-status"
import { dayLabel, mentionKinds, statusOf } from "./conversation"
import { TalkConversation } from "@/lib/models"

describe("channels and DMs in Talk (card #390)", () => {
  test("a new channel's name is cleaned: lowercase, no # or spaces", () => {
    expect(channelName("  #Release Notes! ")).toBe("release-notes")
    expect(channelName("###")).toBe("")
  })
  test("search matches a channel's name or topic, or a DM member's name", () => {
    const c = TalkConversation({
      kind: "dm",
      name: "dm",
      topic: "the move",
      members: [{ kind: "agent", key: "k", name: "kowhai" }],
    })
    expect(convMatches(c, "KOW")).toBe(true)
    expect(convMatches(c, "move")).toBe(true)
    expect(convMatches(c, "lobby")).toBe(false)
  })
  test("a line between days says Today, Yesterday, or the date", () => {
    const now = new Date(2026, 10, 3, 15, 0)
    expect(dayLabel(new Date(2026, 10, 3, 1, 0), now)).toBe("Today")
    expect(dayLabel(new Date(2026, 10, 2, 23, 0), now)).toBe("Yesterday")
    expect(dayLabel(new Date(2026, 9, 30, 9, 0), now)).not.toMatch(
      /Today|Yesterday/
    )
  })
  test("members read their status, gone when off the roster (card #489)", () => {
    const say = (status: string) => agentStatus({ status }).text
    const live = new Map([
      ["d/oak", { status: "shell" }],
      ["d/elm", { status: "unresponsive" }],
      ["d/fir", { status: "starting" }],
    ])
    const agent = (key: string) => say(statusOf({ kind: "agent", key }, live))
    expect(say("busy")).toBe("working")
    expect(say("idle")).toBe("idle")
    expect(say(statusOf({ kind: "person", key: "person:p1" }, live))).toBe("")
    expect(agent("d/oak")).toBe("running a job")
    expect(agent("d/elm")).toBe("not answering")
    expect(agent("d/fir")).toBe("starting")
    expect(agent("d/ash")).toBe("gone")
  })
  test("a person's key names a person, the rest are agents", () => {
    expect(
      mentionKinds([
        { key: "person:p1", name: "Blake" },
        { key: "dev/s1", name: "oak" },
      ])
    ).toEqual([{ kind: "person" }, { kind: "agent" }])
  })
})

import { threadText } from "./conversation"

describe("a thread started from a channel message (design b2, card #390)", () => {
  test("asks the question, then quotes the message and says where it was said", () => {
    expect(
      threadText(
        " Why the Firewall? ",
        "kowhai",
        "Only the PC transfer.\nOne click.",
        "suzuri"
      )
    ).toBe(
      "Why the Firewall?\n\n> Only the PC transfer.\n> One click.\n(kowhai, in #suzuri)"
    )
  })
})

// Card #558: a space's thread is found by the ref the channel pointer says.
describe("talkMatches with a tag", () => {
  test("mlg#12 finds a space's thread, and #12 still does", () => {
    expect(talkMatches(t({ tag: "mlg" }), "mlg#12")).toBe(true)
    expect(talkMatches(t({ tag: "mlg" }), "#12")).toBe(true)
    expect(talkMatches(t({ tag: "" }), "mlg#12")).toBe(false)
  })
})

describe("Resolved threads sit at the bottom of Threads (card #687)", () => {
  const section = (
    live: ThreadSummary[],
    resolved: ThreadSummary[],
    p: { folded?: boolean; showResolved?: boolean; loading?: boolean } = {}
  ) => {
    // Loading's preview flag reads the page's address.
    const was = globalThis.window
    globalThis.window = { location: { search: "" } } as Window &
      typeof globalThis
    try {
      return renderToStaticMarkup(
        createElement(ThreadsSection, {
          live,
          resolved,
          loading: !!p.loading,
          folded: !!p.folded,
          onFold: () => {},
          searching: false,
          open: "",
          why: "",
          onOpen: () => {},
          showResolved: !!p.showResolved,
          onToggleResolved: () => {},
        })
      )
    } finally {
      globalThis.window = was
    }
  }
  const done = t({ id: "d", short: "9", title: "Old ask", state: "resolved" })
  const at = (html: string, s: string) => html.indexOf(s)

  test("the toggle comes after the open threads, inside Threads", () => {
    const html = section([t({ id: "a", title: "Live ask" })], [done])
    expect(at(html, "Threads")).toBeGreaterThan(-1)
    expect(at(html, "Live ask")).toBeGreaterThan(at(html, "Threads"))
    expect(at(html, "Resolved threads: 1")).toBeGreaterThan(
      at(html, "Live ask")
    )
    expect(html).not.toContain("mt-auto")
  })
  test("unfolded, its rows follow the toggle", () => {
    const html = section([], [done], { showResolved: true })
    expect(at(html, "Old ask")).toBeGreaterThan(at(html, "Resolved threads: 1"))
  })
  test("with no open threads it sits under that line", () => {
    const html = section([], [done])
    expect(at(html, "No open threads")).toBeGreaterThan(-1)
    expect(at(html, "Resolved threads: 1")).toBeGreaterThan(
      at(html, "No open threads")
    )
  })
  test("folding Threads hides it", () => {
    const html = section([t({ title: "Live ask" })], [done], { folded: true })
    expect(html).toContain("Threads")
    expect(html).not.toContain("Resolved threads")
    expect(html).not.toContain("Live ask")
  })
  test("while Threads loads, its skeleton shows without the toggle", () => {
    expect(section([], [done], { loading: true })).not.toContain(
      "Resolved threads"
    )
  })
  test("no resolved threads, no toggle", () => {
    expect(section([t({})], [])).not.toContain("Resolved threads")
  })
})
