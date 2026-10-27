// v0.3 gateway: timelines, commits and their links, edits, why, decisions
// and the PR ledger. Like talk.ts, components import from here, never from
// bindings/, and in a plain browser everything runs on the sample in
// sample-capture.ts (recording a decision changes that sample in memory).

import { useCallback, useEffect, useRef, useState } from "react"
import { Events } from "@wailsio/runtime"

import { CaptureService } from "../../bindings/github.com/StephenSHorton/irori"
import type { CaptureRepo } from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  EntryKind,
  type CommitLink,
  type CommitView,
  type DecisionView,
  type EditRow,
  type Entry,
  type Timeline,
  type TurnRow,
  type TurnView,
  type WhyResult,
} from "../../bindings/github.com/StephenSHorton/irori/internal/capture/models"
import type {
  FileChange,
  FileLink,
} from "../../bindings/github.com/StephenSHorton/irori/internal/capture/gitlink/models"
import type { ToolCall } from "../../bindings/github.com/StephenSHorton/irori/internal/capture/transcript/models"
import {
  AnchorState,
  type Decision,
} from "../../bindings/github.com/StephenSHorton/irori/internal/decision/models"
import type { Entry as PREntry } from "../../bindings/github.com/StephenSHorton/irori/internal/ledger/models"
import { inWails, parseTime } from "./irori"
import { sampleCapture, type SampleCapture } from "./sample-capture"
import { errorText } from "./talk"

export type {
  CaptureRepo,
  CommitLink,
  CommitView,
  Decision,
  DecisionView,
  EditRow,
  Entry,
  FileChange,
  FileLink,
  PREntry,
  Timeline,
  ToolCall,
  TurnRow,
  TurnView,
  WhyResult,
}
export { AnchorState, EntryKind }

// EventCaptureChanged mirrors the Go constant in capture_service.go.
export const EventCaptureChanged = "capture:changed"
const EventRosterChanged = "roster:changed"
const EventInboxChanged = "inbox:changed"

// TimelinePage is how many turns a timeline page asks for (Go default 20,
// max 100).
export const TimelinePage = 20
const TimelineMax = 100

// CommitLimit is how many commits the History tab lists.
export const CommitLimit = 100

// Fidelity of a roster card: how much capture sees of that agent.
export type Fidelity = "mod" | "passive" | "mcp"

// Link classes (gitlink.Link*), best first.
export const LinkClasses = ["exact", "partial", "heuristic", "unattributed"]

// CaptureChange mirrors the capture:changed payload.
export type CaptureChange = {
  kinds: string[]
  keys: string[]
  repoKeys: string[]
}

// ---- sample mode ------------------------------------------------------------

let sample: SampleCapture | null = null
const sampleListeners = new Set<(c: CaptureChange) => void>()

function sampleState(): SampleCapture {
  sample ??= sampleCapture(new Date())
  return sample
}

function sampleChanged(c: CaptureChange) {
  for (const fn of sampleListeners) fn(c)
}

const atMs = (v: unknown) => parseTime(v)?.getTime() ?? 0

function samplePage(key: string, before: string, limit: number): Timeline {
  const all = sampleState().timelines.get(key) ?? []
  const b = before ? atMs(before) : Infinity
  const entries: Entry[] = []
  let turns = 0
  let more = false
  for (const e of all) {
    if (atMs(e.at) >= b) continue
    if (e.kind === EntryKind.EntryTurn) {
      if (turns === limit) {
        more = true
        break
      }
      turns++
    }
    entries.push(e)
  }
  return { key, entries, more }
}

// ---- calls ------------------------------------------------------------------

export type DecisionInput = {
  statement: string
  why: string
  path: string
  lines: string
  repoKey: string
  supersedes: string[]
  messageId?: string
  turnId?: string
}

