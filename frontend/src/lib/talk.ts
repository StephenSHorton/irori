// v0.2 gateway: the inbox and reviving gone sessions (the Board has its own
// gateway in board.ts).
// Like irori.ts, components import from here, never from src/gen, and in a
// plain browser everything runs on the sample in sample-talk.ts (actions
// change that sample in memory so the buttons can be tried).

import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { Events, InboxService, RosterService } from "./host"
import { onChange } from "./changes"
import {
  HoldReason,
  Mode,
  State,
  type Delivery,
  type GoneSession,
  type Mention,
  type MentionOutcome,
  type Policy,
  type StartedAgent,
} from "./models"
import { inWails, parseTime } from "./irori"
import { checkUploads, uploadOf } from "./uploads"
import { sampleTalk, type SampleTalk } from "./sample-talk"
import { nameOr } from "./names"
import { noteRead, type ReadReport } from "./window-log"

export type { Delivery, GoneSession, Policy }
export { HoldReason, Mode, State }

// Event names mirror the Go constants in inbox_service.go.
export const EventInboxChanged = "inbox:changed"
const EventRosterChanged = "roster:changed"

// InboxLimit is how many deliveries the Inbox tab asks for.
export const InboxLimit = 200

// The person's message has no box limit: inbox holds it to MAX_PERSON_TEXT
// (512 KiB) and the agent's wake clips it with a show pointer (card #265).

// HeldHistoryLimit is how many past held messages the Held list's History
// asks for.
export const HeldHistoryLimit = 100

// SystemKey mirrors inbox.SystemKey: Irori itself (an unpause, the sweep).
export const SystemKey = "irori"

// HoldWords names what a hold's sentence mentions (card #488): an agent
// by key, and a repo by key (its short name, never the raw key).
export type HoldWords = {
  name?: (key: string) => string
  repo?: (key: string) => string
}

