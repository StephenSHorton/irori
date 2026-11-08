import { describe, expect, test } from "bun:test"

import {
  deskQueue,
  deskShown,
  inputsOf as inputsOfDesk,
  queueCount,
  type Desk,
  type RepoDesk,
  type SnapPr,
} from "./desk"

const REPO = "remote:github.com/StephenSHorton/Game-Repo"

const desk: RepoDesk = {
  repoKey: REPO,
  enabled: true,
  takesPrs: true,
  pollDevice: "pc",
  trusted: ["StephenSHorton"],
  perPrOk: false,
  reviewsOff: false,
}

let n = 100
function pr(over: Partial<SnapPr> = {}): SnapPr {
  n++
  return {
    number: n,
    url: `https://github.com/x/y/pull/${n}`,
    title: `PR ${n}`,
    author: "some-bot",
    isDraft: false,
    headRefName: `b${n}`,
    headRefOid: `sha${n}aaaaaaa`,
    baseRefName: "main",
    mergeable: "MERGEABLE",
    mergeStateStatus: "CLEAN",
    checks: "",
    createdAt: `2026-11-03T10:${String(n % 60).padStart(2, "0")}:00Z`,
    updatedAt: "2026-11-03T12:00:00Z",
    ...over,
  }
}

// mine maps every PR to one of the person's agents (trusted).
function mine(prs: SnapPr[]) {
  return Object.fromEntries(
    prs.map((p) => [p.number, { card: "c", agent: "agent:a" }])
  )
}

function one(p: SnapPr, extra: Parameters<typeof deskQueue>[0] = {}) {
  const lines = deskQueue({
    desk,
    snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] },
    authors: mine([p]),
    ...extra,
  })
  return lines[0]
}

describe("the §3 state table (card #417)", () => {
  test("the snapshot holds only our PRs: one with no author entry is queued as any other (card #625)", () => {
    const p = pr({ author: "stranger" })
    const l = deskQueue({
      desk: { ...desk, trusted: [] },
      snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] },
    })[0]
    expect(l.state).toBe("ready")
    expect(l.note).toBe("no author card yet")
    expect(l.author).toEqual({})
  })
  test("Draft is listed, never queued", () => {
    expect(one(pr({ isDraft: true })).state).toBe("draft")
  })
  test("Needs rebase: BEHIND, DIRTY or CONFLICTING on GitHub", () => {
    expect(one(pr({ mergeStateStatus: "BEHIND" })).state).toBe("rebase")
    expect(one(pr({ mergeStateStatus: "DIRTY" })).state).toBe("rebase")
    expect(one(pr({ mergeable: "CONFLICTING" })).state).toBe("rebase")
  })
  test("Checks running and failed come from GitHub's rollup", () => {
    expect(one(pr({ checks: "pending" })).state).toBe("running")
    expect(one(pr({ checks: "failure" })).state).toBe("failed")
  })
  test("Ready for review when none of the above (card #418: no merging)", () => {
    const l = one(pr({ checks: "success" }))
    expect(l.state).toBe("ready")
    expect(l.note).toBe("ready for review")
  })
  test("Merged: a pr.state row for a PR no longer in the open list", () => {
    const p = pr()
    const lines = deskQueue({
      desk,
      snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] },
      authors: mine([p]),
      merged: [
        { pr: 1, title: "old", sha: "3f2a9c1deadbeef", at: "t" },
        { pr: p.number, title: "still open", sha: "x", at: "t" },
      ],
    })
    expect(lines.map((l) => l.state)).toEqual(["ready", "merged"])
    expect(lines[1].note).toBe("merged as 3f2a9c1")
  })
})

describe("queue order (§3)", () => {
  test("Ready for review oldest first, then the rest", () => {
    const newer = pr({ createdAt: "2026-11-03T12:00:00Z" })
    const older = pr({ createdAt: "2026-11-03T09:00:00Z" })
    const failed = pr({ createdAt: "2026-11-03T08:00:00Z", checks: "failure" })
    const prs = [newer, failed, older]
    const lines = deskQueue({
      desk,
      snapshot: { repoKey: REPO, at: "t", truncated: false, prs },
      authors: mine(prs),
    })
    expect(lines.map((l) => l.pr)).toEqual([
      older.number,
      newer.number,
      failed.number,
    ])
  })
})

describe("panel texts", () => {
  test("the count says when GitHub has more, and never reads empty before a poll", () => {
    expect(queueCount(null)).toBe("not polled yet")
    expect(
      queueCount({ repoKey: REPO, at: "t", truncated: false, prs: [] })
    ).toBe("0 of ours open")
    expect(
      queueCount({ repoKey: REPO, at: "t", truncated: true, prs: [pr()] })
    ).toBe("showing 1 of ours; GitHub has more")
  })
})

describe("the window's DeskView (card #417)", () => {
  test("zero times and empty fields read as absent", async () => {
    const { fromHost } = await import("./desk")
    const Z = "0001-01-01T00:00:00Z"
    const d = fromHost({
      settings: {
        repoKey: REPO,
        enabled: true,
        takesPrs: true,
        pollDevice: "pc",
        trusted: [],
        perPrOk: false,
        reviewsOff: false,
        by: "user:pc",
        at: Z,
      },
      isDefault: false,
      seenOn: "",
      followsSeen: false,
      prs: {
        truncated: false,
        at: "2026-11-03T10:00:00Z",
        prs: [],
        viewer: "",
      },
      poll: {
        here: true,
        lastPollAt: "2026-11-03T10:01:00Z",
        unreachableSince: Z,
        error: "",
      },
      authors: [{ pr: 9, card: "", cardRef: "", agent: "dev/s1" }],
    })
    expect(d.authors?.[9]).toEqual({
      card: undefined,
      cardRef: undefined,
      agent: "dev/s1",
    })
    expect(d.poll).toEqual({
      at: "2026-11-03T10:01:00Z",
      unreachableSince: undefined,
      device: "pc",
    })
    expect(d.isDefault).toBe(false)
    expect(d.seenOn).toBeUndefined()
    expect(deskQueue(inputsOfDesk(d))).toEqual([])
  })
})

describe("which desks are listed (card #625)", () => {
  const snap = (prs: SnapPr[]) => ({
    repoKey: REPO,
    at: "2026-11-05T10:00:00Z",
    truncated: false,
    prs,
  })
  const one = (over: Partial<Desk> = {}): Desk => ({
    settings: { ...desk, trusted: [] },
    isDefault: true,
    seenOn: "pc",
    ...over,
  })

  test("a desk with an open PR of ours is listed", () => {
    expect(deskShown(one({ prs: snap([pr()]) }))).toBe(true)
  })

  test("a desk with none of ours open, or not polled yet, is hidden", () => {
    expect(deskShown(one({ prs: snap([]) }))).toBe(false)
    expect(deskShown(one())).toBe(false)
    expect(deskShown(one({ prs: null }))).toBe(false)
  })

  test("a desk turned off, or taking no PRs, is hidden even with PRs", () => {
    const prs = snap([pr()])
    expect(deskShown(one({ prs, settings: { ...desk, enabled: false } }))).toBe(
      false
    )
    expect(
      deskShown(one({ prs, settings: { ...desk, takesPrs: false } }))
    ).toBe(false)
  })
})
