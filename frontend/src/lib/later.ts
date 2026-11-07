// Not yet and order (card #570). A held card is out of the way until what
// brings it back comes; an order is a run of cards each after the last.
// Both are fields on the card the host reads from coord: later and order.

import type {
  CardLater,
  CardOrder,
  LaterBack,
  OrderState,
  OrderStep,
} from "./models"

export type { LaterBack, OrderState, OrderStep }
export type Later = CardLater
export type Order = CardOrder

// What these helpers read off a card: a summary, a page, or a sample.
export type HasLater = { id: string; later?: CardLater | null }
export type HasOrder = { id: string; order?: CardOrder | null }

export function laterOf(c: HasLater): Later | undefined {
  return c.later ?? undefined
}

export function chainOf(c: HasOrder): OrderStep[] | undefined {
  const steps = c.order?.steps
  return steps && steps.length > 1 ? steps : undefined
}

// headOf is the card a chain hangs under: its first not done.
export function headOf(ch: OrderStep[]): OrderStep | undefined {
  return ch.find((s) => s.state !== "done")
}

// followsOf is the cards that hang under c in a list, in order.
export function followsOf(c: HasOrder): OrderStep[] {
  const ch = chainOf(c)
  if (!ch || headOf(ch)?.id !== c.id) return []
  return ch.slice(ch.findIndex((s) => s.id === c.id) + 1)
}

// follows reports whether c hangs under another card.
export function follows(c: HasOrder): boolean {
  const ch = chainOf(c)
  return !!ch && headOf(ch)?.id !== c.id
}

// whenLater is a held card's line under its title, its parts in order:
// "Not yet: on 1 Dec. Not before the release." Start now follows it.
export function whenLater(l: Later): string {
  const line = l.line.charAt(0).toLowerCase() + l.line.slice(1)
  const why = l.why.trim().replace(/[.!?]+$/, "")
  return why ? `Not yet: ${line}. ${why}.` : `Not yet: ${line}.`
}

// around is the card before c in its order and the one after it.
export function around(c: HasOrder): {
  before?: OrderStep
  after?: OrderStep
} {
  const ch = chainOf(c)
  if (!ch) return {}
  const i = ch.findIndex((s) => s.id === c.id)
  if (i < 0) return {}
  return { before: ch[i - 1], after: ch[i + 1] }
}
