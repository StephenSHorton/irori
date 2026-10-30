// v0.2 gateway: the inbox and reviving gone sessions (the Board has its own
// gateway in board.ts).
// Like irori.ts, components import from here, never from src/gen, and in a
// plain browser everything runs on the sample in sample-talk.ts (actions
// change that sample in memory so the buttons can be tried).

import { useCallback, useEffect, useRef, useState } from "react"

import { Events, InboxService, RosterService } from "./host"
import {
  HoldReason,
  Mode,
  State,
  type Delivery,
  type GoneSession,
  type Policy,
} from "./models"
import { inWails, parseTime, sessionShort } from "./irori"
import { sampleTalk, type SampleTalk } from "./sample-talk"

export type { Delivery, GoneSession, Policy }
export { HoldReason, Mode, State }

// Event names mirror the Go constants in inbox_service.go.
export const EventInboxChanged = "inbox:changed"
const EventRosterChanged = "roster:changed"

// InboxLimit is how many deliveries the Inbox tab asks for.
export const InboxLimit = 200

// MaxText mirrors inbox.MaxText.
export const MaxText = 8000

// HeldHistoryLimit is how many past held messages the Held list's History
// asks for.
export const HeldHistoryLimit = 100

// SystemKey mirrors inbox.SystemKey: Irori itself (an unpause, the sweep).
export const SystemKey = "irori"

// holdReasons mirror notify.HoldReason: why Irori held a message.
export const holdReasons: Record<string, string> = {
  [HoldReason.HoldBlocked]: "The sender is on your block list.",
  [HoldReason.HoldNotAllowed]: "The sender isn't on your allow list.",
  [HoldReason.HoldBypassReceiver]:
    "The recipient runs without permission prompts. Full auto in Settings lets these through.",
  [HoldReason.HoldHopLimit]:
    "It is too many hops down a chain of agent messages.",
  [HoldReason.HoldUnowned]: "No agent in that repo can take it yet.",
  [HoldReason.HoldPaused]: "Wakes are paused.",
  [HoldReason.HoldMemberWake]:
    "Another person asked to wake your agent, and your receive setting for that board asks you first.",
  [HoldReason.HoldCrossPerson]:
    "It comes from another person on a shared board, and your receive setting for that board asks you first.",
  [HoldReason.HoldMemberLease]:
    "Another person asks your agent to give up a lease.",
}

// holdText is why d was (or is) held, in plain words.
export function holdText(d: Delivery): string {
  const h = d.hold || d.heldFor || ""
  return holdReasons[h] ?? h
}

// allowLifts mirrors InboxService.AllowAndRelease: allowing the sender
// lifts only these holds.
export function allowLifts(d: Delivery): boolean {
  return (
    d.state === State.Held &&
    (d.hold === HoldReason.HoldNotAllowed ||
      d.hold === HoldReason.HoldBypassReceiver)
  )
}

// errorText reads a failed call: Wails rejects with an Error or with an
// object carrying message. Go error text is shown as plain text.
export function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  if (err && typeof err === "object" && "message" in err) {
    return String((err as { message: unknown }).message)
  }
  return String(err)
}

const message = errorText

// Names turns agent keys into labels: "you" for the person, the live
// roster name, else a name seen on a message or a gone session, else a
// short session id. Callers pass every source they have.
export function nameLookup(
  ...sources: Array<Iterable<[string, string]>>
): (key: string) => string {
  const names = new Map<string, string>()
  for (const src of sources) {
    for (const [k, n] of src) if (k && n && !names.has(k)) names.set(k, n)
  }
  return (key: string) => {
    if (!key) return ""
    if (key.startsWith("user:")) return "you"
    return names.get(key) || `agent ${sessionShort(key)}`
  }
}

// ---- sample mode ------------------------------------------------------------

let sample: SampleTalk | null = null
const sampleListeners = new Set<() => void>()

function sampleState(): SampleTalk {
  sample ??= sampleTalk(new Date())
  return sample
}

function sampleChanged() {
  for (const fn of sampleListeners) fn()
}

function onSample(fn: () => void): () => void {
  sampleListeners.add(fn)
  return () => sampleListeners.delete(fn)
}

function sampleDelivery(id: string): Delivery {
  const d = sampleState().deliveries.find((x) => x.id === id)
  if (!d) throw new Error("inbox: no such delivery or message")
  return d
}

function sampleSetState(id: string, state: State) {
  const s = sampleState()
  const now = new Date().toISOString()
  s.deliveries = s.deliveries.map((d) => {
    if (d.id !== id) return d
    // As the inbox folds it: the hold is kept in heldFor, and leaving held
    // or dismissed records who decided.
    const decided = d.state === State.Held || d.state === State.Dismissed
    return {
      ...d,
      state,
      hold: state === State.Dismissed ? d.hold : undefined,
      heldFor: d.hold || d.heldFor,
      ...(decided ? { decidedBy: `user:sample-device`, decidedAt: now } : {}),
      updatedAt: now,
    } as Delivery
  })
  sampleChanged()
}

// ---- actions ----------------------------------------------------------------

