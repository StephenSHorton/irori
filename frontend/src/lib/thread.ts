// A card's thread (BoardService.Card's thread): comments, messages about
// the card and what happened, oldest first. These helpers are pure, so
// the thread view and its tests share them.

import {
  BlockerState,
  displayName,
  namesLine,
  RunningJobStatus,
  type AgentRef,
  type CardBlocker,
  type CardThreadItem,
} from "./cards"

import { createContext } from "react"

import { parseTime } from "./irori"

// FocusComment is the comment a notification opened the card at (card
// #177): the thread that holds it shows it, scrolled into view.
export const FocusComment = createContext("")

// StartAgentOn is the card whose thread offers "Start an agent with this"
// on the person's latest comment (card #452, spawned-agents §4): a card on
// their own board, open, not an ask, with nobody on it. "" offers none.
export const StartAgentOn = createContext("")

// startAgentItem is the comment that offers it: the person's latest
// comment on the thread (not a blocker answer), or "".
export function startAgentItem(
  items: readonly {
    id: string
    kind: string
    who: { status: string }
    blockerId?: string | null
  }[]
): string {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i]
    if (it.kind === "comment" && it.who.status === "person" && !it.blockerId)
      return it.id
  }
  return ""
}

// BlockersHere is the ids of the blockers the open card page shows (card
// #490): a thread line about one of them links to it.
export const BlockersHere = createContext<ReadonlySet<string>>(new Set())

// blockerItemId is a blocker's element id on the card page, for scrolling
// to it from a thread line (card #490).
export function blockerItemId(id: string): string {
  return `blocker-item-${id}`
}

// blockerName is how the person reads a blocker in a line (card #490):
// its question, quoted, never its id.
export function blockerName(gist?: string | null): string {
  return gist ? `“${gist}”` : "a blocker"
}

// threadEntryId is a thread entry's element id, for scrolling to it.
export function threadEntryId(id: string): string {
  return `thread-entry-${id}`
}

// ThreadPage is how many of the newest entries show before "Show earlier".
export const ThreadPage = 50

// ThreadKind mirrors the Go constants on CardThreadItem.kind.
export const ThreadKind = {
  Comment: "comment",
  Message: "message",
  Event: "event",
} as const

export type ThreadBadge = {
  label: string
  tone: "warning" | "slate" | "success"
}

// ThreadLabel is an entry's header: who wrote it, who a message went to,
// the device it was written on ("" for this one) and its badges.
export type ThreadLabel = {
  author: string
  arrow?: string
  device: string
  badges: ThreadBadge[]
  // Under the person's comment: who still has to hear it and why it waits
  // ("garnet is busy: it hears this when its current turn ends"), then who
  // read it (card #159).
  waiting: string[]
}

// threadLabel is the header line for one entry: "A -> B" for a message,
// with "recipient gone" when Irori left it on the card for an ended
// recipient, "nobody has heard this yet" on the person's comment that no
// agent was around to hear (until an agent writes on the card), "held" until the wake policy lets it through (its text isn't
// on the card), "held · dismissed" once the person dismissed it, the
// count of files that stayed on the message. The person's comment's
// pending lines go in waiting, shown under it, not as badges. An
// answer or reply on a blocker in the card's own thread (its item is no
// longer kept) says which item; inBlocker leaves that off, as the entry
// already sits under its item.
export function threadLabel(
  item: CardThreadItem,
  inBlocker = false
): ThreadLabel {
  const badges: ThreadBadge[] = []
  if (item.blockerId && item.kind !== ThreadKind.Event && !inBlocker)
    badges.push({ label: `on ${blockerName(item.blockerGist)}`, tone: "slate" })
  if (item.gone) badges.push({ label: "recipient gone", tone: "warning" })
  if (item.unheard)
    badges.push({ label: "nobody has heard this yet", tone: "warning" })
  if (item.held)
    badges.push({
      label: item.dismissed ? "held · dismissed" : "held",
      tone: "slate",
    })
  if (item.edited) badges.push({ label: "edited", tone: "slate" })
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
    waiting: item.pending ?? [],
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

// isWait reports whether an open item waits on other work (card #481): a
// card, a lease, an agent or something outside, not a person. It is never
// the person's to answer and never counts as a decision.
export function isWait(b: CardBlocker): boolean {
  return !b.resolved && !!b.waitsOn
}

// blockerChip is an item's state chip: "Sent to X" (" · session ended"
// once X has gone) while answered, "Replied" while back with the person,
// "Revised" while open after an agent revised it (card #50: what it asks
// and its options changed under the person's reply), none otherwise (an
// open item is amber, a resolved one has its check).
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
  // A done check (card #112): the agent says it's done.
  if (b.doneCheck && (b.state || BlockerState.Open) === BlockerState.Open)
    return { label: "Ready for you", tone: "success" }
  if (
    (b.state || BlockerState.Open) === BlockerState.Open &&
    (b.thread ?? []).some(isRevision)
  )
    return { label: "Revised", tone: "warning" }
  return null
}

