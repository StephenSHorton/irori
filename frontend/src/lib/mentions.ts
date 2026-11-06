// Mentions (card #77, docs/design/board.md "Mentions"): the pure part of
// the @name picker. The text keeps a readable "@name"; the key beside it,
// recorded when the person picks a row, decides who hears. The window never
// resolves a name to a key by itself, except the one case the design
// allows: a query that is exactly one live agent's name when the person
// types a space or sends.

import { createContext, useCallback, useEffect, useState } from "react"

import type { AgentRef } from "./models"

// Mention mirrors the bindings' Mention.
export type Mention = { key: string; name: string; gone?: boolean }

// MentionCandidate mirrors the bindings' MentionCandidate: one picker row.
// board is set for another person on a shared board, or one of their
// agents: the board they can be mentioned on (card #177).
export type MentionCandidate = {
  who: AgentRef
  cwd: string
  repo: string
  board?: string
  // A "the agent on #N" row (card #452, spawned-agents §4): the agent
  // holding card ref (#N, or tag#N on a shared board) now. Picking it
  // mentions that agent, so the row reads by the card, not the name.
  card?: string
}

// agentOnLabel is a card row's words in the picker.
export function agentOnLabel(card: string): string {
  return `the agent on ${card}`
}

// A card the picker may offer "the agent on" for: its ref, and its
// holder as the window knows it.
export type HeldCard = {
  id: string
  short: string
  tag?: string
  holder: { who: AgentRef; state: string }
}

// agentOnRows are the "the agent on #N" rows: one per card an agent
// holds and is live on (not the person, not gone), with the card the box
// sits on (here) first. The holder's own row from cands is reused so its
// device and colour show the same.
export function agentOnRows(
  cards: readonly HeldCard[],
  cands: readonly MentionCandidate[],
  here = ""
): MentionCandidate[] {
  const rows: { here: boolean; n: number; c: MentionCandidate }[] = []
  for (const c of cards) {
    const who = c.holder.who
    if (!c.short || !who?.key || who.key.startsWith("user:")) continue
    if (
      c.holder.state === "person" ||
      c.holder.state === "gone" ||
      c.holder.state === "nobody"
    )
      continue
    if (isGone(who)) continue
    const known = cands.find((x) => x.who.key === who.key && !x.card)
    rows.push({
      here: c.id === here,
      n: Number(c.short) || 0,
      c: {
        who: known?.who ?? who,
        cwd: known?.cwd ?? "",
        repo: known?.repo ?? "",
        board: known?.board,
        card: `${c.tag ?? ""}#${c.short}`,
      },
    })
  }
  rows.sort((a, b) => Number(b.here) - Number(a.here) || b.n - a.n)
  return rows.map((r) => r.c)
}

// rowName is what a row is found by and shows as: its name, or for a card
// row "the agent on #N".
export function rowName(c: MentionCandidate): string {
  return c.card ? agentOnLabel(c.card) : c.who.name
}

// MentionBoard is the shared board (its id) the boxes inside sit on, ""
// on your own board: there the picker also offers the board's other people
// and their agents, and only there (card #177).
export const MentionBoard = createContext("")

// boardRows keeps the candidates a box on board `board` may mention: your
// own agents everywhere, another person and their agents on their board.
export function boardRows(cands: MentionCandidate[], board: string) {
  return cands.filter((c) => !c.board || c.board === board)
}

// MaxMentions mirrors coord's MAX_MENTIONS.
export const MaxMentions = 10

// MaxRows is how many rows the picker shows.
export const MaxRows = 20

