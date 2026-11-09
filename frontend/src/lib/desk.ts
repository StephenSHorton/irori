// Merge desk (card #417, docs/design/merge-desk.md §3, §7): the list of
// open PRs each desk shows for review, worked out from the poller's
// snapshot (desk.prs, slice 1). The desk never merges (card #418): each
// agent asks the person "Can I merge PR #N?" on its own card. deskQueue is pure so every row of §3's state
// table is a test (desk.test.ts); the page only draws what it returns.
// The host's snapshot holds only our PRs (the person's, their space
// members' and their agents'; card #625): anyone else's never reaches it.
//
// Field names follow §7's row schemas so the generated models drop in.

import { useSyncExternalStore } from "react"
import { sameAs } from "@/lib/talk"

import { DeskService, Events } from "./host"
import { timedRead } from "./window-log"
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

// repo.desk: a repo's desk settings, person-only (§7).
export type RepoDesk = {
  repoKey: string
  enabled: boolean
  takesPrs: boolean
  pollDevice: string
  trusted: string[]
  // Kept only so a save writes the row back as it was (card #625: no
  // per-PR OKs any more).
  perPrOk: boolean
  // Other people's agents don't review this repo's PRs: off, the repo
  // behaves like a solo one (card #1077). Absent is on.
  reviewsOff: boolean
  by?: string
  at?: string
}

// Where a PR's thread is (slice 2): the author card, and the agent behind
// it when the author is one of the person's own agents.
export type PrAuthor = {
  card?: string
  cardRef?: string
  agent?: string
  name?: string
  // Why the PR has no review card in a shared space yet (card #1077).
  reviewNote?: string
  // Who reviews it and where the review stands (card #1080).
  review?: PrReview
}

// A PR's review (merge desk §12.9, card #1080), read from the shared
// boards. pick: "none" (no PR card), "off", "waiting", "asked",
// "claimed" or "reviewed".
export type PrReview = {
  pick: string
  // GitHub's review decision: APPROVED, CHANGES_REQUESTED,
  // REVIEW_REQUIRED, or empty.
  review?: string
  // The agent asked, while pick is "asked".
  asked?: string
  claimer?: { agent: string; person?: string; until?: string }
  related?: { agent: string; person?: string; why: string }[]
  // Why nothing is picked, as this viewer can see it.
  whyNone?: string
}

// One person's review claims (card #1080).
export type ClaimCount = { person: string; open: number; recent: number }

export type ReviewParts = {
  github?: string
  pick?: string
  claimer?: string
  until?: string
  related: string[]
  why?: string
}

const githubLabel: Record<string, string> = {
  APPROVED: "Approved",
  CHANGES_REQUESTED: "Changes requested",
  REVIEW_REQUIRED: "Review required",
}

const who = (agent: string, person?: string) =>
  person ? `${agent || "an agent"} (${person})` : agent || "an agent"

// reviewParts is what a desk line shows of a PR's review: each part is
// absent when there is nothing to say, so an empty review draws nothing.
export function reviewParts(r: PrReview | undefined): ReviewParts {
  const out: ReviewParts = { related: [] }
  if (!r) return out
  if (r.review) out.github = githubLabel[r.review] ?? r.review
  if (r.claimer) {
    out.claimer = who(r.claimer.agent, r.claimer.person)
    out.until = r.claimer.until
  } else if (r.pick === "asked") {
    out.pick = `Asked ${r.asked || "an agent"}`
  } else if (r.pick === "waiting") {
    out.pick = "Nobody picked yet"
  } else if (r.pick === "reviewed") {
    out.pick = "Reviewed"
  }
  out.related = (r.related ?? []).map(
    (x) => `${who(x.agent, x.person)} · ${x.why}`
  )
  if (r.whyNone) out.why = r.whyNone
  return out
}

