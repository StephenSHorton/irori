// The Notifications page (card #377): every notification Irori showed the
// person, newest first, read or not, kept on every one of their devices.
// An item names what it is about; its buttons come from that thing as it
// stands now (the card's blocker, the message, the agent's ask), never
// from the notification, so they go once it's resolved anywhere. The
// words here are pure, so they're tested without a window.

import { useCallback, useEffect } from "react"

import { Events, NotificationService } from "./host"
import { inWails } from "./irori"
import { Notification, NotificationAsk, NotificationCard, State } from "./models"
import type { CardBlocker, Delivery } from "./models"
import { EventInboxChanged, useLatest } from "./talk"

export type { Notification } from "./models"

// EventNotificationsChanged is notifications:changed (crates/window
// notifications.rs): one was kept or read, here or on another device.
export const EventNotificationsChanged = "notifications:changed"
const EventBoardChanged = "board:changed"

export type NotificationFilter = "unread" | "all"

// Kind is what an item is about (crates/coord notifications.rs).
export const Kind = {
  Blocker: "blocker",
  DoneCheck: "done_check",
  Input: "input",
  Handed: "handed",
  Returned: "returned",
  Unclaimed: "unclaimed",
  Mention: "mention",
  Ask: "ask",
  Message: "message",
  Held: "held",
  Agent: "agent",
} as const

export const kindLabel: Record<string, string> = {
  blocker: "Decision",
  done_check: "Done check",
  input: "Question",
  handed: "Handed to you",
  returned: "Came back",
  unclaimed: "Nobody took it",
  mention: "Mention",
  ask: "Agent asks",
  message: "Message",
  held: "Held",
  agent: "Agent",
}

export const kindTone: Record<
  string,
  "success" | "destructive" | "warning" | "info" | "secondary"
> = {
  blocker: "destructive",
  done_check: "success",
  input: "destructive",
  handed: "warning",
  returned: "warning",
  unclaimed: "warning",
  mention: "info",
  ask: "destructive",
  message: "info",
  held: "warning",
  agent: "secondary",
}

// onCard: the item is about a card's blocker, done check or question.
export function asksOnCard(n: Notification): boolean {
  return (
    !!n.taskId &&
    (n.kind === Kind.Blocker || n.kind === Kind.DoneCheck || n.kind === Kind.Input)
  )
}

// shown is the list a filter shows, newest first as it comes.
export function shown(list: Notification[], f: NotificationFilter): Notification[] {
  return f === "unread" ? list.filter((n) => !n.read) : list
}

export function unreadCount(list: Notification[]): number {
  return list.filter((n) => !n.read).length
}

const who = (name: string | undefined) => {
  const n = (name ?? "").trim()
  return !n ? "someone" : n === "You" ? "you" : n
}

// blockerOutcome is how a resolved blocker or done check ended, in the
// card page's words: "" while it still waits on the person.
export function blockerOutcome(b: CardBlocker): string {
  const r = b.resolved
  if (!r) {
    if (b.state === "answered") return "Answered; it's with the agent now"
    return ""
  }
  const by = who(r.by?.name)
  const note = r.note?.trim()
  if (r.closed) return `Closed by ${by}${note ? `: ${note}` : ""}`
  const answer = (r.answer ?? "").trim()
  if (b.doneCheck)
    return r.option >= 0
      ? `Marked done by ${by}`
      : `Sent back by ${by}${answer ? `: “${answer}”` : ""}`
  if (r.option >= 0) return `Answered “${answer}” by ${by}`
  return `Answered by ${by}${answer ? `: “${answer}”` : ""}`
}

// blockerOpen: the blocker still waits on this person (open, or replied
// back to them, and not another member's question, as Needs you counts).
export function blockerOpen(b: CardBlocker): boolean {
  return (
    !b.resolved &&
    !b.decider &&
    (b.state === "" || b.state === "open" || b.state === "replied")
  )
}

// messageOpen: an agent's message to the person, not answered or read.
export function messageOpen(d: Delivery): boolean {
  return (
    d.state === State.Queued ||
    d.state === State.Delivered ||
    d.state === State.Engaged ||
    d.state === State.Deferred
  )
}

// deliveryOutcome says what became of a message or a held one: "" while
// it still waits on the person.
export function deliveryOutcome(d: Delivery, held: boolean): string {
  if (held) {
    if (d.state === State.Held) return ""
    if (d.state === State.Dismissed) return "Dismissed"
    if (d.state === State.Rejected) return "Refused"
    return "Released"
  }
  if (messageOpen(d)) return ""
  switch (d.state) {
    case State.Answered:
      return "Replied"
    case State.Skipped:
      return "Marked read"
    case State.Dismissed:
      return "Dismissed"
    case State.Held:
      return "Held by Irori"
    case State.Moved:
      return "Moved to another agent"
    default:
      return "Closed"
  }
}

// askOutcome says how an agent's ask ended: "" while it's open.
export function askOutcome(a: NotificationAsk): string {
  if (a.open) return ""
  if (a.answered && a.choice) return `Answered “${a.choice}”`
  if (a.answered) return "Dismissed"
  return "Closed: no answer in time"
}

