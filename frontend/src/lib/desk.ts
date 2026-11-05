// Merge desk (card #417, docs/design/merge-desk.md §3, §5, §7): the queue
// each desk shows, worked out from the poller's snapshot (desk.prs, slice
// 1) and the desk's own rows. deskQueue is pure so every row of §3's state
// table is a test (desk.test.ts); the page and, after slice 8, the Talk
// desk channel only draw what it returns.
//
// Field names follow §7's row schemas so the generated models drop in.

import { useSyncExternalStore } from "react"
import { sameAs } from "@/lib/talk"

import { DeskService, Events } from "./host"
import { inWails, parseTime } from "./irori"
import type { DeskView } from "./models"
import { previewFlag } from "./preview"
import { sampleDesks } from "./sample-desk"

// A PR as the poller keeps it (coord DeskPr, slice 1): gh pr list's
// fields, with statusCheckRollup summed up in checks: "" (none),
// "pending", "failure" or "success".
export type SnapPr = {
  number: number
  url: string
  title: string
  author?: string
  isDraft?: boolean
  headRefName?: string
  headRefOid?: string
  baseRefName?: string
  mergeable?: string // MERGEABLE, CONFLICTING, UNKNOWN
  mergeStateStatus?: string // BEHIND, DIRTY, CLEAN, BLOCKED, UNSTABLE, ...
  checks?: string
  createdAt: string
  updatedAt: string
}

export type DeskSnapshot = {
  repoKey: string
  at: string
  truncated: boolean
  prs: SnapPr[]
}

export type CheckLine = {
  command: string
  paths?: string[]
  runner?: string
  device?: "mac" | "windows" | "linux"
}

export type MergeMethod = "squash" | "merge" | "rebase"

// repo.desk: a repo's desk settings, person-only (§7).
export type RepoDesk = {
  repoKey: string
  enabled: boolean
  takesPrs: boolean
  holder?: string
  pollDevice: string
  trusted: string[]
  checks: CheckLine[]
  mergeMethod: MergeMethod
  perPrOk: boolean
  by?: string
  at?: string
}

export type DeskHold = {
  repoKey: string
  pr: number
  note?: string
  cleared?: boolean
  by: string
  at: string
}
export type DeskPause = {
  repoKey: string
  paused: boolean
  by: string
  at: string
}
export type DeskOk = {
  repoKey: string
  pr: number
  headSha: string
  by: string
  at: string
}
export type DeskRun = {
  repoKey: string
  pr: number
  headSha: string
  baseSha: string
  by: string
  at: string
}

// The holder's own outcome on a PR's head (slices 4-6): its trial squash
// failed, its checks are running, or they failed.
export type DeskCheck = {
  pr: number
  headSha: string
  state: "rebase" | "running" | "failed"
  note?: string
}

// Where a PR's thread is (slice 2): the author card, and the agent behind
// it when the author is one of the person's own agents.
// outside is slice 2's verdict (Core::desk_verdicts), which also counts
// the trusted list and the person's OKs; without it the page judges by
// the trusted list and OKs alone.
export type PrAuthor = {
  card?: string
  cardRef?: string
  agent?: string
  name?: string
  outside?: boolean
}

export type MergedPr = {
  pr: number
  title: string
  sha: string
  at: string
  card?: string
}

export type DeskState =
  | "outside"
  | "draft"
  | "held"
  | "merging"
  | "rebase"
  | "running"
  | "failed"
  | "waiting"
  | "merged"

export const stateLabel: Record<DeskState, string> = {
  outside: "Outside PR",
  draft: "Draft",
  held: "Held",
  merging: "Merging",
  rebase: "Needs rebase",
  running: "Checks running",
  failed: "Checks failed",
  waiting: "Waiting on desk",
  merged: "Merged",
}

export type QueueLine = {
  pr: number
  url: string
  title: string
  login: string
  author: PrAuthor
  state: DeskState
  // One short line under the title: why it is in this state.
  note: string
  headSha: string
  createdAt: string
  hold?: DeskHold
  // The PR is from an untrusted author and Stephen's OK would let the
  // desk act on this head.
  canOk: boolean
}

export type DeskInputs = {
  desk?: RepoDesk
  snapshot?: DeskSnapshot | null
  holds?: DeskHold[]
  oks?: DeskOk[]
  run?: DeskRun | null
  checks?: DeskCheck[]
  authors?: Record<number, PrAuthor>
  merged?: MergedPr[]
}

// activeHolds is the latest hold row per PR, dropping cleared ones.
export function activeHolds(holds: DeskHold[]): Map<number, DeskHold> {
  const last = new Map<number, DeskHold>()
  for (const h of [...holds].sort((a, b) => a.at.localeCompare(b.at)))
    last.set(h.pr, h)
  for (const [pr, h] of last) if (h.cleared) last.delete(pr)
  return last
}

