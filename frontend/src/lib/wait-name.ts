// waitName says in plain words what a card waits on that isn't the person
// (card #487): "a free Dolphin slot" for ww/dolphin-slot-1..4, not the
// raw lease names. Leases come as kind "lease" with the names held when
// the agent blocked ("ww/dolphin-slot-1 ww/dolphin-slot-2"); on a shared
// board the agent's own text stays kind "external" ("external
// ww/dolphin-slot-1..4 leases (…)"), so a lease family named in that text
// is read the same way. Anything else keeps its words.

import { plainLine } from "./irori"

// A lease family member: a name ending in -<n>, -<lo>..<hi>, -N, -n or -*
// (coord names_lease reads the same shorthand).
const MEMBER = /^([a-z0-9][a-z0-9._-]*?(?:\/[a-z0-9._-]+)*?)-(\d+(?:\.\.\d+)?|[nN*])$/
// The same, found inside free text.
const IN_TEXT = /\b([a-z0-9][a-z0-9._-]*(?:\/[a-z0-9._-]+)+)-(\d+\.\.\d+|\d+|[nN*])(?![a-z0-9._/-])/i

function words(seg: string): string {
  const w = seg.replace(/[-_.]+/g, " ").trim()
  return w ? w[0].toUpperCase() + w.slice(1) : seg
}

// family names a lease family's stem: Irori's own check slots and merge
// desk by what they are, anything else by its last path part in words.
function family(stem: string): string {
  const check = /^check\/([^/]+)\/slot$/.exec(stem)
  if (check) return `check slot on ${check[1]}`
  const parts = stem.split("/")
  return words(parts[parts.length - 1])
}

// one names a single lease.
function one(name: string): string {
  if (name.startsWith("desk/")) return "the merge desk"
  const port = /^port:(\d+)$/.exec(name)
  if (port) return `port ${port[1]}`
  const path = /^path:([^:]+):(.+)$/.exec(name)
  if (path) return `${path[2]} in ${path[1]}`
  const m = MEMBER.exec(name)
  if (m && /^\d+$/.test(m[2])) return `${family(m[1])} ${m[2]}`
  if (m) return `a free ${family(m[1])}`
  return name
}

// leaseNames names the leases a wait is on: several of one family (or a
// range) are "a free <family>", one is itself.
export function leaseNames(names: readonly string[]): string {
  if (!names.length) return ""
  const stems = new Map<string, number>()
  for (const n of names) {
    const m = MEMBER.exec(n)
    const key = m ? m[1] : n
    stems.set(key, (stems.get(key) ?? 0) + 1)
  }
  const first = names[0]
  const m = MEMBER.exec(first)
  const said = m && (stems.get(m[1]) ?? 0) > 1 ? `a free ${family(m[1])}` : one(first)
  const others = stems.size - 1
  return others > 0 ? `${said} and ${others} more` : said
}

// waitName is the thing named, without "Waiting for": "" when the card
// names nothing.
export function waitName(c: { waitKind?: string; waitOn?: string }): string {
  const on = plainLine(c.waitOn ?? "")
  if (!on) return ""
  if (c.waitKind === "lease") return leaseNames(on.split(/\s+/))
  if (c.waitKind && c.waitKind !== "external") return on
  // External, or a card from before waitKind: a lease family in its text.
  const m = IN_TEXT.exec(on)
  if (m) {
    const f = m[1].toLowerCase()
    return m[2].includes("..") || /^[nN*]$/.test(m[2])
      ? `a free ${family(f)}`
      : `${family(f)} ${m[2]}`
  }
  return on.replace(/^external\s+/i, "")
}

// waitLine is the row's words for it: "Waiting for a free Dolphin slot",
// "Waiting on kestrel", "Waiting on #12".
export function waitLine(c: { waitKind?: string; waitOn?: string }): string {
  const n = waitName(c)
  if (!n) return "Waiting on something else"
  const lease =
    c.waitKind === "lease" || (c.waitKind !== "agent" && c.waitKind !== "task" && IN_TEXT.test(plainLine(c.waitOn ?? "")))
  return lease ? `Waiting for ${n}` : `Waiting on ${n}`
}
