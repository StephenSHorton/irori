// Asking someone's assistant from your own chat (card #1063,
// docs/design/assistants-talk.md §9). The chat model can't post: asked to
// "ask Addam's assistant…" it ends its reply with an ask block,
//
//   <<ask Addam | Space name>>
//   the question
//   <</ask>>
//
// and the chat shows the exact normalised text (the host's, so the text
// shown is the text posted) with Send. Only a tap posts it, through a host
// command, as the person's own assistant card.

import type { AssistantAskPrepared } from "@/gen/bindings"
import { AssistantService } from "./host"
import { talkApi } from "./conversations"

export const AskOpen = "<<ask "
export const AskClose = "<</ask>>"

// AskDraft is an ask block as the model wrote it: the names it used and
// the question as written, before the host normalises it.
export type AskDraft = {
  person: string
  space: string
  text: string
  // Still being written, or never closed: not sendable.
  partial: boolean
}

const askOpen = /<<ask ([^\n|>]*)\|([^\n>]*)>>/
// A closing line, near misses too ("</ask>>", "<</ask>").
const askClose = /<{1,2}\s*\/\s*ask\s*>{1,2}/i
// An open line still being written ("…<<ask Add").
const askTail = /<<(a|as|ask|ask [^\n>]*)?$/

// splitAsk takes a reply's ask block out of its text. At most one: the
// first. A second block is left in the text, shown as the words it is.
export function splitAsk(raw: string): { text: string; ask: AskDraft | null } {
  const open = askOpen.exec(raw)
  if (!open) {
    const tail = askTail.exec(raw)
    return tail
      ? { text: raw.slice(0, tail.index).trim(), ask: null }
      : { text: raw, ask: null }
  }
  const before = raw.slice(0, open.index).trim()
  const rest = raw.slice(open.index + open[0].length)
  const close = askClose.exec(rest)
  const body = close ? rest.slice(0, close.index) : rest
  const after = close ? rest.slice(close.index + close[0].length).trim() : ""
  return {
    text: after ? `${before}\n\n${after}`.trim() : before,
    ask: {
      person: open[1].trim(),
      space: open[2].trim(),
      text: body.trim(),
      partial: !close,
    },
  }
}

// prepareAsk asks the host who the block names and for the exact text a
// tap would post. It posts nothing.
export function prepareAsk(ask: AskDraft): Promise<AssistantAskPrepared> {
  return AssistantService.AskPrepare(ask.person, ask.space, ask.text)
}

const sending = new Map<string, Promise<string>>()
const sent = new Set<string>()

const keyOf = (p: AssistantAskPrepared) => `${p.space}\n${p.person}\n${p.text}`

// sendAsk is the tap on Send: posts `p.text` (the text shown) as the
// person's assistant card. A second tap while it posts, or after it
// posted on this device, does nothing more. Returns the post's id.
export function sendAsk(p: AssistantAskPrepared): Promise<string> {
  const k = keyOf(p)
  const going = sending.get(k)
  if (going) return going
  if (sent.has(k)) return Promise.resolve("")
  const run = AssistantService.AskSend(p.space, p.person, p.text)
    .then((r) => {
      sent.add(k)
      return r.postId
    })
    .finally(() => sending.delete(k))
  sending.set(k, run)
  return run
}

// wasSent says this exact text was posted already: on this device by a
// tap, or by any of the person's devices, found in the channel as a post of
// their assistant card.
export async function wasSent(p: AssistantAskPrepared): Promise<boolean> {
  if (sent.has(keyOf(p))) return true
  try {
    const cards = await AssistantService.TalkCards()
    const card = cards.find((c) => c.space === p.space)
    if (!card) return false
    const posts = await talkApi.posts(card.conv)
    return posts.some(
      (x) => x.by === `agent:${card.card}` && !x.replyTo && x.text === p.text
    )
  } catch {
    return false
  }
}

// forgetSent is for tests.
export function forgetSent() {
  sending.clear()
  sent.clear()
}