// trusted: one of the person's own agents is behind it, its login is on
// the desk's list, or Stephen okayed this very head (§5 Trust).
function isTrusted(
  p: SnapPr,
  a: PrAuthor,
  desk: RepoDesk | undefined,
  oks: DeskOk[]
): boolean {
  if (a.outside !== undefined) return !a.outside
  if (a.agent) return true
  const login = (p.author ?? "").toLowerCase()
  if (login && desk?.trusted?.some((t) => t.toLowerCase() === login))
    return true
  return oks.some(
    (o) => o.pr === p.number && !!p.headRefOid && o.headSha === p.headRefOid
  )
}

// deskQueue is the queue panel's lines in order: Merging first, then
// Waiting on desk oldest PR first, then the rest (also oldest first), then
// any just-merged PRs (§3).
export function deskQueue(inp: DeskInputs): QueueLine[] {
  const holds = activeHolds(inp.holds ?? [])
  const oks = inp.oks ?? []
  const checks = inp.checks ?? []
  const lines: QueueLine[] = []
  for (const p of inp.snapshot?.prs ?? []) {
    const author = inp.authors?.[p.number] ?? {}
    const head = p.headRefOid ?? ""
    const login = p.author ?? ""
    const hold = holds.get(p.number)
    const trusted = isTrusted(p, author, inp.desk, oks)
    const mine = checks.find((c) => c.pr === p.number && c.headSha === head)
    const gh = p.checks ?? ""
    const run = inp.run && inp.run.pr === p.number ? inp.run : null
    let state: DeskState
    let note: string
    if (!trusted) {
      state = "outside"
      note = `${login ? "@" + login : "the author"} isn't trusted: never checked or merged until you OK it`
    } else if (p.isDraft) {
      state = "draft"
      note = "draft · listed, never queued"
    } else if (hold) {
      state = "held"
      note = hold.note ? `held: “${hold.note}”` : "held from its queue line"
    } else if (run) {
      state = "merging"
      note = `lease desk/${inp.desk?.repoKey ? leaseRepo(inp.desk.repoKey) : "…"} · head ${short(run.headSha)}`
    } else if (
      p.mergeStateStatus === "BEHIND" ||
      p.mergeStateStatus === "DIRTY" ||
      p.mergeable === "CONFLICTING" ||
      mine?.state === "rebase"
    ) {
      state = "rebase"
      note =
        mine?.state === "rebase"
          ? mine.note || "trial squash on main failed"
          : `GitHub: ${p.mergeable === "CONFLICTING" ? "CONFLICTING" : p.mergeStateStatus}`
    } else if (gh === "pending" || mine?.state === "running") {
      state = "running"
      note =
        mine?.state === "running"
          ? mine.note || "the holder's checks"
          : "GitHub checks running"
    } else if (gh === "failure" || mine?.state === "failed") {
      state = "failed"
      note =
        mine?.state === "failed"
          ? mine.note || "the holder's checks failed"
          : "GitHub checks failed"
    } else {
      state = "waiting"
      note = author.card ? "ready for the desk" : "no author card yet"
    }
    lines.push({
      pr: p.number,
      url: p.url,
      title: p.title,
      login,
      author,
      state,
      note,
      headSha: head,
      createdAt: p.createdAt,
      hold,
      canOk: state === "outside",
    })
  }
  const open = new Set(lines.map((l) => l.pr))
  const byAge = (a: QueueLine, b: QueueLine) =>
    a.createdAt.localeCompare(b.createdAt) || a.pr - b.pr
  const rank = (s: DeskState) => (s === "merging" ? 0 : s === "waiting" ? 1 : 2)
  lines.sort((a, b) => rank(a.state) - rank(b.state) || byAge(a, b))
  for (const m of inp.merged ?? []) {
    if (open.has(m.pr)) continue
    lines.push({
      pr: m.pr,
      url: "",
      title: m.title,
      login: "",
      author: { card: m.card },
      state: "merged",
      note: `merged as ${short(m.sha)}`,
      headSha: m.sha,
      createdAt: m.at,
      canOk: false,
    })
  }
  return lines
}

// leaseRepo is the desk lease's owner/name part: lowercased (§5). A repo
// key looks like remote:github.com/Owner/Name.
export function leaseRepo(repoKey: string): string {
  return ownerName(repoKey).toLowerCase()
}

// ownerName is "Owner/Name" from a repo key, or the key itself.
export function ownerName(repoKey: string): string {
  const m = repoKey.match(/github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/)
  return m ? m[1] : repoKey
}