// Every action acts as the person at this device.
export const talk = {
  async send(to: string, repo: string, text: string): Promise<void> {
    if (inWails()) {
      await InboxService.Send(to, repo, text)
      return
    }
    const s = sampleState()
    const now = new Date().toISOString()
    const n = s.deliveries.length + 1
    s.deliveries = [
      {
        id: `sample-delivery-${n}`,
        messageId: `sample-message-${n}`,
        to: repo ? "" : to,
        repoKey: repo || undefined,
        state: repo ? State.Held : State.Queued,
        hold: repo ? HoldReason.HoldUnowned : undefined,
        reminded: false,
        createdAt: now,
        updatedAt: now,
        message: {
          id: `sample-message-${n}`,
          from: { key: "user:sample-device", name: "you", kind: "user" },
          to: { kind: repo ? "repo" : "agent", value: repo || to },
          text,
          hop: 0,
          trust: "user",
          sentAt: now,
        },
      } as Delivery,
      ...s.deliveries,
    ]
    sampleChanged()
  },

  // get reads one delivery: the message a notification's Open names.
  async get(id: string): Promise<Delivery> {
    if (inWails()) return InboxService.Get(id)
    return { ...sampleDelivery(id) } as Delivery
  },

  // reply answers a message sent to the person; its sender is woken.
  async reply(id: string, text: string): Promise<void> {
    if (inWails()) return InboxService.Reply(id, text)
    sampleDelivery(id)
    if (!text.trim()) throw new Error("inbox: invalid input: write a reply")
    sampleSetState(id, State.Answered)
  },

  // markRead closes a message sent to the person without replying.
  async markRead(id: string): Promise<void> {
    if (inWails()) return InboxService.MarkRead(id)
    sampleDelivery(id)
    sampleSetState(id, State.Skipped)
  },

  // release delivers a held message past the policy, or a dismissed one
  // after all (the Held list's undo).
  async release(id: string): Promise<void> {
    if (inWails()) return InboxService.Release(id)
    const d = sampleDelivery(id)
    if (d.state !== State.Held && d.state !== State.Dismissed)
      throw new Error(
        `inbox: not allowed in the delivery's current state: can't release a ${d.state} delivery`
      )
    sampleSetState(id, State.Queued)
  },

  async dismiss(id: string): Promise<void> {
    if (inWails()) return InboxService.Dismiss(id)
    sampleDelivery(id)
    sampleSetState(id, State.Dismissed)
  },

  async allowAndRelease(id: string): Promise<void> {
    if (inWails()) return InboxService.AllowAndRelease(id)
    const d = sampleDelivery(id)
    const s = sampleState()
    if (!s.policy.allow.includes(d.message.from.key)) {
      s.policy = { ...s.policy, allow: [...s.policy.allow, d.message.from.key] }
    }
    sampleSetState(id, State.Queued)
  },

  async setPaused(paused: boolean): Promise<void> {
    if (inWails()) return InboxService.SetPaused(paused)
    const s = sampleState()
    s.policy = { ...s.policy, paused }
    sampleChanged()
  },

  // setFullAuto switches the wake policy between Full auto (mode open: no
  // bypass_receiver holds, decision 36) and everyone, keeping the lists and
  // the pause. Off is everyone even if the mode was allowlist before.
  async setFullAuto(on: boolean): Promise<void> {
    const mode = on ? Mode.Open : Mode.Everyone
    if (inWails()) {
      const p = await InboxService.Policy()
      if (p.mode === mode) return
      return InboxService.SetPolicy({ ...p, mode })
    }
    const s = sampleState()
    s.policy = { ...s.policy, mode }
    sampleChanged()
  },

  // revive runs claude --bg --resume for a gone session and returns what it
  // printed. The caller confirms first.
  async revive(sessionId: string): Promise<string> {
    if (inWails()) return RosterService.Revive(sessionId)
    const s = sampleState()
    s.gone = s.gone.filter((g) => g.sessionId !== sessionId)
    sampleChanged()
    return "Sample: nothing was run. In the app this runs claude --bg --resume in the session's folder."
  },
}

// ---- hooks ------------------------------------------------------------------

// useLatest runs load and keeps only the newest answer: calls are async and
// can come back out of order.
export function useLatest<T>(load: () => Promise<T>, initial: T) {
  const [state, setState] = useState<{
    value: T
    loading: boolean
    error: string | null
  }>({ value: initial, loading: true, error: null })
  const req = useRef(0)
  const mounted = useRef(true)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const reload = useCallback(async () => {
    const n = ++req.current
    try {
      const value = await load()
      if (!mounted.current || n !== req.current) return
      setState({ value, loading: false, error: null })
    } catch (err) {
      if (!mounted.current || n !== req.current) return
      setState((s) => ({ ...s, loading: false, error: message(err) }))
    }
  }, [load])

  return { ...state, reload }
}

export type InboxState = {
  deliveries: Delivery[]
  policy: Policy | null
  loading: boolean
  error: string | null
  reload: () => Promise<void>
}

type InboxValue = { deliveries: Delivery[]; policy: Policy | null }