// useNotifications is the history, re-read whenever it changes and on
// every card or inbox change (an item's card title follows the card).
export function useNotifications(): {
  list: Notification[]
  loading: boolean
  error: string | null
  reload: () => Promise<void>
} {
  const load = useCallback(
    async (): Promise<Notification[]> =>
      inWails() ? NotificationService.List() : sampleNotifications(),
    []
  )
  const { value, loading, error, reload } = useLatest<Notification[]>(load, [])
  useEffect(() => {
    void reload()
    if (!inWails()) {
      sampleListeners.add(reload)
      return () => {
        sampleListeners.delete(reload)
      }
    }
    const offs = [EventNotificationsChanged, EventBoardChanged, EventInboxChanged].map(
      (e) => Events.On(e, () => void reload())
    )
    const onFocus = () => void reload()
    window.addEventListener("focus", onFocus)
    return () => {
      offs.forEach((off) => off())
      window.removeEventListener("focus", onFocus)
    }
  }, [reload])
  return { list: value, loading, error, reload }
}

// readNotifications marks items read; none named reads them all.
export async function readNotifications(keys: string[]): Promise<void> {
  if (inWails()) return NotificationService.Read(keys)
  const all = keys.length === 0
  sample = sampleNotifications().map((n) =>
    all || keys.includes(n.key) ? { ...n, read: true } : n
  )
  sampleListeners.forEach((f) => void f())
}

// answerAsk answers an agent's ask from its notification; Irori refuses
// once it's answered elsewhere or out of time.
export async function answerAsk(askId: string, choice: string): Promise<void> {
  if (inWails()) return NotificationService.AnswerAsk(askId, choice)
  sample = sampleNotifications().map((n) =>
    n.kind === Kind.Ask && n.ask
      ? { ...n, ask: { ...n.ask, open: false, answered: true, choice } }
      : n
  )
  sampleListeners.forEach((f) => void f())
}

// ---- the browser preview's history ----------------------------------------

const sampleListeners = new Set<() => Promise<void>>()
let sample: Notification[] | null = null

function ago(min: number): string {
  return new Date(Date.now() - min * 60_000).toISOString()
}

function sampleNotifications(): Notification[] {
  if (new URLSearchParams(window.location.search).get("notifications") === "none")
    return []
  if (sample) return sample
  const card = (id: string, short: string, title: string) =>
    NotificationCard({ id, short, title })
  sample = [
    Notification({
      key: "done_check:sample-card-12/b1",
      kind: Kind.DoneCheck,
      title: "otter finished “Dock collider” (#12)",
      body: "Widened the dock collider by 0.4 m, so boats stop at the edge. · 1 screenshot",
      level: "success",
      at: ago(1),
      taskId: "sample-card-12",
      blockerId: "b1",
      card: card("sample-card-12", "12", "Dock collider"),
    }),
    Notification({
      key: "ask:sample-ask",
      kind: Kind.Ask,
      title: "badger: Restart the rig?",
      body: "Slot 2's Dolphin stopped answering. Restart it now? The run on it is lost.",
      level: "warn",
      at: ago(4),
      ask: NotificationAsk({ open: true, actions: ["Restart", "Leave it"], until: ago(-10) }),
    }),
    Notification({
      key: "blocker:sample-card-2/b1",
      kind: Kind.Blocker,
      title: "badger is blocked on you (#2)",
      body: "Which virtual adapter should the LAN build install?",
      level: "warn",
      at: ago(9),
      taskId: "sample-card-2",
      blockerId: "b1",
      card: card("sample-card-2", "2", "LAN play"),
    }),
    Notification({
      key: "mention:board/sample-card-1/c1",
      kind: Kind.Mention,
      title: "Alice mentioned you on “Ship v0.6.0” (wind#1)",
      body: "can you look at the release notes before we tag?",
      at: ago(14),
      read: true,
      taskId: "sample-card-1",
      boardId: "sample-board",
      card: card("sample-card-1", "1", "Ship v0.6.0"),
    }),
    Notification({
      key: "blocker:sample-card-1/b3",
      kind: Kind.Blocker,
      title: "otter is blocked on you (#1)",
      body: "Which changelog goes in the release?",
      level: "warn",
      at: ago(40),
      read: true,
      taskId: "sample-card-1",
      blockerId: "b3",
      card: card("sample-card-1", "1", "Ship v0.6.0"),
    }),
    Notification({
      key: "row:sample:9",
      kind: Kind.Agent,
      title: "heron",
      body: "The nightly build is green again.",
      level: "success",
      at: ago(95),
      read: true,
    }),
    Notification({
      key: "ask:sample-ask-old",
      kind: Kind.Ask,
      title: "otter: Push to main?",
      body: "check.sh is green on the branch.",
      at: ago(180),
      read: true,
      ask: NotificationAsk({ open: false, answered: true, choice: "Push", actions: ["Push", "Wait"] }),
    }),
  ]
  return sample
}
