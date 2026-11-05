// Not yet and order (card #570). A parked card is out of the way until its
// condition comes; an order is a run of cards each waiting on the last.
// MOCK-UP: read from the ?later=1 sample, to become fields on CardSummary.

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

export function chainOf(c: { id: string }): OrderStep[] | undefined {
  return chains.find((ch) => ch.some((s) => s.id === c.id))
}

// headOf is the card a chain hangs under: its first not done.
function headOf(ch: OrderStep[]): OrderStep | undefined {
  return ch.find((s) => s.state !== "done")
}

// followsOf is the cards that hang under c in a list, in order.
export function followsOf(c: { id: string }): OrderStep[] {
  const ch = chainOf(c)
  if (!ch || headOf(ch)?.id !== c.id) return []
  return ch.slice(ch.findIndex((s) => s.id === c.id) + 1)
}

// follows reports whether c hangs under another card.
export function follows(c: { id: string }): boolean {
  const ch = chainOf(c)
  return !!ch && headOf(ch)?.id !== c.id
}
