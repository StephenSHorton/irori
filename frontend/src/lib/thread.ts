// A card's thread (BoardService.Card's thread): comments, messages about
// the card and what happened, oldest first. These helpers are pure, so
// the thread view and its tests share them.

import { displayName, type AgentRef, type CardThreadItem } from "./cards"

// ThreadPage is how many of the newest entries show before "Show earlier".
export const ThreadPage = 50

// ThreadKind mirrors the Go constants on CardThreadItem.kind.
export const ThreadKind = {
  Comment: "comment",
  Message: "message",
  Event: "event",
} as const

export type ThreadBadge = { label: string; tone: "warning" | "slate" }

// ThreadLabel is an entry's header: who wrote it, who a message went to,
// the device it was written on ("" for this one) and its badges.
export type ThreadLabel = {
  author: string
  arrow?: string
  device: string
  badges: ThreadBadge[]
}

// threadLabel is the header line for one entry: "A -> B" for a message,
// with "recipient gone" when Irori left it on the card for an ended
// recipient, "held" until the wake policy lets it through (its text isn't
// on the card), and the count of files that stayed on the message.
export function threadLabel(item: CardThreadItem): ThreadLabel {
  const badges: ThreadBadge[] = []
  if (item.gone) badges.push({ label: "recipient gone", tone: "warning" })
  if (item.held) badges.push({ label: "held", tone: "slate" })
  if (item.files)
    badges.push({
      label: item.files === 1 ? "1 file" : `${item.files} files`,
      tone: "slate",
    })
  const to: AgentRef | null | undefined = item.to
  return {
    author: displayName(item.who),
    arrow: to ? displayName(to) : undefined,
    device: item.device ?? "",
    badges,
  }
}

// visible is the entries to show: all of them, or the newest n.
export function visible<T>(items: T[], showAll: boolean, n = ThreadPage): T[] {
  if (showAll || items.length <= n) return items
  return items.slice(items.length - n)
}

// opensInFull reports whether a message entry has more than the card
// shows: its text was cut (it ends pointing at msg:<id>), it hasn't been
// let through the wake policy (no text yet), or it has files.
// BoardService.Message opens the rest for the person.
export function opensInFull(item: CardThreadItem): boolean {
  if (item.kind !== ThreadKind.Message) return false
  return (
    !!item.held ||
    !!item.files ||
    (item.text ?? "").endsWith(`(full: msg:${item.id})`)
  )
}
