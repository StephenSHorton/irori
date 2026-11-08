import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { AgentCards } from "@/components/cards/agent-hover"
import { CardSummary } from "@/lib/models"
import type { ChatEntry, Reply } from "@/lib/assistant"
import { AssistantPanel } from "./assistant-chat"

const entries: ChatEntry[] = [
  { role: "person", text: "What's blocked?" },
  {
    role: "assistant",
    text: "**Two cards wait on you.**\n\n- #1 needs your call.\n- #3 needs a click.",
  },
  { role: "assistant", text: "Stopped.", note: true },
]

function render(p: { entries?: ChatEntry[]; reply?: Reply | null }) {
  const was = globalThis.window
  globalThis.window = { location: { search: "" } } as Window & typeof globalThis
  try {
    const panel = createElement(AssistantPanel, {
      entries: p.entries ?? entries,
      reply: p.reply ?? null,
      narrow: false,
      onAsk: () => {},
      onStop: () => {},
      onNew: () => {},
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
  test("a non-modal dialog named Ask Irori, with New chat and Close", () => {
    const html = render({})
    expect(html).toContain('role="dialog"')
    expect(html).toContain('aria-label="Ask Irori"')
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
  test("while answering: the status line, the words so far and Stop", () => {
    const html = render({
      reply: {
        text: "Two cards",
        status: "Reading the Board",
        end: "",
        error: "",
      },
    })
    expect(html).toContain("Reading the Board…")
    expect(html).toContain("Two cards")
    expect(html).toContain('aria-label="Stop"')
    expect(html).not.toContain('aria-label="Send"')
  })
  test("empty, it offers questions to start with", () => {
    const html = render({ entries: [] })
    expect(html).toContain("What&#x27;s blocked?")
    expect(html).toContain("What are my agents doing?")
    expect(html).toContain('aria-label="Send"')
  })
  test("reply text is never markup", () => {
    const html = render({
      entries: [{ role: "assistant", text: "<img src=x onerror=alert(1)>" }],
    })
    expect(html).not.toContain("<img")
    expect(html).toContain("&lt;img")
  })
})