export const capture = {
  async timeline(
    key: string,
    before: string,
    limit: number,
    subagents: boolean
  ): Promise<Timeline> {
    if (!inWails()) return samplePage(key, before, limit)
    const t = await CaptureService.Timeline(key, before, limit, subagents)
    return { ...t, entries: t.entries ?? [] }
  },

  async commits(repoKey: string, limit = CommitLimit): Promise<CommitView[]> {
    if (!inWails()) {
      return sampleState().commits.filter(
        (c) => !repoKey || c.repoKey === repoKey
      )
    }
    return (await CaptureService.Commits(repoKey, limit)) ?? []
  },

  async commit(sha: string): Promise<CommitView | null> {
    if (!inWails()) {
      const s = sha.toLowerCase()
      const hits = sampleState().commits.filter((c) => c.sha.startsWith(s))
      return s.length >= 7 && hits.length === 1 ? hits[0] : null
    }
    return CaptureService.Commit(sha)
  },

  async repos(): Promise<CaptureRepo[]> {
    if (!inWails()) return sampleState().repos
    return (await CaptureService.Repos()) ?? []
  },

  async patch(blobId: string): Promise<string> {
    if (!inWails()) {
      const p = sampleState().patches.get(blobId)
      if (p === undefined) throw new Error("blob: not found")
      return p
    }
    return CaptureService.Patch(blobId)
  },

  async why(path: string, line: number, repoKey: string): Promise<WhyResult> {
    if (!inWails()) return sampleWhy(path, line, repoKey)
    const r = await CaptureService.Why(path, line, repoKey)
    return { ...r, edits: r.edits ?? [] }
  },

  async decisions(repoKey: string, query: string): Promise<DecisionView[]> {
    // Always with superseded ones: the chain needs them, and the tab hides
    // them unless asked.
    if (!inWails()) {
      const q = query.trim().toLowerCase()
      return sampleState().decisions.filter(
        (d) =>
          (!repoKey || d.anchor?.repoKey === repoKey) &&
          (!q ||
            d.statement.toLowerCase().includes(q) ||
            (d.why ?? "").toLowerCase().includes(q))
      )
    }
    return (await CaptureService.Decisions(repoKey, "", query, true)) ?? []
  },

  async recordDecision(d: DecisionInput): Promise<Decision> {
    if (inWails()) {
      return CaptureService.RecordDecision(
        d.statement,
        d.why,
        d.path,
        d.lines,
        d.repoKey,
        d.supersedes,
        d.messageId ?? "",
        d.turnId ?? ""
      )
    }
    return sampleRecord(d)
  },

  async prs(repo: string): Promise<PREntry[]> {
    if (!inWails()) return sampleState().prs
    return (await CaptureService.PRs(repo)) ?? []
  },

  async fidelity(): Promise<Record<string, string>> {
    if (!inWails()) return sampleState().fidelity
    const f = await CaptureService.Fidelity()
    const out: Record<string, string> = {}
    for (const [k, v] of Object.entries(f ?? {})) if (v) out[k] = v
    return out
  },
}

function sampleWhy(path: string, line: number, repoKey: string): WhyResult {
  const s = sampleState()
  const p = path.trim().replaceAll("\\", "/")
  if (!p) throw new Error("capture: not inside a known git repo")
  if (!repoKey && !/^([A-Za-z]:)?[\\/]/.test(path)) {
    throw new Error(
      "capture: not inside a known git repo: pick a repo, or give an absolute path"
    )
  }
  const commit = s.commits.find((c) => c.files.some((f) => f.path === p))
  const editIds = new Set(
    commit?.link?.files.find((f) => f.path === p)?.edits ?? []
  )
  const edits: EditRow[] = []
  const turns: TurnRow[] = []
  for (const entries of s.timelines.values()) {
    for (const e of entries) {
      if (!e.turn) continue
      const hit = (e.turn.edits ?? []).filter(
        (x) => x.path === p && (editIds.size === 0 || editIds.has(x.id))
      )
      if (hit.length === 0) continue
      edits.push(...hit)
      turns.push(e.turn)
    }
  }
  return {
    repoKey: repoKey || commit?.repoKey || "",
    root: commit?.root ?? "",
    path: p,
    line: line || undefined,
    blameSha: line && commit ? commit.sha : undefined,
    commit: line ? commit : undefined,
    edits,
    turns,
    decisions: s.decisions.filter(
      (d) => d.anchor?.path === p && !(d.supersededBy?.length ?? 0)
    ),
  }
}

