// Take me there (card #784): Ask Irori can take the person to what they
// asked about (a page, a card or thread, a setting, a PR) and briefly
// highlight it. The assistant ends such a reply with one go line,
// "<<go card #779>>" (crates/window/src/assistant.rs, SYSTEM, which lists
// the same settings as settingSpots below; a test keeps the two in step).
// The chat shows it as a button that goes there, and goes there by itself
// once, when the reply comes in while the chat is open. The history keeps
// the reply whole; the chat splits the line out as it shows it.

import type { SettingsSection } from "@/lib/nav"

export const GoOpen = "<<go "
export const GoClose = ">>"

export type GoPage =
  | "home"
  | "talk"
  | "notes"
  | "boards"
  | "desk"
  | "notifications"
  | "storage"
  | "profile"
  | "held"

export type GoTarget =
  | { kind: "page"; page: GoPage }
  // A card or a thread by its ref: "#779", or "tag#12" on a space's board.
  | { kind: "card"; ref: string }
  | { kind: "setting"; key: string }
  // A PR's page on GitHub (it opens in the browser, never by itself).
  | { kind: "pr"; url: string; number: string }

export const pageLabels: Record<GoPage, string> = {
  home: "Dashboard",
  talk: "Talk",
  notes: "Notes",
  boards: "Boards",
  desk: "Merge desks",
  notifications: "Notifications",
  storage: "Storage",
  profile: "Profile",
  held: "Held messages",
}

// SettingSpot is a place the assistant can point at: a Settings section,
// or one control in it (data-setting="<key>" on the control's row), or
// a top bar switch, Stop here or Stop everywhere (section null, data-spot
// on the button: they aren't in Settings).
export type SettingSpot = {
  key: string
  section: SettingsSection | null
  label: string
}

const sectionLabels: Record<SettingsSection, string> = {
  notifications: "Notifications",
  appearance: "Appearance",
  devices: "Devices and connections",
  people: "People and invites",
  agents: "Agents",
  desks: "Merge desks",
  app: "App",
}

const sectionSpots: SettingSpot[] = (
  Object.keys(sectionLabels) as SettingsSection[]
).map((s) => ({ key: s, section: s, label: sectionLabels[s] }))

export const settingSpots: SettingSpot[] = [
  { key: "stop-here", section: null, label: "Stop here" },
  { key: "stop-everywhere", section: null, label: "Stop everywhere" },
  ...sectionSpots,
  {
    key: "notifications.show",
    section: "notifications",
    label: "Show notifications",
  },
  { key: "notifications.sound", section: "notifications", label: "Sound" },
  {
    key: "notifications.confetti",
    section: "notifications",
    label: "Confetti on good news",
  },
  { key: "notifications.volume", section: "notifications", label: "Volume" },
  { key: "notifications.corner", section: "notifications", label: "Corner" },
  {
    key: "agents.each-card",
    section: "agents",
    label: "Personal",
  },
  { key: "agents.spaces", section: "agents", label: "Spaces" },
  {
    key: "agents.given",
    section: "agents",
    label: "Cards someone gives you",
  },
  {
    key: "agents.model",
    section: "agents",
    label: "What a new agent starts with",
  },
  { key: "agents.names", section: "agents", label: "Agent names" },
  { key: "agents.limits", section: "agents", label: "How many at once" },
  {
    key: "agents.stops",
    section: "agents",
    label: "When Irori stops and asks",
  },
  { key: "agents.weighs", section: "agents", label: "Weigh new cards" },
  { key: "agents.running", section: "agents", label: "Running agents" },
  {
    key: "agents.permissions",
    section: "agents",
    label: "What a new agent may do",
  },
  {
    key: "agents.trusted",
    section: "agents",
    label: "Trusted repos for agents",
  },
  { key: "app.login", section: "app", label: "Launch at login" },
  {
    key: "app.transcribe",
    section: "app",
    label: "Transcribe what I say in huddles",
  },
  {
    key: "app.huddle-notes",
    section: "app",
    label: "Let my assistant add notes from what I say",
  },
  {
    key: "app.huddle-chair",
    section: "app",
    label: "Let my assistant chair huddles I start",
  },
  { key: "app.names", section: "app", label: "Name new threads" },
  { key: "app.updates", section: "app", label: "Updates" },
]