// A token's "@" starts the text or follows whitespace or "(".
const before = /[\s(]/
// A mention in the text ends at the end, whitespace or this punctuation.
const after = /[\s.,;:!?)'"]/

export type Token = { start: number; query: string }

// trailing is closing punctuation typed straight after a name ("@oak,").
const trailing = /[.,;:!?)'"]+$/

// bare is query without closing punctuation at its end.
function bare(query: string): string {
  return query.replace(trailing, "")
}

// composing says a key press belongs to an IME composition. WKWebView
// (the Mac build) sends the Enter that commits one with isComposing false
// and keyCode 229.
export function composing(e: {
  nativeEvent: { isComposing?: boolean; keyCode?: number }
}): boolean {
  return !!e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229
}

// gone says the agent has left (not merely offline).
export function isGone(who: AgentRef): boolean {
  return who.status === "gone"
}

// tokenAt is the "@query" the caret is in, or null. The query runs from
// the "@" to the caret; a space or newline ends it, unless some
// candidate's name still holds the query with the space in it (names may
// hold spaces).
export function tokenAt(
  text: string,
  caret: number,
  names: readonly string[] = []
): Token | null {
  const head = text.slice(0, caret)
  // The last "@" only: an earlier one's query would hold this one.
  const at = head.lastIndexOf("@")
  // a@b is not a token.
  if (at < 0 || (at > 0 && !before.test(head[at - 1]))) return null
  const query = head.slice(at + 1)
  if (query.includes("\n") || query.length > 64) return null
  if (!/\s/.test(query)) return { start: at, query }
  const q = query.toLowerCase()
  if (names.some((n) => n.toLowerCase().includes(q)))
    return { start: at, query }
  return null
}

// filterCandidates are the rows for query: names holding it, ignoring
// case. Live agents first, gone ones last; among each, names that start
// with the query, then this device's agents (no device chip), then by
// name. At most MaxRows.
export function filterCandidates(
  cands: readonly MentionCandidate[],
  query: string
): MentionCandidate[] {
  const q = bare(query).toLowerCase()
  // A card row is found by "the agent on #N", by "#N" and by N, never by
  // the holder's name (its own row is there for that).
  const matches = (c: MentionCandidate) =>
    c.card
      ? agentOnLabel(c.card).toLowerCase().includes(q) ||
        (q.length > 0 &&
          c.card
            .toLowerCase()
            .replace(/^.*#/, "")
            .startsWith(q.replace(/^#/, "")))
      : c.who.name.toLowerCase().includes(q)
  const rank = (c: MentionCandidate) => {
    const n = rowName(c).toLowerCase()
    return [
      isGone(c.who) ? 1 : 0,
      // With nothing typed, the card rows lead: on a card's box the first
      // is "the agent on" this card (sa-4).
      q ? 0 : c.card ? 0 : 1,
      n.startsWith(q) || (c.card && q.startsWith("#")) ? 0 : 1,
      c.who.device ? 1 : 0,
    ]
  }
  return cands
    .filter(matches)
    .map((c, i) => ({ c, i, r: rank(c) }))
    .sort((a, b) => {
      for (let i = 0; i < a.r.length; i++)
        if (a.r[i] !== b.r[i]) return a.r[i] - b.r[i]
      // Card rows keep agentOnRows' order (this card first).
      if (a.c.card && b.c.card) return a.i - b.i
      return rowName(a.c).localeCompare(rowName(b.c))
    })
    .slice(0, MaxRows)
    .map((x) => x.c)
}

// moveHighlight is the next highlighted row: -1 is none (the first row is
// then the default). Forward from none is the first row, back from none
// the last; both wrap.
export function moveHighlight(i: number, count: number, dir: 1 | -1): number {
  if (count <= 0) return -1
  if (i < 0 || i >= count) return dir > 0 ? 0 : count - 1
  return (i + dir + count) % count
}

// insertMention puts "@name " in place of the token and the rest of the
// word the caret is in, and returns the text and where the caret goes.
export function insertMention(
  text: string,
  t: Token,
  caret: number,
  name: string
): { text: string; caret: number } {
  let end = caret
  while (end < text.length && !after.test(text[end])) end++
  const rest = text.slice(end)
  const space = /^\s/.test(rest) ? "" : " "
  const ins = `@${name}${space}`
  return {
    text: text.slice(0, t.start) + ins + rest,
    caret: t.start + ins.length + (space ? 0 : 1),
  }
}

// mentionAt says text holds "@name" at i as a whole mention.
function mentionAt(text: string, i: number, name: string): boolean {
  if (i > 0 && !before.test(text[i - 1])) return false
  const end = i + 1 + name.length
  return end === text.length || after.test(text[end])
}

// hasMention says text still holds "@name" (exact, case as inserted).
// An "@name" that is the start of a longer recorded one ("@oak" in
// "@oak tree") doesn't count for it.
export function hasMention(
  text: string,
  name: string,
  longer: readonly string[] = []
): boolean {
  if (!name) return false
  const needle = `@${name}`
  const over = longer.filter(
    (n) => n.length > name.length && n.startsWith(name)
  )
  for (let i = text.indexOf(needle); i >= 0; i = text.indexOf(needle, i + 1))
    if (
      mentionAt(text, i, name) &&
      !over.some((n) => text.startsWith(n, i + 1) && mentionAt(text, i, n))
    )
      return true
  return false
}

// countMention is how many whole "@name" spans text holds (as
// hasMention counts them).
export function countMention(
  text: string,
  name: string,
  longer: readonly string[] = []
): number {
  if (!name) return 0
  const needle = `@${name}`
  const over = longer.filter(
    (n) => n.length > name.length && n.startsWith(name)
  )
  let n = 0
  for (let i = text.indexOf(needle); i >= 0; i = text.indexOf(needle, i + 1))
    if (
      mentionAt(text, i, name) &&
      !over.some((o) => text.startsWith(o, i + 1) && mentionAt(text, i, o))
    )
      n++
  return n
}

// keepMentions drops the mentions whose "@name" left the text. Two
// mentions with one name (two agents called oak) need two "@oak" spans:
// with fewer, the latest-added go first.
export function keepMentions(
  mentions: readonly Mention[],
  text: string
): Mention[] {
  const names = mentions.map((m) => m.name)
  const left = new Map<string, number>()
  const out = mentions.filter((m) => {
    if (!left.has(m.name)) left.set(m.name, countMention(text, m.name, names))
    const n = left.get(m.name) ?? 0
    if (n <= 0) return false
    left.set(m.name, n - 1)
    return true
  })
  return out.length === mentions.length ? (mentions as Mention[]) : out
}

// isFull says mentions holds MaxMentions already.
export function isFull(mentions: readonly Mention[]): boolean {
  return mentions.length >= MaxMentions
}

// addMention records m, once per key, at most MaxMentions.
export function addMention(
  mentions: readonly Mention[],
  m: Mention
): Mention[] {
  if (mentions.some((x) => x.key === m.key)) return mentions as Mention[]
  if (mentions.length >= MaxMentions) return mentions as Mention[]
  return [...mentions, m]
}

// exactUnique is the one live candidate named query exactly (ignoring
// case), or null when none or several are.
export function exactUnique(
  query: string,
  cands: readonly MentionCandidate[]
): MentionCandidate | null {
  const q = query.toLowerCase()
  if (!q) return null
  const hits = cands.filter(
    (c) => !isGone(c.who) && c.who.name.toLowerCase() === q
  )
  return hits.length === 1 ? hits[0] : null
}

// settle records the token at the caret when it is exactly one live
// agent's name (a space typed after it, or the text sent). The name is
// kept as typed, so the text still holds it.
export function settle(
  text: string,
  caret: number,
  mentions: readonly Mention[],
  cands: readonly MentionCandidate[]
): Mention[] {
  const t = tokenAt(
    text,
    caret,
    cands.map((c) => c.who.name)
  )
  if (!t) return mentions as Mention[]
  const name = bare(t.query)
  const hit = exactUnique(name, cands)
  if (!hit) return mentions as Mention[]
  return addMention(mentions, { key: hit.who.key, name })
}

// settleRange records each whole "@name" in text whose "@" is in
// [from, to) and whose name is exactly one live agent's (pasted text, or
// a mention Ctrl+Z brought back). Longest names first, so "@oak tree"
// wins over "@oak".
export function settleRange(
  text: string,
  from: number,
  to: number,
  mentions: readonly Mention[],
  cands: readonly MentionCandidate[]
): Mention[] {
  const names = [
    ...new Set(cands.filter((c) => !isGone(c.who)).map((c) => c.who.name)),
  ]
    .filter(Boolean)
    .sort((a, b) => b.length - a.length)
  let out = mentions as Mention[]
  const lower = text.toLowerCase()
  for (
    let i = text.indexOf("@", Math.max(0, from));
    i >= 0 && i < to;
    i = text.indexOf("@", i + 1)
  ) {
    if (i > 0 && !before.test(text[i - 1])) continue
    const n = names.find(
      (x) => lower.startsWith(x.toLowerCase(), i + 1) && mentionAt(text, i, x)
    )
    if (!n) continue
    const written = text.slice(i + 1, i + 1 + n.length)
    const hit = exactUnique(written, cands)
    if (hit && !out.some((m) => m.key === hit.who.key))
      out = addMention(out, { key: hit.who.key, name: written })
  }
  return out
}

// MenuKey is what a key does with the picker open: "hold" swallows it.
export type MenuKey = "next" | "prev" | "pick" | "close" | "hold" | null

// menuKey maps a key press to the picker: only while it is open, and
// never while an IME is composing. With no rows (a typo, or the list
// still loading) Enter is held, so it doesn't send what the person can
// see is unfinished; Tab moves focus as usual.
export function menuKey(
  e: { key: string; shiftKey: boolean; isComposing?: boolean },
  open: boolean,
  rows: number
): MenuKey {
  if (!open || e.isComposing) return null
  if (e.key === "Escape") return "close"
  if (rows === 0) return e.key === "Enter" && !e.shiftKey ? "hold" : null
  switch (e.key) {
    case "Tab":
      return e.shiftKey ? "prev" : "next"
    case "ArrowDown":
      return "next"
    case "ArrowUp":
      return "prev"
    case "Enter":
      return e.shiftKey ? null : "pick"
  }
  return null
}

// MentionSpan is a piece of text: plain, or one mention's "@name".
export type MentionSpan<M> =
  { kind: "text"; text: string } | { kind: "mention"; text: string; mention: M }

// splitMentions cuts text around each "@name" a mention names (longest
// name first, so "@oak tree" wins over "@oak"). Any other "@word" stays
// text.
export function splitMentions<M extends { name: string }>(
  text: string,
  mentions: readonly M[]
): MentionSpan<M>[] {
  const named = mentions
    .filter((m) => m.name)
    .slice()
    .sort((a, b) => b.name.length - a.name.length)
  if (!named.length || !text.includes("@")) return [{ kind: "text", text }]
  // Same-named mentions go out in order: the k-th "@oak" is the k-th
  // mention named oak (the last one for any past that).
  const used = new Map<string, number>()
  const nth = (name: string): M => {
    const all = mentions.filter((x) => x.name === name)
    const k = used.get(name) ?? 0
    used.set(name, k + 1)
    return all[Math.min(k, all.length - 1)]
  }
  const out: MentionSpan<M>[] = []
  let from = 0
  for (let i = text.indexOf("@"); i >= 0; i = text.indexOf("@", i + 1)) {
    if (i < from) continue
    const hit = named.find(
      (x) => text.startsWith(x.name, i + 1) && mentionAt(text, i, x.name)
    )
    if (!hit) continue
    const m = nth(hit.name)
    if (i > from) out.push({ kind: "text", text: text.slice(from, i) })
    const end = i + 1 + m.name.length
    out.push({ kind: "mention", text: text.slice(i, end), mention: m })
    from = end
  }
  if (from < text.length) out.push({ kind: "text", text: text.slice(from) })
  return out
}

// notDelivered is the line under a box after a send: each mention that
// didn't reach its agent, or "" when all did.
export function notDelivered(
  outcomes:
    readonly { name: string; state: string; note?: string }[] | undefined
): string {
  const parts = (outcomes ?? [])
    .filter((o) => o.state !== "woke")
    .map((o) => {
      switch (o.state) {
        case "gone":
          return `${o.name} is gone`
        case "unknown":
          return `no agent named ${o.name}`
        case "held":
          return `${o.name} is held${o.note ? ` (${o.note})` : ""}`
        default:
          return `${o.name}${o.note ? `: ${o.note}` : " failed"}`
      }
    })
  return parts.length ? `Not delivered: ${parts.join("; ")}.` : ""
}

// askingLine is the ask box's line while mentions target it: "Asking
// @oak only", "Asking @oak and @fern only".
export function askingLine(mentions: readonly Mention[]): string {
  const names = mentions.map((m) => `@${m.name}`)
  if (!names.length) return ""
  const list =
    names.length === 1
      ? names[0]
      : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`
  return `Asking ${list} only`
}

// MissedMs is how long the "Not delivered" line stays under a box.
const MissedMs = 8000

// useMissed is the "Not delivered: …" line under a box after a send: set
// it with the send's outcomes; it clears itself after a few seconds.
export function useMissed(): [
  string,
  (
    outcomes:
      readonly { name: string; state: string; note?: string }[] | undefined
  ) => void,
] {
  const [line, setLine] = useState("")
  useEffect(() => {
    if (!line) return
    const id = window.setTimeout(() => setLine(""), MissedMs)
    return () => window.clearTimeout(id)
  }, [line])
  const set = useCallback(
    (
      o: readonly { name: string; state: string; note?: string }[] | undefined
    ) => setLine(notDelivered(o)),
    []
  )
  return [line, set]
}

// CardRef is one card ref in text (card #114): "#12" (tag null) or
// "mlg#12", as the Core's card_refs reads them: a whole word, after the
// start, whitespace or "(", and not followed by a letter, digit or "_".
export type CardRef = {
  start: number
  end: number
  tag: string | null
  n: string
}

export function cardRefs(text: string): CardRef[] {
  const out: CardRef[] = []
  const re = /([A-Za-z0-9_-]*)#(\d+)(?![A-Za-z0-9_])/g
  for (let m = re.exec(text); m; m = re.exec(text)) {
    const start = m.index
    if (start > 0 && !/[\s(]/.test(text[start - 1])) continue
    out.push({ start, end: start + m[0].length, tag: m[1] || null, n: m[2] })
  }
  return out
}

// RefHit is what a card ref names among what the window already holds
// (card #144): one live card, one thread, or null (look it up: a done card
// or an old thread, or nothing).
export type RefHit = {
  kind: "card" | "thread"
  id: string
  title: string
} | null

// localRef resolves r against the live cards, then the threads (own board,
// so only an untagged ref). More than one live card leaves it to the
// lookup, which says ambiguous (and the ref stays text). A bare ref whose
// only live card is on a shared board goes to the lookup too (card #343):
// the backend reads "#4" across every board, done cards included, so a
// done #4 of your own and a live mlg#4 is ambiguous, not mlg#4.
export function localRef(
  r: CardRef,
  cards: readonly { id: string; short: string; title: string; tag?: string }[],
  threads: readonly {
    id: string
    short: string
    title: string
    spaceId?: string
  }[]
): RefHit {
  const c = cards.filter(
    (x) => x.short === r.n && (r.tag === null || (x.tag ?? "") === r.tag)
  )
  if (c.length === 1 && (r.tag !== null || !c[0].tag))
    return { kind: "card", id: c[0].id, title: c[0].title }
  if (c.length > 0 || r.tag !== null) return null
  // A space's thread numbers in its own log, so a bare ref is the backend's
  // to read across them (card #558).
  const t = threads.filter((x) => x.short === r.n && !x.spaceId)
  if (t.length === 1) return { kind: "thread", id: t[0].id, title: t[0].title }
  return null
}

// refText is a ref as the backend reads it: "#12" or "mlg#12".
export function refText(r: Pick<CardRef, "tag" | "n">): string {
  return `${r.tag ?? ""}#${r.n}`
}

// cardTokenAt is the "#query" the caret is in (card #114), or null: a "#"
// at the start or after whitespace or "(", up to the caret, no newline.
// A space ends it unless some card title still holds the query.
export function cardTokenAt(
  text: string,
  caret: number,
  titles: readonly string[] = []
): Token | null {
  const head = text.slice(0, caret)
  const at = head.lastIndexOf("#")
  if (at < 0 || (at > 0 && !before.test(head[at - 1]))) return null
  const query = head.slice(at + 1)
  if (query.includes("\n") || query.includes("@") || query.length > 64)
    return null
  if (!/\s/.test(query)) return { start: at, query }
  const q = query.toLowerCase()
  if (titles.some((t) => t.toLowerCase().includes(q)))
    return { start: at, query }
  return null
}

// CardCandidate is one row of the # picker.
export type CardCandidate = {
  id: string
  ref: string
  short: string
  title: string
}

// filterCards are the # picker's rows for query: a number matches the
// cards whose number starts with it, words match titles. Number matches
// first, then title starts, then the rest in the list's order.
export function filterCards<C extends CardCandidate>(
  cards: readonly C[],
  query: string
): C[] {
  const q = bare(query).toLowerCase().trim()
  if (!q) return cards.slice(0, MaxRows)
  const rank = (c: CardCandidate) =>
    c.short.startsWith(q) ? 0 : c.title.toLowerCase().startsWith(q) ? 1 : 2
  return cards
    .filter((c) => c.short.startsWith(q) || c.title.toLowerCase().includes(q))
    .map((c, i) => ({ c, r: rank(c), i }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .slice(0, MaxRows)
    .map((x) => x.c)
}

// insertCardRef puts "#12 " (or "mlg#12 ") in place of the token and the
// rest of the word the caret is in.
export function insertCardRef(
  text: string,
  t: Token,
  caret: number,
  ref: string
): { text: string; caret: number } {
  let end = caret
  while (end < text.length && !after.test(text[end])) end++
  const rest = text.slice(end)
  const space = /^\s/.test(rest) ? "" : " "
  const ins = `${ref}${space}`
  return {
    text: text.slice(0, t.start) + ins + rest,
    caret: t.start + ins.length + (space ? 0 : 1),
  }
}