// holdText is why d was (or is) held, in plain words that name who is
// involved (card #488): "otter and heron have messaged each other 8 times
// without stopping", not an internal reason. Unknown reasons read as a
// plain "Irori held it." rather than their code.
export function holdText(d: Delivery, w: HoldWords = {}): string {
  // A string: the inbox also has holds the window's enum doesn't list.
  const h: string = d.hold || d.heldFor || ""
  const named = (key: string) => (key ? (w.name?.(key) ?? "") : "")
  const from = d.message.from.key.startsWith("user:")
    ? "you"
    : named(d.message.from.key) || d.message.from.name || "an agent"
  // An agent's name starts the sentence as it is ("sample-otter"): it's
  // a handle, not a word to capitalize.
  const lead = from === "you" ? "You" : from
  const to = d.to?.startsWith("user:")
    ? "you"
    : named(d.to) || "the other agent"
  const repo = d.repoKey ? (w.repo ?? repoShort)(d.repoKey) : "its repo"
  switch (h) {
    case HoldReason.HoldBlocked:
      return `${lead} is on your block list.`
    case HoldReason.HoldNotAllowed:
      return `${lead} isn't on your allow list.`
    case HoldReason.HoldBypassReceiver:
      return "An older Irori held it, by a rule that's gone now. Release delivers it."
    case HoldReason.HoldHopLimit: {
      const n = d.message.hop
      return n > 1
        ? `${lead} and ${to} have messaged each other ${n} times without stopping.`
        : `${lead} and ${to} kept messaging each other without stopping.`
    }
    case HoldReason.HoldUnowned:
      return from === "you"
        ? `No agent is working in ${repo} to take your message.`
        : `No agent is working in ${repo} to take it yet.`
    case HoldReason.HoldPaused:
      return "You paused wakes."
    case HoldReason.HoldMemberWake:
      return "Another person asked to wake your agent, and your receive setting for that space asks you first."
    case HoldReason.HoldCrossPerson:
      return "It comes from another person in a shared space, and your receive setting for that space asks you first."
    case HoldReason.HoldMemberLease:
      return "Another person asks your agent to give up a lease."
    case "member_removed":
      return "It comes from a person who is no longer in that space."
    case "editing":
      return "You are editing it."
    case "off_side":
      return `Irori picked ${to} for a card on a side ${to === "you" ? "your" : `${to}'s`} device doesn't run. Release delivers it.`
    default:
      return h ? "Irori held it." : ""
  }
}

// startedLine is what Start an agent did, in the person's words (card
// #488): an agent starting, a free agent handed it, or a card waiting.
export function startedLine(s: StartedAgent): string {
  if (s.started)
    return `Starting an agent in ${s.repo} on ${s.cardRef}, with your message.`
  if (s.offered)
    return `Your message is card ${s.cardRef} in ${s.repo}, handed to a free agent.`
  return `Your message is card ${s.cardRef} in ${s.repo}; it waits for a free agent.`
}

// repoShort is a repo key as a person reads it when no checkout here names
// it (card #488): "example-app" for a hosted repo, "local repo 5a3c1e0"
// for a root-commit key. useRepoNames gives the checkout folder's name.
export function repoShort(key: string): string {
  if (key.startsWith("root:")) return `local repo ${key.slice(5, 12)}`
  const k = key.replace(/\/+$/, "").replace(/\.git$/, "")
  const i = Math.max(
    k.lastIndexOf("/"),
    k.lastIndexOf(":"),
    k.lastIndexOf("\\")
  )
  return i >= 0 && i < k.length - 1 ? k.slice(i + 1) : k
}

// allowLifts says Allow sender & deliver is offered: allowing the sender
// lifts a not-on-your-allow-list hold. InboxService.AllowAndRelease also
// takes an older Irori's retired bypass hold, but there allowing changes
// nothing a plain Release doesn't, so it isn't offered (card #488).
export function allowLifts(d: Delivery): boolean {
  return d.state === State.Held && d.hold === HoldReason.HoldNotAllowed
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
// roster name, else a name seen on a message or a gone session, else
// "unknown agent", never a key (card #478). Callers pass every source
// they have.
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
    return nameOr(names.get(key))
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
  // send sends the person's message; each of mentions gets its own copy,
  // woken. The answer is each mention's outcome.
  async send(
    to: string,
    repo: string,
    text: string,
    mentions: Mention[] = []
  ): Promise<MentionOutcome[]> {
    if (inWails())
      return (await InboxService.Send(to, repo, text, mentions)).mentioned ?? []
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
    return mentions.map((m) => ({ key: m.key, name: m.name, state: "woke" }))
  },

  // get reads one delivery: the message a notification's Open names.
  async get(id: string): Promise<Delivery> {
    if (inWails()) return InboxService.Get(id)
    return { ...sampleDelivery(id) } as Delivery
  },

  // reply answers a message sent to the person; its sender is woken, and
  // so is each of mentions, with its own copy. The answer is each
  // mention's outcome.
  async reply(
    id: string,
    text: string,
    mentions: Mention[] = [],
    files: File[] = []
  ): Promise<MentionOutcome[]> {
    checkUploads(files)
    if (inWails()) {
      if (!files.length)
        return (await InboxService.Reply(id, text, mentions)) ?? []
      return (
        (await InboxService.ReplyWithFiles(
          id,
          text,
          mentions,
          await Promise.all(files.map(uploadOf))
        )) ?? []
      )
    }
    sampleDelivery(id)
    if (!text.trim()) throw new Error("inbox: invalid input: write a reply")
    sampleSetState(id, State.Answered)
    return mentions.map((m) => ({ key: m.key, name: m.name, state: "woke" }))
  },

  // replyStop is Stop and read on a message (card #207): the sender's
  // running turn is cut and the reply is its next prompt.
  async replyStop(id: string, text: string): Promise<void> {
    if (inWails()) {
      await InboxService.ReplyStop(id, text)
      return
    }
    sampleDelivery(id)
    if (!text.trim()) throw new Error("inbox: invalid input: write a reply")
    sampleSetState(id, State.Answered)
  },

  // stop cuts agent key's running turn (card #207); text (empty: "Stop and
  // wait for me.") is its next prompt.
  async stop(key: string, text = ""): Promise<void> {
    if (inWails()) return RosterService.Stop(key, text)
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

  // startAgent is "Start an agent in <repo>" (card #488) on the person's
  // message no agent in its repo can take yet: it becomes a card there,
  // an agent is started for it, and the held message closes. The answer
  // says which card and what happened (startedLine words it).
  async startAgent(id: string): Promise<StartedAgent> {
    if (inWails()) return InboxService.StartAgent(id)
    const d = sampleDelivery(id)
    if (d.state !== State.Held || d.hold !== HoldReason.HoldUnowned)
      throw new Error("inbox: not allowed in the delivery's current state")
    sampleSetState(id, State.Dismissed)
    return {
      cardRef: "#99",
      repo: "ww-multiplayer",
      started: true,
      offered: false,
    }
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

// StuckReadMs is how long a read may run before the next reload stops
// waiting for it and reads beside it, so a read that never answers doesn't
// hold its list past the next event. There is no timer: reloads already
// waiting on it end with that next reload's read.
export const StuckReadMs = 15_000

export type Answer<T> = { value: T } | { error: unknown }

// latestReads is useLatest's rule (card #551): the reload function it
// returns keeps one read running. A reload during a read asks for one more
// when that read ends, however many came, so every answer shows and a slow
// read can't be starved by events that come faster than it finishes, nor
// can they pile reads up on a slow device. Each reload's promise ends with
// a read begun after it was called, so `await reload()` after an action
// sees the action.
//
// A different load (another board or conversation) reads at once and the
// old load's answer is dropped. A read stuck past StuckReadMs is left
// running beside a new one, so answers can still arrive out of order: one
// older than the last shown is dropped.
export function latestReads<T>(
  show: (answer: Answer<T>) => void,
  now: () => number = Date.now,
  report: (r: ReadReport) => void = () => {},
  // dropped hears an answer that is not shown (an older one than the last
  // shown, or one of a load that is no longer the newest), so a read that
  // covered something can see to it again (patched.ts).
  dropped: (answer: Answer<T>) => void = () => {}
): (load: () => Promise<T>) => Promise<void> {
  let started = 0
  let landed = 0
  let newest: (() => Promise<T>) | null = null
  let running: { n: number; load: () => Promise<T>; since: number } | null =
    null
  // The reloads the next read to start will answer.
  let waiting: (() => void)[] = []

  const begin = async (load: () => Promise<T>) => {
    const n = ++started
    const covered = waiting
    waiting = []
    const since = now()
    running = { n, load, since }
    let answer: Answer<T>
    try {
      answer = { value: await load() }
    } catch (error) {
      answer = { error }
    }
    const shows = load === newest && n > landed
    report({ ms: now() - since, dropped: !shows, reloads: covered.length })
    if (shows) {
      landed = n
      show(answer)
    } else dropped(answer)
    for (const done of covered) done()
    if (running?.n !== n) return
    running = null
    if (waiting.length && newest) void begin(newest)
  }

  return (load) =>
    new Promise<void>((done) => {
      newest = load
      waiting.push(done)
      if (
        !running ||
        running.load !== load ||
        now() - running.since >= StuckReadMs
      )
        void begin(load)
    })
}

// useLatest runs load and keeps the newest answer, by latestReads. An
// answer equal to the one shown keeps the state as it is (0.11.1): App
// reloads several lists on every board or inbox change, several times a
// second while agents work, and each new array re-rendered the whole window.
// name is what irori.log calls this list (window-log.ts).
export function useLatest<T>(
  load: () => Promise<T>,
  initial: T,
  name = "a list"
) {
  const [state, setState] = useState<{
    value: T
    loading: boolean
    error: string | null
  }>({ value: initial, loading: true, error: null })
  const mounted = useRef(true)
  const shown = useRef<string | null>(null)

  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  const reads = useRef<ReturnType<typeof latestReads<T>> | null>(null)

  const reload = useCallback(() => {
    reads.current ??= latestReads<T>(
      (answer) => {
        if (!mounted.current) return
        if ("error" in answer) {
          shown.current = null
          setState((s) => ({
            ...s,
            loading: false,
            error: message(answer.error),
          }))
          return
        }
        const { value } = answer
        const same = sameAs(shown, value)
        setState((s) =>
          same && !s.loading && s.error === null
            ? s
            : { value, loading: false, error: null }
        )
      },
      Date.now,
      (r) => noteRead(name, r)
    )
    return reads.current(load)
  }, [load, name])

  return { ...state, reload }
}

// sameAs says whether value serializes the same as the last one seen in
// key, and remembers it. Values that can't be serialized never match.
export function sameAs(
  key: { current: string | null },
  value: unknown
): boolean {
  let next: string | null
  try {
    next = JSON.stringify(value) ?? null
  } catch {
    next = null
  }
  const same = next !== null && next === key.current
  key.current = next
  return same
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
  const { value, loading, error, reload } = useLatest<InboxValue>(
    loadInbox,
    { deliveries: [], policy: null },
    "inbox"
  )

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    return onChange(EventInboxChanged, () => void reload())
  }, [reload])

  return { ...value, loading, error, reload }
}

export type HeldState = {
  // held is every message waiting on the person, Pause all's too, newest
  // first.
  held: Delivery[]
  // history is past held messages, the most recently decided first; only
  // loaded while asked for.
  history: Delivery[]
  // loading is true until the first read lands, never again (card #598).
  loading: boolean
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
  const { value, loading, error, reload } = useLatest<HeldValue>(
    load,
    { held: [], history: [] },
    "held"
  )

  useEffect(() => {
    void reload()
    if (!inWails()) return onSample(() => void reload())
    return onChange(EventInboxChanged, () => void reload())
  }, [reload])

  return { ...value, loading, error, reload }
}

// sampleRepoNames are the sample checkouts' folder names.
const sampleRepoNames: Array<[string, string]> = [
  ["root:5a3c1e0b9d7f2a4c6e8b0d1f3a5c7e9b1d3f5a7c", "ww-multiplayer"],
]

async function loadRepoNames(): Promise<Array<[string, string]>> {
  if (!inWails()) return sampleRepoNames
  return ((await InboxService.RepoNames()) ?? []).map((r): [string, string] => [
    r.repoKey,
    r.name,
  ])
}

// useRepoNames names repo keys (card #488): the checkout folder's name for
// a repo this device has one of ("ww-multiplayer"), else repoShort. Read
// once a held message to a repo shows, and again on each inbox change.
export function useRepoNames(): (key: string) => string {
  const { value, reload } = useLatest<Array<[string, string]>>(
    loadRepoNames,
    [],
    "repo names"
  )
  useEffect(() => {
    void reload()
    if (!inWails()) return
    return onChange(EventInboxChanged, () => void reload())
  }, [reload])
  return useMemo(() => {
    const names = new Map(value)
    return (key: string) => names.get(key) || repoShort(key)
  }, [value])
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
  const { value, error, reload } = useLatest<GoneSession[]>(
    loadGone,
    [],
    "gone"
  )
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
