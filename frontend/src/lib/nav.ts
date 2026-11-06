// Where the window is (card #96): Home (your own cards), Talk (card #390:
// threads, and channels and DMs once Talk 1 lands), Notes (card #413), the
// Boards page, or one board's page. It lives in the address, like an open
// card ("#card/<id>"), so back and forward move between them: "#talk",
// "#talk/<id>" (a conversation open), "#notes", "#profile" (card #393: your name, colour and
// picture, and your spaces), "#boards", "#board/<id>", and no hash for
// Home. A card's page opens over whichever
// of them it was opened from.

export type Section =
  | { kind: "home" }
  | { kind: "talk"; id: string }
  | { kind: "notes" }
  // A merge desk (card #417): "#desk", or "#desk/<repoKey>" for one repo.
  | { kind: "desk"; repo: string }
  | { kind: "boards" }
  // A space's page (card #558): its boards. With board, that one board's
  // list ("#board/<space>/<board>").
  | { kind: "board"; id: string; board?: string }
  | { kind: "profile" }
  // Card #377: every notification sent to the person.
  | { kind: "notifications" }

export const Home: Section = { kind: "home" }

// sectionFromHash is the section an address names (a card's page counts
// as the section history has behind it, so Home here).
export function sectionFromHash(hash: string): Section {
  if (hash === "#boards") return { kind: "boards" }
  if (hash === "#profile") return { kind: "profile" }
  if (hash === "#talk") return { kind: "talk", id: "" }
  const t = hash.match(/^#talk\/(.+)$/)
  if (t) return { kind: "talk", id: decodeURIComponent(t[1]) }
  // "#notes/<board tag>" was a board's tab before notes were Mine only
  // (card #616): it opens Notes.
  if (hash === "#notes" || hash.startsWith("#notes/")) return { kind: "notes" }
  if (hash === "#desk") return { kind: "desk", repo: "" }
  const d = hash.match(/^#desk\/(.+)$/)
  if (d) return { kind: "desk", repo: decodeURIComponent(d[1]) }
  if (hash === "#notifications") return { kind: "notifications" }
  const l = hash.match(/^#board\/([^/]+)\/([^/]+)$/)
  if (l)
    return {
      kind: "board",
      id: decodeURIComponent(l[1]),
      board: decodeURIComponent(l[2]),
    }
  const m = hash.match(/^#board\/(.+)$/)
  if (m) return { kind: "board", id: decodeURIComponent(m[1]) }
  return Home
}

export function sectionHash(s: Section): string {
  if (s.kind === "boards") return "#boards"
  if (s.kind === "profile") return "#profile"
  if (s.kind === "talk")
    return s.id ? `#talk/${encodeURIComponent(s.id)}` : "#talk"
  if (s.kind === "notes") return "#notes"
  if (s.kind === "desk")
    return s.repo ? `#desk/${encodeURIComponent(s.repo)}` : "#desk"
  if (s.kind === "notifications") return "#notifications"
  if (s.kind === "board")
    return s.board
      ? `#board/${encodeURIComponent(s.id)}/${encodeURIComponent(s.board)}`
      : `#board/${encodeURIComponent(s.id)}`
  return ""
}

// go moves the window to section s as a new history entry; App follows
// it through popstate, the same as the browser's back and forward.
export function go(s: Section): void {
  const url = window.location.pathname + window.location.search + sectionHash(s)
  window.history.pushState({ section: s }, "", url)
  window.dispatchEvent(new PopStateEvent("popstate", { state: { section: s } }))
}

export function sameSection(a: Section, b: Section): boolean {
  if (a.kind !== b.kind) return false
  if (a.kind === "board")
    return b.kind === "board" && a.id === b.id && a.board === b.board
  if (a.kind === "talk") return b.kind === "talk" && a.id === b.id
  if (a.kind === "desk") return b.kind === "desk" && a.repo === b.repo
  return true
}