async function loadInbox(): Promise<InboxValue> {
  if (!inWails()) {
    const s = sampleState()
    return { deliveries: s.deliveries, policy: s.policy }
  }
  const [recent, held, policy] = await Promise.all([
    InboxService.List(InboxLimit),
    InboxService.Held(),
    InboxService.Policy(),
  ])
  // Held items older than the newest InboxLimit still need the person, so
  // they are merged in, newest first. Held is as of the last inbox:changed
  // and List is live, so List's copy wins.
  const byId = new Map<string, Delivery>()
  for (const d of [...(held ?? []), ...(recent ?? [])]) byId.set(d.id, d)
  const created = (d: Delivery) => parseTime(d.createdAt)?.getTime() ?? 0
  const deliveries = [...byId.values()].sort((a, b) => created(b) - created(a))
  return { deliveries, policy }
}

// useInbox loads recent deliveries and the policy, and re-reads on every
// inbox:changed (which the Go side already coalesces to 4 a second).
export function useInbox(): InboxState {
  const { value, loading, error, reload } = useLatest<InboxValue>(loadInbox, {
    deliveries: [],
    policy: null,
  })

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    return Events.On(EventInboxChanged, () => void reload())
  }, [reload])

  return { ...value, loading, error, reload }
}

async function loadPolicy(): Promise<Policy | null> {
  if (!inWails()) return sampleState().policy
  return InboxService.Policy()
}

// usePolicy reads this device's wake policy, re-read on every
// inbox:changed (a policy change sends one).
export function usePolicy(): {
  policy: Policy | null
  error: string | null
  reload: () => Promise<void>
} {
  const { value, error, reload } = useLatest<Policy | null>(loadPolicy, null)

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    return Events.On(EventInboxChanged, () => void reload())
  }, [reload])

  return { policy: value, error, reload }
}

export type HeldState = {
  // held is every message waiting on the person, Pause all's too, newest
  // first.
  held: Delivery[]
  // history is past held messages, the most recently decided first; only
  // loaded while asked for.
  history: Delivery[]
  error: string | null
  reload: () => Promise<void>
}

type HeldValue = { held: Delivery[]; history: Delivery[] }

const created = (d: Delivery) => parseTime(d.createdAt)?.getTime() ?? 0
const decided = (d: Delivery) =>
  parseTime(d.decidedAt || d.updatedAt)?.getTime() ?? 0

// useHeld loads the Held list (and, with history, its History), re-read on
// every inbox:changed.
export function useHeld(history: boolean): HeldState {
  const load = useCallback(async (): Promise<HeldValue> => {
    if (!inWails()) {
      const all = sampleState().deliveries
      return {
        held: all
          .filter((d) => d.state === State.Held)
          .sort((a, b) => created(b) - created(a)),
        history: history
          ? all
              .filter((d) => d.heldFor && d.state !== State.Held)
              .sort((a, b) => decided(b) - decided(a))
          : [],
      }
    }
    const [held, past] = await Promise.all([
      InboxService.Held(),
      history
        ? InboxService.HeldHistory(HeldHistoryLimit)
        : Promise.resolve([]),
    ])
    return { held: held ?? [], history: past ?? [] }
  }, [history])
  const { value, error, reload } = useLatest<HeldValue>(load, {
    held: [],
    history: [],
  })

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    return Events.On(EventInboxChanged, () => void reload())
  }, [reload])

  return { ...value, error, reload }
}

async function loadGone(): Promise<GoneSession[]> {
  if (!inWails()) return sampleState().gone
  return (await RosterService.Gone(20)) ?? []
}

// useGone lists recently gone sessions the person can revive, re-read when
// the roster changes (a session leaving or coming back).
export function useGone(): {
  gone: GoneSession[]
  error: string | null
  reload: () => Promise<void>
} {
  const { value, error, reload } = useLatest<GoneSession[]>(loadGone, [])
  const timer = useRef<number | undefined>(undefined)

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    const off = Events.On(EventRosterChanged, () => {
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => void reload(), 500)
    })
    return () => {
      off()
      window.clearTimeout(timer.current)
    }
  }, [reload])

  return { gone: value, error, reload }
}

// ---- derived ----------------------------------------------------------------

// Counts per agent key: deliveries not yet picked up (queued or delivered)
// and deliveries held for the person.
export type AgentCounts = { unread: number; held: number }

export function countsByAgent(
  deliveries: Delivery[]
): Map<string, AgentCounts> {
  const out = new Map<string, AgentCounts>()
  for (const d of deliveries) {
    if (!d.to) continue
    const c = out.get(d.to) ?? { unread: 0, held: 0 }
    if (d.state === State.Queued || d.state === State.Delivered) c.unread++
    else if (d.state === State.Held) c.held++
    else continue
    out.set(d.to, c)
  }
  return out
}

// isOpen mirrors inbox.State.Open.
export function isOpen(s: State): boolean {
  return (
    s === State.Queued ||
    s === State.Held ||
    s === State.Delivered ||
    s === State.Engaged ||
    s === State.Deferred
  )
}
