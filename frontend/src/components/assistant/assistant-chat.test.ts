import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { AgentCards } from "@/components/cards/agent-hover"
import { CardSummary } from "@/lib/models"
import type { AssistantHomeStatus, ChatEntry, Reply } from "@/lib/assistant"
import { AssistantPanel } from "./assistant-chat"

const entries: ChatEntry[] = [
  { role: "person", text: "What's blocked?" },
  {
    role: "assistant",
    text: "**Two cards wait on you.**\n\n- #1 needs your call.\n- #3 needs a click.",
  },
  { role: "assistant", text: "Stopped.", note: true },
]

function render(p: {
  entries?: ChatEntry[]
  streams?: Record<string, Reply>
  home?: AssistantHomeStatus | null
}) {
  const was = globalThis.window
  globalThis.window = { location: { search: "" } } as Window & typeof globalThis
  try {
    const panel = createElement(AssistantPanel, {
      entries: p.entries ?? entries,
      streams: p.streams ?? {},
      home: p.home ?? null,
      narrow: false,
      onAsk: () => {},
      onAskAgain: () => {},
      onUsePc: () => Promise.resolve(""),
      onStop: () => {},
      onNew: () => {},
      onGo: () => Promise.resolve(""),
      onPost: () => Promise.resolve(),
      onClose: () => {},
    })
    return renderToStaticMarkup(
      createElement(
        AgentCards.Provider,
        {
          value: {
            cards: [
              CardSummary({ id: "c1", short: "1", title: "Ship the release" }),
            ],
            onOpen: () => {},
          },
        },
        panel
      )
    )
  } finally {
    globalThis.window = was
  }
}

