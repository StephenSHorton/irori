// Fuzzy matching for the command palette (card #213). A query is split into
// words; every word has to match the item (in its title, or else one of
// its other texts), and the item's score is the sum. A word scores best as
// the start of the title, then the start of a word in it, then anywhere in
// it, then as letters in order ("nwcd" finds "New card"), and less again
// when it only matches a keyword.

export type Scored<T> = { item: T; score: number }

// wordScore is how well one lower-cased query word matches one lower-cased
// text, or 0 for no match. loose: letters in order count too.
export function wordScore(word: string, text: string, loose = true): number {
  if (!word) return 1
  if (!text) return 0
  if (text === word) return 120
  if (text.startsWith(word)) return 100
  const at = text.indexOf(word)
  if (at >= 0) {
    // The start of a word in the text ("card" in "new card").
    if (/[\s\-_/#.:]/.test(text[at - 1])) return 80
    return 60
  }
  if (!loose || word.length < 2) return 0
  // Letters in order, each preferring the start of a word.
  let i = 0
  let bonus = 0
  let last = -2
  for (let j = 0; j < text.length && i < word.length; j++) {
    if (text[j] !== word[i]) continue
    if (j === 0 || /[\s\-_/#.:]/.test(text[j - 1])) bonus += 2
    if (j === last + 1) bonus += 1
    last = j
    i++
  }
  if (i < word.length) return 0
  return Math.min(50, 20 + bonus * 2)
}

// queryParts is the query's words, lower-cased.
export function queryParts(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean)
}

// score is how well every word of query matches title, with others (ref,
// keywords, names) as fallbacks at a lower weight and only as whole
// substrings, or 0 when one word matches nothing.
export function score(query: string, title: string, others: string[] = []): number {
  const words = queryParts(query)
  if (!words.length) return 1
  const t = title.toLowerCase()
  const o = others.filter(Boolean).map((x) => x.toLowerCase())
  let total = 0
  for (const w of words) {
    let best = wordScore(w, t)
    for (const x of o) best = Math.max(best, wordScore(w, x, false) * 0.8)
    if (!best) return 0
    total += best
  }
  // A shorter title is a closer match for the same words.
  return total - Math.min(t.length, 80) * 0.05
}

// rank is the items that match query, best first; ties keep their order.
export function rank<T>(
  items: T[],
  query: string,
  texts: (item: T) => { title: string; others?: string[] }
): Scored<T>[] {
  const out: (Scored<T> & { i: number })[] = []
  items.forEach((item, i) => {
    const { title, others } = texts(item)
    const s = score(query, title, others)
    if (s > 0) out.push({ item, score: s, i })
  })
  out.sort((a, b) => b.score - a.score || a.i - b.i)
  return out.map(({ item, score }) => ({ item, score }))
}
