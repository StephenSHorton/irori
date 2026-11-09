// The Notifications page (card #377): every notification Irori showed the
// person, newest first, read or not, kept on every one of their devices.
// An item names what it is about; its buttons come from that thing as it
// stands now (the card's blocker, the message, the agent's ask), never
// from the notification, so they go once it's resolved anywhere. The
// words here are pure, so they're tested without a window.

import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react"

import { onChange } from "./changes"
import { NotificationService } from "./host"
import { inWails } from "./irori"
import {
  Notification,
  NotificationAsk,
  NotificationCard,
  State,
} from "./models"
import type { CardBlocker, Delivery } from "./models"
import { pending } from "./pending"
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
  // The one card for several messages that waited (card #482).
  Waited: "waited",
  // No device can start an agent for a card or thread (card #541).
  NoAgent: "no_agent",
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
  waited: "Messages",
  no_agent: "No agent",
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
  waited: "info",
  no_agent: "warning",
}

// onCard: the item is about a card's blocker, done check or question.
export function asksOnCard(n: Notification): boolean {
  return (
    !!n.taskId &&
    (n.kind === Kind.Blocker ||
      n.kind === Kind.DoneCheck ||
      n.kind === Kind.Input)
  )
}

// waitedSummary: the item is the "N messages waited for you" card, which
// stands for several messages, not one (card #482). Before #482 it was a
// message about delivery "waiting".
export function waitedSummary(n: Notification): boolean {
  return (
    n.kind === Kind.Waited ||
    (n.kind === Kind.Message && n.deliveryId === "waiting")
  )
}

// shown is the list a filter shows, newest first as it comes.
export function shown(
  list: Notification[],
  f: NotificationFilter
): Notification[] {
  return f === "unread" ? list.filter((n) => !n.read) : list
}

export function unreadCount(list: Notification[]): number {
  return list.filter((n) => !n.read).length
}

const who = (name: string | undefined) => {
  const n = (name ?? "").trim()
  return !n ? "someone" : n === "You" ? "you" : n
}

// How an agent's next done check resolves the one it replaces (coord's
// ASKED_AGAIN).
export const AskedAgain = "asked again: ready for the person"

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
  // The agent's next done check replaced this one (card #476): the reason
  // on it is the person's, not the agent's.
  if (b.doneCheck && note === AskedAgain)
    return answer && answer !== AskedAgain
      ? `You sent it back: “${answer}”; ${by} asked again`
      : `${by} asked again`
  if (b.doneCheck)
    return r.option >= 0
      ? `Marked done by ${by}`
      : `Sent back by ${by}${answer ? `: “${answer}”` : ""}`
  if (r.option >= 0) return `Answered “${answer}” by ${by}`
  return `Answered by ${by}${answer ? `: “${answer}”` : ""}`
}

// blockerOpen: the blocker still waits on this person (open, or replied
// back to them, and not another member's question nor a wait on other
// work (card #481), as Needs you counts).
export function blockerOpen(b: CardBlocker): boolean {
  return (
    !b.resolved &&
    !b.decider &&
    !b.waitsOn &&
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
// every card or inbox change (an item's card title follows the card),
// with the person's pending answers on it (card #644).
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
  const { value, loading, error, reload } = useLatest<Notification[]>(
    load,
    [],
    "notifications"
  )
  // A read ends the answers it agrees with once it is shown (useCards).
  useEffect(() => {
    if (!loading && !error) pending.observeNotifications(value)
  }, [value, loading, error])
  const v = useSyncExternalStore(pending.subscribe, pending.version)
  const list = useMemo(
    () => pending.notifications(value),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- v is when pending changes
    [value, v]
  )
  useEffect(() => {
    void reload()
    if (!inWails()) {
      sampleListeners.add(reload)
      return () => {
        sampleListeners.delete(reload)
      }
    }
    const offs = (
      [EventNotificationsChanged, EventBoardChanged, EventInboxChanged] as const
    ).map((e, i) => onChange(e, () => void reload(), { focus: i === 0 }))
    return () => offs.forEach((off) => off())
  }, [reload])
  return { list, loading, error, reload }
}

// useUnreadNotifications is the sidebar's badge: just the count, re-read
// when notifications change or the window comes back (0.11.1; the whole
// list used to be rebuilt on every card or inbox change for it).
export function useUnreadNotifications(): number {
  const load = useCallback(
    async (): Promise<number> =>
      inWails()
        ? NotificationService.Unread()
        : unreadCount(sampleNotifications()),
    []
  )
  const { value, reload } = useLatest<number>(load, 0, "unread")
  useEffect(() => {
    void reload()
    if (!inWails()) {
      sampleListeners.add(reload)
      return () => {
        sampleListeners.delete(reload)
      }
    }
    return onChange(EventNotificationsChanged, () => void reload(), {
      focus: true,
    })
  }, [reload])
  return value
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
  if (
    new URLSearchParams(window.location.search).get("notifications") === "none"
  )
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
      ask: NotificationAsk({
        open: true,
        actions: ["Restart", "Leave it"],
        until: ago(-10),
      }),
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
      body: "The tests are green on the branch.",
      at: ago(180),
      read: true,
      ask: NotificationAsk({
        open: false,
        answered: true,
        choice: "Push",
        actions: ["Push", "Wait"],
      }),
    }),
  ]
  return sample
}
