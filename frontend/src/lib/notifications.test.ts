import { describe, expect, test } from "bun:test"

import {
  CardBlocker,
  CardResolution,
  Notification,
  NotificationAsk,
  AgentRef,
  Delivery,
  Message,
} from "./models"
import {
  askOutcome,
  asksOnCard,
  blockerOpen,
  blockerOutcome,
  deliveryOutcome,
  shown,
  unreadCount,
  waitedSummary,
} from "./notifications"

describe("the messages that waited (card #482)", () => {
  test("the summary card is one row for several messages, old rows too", () => {
    expect(waitedSummary(Notification({ kind: "waited", deliveryId: "d2" }))).toBe(true)
    expect(waitedSummary(Notification({ kind: "message", deliveryId: "waiting" }))).toBe(true)
    expect(waitedSummary(Notification({ kind: "message", deliveryId: "d1" }))).toBe(false)
  })
})

const you = AgentRef({ key: "user:dev", name: "You" })
const civet = AgentRef({ key: "dev/abc", name: "civet" })

describe("the list (card #377)", () => {
  const list = [
    Notification({ key: "a", read: false }),
    Notification({ key: "b", read: true }),
    Notification({ key: "c", read: false }),
  ]
  test("Unread shows only unread ones, All every one, in order", () => {
    expect(shown(list, "unread").map((n) => n.key)).toEqual(["a", "c"])
    expect(shown(list, "all").map((n) => n.key)).toEqual(["a", "b", "c"])
    expect(unreadCount(list)).toBe(2)
  })
  test("only a card's question or done check reads the card's blocker", () => {
    expect(asksOnCard(Notification({ kind: "blocker", taskId: "T" }))).toBe(true)
    expect(asksOnCard(Notification({ kind: "done_check", taskId: "T" }))).toBe(true)
    expect(asksOnCard(Notification({ kind: "mention", taskId: "T" }))).toBe(false)
    expect(asksOnCard(Notification({ kind: "blocker" }))).toBe(false)
  })
})

describe("a card's blocker as it stands now", () => {
  test("open keeps its buttons; another member's question doesn't", () => {
    expect(blockerOpen(CardBlocker({ id: "b1", state: "open" }))).toBe(true)
    expect(blockerOpen(CardBlocker({ id: "b1", state: "replied" }))).toBe(true)
    expect(blockerOpen(CardBlocker({ id: "b1", state: "open", decider: civet }))).toBe(false)
  })
  test("a wait on another card has no buttons (card #481)", () => {
    const wait = CardBlocker({
      id: "b1",
      state: "open",
      waitsOn: { kind: "task", label: "#392", taskId: "t392", title: "Talk 4" },
    })
    expect(blockerOpen(wait)).toBe(false)
  })
  test("resolved by anyone: no buttons, the outcome and who", () => {
    const done = CardBlocker({
      doneCheck: true,
      state: "resolved",
      resolved: CardResolution({ answer: "Done", option: 0, by: you, at: "2026-11-03T16:12:00Z" }),
    })
    expect(blockerOpen(done)).toBe(false)
    expect(blockerOutcome(done)).toBe("Marked done by you")
    const back = CardBlocker({
      doneCheck: true,
      resolved: CardResolution({ answer: "the far dock clips", option: -1, by: you }),
    })
    expect(blockerOutcome(back)).toBe("Sent back by you: “the far dock clips”")
    // Card #476: the agent's next check replaced it; the reason is yours.
    const again = CardBlocker({
      doneCheck: true,
      resolved: CardResolution({
        answer: "the far dock clips",
        option: -1,
        by: civet,
        note: "asked again: ready for the person",
      }),
    })
    expect(blockerOutcome(again)).toBe(
      "You sent it back: “the far dock clips”; civet asked again"
    )
    const picked = CardBlocker({
      resolved: CardResolution({ answer: "Wintun", option: 0, by: civet }),
    })
    expect(blockerOutcome(picked)).toBe("Answered “Wintun” by civet")
    const closed = CardBlocker({
      resolved: CardResolution({ option: -1, by: civet, closed: true, note: "the lease came back" }),
    })
    expect(blockerOutcome(closed)).toBe("Closed by civet: the lease came back")
    expect(blockerOutcome(CardBlocker({ state: "answered" }))).toBe(
      "Answered; it's with the agent now"
    )
  })
})

describe("messages and asks", () => {
  const d = (state: string) =>
    Delivery({ id: "d", state: state as Delivery["state"], message: Message({}) })
  test("a message to the person waits until replied or read", () => {
    expect(deliveryOutcome(d("delivered"), false)).toBe("")
    expect(deliveryOutcome(d("answered"), false)).toBe("Replied")
    expect(deliveryOutcome(d("skipped"), false)).toBe("Marked read")
  })
  test("a held message waits until released or dismissed", () => {
    expect(deliveryOutcome(d("held"), true)).toBe("")
    expect(deliveryOutcome(d("delivered"), true)).toBe("Released")
    expect(deliveryOutcome(d("dismissed"), true)).toBe("Dismissed")
  })
  test("an agent's ask: open, answered, or out of time", () => {
    expect(askOutcome(NotificationAsk({ open: true }))).toBe("")
    expect(askOutcome(NotificationAsk({ answered: true, choice: "Restart" }))).toBe(
      "Answered “Restart”"
    )
    expect(askOutcome(NotificationAsk({}))).toBe("Closed: no answer in time")
  })
})
