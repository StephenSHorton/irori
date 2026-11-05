// Friends (card #517, docs/design/talk-channels.md §10): a friend is
// anyone you share a space with (Stephen's b3), the pair's hidden friend
// space a friend invite makes included. The profile lists them, and New
// board's "Who's on it?" picks among them.

import type { BoardSummary } from "@/lib/boards"

// Friend is another person you share a space with, and the spaces.
export type Friend = {
  personId: string
  name: string
  color: string
  avatar: string
  // The shown spaces you share with them, by name (friend spaces aren't
  // listed: they're hidden).
  spaces: string[]
  // You made friends with a friend invite: you share a friend space.
  invited: boolean
}

// friendsOf is everyone else in your spaces, once each, by name: their
// newest name, colour and avatar as the spaces list them. `boards` is the
// whole list, friend spaces included (useAllBoards).
export function friendsOf(boards: BoardSummary[]): Friend[] {
  const by = new Map<string, Friend>()
  for (const b of boards) {
    if (b.leaving) continue
    for (const m of b.members) {
      if (m.me) continue
      const f = by.get(m.personId) ?? {
        personId: m.personId,
        name: m.name,
        color: m.color ?? "",
        avatar: m.avatar ?? "",
        spaces: [],
        invited: false,
      }
      f.color ||= m.color ?? ""
      f.avatar ||= m.avatar ?? ""
      if (b.friend) f.invited = true
      else f.spaces.push(b.name || b.tag)
      by.set(m.personId, f)
    }
  }
  return [...by.values()].sort((a, b) => a.name.localeCompare(b.name))
}

// friendLine says where you share with a friend: "in mlg, site", or
// "friend" when it's only the friend space.
export function friendLine(f: Friend): string {
  return f.spaces.length ? `in ${f.spaces.join(", ")}` : "friend"
}
