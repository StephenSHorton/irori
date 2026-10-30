import { CurvePath, LineCurve3, QuadraticBezierCurve3, Vector3, type Curve } from 'three'

/**
 * Board layout for the hero. Coordinates are [x, z] on the board; y is up.
 * Machine A ("laptop") holds the hearth and six agents. Machine B ("desk-pc")
 * is a smaller board to the right with two agents, linked by one cable.
 */

export type P2 = [number, number]

/** Top surface of both boards. */
export const TOP = 0.48
/** Trace centre height (just above the board). */
export const TRACE_Y = TOP + 0.016
export const BOARD_A_HALF = 6.25
/** Half size of the hearth's square pit. */
export const PIT = 1.35
export const BOARD_B = { center: [12, -5] as P2, half: 3 }

export type NodeId = 'n0' | 'n1' | 'n2' | 'n3' | 'n4' | 'n5' | 'm0' | 'm1' | 'hearth'

export const NODES: { id: Exclude<NodeId, 'hearth'>; pos: P2 }[] = [
  { id: 'n0', pos: [-4.2, -3.2] },
  { id: 'n1', pos: [0, -4.4] },
  { id: 'n2', pos: [4.2, -3.2] },
  { id: 'n3', pos: [4.2, 3.2] },
  { id: 'n4', pos: [0, 4.4] },
  { id: 'n5', pos: [-4.2, 3.2] },
  { id: 'm0', pos: [10.6, -3.9] },
  { id: 'm1', pos: [13.3, -6.3] },
]

export type Route = {
  id: string
  a: NodeId
  b: NodeId
  /** Flat traces: polylines on the board. */
  lines: P2[][]
  /** Optional raised cable between boards, inserted after lines[0]. */
  arc?: { from: P2; to: P2; lift: number }
}

const P = PIT + 0.35 // where traces meet the hearth's frame

export const ROUTES: Route[] = [
  // agents to the hearth
  { id: 'h0', a: 'n0', b: 'hearth', lines: [[[-4.2, -3.2], [-2.6, -3.2], [-2.6, -0.6], [-P, -0.6]]] },
  { id: 'h1', a: 'n1', b: 'hearth', lines: [[[0, -4.4], [0, -P]]] },
  { id: 'h2', a: 'n2', b: 'hearth', lines: [[[4.2, -3.2], [2.6, -3.2], [2.6, -0.6], [P, -0.6]]] },
  { id: 'h3', a: 'n3', b: 'hearth', lines: [[[4.2, 3.2], [2.6, 3.2], [2.6, 0.6], [P, 0.6]]] },
  { id: 'h4', a: 'n4', b: 'hearth', lines: [[[0, 4.4], [0, P]]] },
  { id: 'h5', a: 'n5', b: 'hearth', lines: [[[-4.2, 3.2], [-2.6, 3.2], [-2.6, 0.6], [-P, 0.6]]] },
  // the ring between neighbours
  { id: 'r01', a: 'n0', b: 'n1', lines: [[[-4.2, -3.2], [-4.2, -5.3], [-0.9, -5.3], [-0.9, -4.4], [0, -4.4]]] },
  { id: 'r12', a: 'n1', b: 'n2', lines: [[[0, -4.4], [0.9, -4.4], [0.9, -5.3], [4.2, -5.3], [4.2, -3.2]]] },
  { id: 'r23', a: 'n2', b: 'n3', lines: [[[4.2, -3.2], [4.2, -1.7], [5.3, -1.7], [5.3, 3.2], [4.2, 3.2]]] },
  { id: 'r34', a: 'n3', b: 'n4', lines: [[[4.2, 3.2], [4.2, 5.3], [0.9, 5.3], [0.9, 4.4], [0, 4.4]]] },
  { id: 'r45', a: 'n4', b: 'n5', lines: [[[0, 4.4], [-0.9, 4.4], [-0.9, 5.3], [-4.2, 5.3], [-4.2, 3.2]]] },
  { id: 'r50', a: 'n5', b: 'n0', lines: [[[-4.2, 3.2], [-5.3, 3.2], [-5.3, -3.2], [-4.2, -3.2]]] },
  // machine B
  { id: 'b01', a: 'm0', b: 'm1', lines: [[[10.6, -3.9], [10.6, -6.3], [13.3, -6.3]]] },
  // the cross-device cable: n2 on the laptop to m0 on the desk PC
  {
    id: 'x',
    a: 'n2',
    b: 'm0',
    lines: [
      [
        [4.2, -3.2],
        [6.1, -3.2],
      ],
      [
        [9.1, -3.9],
        [10.6, -3.9],
      ],
    ],
    arc: { from: [6.1, -3.2], to: [9.1, -3.9], lift: 1.4 },
  },
]

