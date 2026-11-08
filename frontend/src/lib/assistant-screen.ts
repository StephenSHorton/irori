// What Ask Irori can see of the window (card #1009): a plain-text snapshot
// of the current page, what is open on it and the card or thread in view.
// It is sent with each question; the assistant's read-only `screen` tool
// returns it, and only when the question depends on where the person is.
// Text only: no pixels, nothing from any other screen.
//
// The snapshot's first lines are read by the Rust side
// (crates/window/src/assistant.rs, screen_label) for the status line:
//   Page: Home
//   Open: your own cards
//   In view: card #12 “Title”

import { cardsById, cardFromHash } from "./cards"
import { TalkService } from "./host"
import { inWails } from "./irori"
import { sectionFromHash, type Section } from "./nav"

// StatusHead is how the "tool" event for the screen tool begins
// (SCREEN_STATUS on the Rust side); the chat keys "Looked at" on it.
export const StatusHead = "Looking at your screen"

export type Look = {
  page: string
  // What is open on the page: a board, a repo's desk, a storage tab.
  open: string
  // The card or conversation in view, as one line.
  view: string
}

// pageName is a section in the words the person knows it by.
export function pageName(s: Section): string {
  switch (s.kind) {
    case "home":
      return "Home"
    case "talk":
      return "Talk"
    case "notes":
      return "Notes"
    case "desk":
      return "Merge desk"
    case "boards":
      return "Boards (no space picked yet)"
    case "board":
      return "Boards"
    case "profile":
      return "Profile"
    case "notifications":
      return "Notifications"
    case "storage":
      return "Storage"
    case "settings":
      return "Settings"
  }
}

// openOn is what a section has open beyond its page.
export function openOn(s: Section): string {
  switch (s.kind) {
    case "home":
      return "your own cards"
    case "board":
      return s.board
        ? `board ${s.board} in space ${s.id}`
        : `the boards of space ${s.id}`
    case "desk":
      return s.repo ? `the desk of repo ${s.repo}` : "all merge desks"
    case "storage":
      return s.tab ? `the ${s.tab} tab` : "the overview"
    case "settings":
      return s.tab ? `the ${s.tab} section` : "the list of settings"
    case "talk":
      return s.id ? "a conversation" : "the list of conversations"
    default:
      return ""
  }
}

// snapshotText is a Look as the snapshot's lines.
export function snapshotText(l: Look): string {
  const lines = [`Page: ${l.page}`]
  if (l.open) lines.push(`Open: ${l.open}`)
  if (l.view) lines.push(`In view: ${l.view}`)
  return lines.join("\n")
}

// screenLabel is a snapshot on one line, as the status line names it:
// the Page and In view values.
export function screenLabel(text: string): string {
  const get = (key: string) => {
    for (const l of text.split("\n"))
      if (l.startsWith(key)) return l.slice(key.length).trim()
    return ""
  }
  const label = [get("Page:"), get("In view:")].filter(Boolean).join(", ")
  return label.length > 90 ? `${label.slice(0, 89)}…` : label
}

// quoted is a title between curly quotes, on one line.
function quoted(t: string): string {
  const one = t.split(/\s+/).filter(Boolean).join(" ")
  return `“${one.length > 100 ? `${one.slice(0, 99)}…` : one}”`
}

// lookNow reads what the window shows from its address, and the card's or
// conversation's name from the host. A name it can't read leaves the id's
// kind alone ("a card"): the snapshot is never worth failing a question for.
export async function lookNow(
  hash = window.location.hash,
  nameOf: (hash: string, s: Section) => Promise<string> = viewOf
): Promise<Look> {
  const s = sectionFromHash(hash)
  let view: string
  try {
    view = await nameOf(hash, s)
  } catch {
    view = ""
  }
  return { page: pageName(s), open: openOn(s), view }
}

async function viewOf(hash: string, s: Section): Promise<string> {
  const id = cardFromHash(hash)
  if (id) {
    const slot = (await cardsById([id])).get(id)
    const c = slot?.detail
    if (!c) return "a card"
    const ref = `${c.tag ?? ""}#${c.short}`
    return `card ${ref} ${quoted(c.title)}`
  }
  if (s.kind === "talk" && s.id) {
    if (!inWails()) return "a conversation"
    const conv = (await TalkService.Conversations()).find(
      (c) => c.convId === s.id
    )
    return conv ? `${conv.kind} ${quoted(conv.name)}` : "a conversation"
  }
  return ""
}

// screenSnapshot is what an ask sends.
export async function screenSnapshot(): Promise<string> {
  return snapshotText(await lookNow())
}
