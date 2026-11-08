// Why a card sits where it does on the canvas (card #776). The policy is
// docs/canvas-layout.md; this is it in code: named weights for what makes
// a card prominent (it comes first, top left in its area, and its area
// comes first) and what fades it, plus the reasons the canvas shows on
// hover. Pulls (which area a card is in, a subtask under its parent) are
// structure, not weights: toGraph and the ELK graph (elk.ts) carry them.
//
// Pure and deterministic: the same cards and the same now give the same
// scores on every device.

// Weight is what each fact adds to a card's prominence. Bigger comes
// first; a negative weight pushes a card back.
export const Weight = {
  needsYou: 100, // it waits on your answer or your Done
  live: 40, // an agent is working on it right now
  recent: 20, // it changed in the last hour
  stale: -30, // nothing has happened on it for a while
} as const

// AreaWeight orders the areas: you first, then the busiest hubs; an idle
// agent's hub falls behind the working ones.
export const AreaWeight = {
  you: 1000, // your own hub leads
  working: 40, // the hub's agent is working now
  idle: -20, // the hub's agent is idle
} as const

export const RecentMs = 60 * 60 * 1000

// Facts is what a card's prominence is made of (toGraph fills it).
export type Facts = {
  needs: boolean
  live: boolean
  recent: boolean
  stale: boolean
  done: boolean
}

// score is a card's prominence: the sum of the weights its facts carry.
// A done card has none: it sits in Done whatever it was.
export function score(f: Facts): number {
  if (f.done) return 0
  return (
    (f.needs ? Weight.needsYou : 0) +
    (f.live ? Weight.live : 0) +
    (f.recent ? Weight.recent : 0) +
    (f.stale ? Weight.stale : 0)
  )
}

// Where says which area a card is in and why, for its first reason.
export type Where =
  | { by: "you" }
  | { by: "agent"; name: string }
  | { by: "unheld" }
  | { by: "done" }

// Reason is one line of "why it's here", with how much it counts (the
// area and the parent always lead: they say where, the rest say how far
// forward).
export type Reason = { text: string; weight: number }

const Structure = 10_000

// reasons is why a card sits where it does, strongest first; the canvas
// shows the top few on hover.
export function reasons(
  f: Facts,
  where: Where,
  parent: string | null
): Reason[] {
  const out: Reason[] = []
  if (where.by === "you")
    out.push({ text: "In your hub: you hold it", weight: Structure + 2 })
  else if (where.by === "agent")
    out.push({
      text: `In ${where.name}'s hub: ${where.name} holds it`,
      weight: Structure + 2,
    })
  else if (where.by === "unheld")
    out.push({
      text: "With the cards nobody is on",
      weight: Structure + 2,
    })
  else
    out.push({
      text: "In Done, at the end and faded: it's finished",
      weight: Structure + 2,
    })
  if (parent)
    out.push({
      text: `Under ${parent}: it's a subtask of it`,
      weight: Structure + 1,
    })
  if (f.done) return out
  if (f.needs)
    out.push({
      text: "Near the front: it waits on you",
      weight: Weight.needsYou,
    })
  if (f.live)
    out.push({
      text: "Near the front: an agent is working on it now",
      weight: Weight.live,
    })
  if (f.recent)
    out.push({
      text: "Near the front: it changed in the last hour",
      weight: Weight.recent,
    })
  if (f.stale)
    out.push({
      text: "Further back: nothing has happened on it for a while",
      weight: Weight.stale,
    })
  return out.sort((a, b) => Math.abs(b.weight) - Math.abs(a.weight))
}

// byProminence orders cards: most prominent first, then by their number
// (the order they were made), then id, so every device agrees.
export function byProminence<
  T extends { score: number; short: string; id: string },
>(a: T, b: T): number {
  return (
    b.score - a.score ||
    num(a.short) - num(b.short) ||
    (a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
  )
}

function num(short: string): number {
  const n = Number.parseInt(short.replace(/^#/, ""), 10)
  return Number.isFinite(n) ? n : Number.MAX_SAFE_INTEGER
}