// claimCountsLine is the header's "Maya 1 open · 3 in 14 days", one
// entry per person with any claim; empty when nobody has one.
export function claimCountsLine(counts: ClaimCount[] | undefined): string {
  return (counts ?? [])
    .filter((c) => c.open > 0 || c.recent > 0)
    .map((c) => `${c.person} ${c.open} open · ${c.recent} in 14 days`)
    .join(" · ")
}

export type MergedPr = {
  pr: number
  title: string
  sha: string
  at: string
  card?: string
}

export type DeskState =
  "draft" | "rebase" | "running" | "failed" | "ready" | "merged"

export const stateLabel: Record<DeskState, string> = {
  draft: "Draft",
  rebase: "Needs rebase",
  running: "Checks running",
  failed: "Checks failed",
  ready: "Ready for review",
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
}

export type DeskInputs = {
  desk?: RepoDesk
  snapshot?: DeskSnapshot | null
  authors?: Record<number, PrAuthor>
  merged?: MergedPr[]
}

// deskQueue is the list's lines in order: Ready for review oldest PR
// first, then the rest (also oldest first), then any just-merged PRs (§3).
export function deskQueue(inp: DeskInputs): QueueLine[] {
  const lines: QueueLine[] = []
  for (const p of inp.snapshot?.prs ?? []) {
    const author = inp.authors?.[p.number] ?? {}
    const head = p.headRefOid ?? ""
    const login = p.author ?? ""
    const gh = p.checks ?? ""
    let state: DeskState
    let note: string
    if (p.isDraft) {
      state = "draft"
      note = "draft · listed, never queued"
    } else if (
      p.mergeStateStatus === "BEHIND" ||
      p.mergeStateStatus === "DIRTY" ||
      p.mergeable === "CONFLICTING"
    ) {
      state = "rebase"
      note = `GitHub: ${p.mergeable === "CONFLICTING" ? "CONFLICTING" : p.mergeStateStatus}`
    } else if (gh === "pending") {
      state = "running"
      note = "GitHub checks running"
    } else if (gh === "failure") {
      state = "failed"
      note = "GitHub checks failed"
    } else {
      state = "ready"
      note = author.card ? "ready for review" : "no author card yet"
    }
    // The review's own line says it better than the desk's note (#1080).
    if (author.reviewNote && !author.review?.whyNone)
      note = `${note} · ${author.reviewNote}`
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
    })
  }
  const open = new Set(lines.map((l) => l.pr))
  const byAge = (a: QueueLine, b: QueueLine) =>
    a.createdAt.localeCompare(b.createdAt) || a.pr - b.pr
  const rank = (s: DeskState) => (s === "ready" ? 0 : 1)
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
    })
  }
  return lines
}

// ownerName is "Owner/Name" from a repo key, or the key itself.
export function ownerName(repoKey: string): string {
  const m = repoKey.match(/github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/)
  return m ? m[1] : repoKey
}

export function short(sha: string): string {
  return sha.slice(0, 7)
}

// queueCount is the panel head's count: "n of ours open", or what the
// panel shows when the list is cut short or can't be read.
export function queueCount(snap: DeskSnapshot | null | undefined): string {
  if (!snap) return "not polled yet"
  const n = snap.prs.length
  return snap.truncated
    ? `showing ${n} of ours; GitHub has more`
    : `${n} of ours open`
}

// ---- the gateway ------------------------------------------------------------
//
// Desk is one repo's desk as the Core folds it (coord Desk, slice 1),
// plus what only the polling device knows (its last poll, and since when
// GitHub can't be asked) and slice 2's author mapping.

export type Desk = {
  settings: RepoDesk
  prs?: DeskSnapshot | null
  poll?: { at?: string; unreachableSince?: string; device?: string }
  authors?: Record<number, PrAuthor>
  merged?: MergedPr[]
  // The settings are the default every GitHub repo an agent works in gets
  // (card #521): on, polled where the repo was seen last.
  isDefault?: boolean
  // The device that saw the repo last ("" never seen).
  seenOn?: string
  // The poll device follows seenOn (the default, or a saved row that left
  // it blank): a save keeps it blank.
  followsSeen?: boolean
  // Each person's open and 14-day claims (card #1080).
  claimCounts?: ClaimCount[]
}

