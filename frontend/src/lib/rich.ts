// Rich text for what agents write (a blocker's question and context, an
// option and its why, comments, thread items, a card's objective): no HTML
// and no markdown, only what makes it usable. http(s) URLs, and bare links
// like huggingface.co/x/y, become links that open in the person's browser;
// `inline code` becomes code with a Copy; line breaks and "1)" / "- " lists
// keep their shape. Anything else stays text. The input is untrusted: it
// is made plain first (plainLine / plainBlock), every link is checked by
// safeHref, and nothing here ever builds markup from it.

import { plainBlock, plainLine } from "./irori"

// Inline is one piece of a line. A link's text is what the agent wrote,
// split around its host (pre, host, rest) so the host can stand out; when
// the host can't be found in the text (an IDN host shows as punycode),
// host is "" and shownHost names it beside the text.
export type Inline =
  | { kind: "text"; text: string }
  | {
      kind: "link"
      text: string
      href: string
      pre: string
      host: string
      rest: string
      shownHost: string
    }
  | { kind: "code"; text: string }

// Block is a paragraph (its lines, in order) or a list whose items keep the
// marker the agent wrote ("1)", "2.", "-").
export type Block =
  | { kind: "para"; lines: Inline[][] }
  | { kind: "list"; items: { marker: string; inline: Inline[] }[] }

// TLDs a bare link may end in. Kept short on purpose, and without the ones
// that are also file extensions here (.go, .ts, .md, .js, .py, .rs, .sh,
// .so), so "board.go" or "cards.ts" stays text.
const bareTLDs = new Set([
  "com", "org", "net", "io", "ai", "dev", "app", "co", "me", "gg", "tv",
  "xyz", "edu", "gov", "info", "uk", "de", "jp", "fr", "ca", "us", "eu",
  "nl", "se", "ch", "au", "in", "ly", "link", "page", "site", "cloud",
  "tech", "run", "to", "fm", "im", "cc", "be", "it", "es", "kr", "cn",
])

