// Card #468: the window on mocked IPC (perf.html). Every command answers
// from made-up data; the harness waits for the window to settle, emits
// board:changed (and talk:changed) a few times, and writes what each one
// cost into #perf-out as JSON: commands invoked by name, React commits,
// and components rendered. Run with scripts/perf.sh.
//
// ?notifs=K gives the Notifications list K card rows; ?notes=N gives
// notes_list N notes. The hash picks the page (#notifications).

import { emit } from "@tauri-apps/api/event"
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks"

import bindings from "../gen/bindings.ts?raw"
import { sampleCards } from "../lib/sample-cards"

const hook = { commits: 0, rendered: 0, names: new Map<string, number>() }

// While measuring, interval timers (App's 1 s clock, the pollers) skip
// their ticks, so what's counted is what the event caused.
let quiet = false
const realInterval = window.setInterval.bind(window)
window.setInterval = ((fn: TimerHandler, ms?: number, ...rest: unknown[]) =>
  realInterval(
    (...a: unknown[]) => {
      if (!quiet && typeof fn === "function")
        (fn as (...x: unknown[]) => void)(...a)
    },
    ms,
    ...rest
  )) as typeof window.setInterval

// React's devtools hook, before React loads: count commits, and the
// function and class components that rendered in each (PerformedWork),
// as React's devtools do: a subtree whose children are the ones it had
// (bailed out) wasn't visited, so its old flags don't count.
type Fiber = {
  tag: number
  flags: number
  type: unknown
  child: Fiber | null
  sibling: Fiber | null
  alternate: Fiber | null
}
function nameOf(t: unknown): string {
  if (!t) return "?"
  if (typeof t === "function")
    return (
      (t as { displayName?: string; name: string }).displayName || t.name || "?"
    )
  const o = t as { displayName?: string; render?: unknown; type?: unknown }
  return o.displayName || nameOf(o.render ?? o.type)
}
function countRendered(root: Fiber | null): number {
  let n = 0
  const stack: Fiber[] = []
  if (root) stack.push(root)
  while (stack.length) {
    const x = stack.pop()!
    if (x.sibling) stack.push(x.sibling)
    // 0 function, 1 class, 11 forwardRef, 14/15 memo; PerformedWork is 1.
    const component =
      x.tag === 0 || x.tag === 1 || x.tag === 11 || x.tag === 14 || x.tag === 15
    if (component && (x.flags & 1 || !x.alternate)) {
      n++
      const k = nameOf(x.type)
      hook.names.set(k, (hook.names.get(k) ?? 0) + 1)
    }
    if (x.child && (!x.alternate || x.child !== x.alternate.child))
      stack.push(x.child)
  }
  return n
}
;(window as unknown as Record<string, unknown>).__REACT_DEVTOOLS_GLOBAL_HOOK__ =
  {
    supportsFiber: true,
    isDisabled: false,
    renderers: new Map(),
    inject() {
      return 1
    },
    checkDCE() {},
    onScheduleFiberRoot() {},
    onCommitFiberRoot(_id: number, root: { current: Fiber }) {
      hook.commits++
      hook.rendered += countRendered(root.current.child)
    },
    onCommitFiberUnmount() {},
    onPostCommitFiberRoot() {},
  }