// isRevision reports whether a blocker thread entry is an agent revising
// the item: a new question, context or options (or none).
export function isRevision(item: CardThreadItem): boolean {
  return item.kind === ThreadKind.Event && !!item.revised
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
  if (answers.length && mode === "stop")
    parts.push(
      `${names(answers)} ${answers.length > 1 ? "are" : "is"} stopped and will answer here first.`
    )
  else if (answers.length)
    parts.push(`${how(answers, mode === "wake")} and will answer here.`)
  if (watching.length)
    parts.push(
      answers.length
        ? `${how(watching, false)}, watching: they answer only if asked.`
        : `${how(watching, mode === "wake" || mode === "stop")}, watching.`
    )
  return parts.join(" ")
}

// LiveRoster is each agent's live roster state by key (App provides it from
// roster:changed, card #299): a card's AgentRef carries the status of when
// the card was read, the roster the status of now.
export type LiveAgent = {
  status: string
  updatedAt?: string
  lastTool?: string
  // Card #453: the person's "Offer it cards", and whether Irori started it.
  offersCards?: boolean
  spawned?: boolean
}
export const LiveRoster = createContext<ReadonlyMap<string, LiveAgent>>(
  new Map()
)

// StoppedGrace is how long an agent must sit idle before a thread says it
// stopped without replying: between the end of one turn and the wake that
// delivers the person's comment it is idle for a moment.
export const StoppedGrace = 3000

// WorkingLine is the live line's text and tone. A working line also gives
// its parts (card #317): who ("oak", "oak and elm"), the verb and what it
// is doing ("" for no running tool), so the view can swap in a changing
// word (ponderWord) while no tool says more.
export type WorkingLine = {
  text: string
  tone: "working" | "stopped"
  who?: string
  verb?: "is" | "are"
  doing?: string
}

// PonderWords are the working line's changing words while an agent works
// and no running tool says what on (card #317, like Claude Code's status
// word). The first is the still word Reduce Motion keeps.
export const PonderWords = [
  "working",
  "pondering",
  "thinking",
  "mulling it over",
  "tinkering",
  "puzzling it out",
  "figuring",
  "noodling",
] as const

// PonderEvery is how often the word changes, in ms.
export const PonderEvery = 3000

// ponderWord is the word for the tick'th change: the words in turn, from
// the first. Reduce Motion shows ponderWord(0), still.
export function ponderWord(tick: number): string {
  const n = PonderWords.length
  return PonderWords[((Math.floor(tick) % n) + n) % n]
}