function sampleRecord(d: DecisionInput): Decision {
  const s = sampleState()
  const statement = d.statement.trim()
  if (!statement) throw new Error("decision: statement is empty")
  const now = new Date().toISOString()
  const dec: DecisionView = {
    id: `d-sample-${s.decisions.length + 10}`,
    by: { key: "user:sample-device", name: "you", kind: "user" },
    statement,
    why: d.why.trim() || undefined,
    trust: "user",
    at: now,
    supersedes: d.supersedes.length ? d.supersedes : undefined,
    source:
      d.messageId || d.turnId
        ? { messageId: d.messageId, turnId: d.turnId }
        : undefined,
    anchor: d.path.trim()
      ? {
          repoKey: d.repoKey,
          root: "C:\\Users\\sample\\projects\\example-app",
          path: d.path.trim().replaceAll("\\", "/"),
          start: Number(d.lines.split("-")[0]) || undefined,
          end: Number(d.lines.split("-").pop()) || undefined,
          hash: "sample",
        }
      : undefined,
    status: d.path.trim() ? { state: AnchorState.OK } : undefined,
  }
  s.decisions = [
    dec,
    ...s.decisions.map((x) =>
      d.supersedes.includes(x.id)
        ? { ...x, supersededBy: [...(x.supersededBy ?? []), dec.id] }
        : x
    ),
  ]
  sampleChanged({ kinds: ["decision"], keys: [], repoKeys: [d.repoKey] })
  return dec
}

// ---- hooks ------------------------------------------------------------------

// onCaptureChanged calls fn for every capture:changed (the Go side already
// coalesces them to one per 300ms).
export function onCaptureChanged(fn: (c: CaptureChange) => void): () => void {
  if (!inWails()) {
    sampleListeners.add(fn)
    return () => sampleListeners.delete(fn)
  }
  return Events.On(EventCaptureChanged, (ev) => {
    const d = ev.data as Partial<CaptureChange> | undefined
    fn({
      kinds: d?.kinds ?? [],
      keys: d?.keys ?? [],
      repoKeys: d?.repoKeys ?? [],
    })
  })
}

type Loaded<T> = { value: T; loading: boolean; error: string | null }

// useLoad runs load, keeps only the newest answer, and re-runs it when
// refresh(change) says a capture:changed matters. deps restart it.
function useLoad<T>(
  load: () => Promise<T>,
  initial: T,
  refresh: ((c: CaptureChange) => boolean) | null
) {
  const [state, setState] = useState<Loaded<T>>({
    value: initial,
    loading: true,
    error: null,
  })
  const req = useRef(0)

  const reload = useCallback(async () => {
    const n = ++req.current
    try {
      const value = await load()
      if (n !== req.current) return
      setState({ value, loading: false, error: null })
    } catch (err) {
      if (n !== req.current) return
      setState((s) => ({ ...s, loading: false, error: errorText(err) }))
    }
  }, [load])

  useEffect(() => {
    void reload()
    if (!refresh) return
    const off = onCaptureChanged((c) => {
      if (refresh(c)) void reload()
    })
    return off
  }, [reload, refresh])

  return { ...state, reload }
}

export type TimelineState = {
  entries: Entry[]
  more: boolean
  loading: boolean
  loadingMore: boolean
  error: string | null
  loadMore: () => Promise<void>
}

const turnCount = (es: Entry[]) =>
  es.filter((e) => e.kind === EntryKind.EntryTurn).length

// useTimeline loads one agent's newest page and re-reads it when capture
// reports that agent (or a commit, PR or decision) changed. Older pages
// come from loadMore and stay loaded across those re-reads: the re-read
// asks for as many turns as are shown, up to the Go max.
export function useTimeline(key: string, subagents: boolean): TimelineState {
  const [older, setOlder] = useState<{
    key: string
    entries: Entry[]
    more: boolean
  }>({
    key: "",
    entries: [],
    more: false,
  })
  const [loadingMore, setLoadingMore] = useState(false)
  const [moreError, setMoreError] = useState<string | null>(null)
  const shown = useRef(TimelinePage)

  const load = useCallback(
    () =>
      key
        ? capture.timeline(key, "", shown.current, subagents)
        : Promise.resolve<Timeline>({ key: "", entries: [], more: false }),
    [key, subagents]
  )
  const refresh = useCallback(
    (c: CaptureChange) =>
      c.keys.includes(key) ||
      c.kinds.includes("commit") ||
      c.kinds.includes("pr"),
    [key]
  )
  const head = useLoad<Timeline>(
    load,
    { key: "", entries: [], more: false },
    refresh
  )

  useEffect(() => {
    shown.current = TimelinePage
  }, [key, subagents])

  const own = older.key === `${key}|${subagents}` ? older : null
  const headEntries = head.value.key === key ? head.value.entries : []
  // The head re-read can overlap pages loaded before it; each entry once.
  const entries = own ? dedupe([...headEntries, ...own.entries]) : headEntries
  const more = own ? own.more : head.value.key === key && head.value.more

  const loadMore = useCallback(async () => {
    const last = entries.at(-1)
    if (!last || !more) return
    setLoadingMore(true)
    setMoreError(null)
    try {
      const page = await capture.timeline(
        key,
        String(last.at),
        TimelinePage,
        subagents
      )
      shown.current = Math.min(
        TimelineMax,
        shown.current + turnCount(page.entries)
      )
      setOlder((o) => {
        const base = o.key === `${key}|${subagents}` ? o.entries : []
        return {
          key: `${key}|${subagents}`,
          entries: [...base, ...page.entries],
          more: page.more,
        }
      })
    } catch (err) {
      setMoreError(errorText(err))
    } finally {
      setLoadingMore(false)
    }
  }, [entries, more, key, subagents])

  return {
    entries,
    more,
    loading: head.loading,
    loadingMore,
    error: head.error ?? moreError,
    loadMore,
  }
}