// Each command's answer type, read from the bindings: arrays answer [],
// numbers 0, strings "", booleans false, an object type each of its
// fields so (optional ones null), unless set in answer below.
const returns = new Map<string, string>()
for (const m of bindings.matchAll(/__TAURI_INVOKE<(.+?)>\("(\w+)"/g))
  returns.set(m[2], m[1])
const types = new Map<string, [string, string][]>()
for (const m of bindings.matchAll(
  /^export type (\w+) = \{\n([\s\S]*?)^\};/gm
)) {
  const fields: [string, string][] = []
  for (const f of m[2].matchAll(/^\t(\w+)(\??): (.+?),$/gm))
    fields.push([f[1], f[2] ? "null" : f[3]])
  types.set(m[1], fields)
}
function zero(t: string, depth = 0): unknown {
  t = t.trim()
  if (t.endsWith("[]") || t.startsWith("Partial<{"))
    return t.endsWith("[]") ? [] : {}
  if (t === "number") return 0
  if (t === "string") return ""
  if (t === "boolean") return false
  if (t.includes("| null") || t === "null") return null
  const fields = types.get(t)
  if (!fields || depth > 3) return null
  return Object.fromEntries(fields.map(([k, ft]) => [k, zero(ft, depth + 1)]))
}

const params = new URLSearchParams(window.location.search)
const K = Number(params.get("notifs") ?? "0")
const N = Number(params.get("notes") ?? "50")
const now = new Date()
const cards = sampleCards(now)
// Each board:changed the harness sends changes one card (its first), as
// an agent ticking a step does, so renders show what a real change costs.
let gen = 0
let talkGen = 0
let seq = 0
function current() {
  if (!gen) return cards
  const c = { ...cards[0], title: `${cards[0].title} (${gen})` }
  return [c, ...cards.slice(1)]
}
const iso = now.toISOString()

function notifications() {
  const out = []
  for (let i = 0; i < K; i++) {
    const c = cards[i % cards.length]
    const b = c.blockers[0]
    out.push({
      key: `n${i}`,
      kind: b ? "blocker" : "input",
      title: `Row ${i}`,
      body: "",
      level: "",
      at: iso,
      read: false,
      taskId: c.id,
      blockerId: b?.id ?? "",
      commentId: "",
      deliveryId: "",
      boardId: "",
      card: { id: c.id, short: String(i + 1), title: c.title },
    })
  }
  return out
}

function notes() {
  return Array.from({ length: N }, (_, i) => ({
    id: `note${i}`,
    scope: "mine",
    boardId: "",
    kind: "rule",
    text: `Note ${i}`,
    tags: [`path:repo:src/f${i}.ts`],
    source: "",
    taskId: "",
    by: "agent",
    byPerson: false,
    at: iso,
    state: i % 10 === 0 ? "proposed" : "accepted",
    acceptedBy: "",
    acceptedAt: iso,
    updated: iso,
    pending: null,
    history: [],
    stale: "",
  }))
}

function answer(cmd: string, args: Record<string, unknown>): unknown {
  switch (cmd) {
    case "core_ready":
      return null
    case "board_cards":
    case "board_cards_cold":
      return current()
    case "board_cards_patch": {
      // The rows named and the whole order (card #1147).
      const ids = args.ids as string[]
      const all = current()
      return {
        rows: all.filter((c) => ids.includes(c.id)),
        order: all.map((c) => c.id),
      }
    }
    case "board_card":
    case "board_cards_by_id": {
      const ids = cmd === "board_card" ? [args.id] : (args.ids as string[])
      // The rev the host stamps a card with (card #1136): it moves when the
      // card does, so a card that didn't change comes back `unchanged`.
      const have = new Map(
        ((args.have as { id: string; rev: string }[] | undefined) ?? []).map(
          (h) => [h.id, h.rev]
        )
      )
      const got = ids.map((id) => {
        const c = current().find((x) => x.id === id)
        if (!c) throw new Error(`no such card: ${String(id)}`)
        return c
      })
      return cmd === "board_card"
        ? got[0]
        : got.map((card) => {
            const rev = `r${card.id}:${card.title}`
            return have.get(card.id) === rev
              ? { id: card.id, card: null, error: "", rev, unchanged: true }
              : { id: card.id, card, error: "", rev, unchanged: false }
          })
    }
    case "talk_conversations_by_id":
    case "talk_conversations": {
      // One conversation a talk:changed edits, so a reload shows a change.
      const c = zero(
        returns.get("talk_conversations")?.replace(/\[\]$/, "") ?? "null"
      ) as Record<string, unknown> | null
      return c ? [{ ...c, id: "conv1", name: `general (${talkGen})` }] : []
    }
    case "notifications_list":
      return notifications()
    case "notes_list":
      return notes()
    case "notes_waiting":
      return Math.ceil(N / 10)
  }
  // The by-id and by-key reads the event-driven window adds (design
  // list-signals.md): a list of rows, none until a build fills them.
  if (/_by_(id|key)$/.test(cmd) || cmd.endsWith("_patch")) return []
  return zero(returns.get(cmd) ?? "null")
}

const calls = new Map<string, number>()
// The ids each CardsById read asked for, since the last reset (card #1148).
const cardIds: string[][] = []
const errors = new Map<string, number>()
// The window's own state, which visibility.ts (card #1134) asks: a window
// the person can see. Without a mocked window the page never mounts.
mockWindows("main")
mockIPC(
  (cmd, args) => {
    if (cmd === "plugin:window|is_visible") return true
    if (cmd.startsWith("plugin:")) return null
    calls.set(cmd, (calls.get(cmd) ?? 0) + 1)
    if (cmd === "board_cards_by_id")
      cardIds.push([...((args as { ids?: string[] })?.ids ?? [])])
    try {
      return answer(cmd, (args ?? {}) as Record<string, unknown>)
    } catch (e) {
      errors.set(cmd, (errors.get(cmd) ?? 0) + 1)
      throw e
    }
  },
  { shouldMockEvents: true }
)

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

function snapshot() {
  return {
    ipc: [...calls.values()].reduce((a, b) => a + b, 0),
    commits: hook.commits,
    rendered: hook.rendered,
    top: [...hook.names]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 12)
      .map(([k, v]) => `${k} ${v}`)
      .join(", "),
    calls: Object.fromEntries([...calls].sort((a, b) => b[1] - a[1])),
    // CardsById ids per read: after a notice, touched ∩ watched.
    cardIds: cardIds.map((ids) => ids.length),
  }
}
function reset() {
  calls.clear()
  cardIds.length = 0
  hook.commits = 0
  hook.rendered = 0
  hook.names.clear()
}

// per emits event rounds times, each with payload when given (the signals
// carry ids and a seq once card 4 of list-signals.md lands), and counts
// what each cost.
async function per(event: string, rounds: number, payload?: () => unknown) {
  const each = []
  quiet = true
  for (let i = 0; i < rounds; i++) {
    await wait(300)
    reset()
    if (event === "board:changed") gen++
    if (event === "talk:changed") talkGen++
    if (event) await emit(event, payload?.())
    await wait(1500)
    each.push(snapshot())
  }
  quiet = false
  return each
}

async function run() {
  await import("../main.tsx")
  await wait(4000)
  const mount = snapshot()
  const result = {
    page: window.location.hash || "#",
    notifs: K,
    notes: N,
    mount,
    // No event: what timers alone cost in the same time.
    idle: await per("", 2),
    // The payloads of list-signals.md §4; a window that predates them
    // ignores them, so the same run measures before and after.
    boardChanged: await per("board:changed", 3, () => ({
      seq: ++seq,
      all: false,
      cards: [cards[0].id],
      notes: [],
      desk: false,
    })),
    talkChanged: await per("talk:changed", 2, () => ({
      seq: ++seq,
      all: false,
      convs: ["conv1"],
      calls: [],
    })),
    errors: Object.fromEntries(errors),
    mounted: document.getElementById("root")!.childElementCount > 0,
  }
  document.getElementById("perf-out")!.textContent = JSON.stringify(result)
  document.title = "perf-done"
}
void run()