function v(p: P2, y = TRACE_Y) {
  return new Vector3(p[0], y, p[1])
}

/** The whole route as one curve from a to b, for pulses. */
export function routeCurve(route: Route): CurvePath<Vector3> {
  const path = new CurvePath<Vector3>()
  const addLine = (pts: P2[]) => {
    for (let i = 0; i < pts.length - 1; i += 1) path.add(new LineCurve3(v(pts[i]), v(pts[i + 1])) as Curve<Vector3>)
  }
  addLine(route.lines[0])
  if (route.arc) path.add(arcCurve(route.arc) as Curve<Vector3>)
  for (const line of route.lines.slice(1)) addLine(line)
  return path
}

export function arcCurve(arc: NonNullable<Route['arc']>) {
  const mid: P2 = [(arc.from[0] + arc.to[0]) / 2, (arc.from[1] + arc.to[1]) / 2]
  return new QuadraticBezierCurve3(v(arc.from), v(mid, TRACE_Y + arc.lift), v(arc.to))
}

export function nodePos(id: NodeId): P2 {
  if (id === 'hearth') return [0, 0]
  return NODES.find((n) => n.id === id)!.pos
}

/* ------------------------------------------------------------------ */
/* Timeline. One loop; everything below is a pure function of time.  */
/* ------------------------------------------------------------------ */

export const LOOP = 16
export const PULSE_SPEED = 2.6

export type Message = { route: string; t0: number; reverse?: boolean; warm?: boolean }

/**
 * Messages in one loop. `reverse` runs b to a. Warm pulses go to the hearth
 * (status and capture), cool ones are agent-to-agent messages and wakes.
 */
export const MESSAGES: Message[] = [
  { route: 'h0', t0: 0.4, warm: true },
  { route: 'r12', t0: 1.0 },
  { route: 'h3', t0: 1.8, warm: true },
  { route: 'h2', t0: 2.9, warm: true }, // n2 reports it is blocked
  { route: 'x', t0: 4.6, reverse: true }, // the answer comes from the desk PC
  { route: 'r45', t0: 5.2, reverse: true },
  { route: 'h5', t0: 6.4, warm: true },
  { route: 'b01', t0: 7.0, reverse: true },
  { route: 'r23', t0: 8.4 },
  { route: 'h1', t0: 9.4, warm: true },
  { route: 'r50', t0: 10.2 },
  { route: 'h4', t0: 11.2, warm: true },
  { route: 'x', t0: 12.4 }, // n2 tells m0 it is done
  { route: 'r01', t0: 13.4, reverse: true },
  { route: 'h2', t0: 14.6, warm: true },
]

/** Time the story's answer reaches n2. Filled in once curves are built. */
export const STORY = {
  blockedAt: 3.0,
  answerRoute: 'x',
  answerT0: 4.6,
  doneAt: 12.0,
}

export type CardState = 'working' | 'blocked' | 'answered' | 'done'

/** The blocked card on n2: progress, then blocked, then answered by a pulse from m0. */
export function storyCard(t: number, answerArrives: number): { progress: number; state: CardState } {
  const { blockedAt, doneAt } = STORY
  if (t < blockedAt) return { progress: 0.18 + (t / blockedAt) * 0.34, state: 'working' }
  if (t < answerArrives) return { progress: 0.52, state: 'blocked' }
  if (t < doneAt) {
    const k = (t - answerArrives) / (doneAt - answerArrives)
    return { progress: 0.52 + k * 0.48, state: t - answerArrives < 2.4 ? 'answered' : 'working' }
  }
  return { progress: 1, state: 'done' }
}

/** Reduced motion shows this instant: the card is blocked and the answer is on the cable. */
export const STILL_T = 5.9
