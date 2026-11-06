// Card #478: Talk showed "board:b3f5…/f052…" as a DM's name, every
// sender and the composer's placeholder; cards showed the hex tail of a
// key for an agent nothing named. These render Talk's rows, posts and
// members and the cards' faces and holder line for such keys and grep the
// HTML for any key, or any piece of one.

import { describe, expect, test } from "bun:test"
import { createElement, type ReactElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { Face } from "@/components/cards/faces"
import { holderText } from "@/components/cards/bits"
import { displayName } from "@/lib/cards"
import type { Conversation, Member } from "@/lib/conversations"
import { AgentRef, Holding, TalkConversation, TalkPost } from "@/lib/models"
import { rawKey } from "@/lib/names"
import { nameLookup } from "@/lib/talk"
import { LiveRoster } from "@/lib/thread"

// Server rendering with no DOM: what the components read off window.
const g = globalThis as unknown as { window?: unknown }
g.window ??= {
  matchMedia: () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  }),
  addEventListener() {},
  removeEventListener() {},
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
}

const { Members, PostRow, statusOf, whoseWord } = await import("./conversation")
const { ConvItem } = await import("./talk-page")
const { ReplyingRow } = await import("./replying-row")
const { replyingLine } = await import("@/lib/conversations")

const board = "b3f5895439ea36e68e99533be49d066b"
const card = "f052f6f42153dfb4f70ff159e3830ce2"
const spencersKey = `board:${board}/${card}`
const you: Member = { kind: "person", key: "person:me", name: "you" }
// As the Core names it now.
const billing: Member = {
  kind: "agent",
  key: spencersKey,
  name: "BillingEng1 · Spencer",
  status: "idle",
  owner: "Spencer",
}
// What an older Core sent, or a key nothing names.
const raw: Member = {
  kind: "agent",
  key: `board:${board}/c0ffee00c0ffee00`,
  name: `board:${board}/c0ffee00c0ffee00`,
}
const nameless: Member = { kind: "agent", key: `agent:${card}` }
const goneMine: Member = {
  kind: "agent",
  key: "3dc30c94c6a74af3c59d39f4ca02d6f1/7ff86a62-6cc4-4916-be2d-602034fc281a",
  name: "",
}
const others = [billing, raw, nameless, goneMine]

const live = new Map<string, { status: string }>()

function html(el: ReactElement): string {
  return renderToStaticMarkup(
    createElement(LiveRoster.Provider, { value: live }, el)
  )
}

function dm(m: Member): Conversation {
  return TalkConversation({
    convId: `dm:${m.key.length}`,
    kind: "dm",
    name: m.name ?? m.key,
    members: [you, m],
  })
}

function noKeys(out: string) {
  const hit = out.match(rawKey)
  expect(hit?.[0] ?? null).toBeNull()
  expect(out).not.toContain(card)
  expect(out).not.toContain(board)
}

describe("no raw key reaches a person (card #478)", () => {
  test("Talk's list rows", () => {
    for (const m of others)
      noKeys(
        html(
          createElement(ConvItem, {
            c: dm(m),
            current: false,
            onClick: () => {},
          })
        )
      )
    const row = html(
      createElement(ConvItem, {
        c: dm(billing),
        current: false,
        onClick: () => {},
      })
    )
    expect(row).toContain("BillingEng1 · Spencer")
    expect(row).toContain("Spencer&#x27;s agent · idle")
  })

  test("posts from another member's agent, and their mentions", () => {
    for (const m of others) {
      const names = new Map(others.map((x) => [x.key, x]))
      const post = TalkPost({
        postId: "p",
        convId: "c",
        by: m.key,
        text: "taken",
        mentions: others.map((x) => x.key),
      })
      noKeys(html(createElement(PostRow, { post, who: m, names, live })))
    }
  })

  test("the members list", () => {
    noKeys(html(createElement(Members, { members: [you, ...others], live })))
  })

  test("cards: faces, the holder line, display names", () => {
    for (const m of others) {
      const r = AgentRef({ key: m.key, name: m.name ?? "", status: "gone" })
      expect(displayName(r)).not.toMatch(rawKey)
      noKeys(html(createElement(Face, { who: r })))
      for (const state of [
        Holding.HoldIdle,
        Holding.HoldGone,
        Holding.HoldActive,
      ])
        noKeys(
          holderText({ state, who: r, others: 0 } as Parameters<
            typeof holderText
          >[0])
        )
    }
    expect(nameLookup()(spencersKey)).toBe("unknown agent")
  })
})

describe("another member's agent isn't gone for not being on my roster (card #478)", () => {
  test("its board status, or unknown, never gone", () => {
    expect(statusOf(billing, live)).toBe("idle")
    expect(
      statusOf({ kind: "agent", key: spencersKey, owner: "Spencer" }, live)
    ).toBe("unknown")
    expect(statusOf(nameless, live)).toBe("unknown")
    expect(
      whoseWord(
        { kind: "agent", key: spencersKey, owner: "Spencer" },
        "unknown"
      )
    ).toBe("Spencer's agent")
    // One of mine that left the roster is gone, as before.
    expect(statusOf(goneMine, live)).toBe("gone")
    // Its board card says gone: it is.
    expect(statusOf({ ...billing, status: "gone" }, live)).toBe("gone")
  })
})

describe("the replying row (card #645)", () => {
  test("names its agents and never shows a key", () => {
    const since = new Date().toISOString()
    const line = replyingLine(
      others.map((m) => ({ key: m.key, name: m.name ?? "", since })),
      Date.now()
    )
    expect(line?.text).toBe("BillingEng1 · Spencer is replying…")
    const out = html(createElement(ReplyingRow, { line: line! }))
    noKeys(out)
    expect(out).toContain("is replying…")
  })
})
