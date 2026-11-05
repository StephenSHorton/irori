// Later and order (card #570). A parked card is out of the way until its
// condition comes; an order is a run of cards each waiting on the last.
// MOCK-UP: read from the ?later=1 sample, to become fields on CardSummary.

import type { CardSummary } from "@/lib/cards"

export type LaterBack = "card" | "date" | "version" | "word"

export type Later = {
  // why is the line the card was parked with.
  why: string
  back: LaterBack
  // line says what brings it back, in the person's words.
  line: string
  by: string
}

export type OrderState = "done" | "working" | "waiting" | "later"

export type OrderStep = {
  id: string
  short: string
  title: string
  state: OrderState
}

const parked = new Map<string, Later>()
const chains: OrderStep[][] = []

export function sampleLater(id: string, l: Later) {
  parked.set(id, l)
}

export function sampleChain(c: OrderStep[]) {
  if (!chains.some((x) => x[0].id === c[0].id)) chains.push(c)
}

export function laterOf(c: { id: string }): Later | undefined {
  return parked.get(c.id)
}

// The chains among these cards, each in order.
export function chainsOf(cards: readonly CardSummary[]): OrderStep[][] {
  return chains.filter((ch) => ch.some((s) => cards.some((c) => c.id === s.id)))
}

export function chainOf(c: { id: string }): OrderStep[] | undefined {
  return chains.find((ch) => ch.some((s) => s.id === c.id))
}

// orderLine is a card's place: "3 of 4, after #81, then #83".
export function orderLine(c: { id: string }): string {
  const ch = chainOf(c)
  if (!ch) return ""
  const i = ch.findIndex((s) => s.id === c.id)
  const parts = [`${i + 1} of ${ch.length}`]
  if (i > 0) parts.push(`after #${ch[i - 1].short}`)
  if (i < ch.length - 1) parts.push(`then #${ch[i + 1].short}`)
  return parts.join(", ")
}