// entryId names a timeline entry across pages.
export function entryId(e: Entry): string {
  if (e.turn) return `turn:${e.turn.id}`
  if (e.commit) return `commit:${e.commit.sha}`
  if (e.summary) return `summary:${e.summary.id}`
  if (e.pr) return `pr:${e.pr.key}|${e.pr.session}|${e.pr.role}`
  return `${e.kind}:${String(e.at)}`
}

function dedupe(es: Entry[]): Entry[] {
  const seen = new Set<string>()
  return es.filter((e) => {
    const id = entryId(e)
    if (seen.has(id)) return false
    seen.add(id)
    return true
  })
}

// useRepos lists the repos capture has seen, re-read on new commits.
export function useRepos() {
  const refresh = useCallback(
    (c: CaptureChange) => c.kinds.includes("commit"),
    []
  )
  const r = useLoad<CaptureRepo[]>(capture.repos, [], refresh)
  return { repos: r.value, loading: r.loading, error: r.error }
}

// useCommits lists a repo's commits ("" = every repo).
export function useCommits(repoKey: string) {
  const load = useCallback(() => capture.commits(repoKey), [repoKey])
  const refresh = useCallback(
    (c: CaptureChange) =>
      c.kinds.includes("commit") &&
      (!repoKey || c.repoKeys.length === 0 || c.repoKeys.includes(repoKey)),
    [repoKey]
  )
  const r = useLoad<CommitView[]>(load, [], refresh)
  return { commits: r.value, loading: r.loading, error: r.error }
}

// useDecisions lists decisions (superseded included) for a repo and query.
export function useDecisions(repoKey: string, query: string) {
  const load = useCallback(
    () => capture.decisions(repoKey, query),
    [repoKey, query]
  )
  const refresh = useCallback(
    (c: CaptureChange) => c.kinds.includes("decision"),
    []
  )
  const r = useLoad<DecisionView[]>(load, [], refresh)
  return {
    decisions: r.value,
    loading: r.loading,
    error: r.error,
    reload: r.reload,
  }
}

// useFidelity maps live agent keys to their fidelity. Attach streams come
// and go with inbox:changed; cards with roster:changed.
export function useFidelity(): Record<string, string> {
  const r = useLoad<Record<string, string>>(capture.fidelity, {}, null)
  const { reload } = r
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (!inWails()) return
    const later = () => {
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => void reload(), 300)
    }
    const offs = [
      Events.On(EventRosterChanged, later),
      Events.On(EventInboxChanged, later),
    ]
    return () => {
      for (const off of offs) off()
      window.clearTimeout(timer.current)
    }
  }, [reload])
  return r.value
}

// ---- derived ----------------------------------------------------------------

// linkCounts counts a commit's files by link class.
export function linkCounts(link?: CommitLink | null): Record<string, number> {
  const out: Record<string, number> = {}
  for (const f of link?.files ?? []) out[f.link] = (out[f.link] ?? 0) + 1
  return out
}

// shortSha is the first 7 hex digits.
export function shortSha(sha: string): string {
  return (sha ?? "").slice(0, 7)
}

// repoRel shows an edit path: repo-relative paths as they are, absolute
// ones with only the last three segments.
export function repoRel(p: string): string {
  if (!p) return ""
  if (!/^([A-Za-z]:)?[\\/]/.test(p)) return p
  const parts = p.split(/[\\/]/).filter(Boolean)
  return parts.length > 3 ? `…/${parts.slice(-3).join("/")}` : p
}

// lineRange formats an anchor's lines: "" (whole file), "12" or "12-18".
export function lineRange(start?: number, end?: number): string {
  if (!start) return ""
  return end && end !== start ? `${start}-${end}` : `${start}`
}