describe("Ask Irori's panel (card #783)", () => {
  test("a followed thread's line has no reply box (card #1000)", () => {
    const html = render({
      entries: [
        {
          role: "assistant",
          text: "#21 is waiting for your permission: Run it?",
          update: "task:1",
        },
      ],
    })
    expect(html).toContain("data-assistant-update")
    expect(html).toContain("waiting for your permission")
    expect(html).not.toContain("data-assistant-reply")
    expect(html).not.toContain("Reply to")
  })
  test("a non-modal dialog named Office Assistant, with New chat and Close", () => {
    const html = render({})
    expect(html).toContain('role="dialog"')
    expect(html).toContain('aria-label="Office Assistant"')
    expect(html).not.toContain("aria-modal")
    expect(html).toContain('aria-label="New chat"')
    expect(html).toContain('aria-label="Close"')
    expect(html).toContain("Ask about your cards, threads and agents")
  })
  test("the person's words on the right, the reply's bold, list and refs", () => {
    const html = render({})
    expect(html).toContain('data-align="end"')
    expect(html).toContain("What&#x27;s blocked?")
    expect(html).toContain(
      '<strong class="font-semibold">Two cards wait on you.</strong>'
    )
    expect(html).not.toContain("**")
    expect((html.match(/<li/g) ?? []).length).toBe(2)
    // #1 is a card the window holds: a chip that opens it.
    expect(html).toContain("Ship the release")
    expect(html).toContain("Stopped.")
  })
  const waiting: ChatEntry[] = [
    {
      id: "q:t1",
      role: "person",
      text: "What's blocked?",
      turn: "t1",
      pending: true,
    },
    {
      id: "q:t2",
      role: "person",
      text: "And the lobby?",
      turn: "t2",
      pending: true,
    },
  ]
  const mac = (here: boolean, state = "connected"): AssistantHomeStatus => ({
    device: "mac",
    name: "Mac",
    here,
    state,
    lastSeenMs: 0,
    claude: true,
    chat: true,
    none: "",
    candidates: [],
  })
  test("while answering: the status line and the words so far under the question, with Stop, and the composer stays on", () => {
    const html = render({
      entries: waiting.slice(0, 1),
      streams: {
        t1: {
          text: "Two cards",
          status: "Reading the Board",
          end: "",
          error: "",
          looked: "",
        },
      },
      home: mac(true),
    })
    expect(html).toContain("Reading the Board…")
    expect(html).toContain("Two cards")
    expect(html).toContain('aria-label="Stop this question"')
    expect(html).toContain('aria-label="Send"')
  })
  test("on another device: the oldest is being answered, the rest wait", () => {
    const html = render({ entries: waiting, home: mac(false) })
    expect(html).toContain("Mac is answering…")
    expect(html).toContain("Waiting for Mac")
    expect((html.match(/Stop this question/g) ?? []).length).toBe(2)
  })
  test("when the computer isn't connected, every question waits for it", () => {
    const html = render({ entries: waiting, home: mac(false, "offline") })
    expect(html).not.toContain("is answering")
    expect((html.match(/Waiting for Mac/g) ?? []).length).toBe(2)
  })
  describe("when the computer is away (card 5, design §6)", () => {
    const pc = {
      device: "pc",
      name: "PC",
      here: false,
      state: "connected",
      lastSeenMs: 0,
    }
    const away = (over: Partial<AssistantHomeStatus>): AssistantHomeStatus => ({
      ...mac(false, "offline"),
      lastSeenMs: Date.now() - 2 * 3600_000,
      candidates: [pc],
      ...over,
    })
    const none = (n: string): AssistantHomeStatus => ({
      device: "",
      name: "",
      here: false,
      state: "",
      lastSeenMs: 0,
      claude: false,
      chat: false,
      none: n,
      candidates: [],
    })
    const composerOn = (html: string) =>
      !html.includes("readOnly") && !/aria-label="Send"[^>]*disabled/.test(html)
    test("connected: the header says where, and nothing extra", () => {
      const html = render({ home: mac(false) })
      expect(html).toContain("on Mac")
      expect(html).toContain('data-dot="connected"')
      expect(html).not.toContain("data-assistant-away")
    })
    test("offline: the away line, Use PC instead, and the box takes a question", () => {
      const html = render({ home: away({}) })
      expect(html).toContain('data-dot="offline"')
      expect(html).toContain(
        "Your assistant runs on Mac, which isn&#x27;t connected (seen 2h ago). Questions wait for it."
      )
      expect(html).toContain("Use PC instead")
      expect(html).toContain("Ask anything…")
      expect(html).not.toContain("readOnly")
    })
    test("no other computer can answer: no Use PC instead", () => {
      const html = render({ home: away({ candidates: [] }) })
      expect(html).toContain("which isn&#x27;t connected")
      expect(html).not.toContain("Use ")
    })
    test("an offline PC is not offered", () => {
      const html = render({
        home: away({ candidates: [{ ...pc, state: "offline" }] }),
      })
      expect(html).not.toContain("instead")
    })
    test("connected without Claude Code", () => {
      const html = render({
        home: away({ state: "connected", claude: false }),
      })
      expect(html).toContain(
        "which can&#x27;t find Claude Code. Install it there."
      )
      expect(html).toContain("Use PC instead")
      expect(html).toContain("Ask anything…")
    })
    test("connected, an older Irori", () => {
      const html = render({ home: away({ state: "connected", chat: false }) })
      expect(html).toContain(
        "Mac needs the latest Irori to answer from your other devices."
      )
      expect(html).toContain("Use PC instead")
      expect(html).toContain("Ask anything…")
    })
    test("none, setting up: the box takes a question", () => {
      const html = render({
        home: {
          ...none("setting_up"),
          candidates: [{ ...pc, name: "Mac" }],
        },
      })
      expect(html).toContain("Setting up on Mac…")
      expect(html).toContain("Ask anything…")
    })
    test("none, needs the latest Irori: the box is off", () => {
      const html = render({ home: none("needs_update") })
      expect(html).toContain("Update Irori on the one that has Claude Code")
      expect(html).toContain("readOnly")
      expect(html).toMatch(/aria-label="Send"[^>]*disabled/)
      expect(composerOn(html)).toBe(false)
    })
    test("none, no Claude Code anywhere: the box is off", () => {
      const html = render({ home: none("no_claude") })
      expect(html).toContain(
        "Your assistant needs a computer with Claude Code. None of your devices has it yet."
      )
      expect(composerOn(html)).toBe(false)
    })
    test("on the assistant's computer itself: nothing extra", () => {
      const html = render({ home: mac(true) })
      expect(html).not.toContain("data-assistant-away")
    })
    test("an error turn offers Ask again", () => {
      const html = render({
        entries: [
          { id: "q:t1", role: "person", text: "Close #5", turn: "t1" },
          {
            id: "e:t1",
            role: "assistant",
            text: "Claude hit a usage limit.",
            note: true,
            turn: "t1",
            retry: true,
          },
        ],
      })
      expect(html).toContain("Ask again")
    })
  })
  test("a skipped or stopped turn offers Ask again", () => {
    const html = render({
      entries: [
        { id: "q:t1", role: "person", text: "Close #5", turn: "t1" },
        {
          id: "e:t1",
          role: "assistant",
          text: "Stopped.",
          note: true,
          turn: "t1",
          retry: true,
        },
      ],
    })
    expect(html).toContain("Ask again")
  })
  test("empty, it offers questions to start with", () => {
    const html = render({ entries: [] })
    expect(html).toContain("What&#x27;s blocked?")
    expect(html).toContain("What are my agents doing?")
    expect(html).toContain('aria-label="Send"')
  })
  test("a message of files alone has no empty text line (card #999)", () => {
    const html = render({
      entries: [
        {
          role: "person",
          text: "",
          files: [
            {
              sha256: "a".repeat(64),
              name: "report.pdf",
              size: 2048,
              mime: "application/pdf",
              device: "d1",
            },
          ],
        },
      ],
    })
    expect(html).toContain("report.pdf")
    expect(html).not.toContain('data-slot="bubble-content"')
  })
  test("the box has the clip (card #999)", () => {
    expect(render({})).toContain('aria-label="Add files"')
  })
  test("reply text is never markup", () => {
    const html = render({
      entries: [{ role: "assistant", text: "<img src=x onerror=alert(1)>" }],
    })
    expect(html).not.toContain("<img")
    expect(html).toContain("&lt;img")
  })
  test("a go line shows as a button, never as its raw line (card #784)", () => {
    const html = render({
      entries: [
        {
          role: "assistant",
          text: "It's the **Stop here** button in the top bar.\n\n<<go setting stop-here>>",
        },
      ],
    })
    expect(html).toContain("data-assistant-go")
    expect(html).toContain("Show Stop here")
    expect(html).not.toContain("&lt;&lt;go")
  })
  test("while answering, a half-written go line doesn't show (card #784)", () => {
    const html = render({
      entries: [
        { id: "q:t1", role: "person", text: "q", turn: "t1", pending: true },
      ],
      streams: {
        t1: {
          text: "It's #779.\n\n<<go car",
          status: "",
          end: "",
          error: "",
          looked: "",
        },
      },
    })
    expect(html).not.toContain("&lt;&lt;")
    expect(html).not.toContain("data-assistant-go")
  })
})