// Spots that moved, by their old key, so a saved chat's go line still
// lands: Weigh new cards moved from App to Agents (card #1007). Kept out
// of settingSpots, so the assistant's prompt list doesn't name them.
const movedSpots: Record<string, string> = { "app.weighs": "agents.weighs" }

export function settingSpot(key: string): SettingSpot | undefined {
  const k = movedSpots[key] ?? key
  return settingSpots.find((s) => s.key === k)
}

// parseGo reads a go line's body ("card #779"), or null when it names
// nothing the chat knows.
export function parseGo(body: string): GoTarget | null {
  const m = /^\s*([a-z]+)\s+(\S+)\s*$/i.exec(body)
  if (!m) return null
  const kind = m[1].toLowerCase()
  const arg = m[2]
  switch (kind) {
    case "page": {
      const p = arg.toLowerCase()
      return p in pageLabels ? { kind: "page", page: p as GoPage } : null
    }
    case "card":
    case "thread": {
      const r = /^([a-z0-9_-]*)#(\d+)$/i.exec(arg)
      return r ? { kind: "card", ref: `${r[1].toLowerCase()}#${r[2]}` } : null
    }
    case "setting": {
      const spot = settingSpot(arg.toLowerCase())
      return spot ? { kind: "setting", key: spot.key } : null
    }
    case "pr": {
      const u =
        /^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/(\d+)\/?$/i.exec(arg)
      return u ? { kind: "pr", url: arg, number: u[1] } : null
    }
    default:
      return null
  }
}

// goLabel is the go button's words.
export function goLabel(t: GoTarget): string {
  switch (t.kind) {
    case "page":
      return `Open ${pageLabels[t.page]}`
    case "card":
      return `Open ${t.ref}`
    case "setting": {
      const s = settingSpot(t.key)
      if (!s) return "Show it"
      if (!s.section) return `Show ${s.label}`
      const sec = sectionLabels[s.section]
      return s.key === s.section
        ? `Open Settings › ${sec}`
        : `Show Settings › ${sec} › ${s.label}`
    }
    case "pr":
      return `Open PR #${t.number} on GitHub`
  }
}

// goesByItself: a PR opens the browser, so only a tap opens it.
export function goesByItself(t: GoTarget): boolean {
  return t.kind !== "pr"
}

export type GoSplit = {
  // The reply without its go lines.
  text: string
  // The first go line that names something, or null.
  go: GoTarget | null
}

// splitGo takes the go lines out of a reply's text (outside its drafted
// thread: lib/assistant.ts splitReply). One that names nothing is dropped
// too. While the reply is being written (writing), a go line not yet
// closed ("<<go car", or "<<g" at the very end) is dropped so it never
// flashes; in a finished reply a malformed one stays as text, so nothing
// after it is lost.
export function splitGo(raw: string, writing = false): GoSplit {
  let go: GoTarget | null = null
  let text = raw.replace(/<<go\s+([^<>\n]*?)\s*>>[ \t]*\n?/gi, (_m, body) => {
    go ??= parseGo(body as string)
    return ""
  })
  if (writing) {
    const open = text.lastIndexOf("<<go")
    if (open >= 0 && !text.slice(open).includes(">>"))
      text = text.slice(0, open)
    else text = cutTail(text, GoOpen)
  }
  return { text: text.replace(/\n{3,}/g, "\n\n").trim(), go }
}

// cutTail drops a marker's first characters from the end of text being
// written ("…<<g"). Only from "<<" on: a lone "<" may be the reply's own.
function cutTail(s: string, marker: string): string {
  for (let n = Math.min(marker.length - 1, s.length); n > 1; n--)
    if (s.endsWith(marker.slice(0, n))) return s.slice(0, s.length - n)
  return s
}
