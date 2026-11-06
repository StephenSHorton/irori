// The Board's search box (card #171): it narrows the cards already loaded,
// List and Canvas alike, to the ones every typed word matches. A word with
// a # is a card ref (#12, mlg#12) and matches that card exactly; any other
// word matches, case-insensitively, anywhere in the card's ref, title,
// open question, objective, done-when, step texts (searchText) or the
// names of who holds, is offered, watches or made it.

import { displayName, type AgentRef, type CardSummary } from "./cards"

// queryWords are a query's words, lower-cased; an empty or blank query has
// none and matches every card.
export function queryWords(query: string): string[] {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w && w !== "#")
}

function names(rs: (AgentRef | null | undefined)[]): string[] {
  const out: string[] = []
  for (const r of rs) {
    if (!r) continue
    out.push(displayName(r), r.name)
  }
  return out
}

// haystack is every word of the card a plain query word can match, lower-cased.
function haystack(c: CardSummary): string {
  return [
    `#${c.short}`,
    c.tag ? `${c.tag}#${c.short}` : "",
    c.title,
    c.blockerQuestion,
    c.searchText ?? "",
    ...names([
      ...c.working,
      ...c.watching,
      c.holder?.who,
      c.ask?.taker,
      c.returned,
    ]),
  ]
    .join("\n")
    .toLowerCase()
}

// refs are the ways the card's ref can be typed: #12, mlg#12 and the
// short it had before it was shared (shortWas).
function refs(c: CardSummary): string[] {
  const out = [`#${c.short}`]
  if (c.tag) out.push(`${c.tag}#${c.short}`)
  if (c.shortWas) out.push(`#${c.shortWas}`)
  return out.map((r) => r.toLowerCase())
}

// cardMatches says whether every word of query matches c.
export function cardMatches(c: CardSummary, query: string): boolean {
  const words = queryWords(query)
  if (!words.length) return true
  let hay: string | undefined
  return words.every((w) => {
    if (w.includes("#")) return refs(c).includes(w)
    hay ??= haystack(c)
    return hay.includes(w)
  })
}

// matching is the cards every word of query matches, in their order.
export function matching<T extends CardSummary>(
  cards: T[],
  query: string
): T[] {
  if (!queryWords(query).length) return cards
  return cards.filter((c) => cardMatches(c, query))
}
