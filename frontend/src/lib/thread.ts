// A card's thread (BoardService.Card's thread): comments, messages about
// the card and what happened, oldest first. These helpers are pure, so
// the thread view and its tests share them.

import {
  BlockerState,
  displayName,
  type AgentRef,
  type CardBlocker,
  type CardThreadItem,
} from "./cards"

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
// recipient, "nobody has heard this yet" on the person's comment that no
// agent was around to hear (until an agent writes on the card), "held" until the wake policy lets it through (its text isn't
// on the card), "held · dismissed" once the person dismissed it, and the count of files that stayed on the message. An
// answer or reply on a blocker in the card's own thread (its item is no
// longer kept) says which item; inBlocker leaves that off, as the entry
// already sits under its item.
export function threadLabel(
  item: CardThreadItem,
  inBlocker = false
): ThreadLabel {
  const badges: ThreadBadge[] = []
  if (item.blockerId && item.kind !== ThreadKind.Event && !inBlocker)
    badges.push({ label: `on ${item.blockerId}`, tone: "slate" })
  if (item.gone) badges.push({ label: "recipient gone", tone: "warning" })
  if (item.unheard)
    badges.push({ label: "nobody has heard this yet", tone: "warning" })
  if (item.held)
    badges.push({
      label: item.dismissed ? "held · dismissed" : "held",
      tone: "slate",
    })
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

export { BlockerState }

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

export type BlockerTone = "amber" | "grey" | "green" | "closed"

// blockerTone is how an item looks: amber while it waits on the person,
// grey once answered (never a check: it isn't resolved), green with the
// check when resolved, and the closed look. "" is an open item from an
// older app.
export function blockerTone(state: string): BlockerTone {
  switch (state) {
    case BlockerState.Answered:
      return "grey"
    case BlockerState.Resolved:
      return "green"
    case BlockerState.Closed:
      return "closed"
  }
  return "amber"
}

// needsPerson reports whether the item waits on the person: open, or
// replied (the agent asked back).
export function needsPerson(state: string): boolean {
  return blockerTone(state) === "amber"
}

// answerable reports whether the person can still answer the item:
// open, answered (to add to their answer) or replied.
export function answerable(state: string): boolean {
  return state !== BlockerState.Resolved && state !== BlockerState.Closed
}

// blockerChip is an item's state chip: "Sent to X" (" · session ended"
// once X has gone) while answered, "Replied" while back with the person,
// none otherwise (an open item is amber, a resolved one has its check).
export function blockerChip(b: CardBlocker): ThreadBadge | null {
  if (b.state === BlockerState.Answered) {
    const to = b.sentTo ?? b.raisedBy
    return {
      label: `Sent to ${displayName(to)}${to.live ? "" : " · session ended"}`,
      tone: "slate",
    }
  }
  if (b.state === BlockerState.Replied)
    return { label: "Replied", tone: "warning" }
  return null
}

// heardLine says who heard the person's comment (BoardService.Comment):
// who answers it on the card, woken or at their next turn, and the
// watchers, who answer only if it asks them. Nobody: "Nobody has heard
// this yet."
export function heardLine(
  answers: AgentRef[],
  watching: AgentRef[],
  mode: string
): string {
  const names = (rs: AgentRef[]) => {
    const n = rs.map(displayName)
    return n.length > 1
      ? `${n.slice(0, -1).join(", ")} and ${n[n.length - 1]}`
      : n[0]
  }
  const how = (rs: AgentRef[], woken: boolean) =>
    `${names(rs)} ${woken ? (rs.length > 1 ? "are woken" : "is woken") : rs.length > 1 ? "hear it at their next turn" : "hears it at its next turn"}`
  if (!answers.length && !watching.length) return "Nobody has heard this yet."
  const parts: string[] = []
  if (answers.length)
    parts.push(`${how(answers, mode === "wake")} and will answer here.`)
  if (watching.length)
    parts.push(
      answers.length
        ? `${how(watching, false)}, watching: they answer only if asked.`
        : `${how(watching, mode === "wake")}, watching.`
    )
  return parts.join(" ")
}
