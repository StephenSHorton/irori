// v0.2 gateway: the inbox, tasks and leases, and reviving gone sessions.
// Like irori.ts, components import from here, never from bindings/, and in a
// plain browser everything runs on the sample in sample-talk.ts (actions
// change that sample in memory so the buttons can be tried).

import { useCallback, useEffect, useRef, useState } from "react"
import { Events } from "@wailsio/runtime"

import {
  InboxService,
  RosterService,
  WorkService,
} from "../../bindings/github.com/StephenSHorton/irori"
import type { GoneSession } from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  TaskState,
  type Lease,
  type Task,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import {
  HoldReason,
  State,
  type Delivery,
  type Policy,
} from "../../bindings/github.com/StephenSHorton/irori/internal/inbox/models"
import { inWails, parseTime, sessionShort } from "./irori"
import { sampleTalk, type SampleTalk } from "./sample-talk"

export type { Delivery, GoneSession, Lease, Policy, Task }
export { HoldReason, State, TaskState }

// Event names mirror the Go constants in inbox_service.go / work_service.go.
export const EventInboxChanged = "inbox:changed"
export const EventWorkChanged = "work:changed"
const EventRosterChanged = "roster:changed"

// InboxLimit is how many deliveries the Inbox tab asks for.
export const InboxLimit = 200

// MaxText mirrors inbox.MaxText.
export const MaxText = 8000

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
  s.deliveries = s.deliveries.map((d) =>
    d.id === id
      ? { ...d, state, hold: undefined, updatedAt: new Date().toISOString() }
      : d
  )
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

  async release(id: string): Promise<void> {
    if (inWails()) return InboxService.Release(id)
    sampleDelivery(id)
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

  async cancelTask(id: string): Promise<void> {
    if (inWails()) {
      await WorkService.CancelTask(id)
      return
    }
    const s = sampleState()
    s.tasks = s.tasks.map((t) =>
      t.id === id
        ? {
            ...t,
            state: TaskState.Canceled,
            updatedAt: new Date().toISOString(),
          }
        : t
    )
    sampleChanged()
  },

  async releaseLease(resource: string): Promise<void> {
    if (inWails()) return WorkService.ReleaseLease(resource)
    const s = sampleState()
    s.leases = s.leases.filter((l) => l.resource !== resource)
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
function useLatest<T>(load: () => Promise<T>, initial: T) {
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

export type WorkState = {
  tasks: Task[]
  leases: Lease[]
  loading: boolean
  error: string | null
  reload: () => Promise<void>
}

type WorkValue = { tasks: Task[]; leases: Lease[] }

async function loadWork(): Promise<WorkValue> {
  if (!inWails()) {
    const s = sampleState()
    return { tasks: s.tasks, leases: s.leases }
  }
  const [tasks, leases] = await Promise.all([
    WorkService.Tasks(false),
    WorkService.Leases(),
  ])
  return { tasks: tasks ?? [], leases: leases ?? [] }
}

// useWork loads tasks (finished ones too; the tab filters) and leases. It
// re-reads on work:changed (every task or lease change, lease expiry too),
// and when the tab opens or the window regains focus.
// WorkRetryMs is the wait before re-reading after a failed Work read.
const WorkRetryMs = 4000

export function useWork(active: boolean): WorkState {
  const { value, loading, error, reload } = useLatest<WorkValue>(loadWork, {
    tasks: [],
    leases: [],
  })

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    return Events.On(EventWorkChanged, () => void reload())
  }, [reload])

  useEffect(() => {
    if (!active || !inWails()) return
    void reload()
    const onFocus = () => void reload()
    window.addEventListener("focus", onFocus)
    return () => window.removeEventListener("focus", onFocus)
  }, [active, reload])

  // A failed read has no next event to recover it (there is no poll), so
  // retry once a few seconds later while the tab is open.
  useEffect(() => {
    if (!active || !error) return
    const id = window.setTimeout(() => void reload(), WorkRetryMs)
    return () => window.clearTimeout(id)
  }, [active, error, reload])

  return { ...value, loading, error, reload }
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

// taskOpen is true for task states that aren't finished.
export function taskOpen(s: TaskState): boolean {
  return (
    s !== TaskState.Completed &&
    s !== TaskState.Failed &&
    s !== TaskState.Canceled
  )
}