// A URL with its scheme, or a bare host (two or more labels) with an
// optional port and path. Both stop at whitespace, quotes, angle brackets
// and backticks.
const linkRe =
  /https?:\/\/[^\s<>"'`]+|(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,24}(?::\d{2,5})?(?:\/[^\s<>"'`]*)?/gi

// What may not come right before a link: a word character, or something
// that makes it part of a bigger token (an email, a path, a scheme like
// javascript:// or file://, a longer host).
const notBefore = /[\p{L}\p{N}_/@.:\\-]/u

// safeHref is the URL to open for a link's text, or null: only http(s),
// with a host, and no user:password@ (it hides the real host).
export function safeHref(raw: string): string | null {
  const s = /^https?:\/\//i.test(raw) ? raw : `https://${raw}`
  let u: URL
  try {
    u = new URL(s)
  } catch {
    return null
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null
  if (!u.hostname || u.username || u.password) return null
  return u.href
}

// trimEnd drops punctuation that ends the sentence, not the link: a
// trailing . , ; : ! ? or quote, and a closing bracket with no opening one
// in the link.
function trimEnd(s: string): string {
  for (;;) {
    const last = s.at(-1)
    if (!last) return s
    if (".,;:!?'\"*".includes(last)) {
      s = s.slice(0, -1)
      continue
    }
    const pairs: Record<string, string> = { ")": "(", "]": "[", "}": "{" }
    const open = pairs[last]
    if (open && s.split(open).length <= s.split(last).length - 1) {
      s = s.slice(0, -1)
      continue
    }
    return s
  }
}

// linkOf is the link token for text, or null when it isn't one.
function linkOf(text: string, bare: boolean): Inline | null {
  if (bare) {
    const host = text.split(/[/:]/)[0].toLowerCase()
    const tld = host.split(".").at(-1) ?? ""
    if (!bareTLDs.has(tld)) return null
  }
  const href = safeHref(text)
  if (!href) return null
  const host = new URL(href).hostname
  const at = text.toLowerCase().indexOf(host)
  if (at < 0)
    return { kind: "link", text, href, pre: text, host: "", rest: "", shownHost: host }
  return {
    kind: "link",
    text,
    href,
    pre: text.slice(0, at),
    host: text.slice(at, at + host.length),
    rest: text.slice(at + host.length),
    shownHost: "",
  }
}

// links splits text (no code in it) into text and links.
function links(text: string): Inline[] {
  const out: Inline[] = []
  let last = 0
  const push = (t: string) => {
    if (!t) return
    const prev = out.at(-1)
    if (prev?.kind === "text") prev.text += t
    else out.push({ kind: "text", text: t })
  }
  for (const m of text.matchAll(linkRe)) {
    const start = m.index ?? 0
    if (start < last) continue
    const before = text[start - 1]
    if (before !== undefined && notBefore.test(before)) continue
    const raw = trimEnd(m[0])
    const tok = raw ? linkOf(raw, !/^https?:\/\//i.test(raw)) : null
    if (!tok) continue
    push(text.slice(last, start))
    out.push(tok)
    last = start + raw.length
  }
  push(text.slice(last))
  return out
}

// codeSpans finds `code` in a line: [start, end) of each span, backticks
// included. An unclosed backtick stays text.
function codeSpans(line: string): [number, number][] {
  const out: [number, number][] = []
  for (const m of line.matchAll(/`([^`]+)`/g)) {
    const start = m.index ?? 0
    if (m[1].trim()) out.push([start, start + m[0].length])
  }
  return out
}

// inline tokenizes one line (already plain).
function inline(line: string): Inline[] {
  const out: Inline[] = []
  let last = 0
  for (const [s, e] of codeSpans(line)) {
    out.push(...links(line.slice(last, s)))
    out.push({ kind: "code", text: line.slice(s + 1, e - 1) })
    last = e
  }
  out.push(...links(line.slice(last)))
  return out
}

// richInline is one line of untrusted text as tokens (plainLine first).
export function richInline(s: string): Inline[] {
  return inline(plainLine(s))
}

const listLine = /^\s*([-*•]|\d{1,3}[.)])\s+(\S.*)$/

// enumerated splits a line that counts "1) a 2) b" inline into its lead
// text and items, or returns null. Only the "N)" form, counting up from 1
// with at least two items, outside code.
function enumerated(line: string): { lead: string; items: string[] } | null {
  const spans = codeSpans(line)
  const inCode = (i: number) => spans.some(([s, e]) => i >= s && i < e)
  const marks: { at: number; end: number }[] = []
  for (const m of line.matchAll(/(^|\s)(\d{1,2})\)\s+/g)) {
    const at = (m.index ?? 0) + m[1].length
    if (inCode(at) || Number(m[2]) !== marks.length + 1) continue
    marks.push({ at, end: (m.index ?? 0) + m[0].length })
  }
  if (marks.length < 2) return null
  const items = marks.map((k, i) =>
    line.slice(k.end, marks[i + 1]?.at ?? line.length).trim()
  )
  if (items.some((t) => !t)) return null
  return { lead: line.slice(0, marks[0].at).trim(), items }
}

// richBlocks is untrusted multi-line text as blocks (plainBlock first):
// blank lines end a paragraph, list lines group into a list, and a line
// that counts "1) … 2) …" inline is shown as that list.
export function richBlocks(s: string): Block[] {
  const out: Block[] = []
  let para: Inline[][] | null = null
  let list: { marker: string; inline: Inline[] }[] | null = null
  const endPara = () => {
    if (para?.length) out.push({ kind: "para", lines: para })
    para = null
  }
  const endList = () => {
    if (list?.length) out.push({ kind: "list", items: list })
    list = null
  }
  const addItem = (marker: string, text: string) => {
    endPara()
    list ??= []
    list.push({ marker, inline: inline(text) })
  }
  const addLine = (text: string) => {
    endList()
    para ??= []
    para.push(inline(text))
  }
  for (const line of plainBlock(s).split("\n")) {
    if (!line.trim()) {
      endPara()
      endList()
      continue
    }
    const m = listLine.exec(line)
    if (m) {
      addItem(m[1], m[2])
      continue
    }
    const en = enumerated(line)
    if (en) {
      if (en.lead) addLine(en.lead)
      en.items.forEach((t, i) => addItem(`${i + 1})`, t))
      endList()
      continue
    }
    addLine(line)
  }
  endPara()
  endList()
  return out
}