export function short(sha: string): string {
  return sha.slice(0, 7)
}

// queueCount is the panel head's count: "n open on GitHub", or what the
// panel shows when the list is cut short or can't be read.
export function queueCount(snap: DeskSnapshot | null | undefined): string {
  if (!snap) return "not polled yet"
  const n = snap.prs.length
  return snap.truncated
    ? `showing the first ${n}; GitHub has more`
    : `${n} open on GitHub`
}

// ---- the gateway ------------------------------------------------------------
//
// Desk is one repo's desk as the Core folds it (coord Desk, slice 1),
// plus what only the polling device knows (its last poll, and since when
// GitHub can't be asked) and slice 2's author mapping.

export type Desk = {
  settings: RepoDesk
  prs?: DeskSnapshot | null
  holds: Record<number, DeskHold>
  pause?: DeskPause | null
  oks: Record<number, DeskOk>
  run?: DeskRun | null
  poll?: { at?: string; unreachableSince?: string; device?: string }
  authors?: Record<number, PrAuthor>
  checks?: DeskCheck[]
  merged?: MergedPr[]
}

// inputsOf is what deskQueue needs from one desk.
export function inputsOf(d: Desk): DeskInputs {
  return {
    desk: d.settings,
    snapshot: d.prs ?? null,
    holds: Object.values(d.holds ?? {}),
    oks: Object.values(d.oks ?? {}),
    run: d.run ?? null,
    checks: d.checks ?? [],
    authors: d.authors ?? {},
    merged: d.merged ?? [],
  }
}

// deskOn: enabled, and the repo takes PRs.
export function deskOn(d: Desk): boolean {
  return d.settings.enabled && d.settings.takesPrs
}

export function paused(d: Desk): boolean {
  return Boolean(d.pause?.paused)
}

// newDesk is the settings a new desk starts from (coord new_desk).
export function newDesk(repoKey: string, pollDevice: string): RepoDesk {
  return {
    repoKey,
    enabled: true,
    takesPrs: true,
    pollDevice,
    trusted: [],
    checks: [],
    mergeMethod: "squash",
    perPrOk: false,
  }
}

// ---- the host ----------------------------------------------------------------

function time(v: string | undefined): string | undefined {
  return parseTime(v) ? v : undefined
}

// fromHost is a window DeskView as the page's Desk.
export function fromHost(v: DeskView): Desk {
  const st = v.settings
  const repoKey = st.repoKey
  return {
    settings: {
      repoKey,
      enabled: st.enabled,
      takesPrs: st.takesPrs,
      holder: st.holder || undefined,
      pollDevice: st.pollDevice ?? "",
      trusted: st.trusted ?? [],
      checks: (st.checks ?? []).map((c) => ({
        command: c.command,
        paths: c.paths?.length ? c.paths : undefined,
        runner: c.runner || undefined,
        device: (c.device || undefined) as CheckLine["device"],
      })),
      mergeMethod: (st.mergeMethod || "squash") as MergeMethod,
      perPrOk: st.perPrOk ?? false,
      by: st.by,
      at: time(st.at),
    },
    prs: v.prs
      ? {
          repoKey,
          at: v.prs.at,
          truncated: v.prs.truncated,
          prs: v.prs.prs.map((p) => ({ ...p })),
        }
      : null,
    holds: Object.fromEntries(
      v.holds.map((h) => [
        h.pr,
        { repoKey, pr: h.pr, note: h.note || undefined, by: h.by, at: h.at },
      ])
    ),
    pause: v.paused
      ? { repoKey, paused: true, by: v.paused.by, at: v.paused.at }
      : null,
    oks: Object.fromEntries(
      v.oks.map((o) => [
        o.pr,
        { repoKey, pr: o.pr, headSha: o.headSha, by: o.by, at: o.at },
      ])
    ),
    run: v.run
      ? {
          repoKey,
          pr: v.run.pr,
          headSha: v.run.headSha,
          baseSha: v.run.note,
          by: v.run.by,
          at: v.run.at,
        }
      : null,
    poll: v.poll.here
      ? {
          at: time(v.poll.lastPollAt),
          unreachableSince: time(v.poll.unreachableSince),
          device: st.pollDevice,
        }
      : undefined,
    authors: Object.fromEntries(
      (v.authors ?? []).map((a) => [
        a.pr,
        {
          card: a.card || undefined,
          cardRef: a.cardRef || undefined,
          agent: a.agent || undefined,
          outside: a.outside,
        },
      ])
    ),
  }
}