// inputsOf is what deskQueue needs from one desk.
export function inputsOf(d: Desk): DeskInputs {
  return {
    desk: d.settings,
    snapshot: d.prs ?? null,
    authors: d.authors ?? {},
    merged: d.merged ?? [],
  }
}

// deskOn: enabled, and the repo takes PRs.
export function deskOn(d: Desk): boolean {
  return d.settings.enabled && d.settings.takesPrs
}

// deskShown: on, with an open PR of ours (card #625). The page and the
// sidebar list only these; a desk comes back once one of ours opens.
export function deskShown(d: Desk): boolean {
  return deskOn(d) && (d.prs?.prs.length ?? 0) > 0
}

// newDesk is the settings a new desk starts from (coord new_desk).
export function newDesk(repoKey: string, pollDevice: string): RepoDesk {
  return {
    repoKey,
    enabled: true,
    takesPrs: true,
    pollDevice,
    trusted: [],
    perPrOk: false,
    reviewsOff: false,
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
    isDefault: v.isDefault,
    seenOn: v.seenOn || undefined,
    followsSeen: v.followsSeen,
    claimCounts: (v.claimCounts ?? []).map((c) => ({ ...c })),
    settings: {
      repoKey,
      enabled: st.enabled,
      takesPrs: st.takesPrs,
      pollDevice: st.pollDevice ?? "",
      trusted: st.trusted ?? [],
      perPrOk: st.perPrOk ?? false,
      reviewsOff: st.reviewsOff ?? false,
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
          reviewNote: a.reviewNote || undefined,
          review: a.review?.pick
            ? {
                pick: a.review.pick,
                review: a.review.review || undefined,
                asked: a.review.asked || undefined,
                claimer: a.review.claimer
                  ? {
                      agent: a.review.claimer.agent,
                      person: a.review.claimer.person || undefined,
                      until: a.review.claimer.until || undefined,
                    }
                  : undefined,
                related: (a.review.related ?? []).map((x) => ({
                  agent: x.agent,
                  person: x.person || undefined,
                  why: x.why,
                })),
                whyNone: a.review.whyNone || undefined,
              }
            : undefined,
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
    pollDevice: d.pollDevice,
    trusted: d.trusted,
    perPrOk: d.perPrOk,
    reviewsOff: d.reviewsOff,
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
      })
    changed()
  },
}

// loaded is false until the first read lands (card #598).
export type Desks = { desks: Desk[]; loaded: boolean }

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
  timedRead("desk", () => DeskService.List())
    .then((list) => {
      const first = !hostRead
      hostRead = true
      // An unchanged list keeps its snapshot, so App doesn't re-render on
      // every board change (0.11.1).
      if (sameAs(shownDesks, list ?? []) && !first) return
      snapshot = (list ?? []).map(fromHost)
      for (const fn of listeners) fn()
    })
    .catch(() => {
      // A failed first read still ends the skeleton: the pane shows what
      // it has rather than loading for ever.
      if (hostRead) return
      hostRead = true
      for (const fn of listeners) fn()
    })
}

// hostRead is whether the window's list has answered once (card #598): the
// Merge desks pane shows its skeleton until it has, never on a re-read.
let hostRead = false
const isRead = () => !inWails() || hostRead

// The desk rows are coord rows, so a board change may be one; the poll's
// own times live on this device only, so the list is also read again
// every half minute while anything watches it.
const RefreshMs = 30_000
let stopHost: (() => void) | null = null

function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  if (inWails() && !stopHost) {
    refresh()
    const off = Events.OnShown("board:changed", () => refresh())
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
  const loaded = useSyncExternalStore(subscribe, isRead, isRead)
  return { desks, loaded }
}