// workingLine is the live line under a thread (card #299, GitHub #5):
// "oak is working…" while an agent on it (a holder, or the one an ask was
// handed to) is in a turn, by the live roster; "oak stopped without
// replying" once an agent read the person's latest comment, had a turn
// after it and went idle with nothing on the thread since. None when the
// card is over or nobody is on it.
export function workingLine(
  agents: readonly AgentRef[],
  items: readonly CardThreadItem[],
  live: ReadonlyMap<string, LiveAgent>,
  now: Date,
  over = false,
  // working: who the card's own rule says is working on it now (the card
  // page passes workingNow, card #317), so its line agrees with the page's
  // working row and Home's Agent progress. Left out, any agent on the
  // thread busy in the live roster counts (an ask's taker, card #299).
  working?: readonly AgentRef[],
  // jobs: who the card's rule says is running a background job between
  // turns (runningJobNow, card #431), given with working. Left out, any
  // agent on the thread "shell" in the live roster counts.
  jobs?: readonly AgentRef[]
): WorkingLine | null {
  if (over) return null
  const seen = new Set<string>()
  const on = agents.filter((a) => {
    if (a.status === "person" || !a.live || a.offline || seen.has(a.key))
      return false
    seen.add(a.key)
    return true
  })
  const status = (a: AgentRef) => live.get(a.key)?.status ?? a.status
  const names = (rs: readonly AgentRef[]) => namesLine(rs.map(displayName))
  const busy = working ?? on.filter((a) => status(a) === "busy")
  if (busy.length) {
    // One agent: what it is doing right now, when a tool is running.
    const doing =
      busy.length === 1 ? toolPhrase(live.get(busy[0].key)?.lastTool ?? "") : ""
    const who = names(busy)
    const verb = busy.length > 1 ? "are" : "is"
    return {
      text: `${who} ${verb} ${doing || "working"}…`,
      tone: "working",
      who,
      verb,
      doing,
    }
  }

  // Card #431: no turn running, but a holder's background job is (Claude
  // Code's "shell"): it isn't stopped, and it isn't idle.
  const running =
    jobs ?? (working ? [] : on.filter((a) => status(a) === RunningJobStatus))
  if (running.length) {
    const who = names(running)
    const verb = running.length > 1 ? "are" : "is"
    const doing = "running a background job"
    return {
      text: `${who} ${verb} ${doing}…`,
      tone: "working",
      who,
      verb,
      doing,
    }
  }

  // The person's latest comment, and whether anyone answered it since.
  let p = -1
  for (let i = items.length - 1; i >= 0; i--) {
    if (
      items[i].kind !== ThreadKind.Event &&
      items[i].who.status === "person"
    ) {
      p = i
      break
    }
  }
  if (p < 0) return null
  const comment = items[p]
  const at = parseTime(comment.at)
  if (!at) return null
  const heard = comment.heardBy ?? []
  const after = items.slice(p + 1)
  const stopped = on.filter((a) => {
    // It read the comment (an agent still waiting to hear it hasn't
    // stopped on it) and is idle now.
    if (!heard.includes(a.key) || status(a) !== "idle") return false
    if (after.some((it) => it.who.key === a.key)) return false
    const since = parseTime(live.get(a.key)?.updatedAt)
    return (
      !!since && since > at && now.getTime() - since.getTime() >= StoppedGrace
    )
  })
  if (!stopped.length) return null
  return { text: `${names(stopped)} stopped without replying`, tone: "stopped" }
}

// toolPhrase says what a running tool is doing, for the working line
// ("running a command"); "" for none.
export function toolPhrase(tool: string): string {
  if (!tool) return ""
  const fixed: Record<string, string> = {
    Bash: "running a command",
    BashOutput: "running a command",
    PowerShell: "running a command",
    Read: "reading files",
    Edit: "editing files",
    MultiEdit: "editing files",
    Write: "editing files",
    NotebookEdit: "editing files",
    Grep: "searching files",
    Glob: "searching files",
    WebSearch: "searching the web",
    WebFetch: "reading a web page",
    Agent: "running a subagent",
    Task: "running a subagent",
    TodoWrite: "planning",
    advisor: "asking for advice",
  }
  if (fixed[tool]) return fixed[tool]
  const m = /^mcp__([^_]+(?:[-_][^_]+)*?)__(.+)$/.exec(tool)
  if (m) {
    const [, server, name] = m
    if (server === "irori") {
      if (name.startsWith("board")) return "on the Board"
      if (name === "send" || name === "reply") return "writing a message"
      if (name === "search") return "searching Irori"
      return "using Irori"
    }
    if (server.includes("chrome") || server.includes("browser"))
      return "using the browser"
    return `using ${server.replace(/[-_]/g, " ")}`
  }
  return `using ${tool}`
}