// toHost is the settings as the window's DeskSettings.
function toHost(d: RepoDesk) {
  return {
    repoKey: d.repoKey,
    enabled: d.enabled,
    takesPrs: d.takesPrs,
    holder: d.holder ?? "",
    pollDevice: d.pollDevice,
    trusted: d.trusted,
    checks: d.checks.map((c) => ({
      command: c.command,
      paths: c.paths ?? [],
      runner: c.runner ?? "",
      device: c.device ?? "",
    })),
    mergeMethod: d.mergeMethod,
    perPrOk: d.perPrOk,
    by: "",
    at: "0001-01-01T00:00:00Z",
  }
}

// ---- sample mode and the hook --------------------------------------------------

let sample: Desk[] | null = null
const listeners = new Set<() => void>()

function sampleState(): Desk[] {
  sample ??= sampleDesks(new Date(), previewFlag("desk"))
  return sample
}

function changed() {
  sample = (sample ?? []).map((d) => ({ ...d }))
  for (const fn of listeners) fn()
}

function sampleDesk(repoKey: string): Desk {
  const d = sampleState().find((x) => x.settings.repoKey === repoKey)
  if (!d) throw new Error(`${repoKey} has no merge desk`)
  return d
}

const now = () => new Date().toISOString()

// The person's writes (§7: person-only rows). Inside the app they are
// the window's desk commands; in the preview they change the sample.
export const deskActions = {
  async setDesk(d: RepoDesk): Promise<void> {
    if (inWails()) return DeskService.Set(toHost(d)).then(refresh)
    const list = sampleState()
    const i = list.findIndex((x) => x.settings.repoKey === d.repoKey)
    if (i >= 0)
      list[i] = { ...list[i], settings: { ...d, by: "user:you", at: now() } }
    else
      list.push({
        settings: { ...d, by: "user:you", at: now() },
        holds: {},
        oks: {},
      })
    changed()
  },
  async hold(
    repoKey: string,
    pr: number,
    note: string,
    cleared: boolean
  ): Promise<void> {
    if (inWails())
      return DeskService.Hold(repoKey, pr, note, cleared).then(refresh)
    const d = sampleDesk(repoKey)
    const holds = { ...d.holds }
    if (cleared) delete holds[pr]
    else
      holds[pr] = {
        repoKey,
        pr,
        note: note || undefined,
        by: "user:you",
        at: now(),
      }
    d.holds = holds
    changed()
  },
  async pause(repoKey: string, on: boolean): Promise<void> {
    if (inWails()) return DeskService.Pause(repoKey, on).then(refresh)
    sampleDesk(repoKey).pause = {
      repoKey,
      paused: on,
      by: "user:you",
      at: now(),
    }
    changed()
  },
  async ok(repoKey: string, pr: number, headSha: string): Promise<void> {
    if (inWails()) return DeskService.Ok(repoKey, pr, headSha).then(refresh)
    const d = sampleDesk(repoKey)
    d.oks = {
      ...d.oks,
      [pr]: { repoKey, pr, headSha, by: "user:you", at: now() },
    }
    changed()
  },
}

export type Desks = { desks: Desk[] }

// snapshot is the current list; a new array after every change so
// useSyncExternalStore sees it.
let snapshot: Desk[] = []
let loaded = false

function current(): Desk[] {
  if (!inWails() && !loaded) {
    loaded = true
    snapshot = [...sampleState()]
  }
  return snapshot
}

listeners.add(() => {
  if (!inWails()) snapshot = [...sampleState()]
})

const shownDesks: { current: string | null } = { current: null }

// refresh reads every desk from the window again.
function refresh(): void {
  if (!inWails()) return
  DeskService.List()
    .then((list) => {
      // An unchanged list keeps its snapshot, so App doesn't re-render on
      // every board change (0.11.1).
      if (sameAs(shownDesks, list ?? [])) return
      snapshot = (list ?? []).map(fromHost)
      for (const fn of listeners) fn()
    })
    .catch(() => {})
}

// The desk rows are coord rows, so a board change may be one; the poll's
// own times live on this device only, so the list is also read again
// every half minute while anything watches it.
const RefreshMs = 30_000
let stopHost: (() => void) | null = null

function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  if (inWails() && !stopHost) {
    refresh()
    const off = Events.On("board:changed", () => refresh())
    const id = window.setInterval(refresh, RefreshMs)
    stopHost = () => {
      off()
      window.clearInterval(id)
    }
  }
  return () => {
    listeners.delete(fn)
    if (stopHost && [...listeners].length <= 1) {
      stopHost()
      stopHost = null
    }
  }
}

// useDesks is every merge desk in the person's log.
export function useDesks(): Desks {
  const desks = useSyncExternalStore(subscribe, current, current)
  return { desks }
}
