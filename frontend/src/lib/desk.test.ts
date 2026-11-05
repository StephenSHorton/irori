import { describe, expect, test } from "bun:test"

import {
  deskQueue,
  deskSub,
  inputsOf as inputsOfDesk,
  leaseRepo,
  queueCount,
  talkDesks,
  type Desk,
  type RepoDesk,
  type SnapPr,
} from "./desk"

const REPO = "remote:github.com/StephenSHorton/Game-Repo"

const desk: RepoDesk = {
  repoKey: REPO,
  enabled: true,
  takesPrs: true,
  holder: "agent:k",
  pollDevice: "pc",
  trusted: ["StephenSHorton"],
  checks: [{ command: "cargo test" }],
  mergeMethod: "squash",
  perPrOk: false,
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
  return Object.fromEntries(prs.map((p) => [p.number, { card: "c", agent: "agent:a" }]))
}

function one(p: SnapPr, extra: Parameters<typeof deskQueue>[0] = {}) {
  const lines = deskQueue({ desk, snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] }, authors: mine([p]), ...extra })
  return lines[0]
}

describe("the §3 state table (card #417)", () => {
  test("Outside PR: an untrusted author with no agent behind it", () => {
    const p = pr({ author: "stranger" })
    const l = deskQueue({ desk, snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] } })[0]
    expect(l.state).toBe("outside")
    expect(l.canOk).toBe(true)
  })
  test("a login on the trusted list, any case, is not outside", () => {
    const p = pr({ author: "stephenshorton" })
    expect(deskQueue({ desk, snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] } })[0].state).toBe("waiting")
  })
  test("Stephen's OK trusts that head only; a new push needs a new OK", () => {
    const p = pr({ author: "stranger" })
    const ok = { repoKey: REPO, pr: p.number, headSha: p.headRefOid!, by: "user:s", at: "t" }
    const snap = (q: SnapPr) => ({ repoKey: REPO, at: "t", truncated: false, prs: [q] })
    expect(deskQueue({ desk, snapshot: snap(p), oks: [ok] })[0].state).toBe("waiting")
    expect(deskQueue({ desk, snapshot: snap({ ...p, headRefOid: "pushed" }), oks: [ok] })[0].state).toBe("outside")
  })
  test("slice 2's verdict decides when there is one", () => {
    const p = pr({ author: "stephenshorton" })
    const snap = { repoKey: REPO, at: "t", truncated: false, prs: [p] }
    expect(deskQueue({ desk, snapshot: snap, authors: { [p.number]: { outside: true } } })[0].state).toBe("outside")
    const q = pr({ author: "stranger" })
    const snap2 = { repoKey: REPO, at: "t", truncated: false, prs: [q] }
    expect(deskQueue({ desk, snapshot: snap2, authors: { [q.number]: { outside: false } } })[0].state).toBe("waiting")
  })
  test("Draft is listed, never queued", () => {
    expect(one(pr({ isDraft: true })).state).toBe("draft")
  })
  test("Held from a desk.hold row; a later cleared row lifts it", () => {
    const p = pr()
    const h = { repoKey: REPO, pr: p.number, note: "after the playtest", by: "user:s", at: "1" }
    expect(one(p, { holds: [h] }).state).toBe("held")
    expect(one(p, { holds: [h] }).note).toContain("after the playtest")
    expect(one(p, { holds: [h, { ...h, cleared: true, at: "2" }] }).state).toBe("waiting")
  })
  test("Needs rebase: BEHIND, DIRTY, CONFLICTING, or the holder's trial squash", () => {
    expect(one(pr({ mergeStateStatus: "BEHIND" })).state).toBe("rebase")
    expect(one(pr({ mergeStateStatus: "DIRTY" })).state).toBe("rebase")
    expect(one(pr({ mergeable: "CONFLICTING" })).state).toBe("rebase")
    const p = pr()
    expect(one(p, { checks: [{ pr: p.number, headSha: p.headRefOid!, state: "rebase" }] }).state).toBe("rebase")
  })
  test("Checks running: a pending rollup entry or the holder's run", () => {
    expect(one(pr({ checks: "pending" })).state).toBe("running")
    const p = pr()
    expect(one(p, { checks: [{ pr: p.number, headSha: p.headRefOid!, state: "running" }] }).state).toBe("running")
  })
  test("Checks failed: a failing rollup entry or the holder's line", () => {
    expect(one(pr({ checks: "failure" })).state).toBe("failed")
    const p = pr()
    expect(one(p, { checks: [{ pr: p.number, headSha: p.headRefOid!, state: "failed", note: "2 failed" }] }).note).toBe("2 failed")
  })
  test("the holder's outcome on an older head no longer counts", () => {
    const p = pr()
    expect(one(p, { checks: [{ pr: p.number, headSha: "old", state: "failed" }] }).state).toBe("waiting")
  })
  test("Waiting on desk when none of the above", () => {
    expect(one(pr({ checks: "success" })).state).toBe("waiting")
  })
  test("Merging: a desk.run row for this PR", () => {
    const p = pr()
    const l = one(p, { run: { repoKey: REPO, pr: p.number, headSha: p.headRefOid!, baseSha: "b", by: "agent:k", at: "t" } })
    expect(l.state).toBe("merging")
    expect(l.note).toContain("desk/stephenshorton/game-repo")
  })
  test("Merged: a pr.state row for a PR no longer in the open list", () => {
    const p = pr()
    const lines = deskQueue({ desk, snapshot: { repoKey: REPO, at: "t", truncated: false, prs: [p] }, authors: mine([p]), merged: [
      { pr: 1, title: "old", sha: "3f2a9c1deadbeef", at: "t" },
      { pr: p.number, title: "still open", sha: "x", at: "t" },
    ] })
    expect(lines.map((l) => l.state)).toEqual(["waiting", "merged"])
    expect(lines[1].note).toBe("merged as 3f2a9c1")
  })
})

describe("queue order (§3)", () => {
  test("Merging first, then Waiting oldest first, then the rest", () => {
    const newer = pr({ createdAt: "2026-11-03T12:00:00Z" })
    const older = pr({ createdAt: "2026-11-03T09:00:00Z" })
    const failed = pr({ createdAt: "2026-11-03T08:00:00Z", checks: "failure" })
    const merging = pr({ createdAt: "2026-11-03T13:00:00Z" })
    const prs = [newer, failed, merging, older]
    const lines = deskQueue({
      desk,
      snapshot: { repoKey: REPO, at: "t", truncated: false, prs },
      authors: mine(prs),
      run: { repoKey: REPO, pr: merging.number, headSha: merging.headRefOid!, baseSha: "b", by: "agent:k", at: "t" },
    })
    expect(lines.map((l) => l.pr)).toEqual([merging.number, older.number, newer.number, failed.number])
  })
})

describe("panel texts", () => {
  test("the count says when GitHub has more, and never reads empty before a poll", () => {
    expect(queueCount(null)).toBe("not polled yet")
    expect(queueCount({ repoKey: REPO, at: "t", truncated: false, prs: [] })).toBe("0 open on GitHub")
    expect(queueCount({ repoKey: REPO, at: "t", truncated: true, prs: [pr()] })).toBe("showing the first 1; GitHub has more")
  })
  test("the lease name is lowercase owner/name", () => {
    expect(leaseRepo("remote:github.com/You/Game-Repo.git")).toBe("you/game-repo")
  })
})

describe("the window's DeskView (card #417)", () => {
  test("zero times and empty fields read as absent", async () => {
    const { fromHost } = await import("./desk")
    const Z = "0001-01-01T00:00:00Z"
    const d = fromHost({
      settings: {
        repoKey: REPO, enabled: true, takesPrs: true, holder: "", pollDevice: "pc", trusted: [],
        checks: [{ command: "cargo test", paths: [], runner: "", device: "" }],
        mergeMethod: "", perPrOk: false, by: "user:pc", at: Z,
      },
      isDefault: false,
      seenOn: "",
      followsSeen: false,
      prs: { truncated: false, at: "2026-11-03T10:00:00Z", prs: [], viewer: "" },
      holds: [{ pr: 7, note: "", headSha: "", by: "user:pc", at: "2026-11-03T10:00:00Z" }],
      oks: [],
      poll: { here: true, lastPollAt: "2026-11-03T10:01:00Z", unreachableSince: Z, error: "" },
      authors: [{ pr: 9, card: "", cardRef: "", agent: "dev/s1", outside: false }],
    })
    expect(d.authors?.[9]).toEqual({ card: undefined, cardRef: undefined, agent: "dev/s1", outside: false })
    expect(d.settings.holder).toBeUndefined()
    expect(d.settings.mergeMethod).toBe("squash")
    expect(d.settings.checks[0]).toEqual({ command: "cargo test", paths: undefined, runner: undefined, device: undefined })
    expect(d.poll).toEqual({ at: "2026-11-03T10:01:00Z", unreachableSince: undefined, device: "pc" })
    expect(d.holds[7].note).toBeUndefined()
    expect(d.isDefault).toBe(false)
    expect(d.seenOn).toBeUndefined()
    expect(deskQueue(inputsOfDesk(d))).toEqual([])
  })
})

describe("desks in Talk › Channels (card #521)", () => {
  const snap = (prs: SnapPr[]) => ({ repoKey: REPO, at: "2026-11-05T10:00:00Z", truncated: false, prs })
  const one = (over: Partial<Desk> = {}): Desk => ({
    settings: { ...desk, holder: undefined, trusted: [], checks: [] },
    holds: {},
    oks: {},
    isDefault: true,
    seenOn: "pc",
    ...over,
  })

  test("a default desk is listed, held by nobody", () => {
    const [d] = talkDesks([one({ prs: snap([pr()]) })], "")
    expect(d.name).toBe("StephenSHorton/Game-Repo")
    expect(d.sub).toBe("merge desk · 1 open on GitHub")
  })

  test("an empty desk reads No open PRs on GitHub", () => {
    expect(talkDesks([one({ prs: snap([]) })], "")[0].sub).toBe("No open PRs on GitHub")
    expect(deskSub(null)).toBe("merge desk · not polled yet")
  })

  test("a desk turned off is hidden", () => {
    const off = one({ isDefault: false, settings: { ...desk, enabled: false } })
    expect(talkDesks([off], "")).toEqual([])
  })

  test("desks live in the person's own log: Personal lists them, a shared space doesn't", () => {
    expect(talkDesks([one()], "personal")).toHaveLength(1)
    expect(talkDesks([one()], "board-abc")).toEqual([])
  })

  test("search finds a desk by its repo", () => {
    expect(talkDesks([one()], "", "game")).toHaveLength(1)
    expect(talkDesks([one()], "", "kowhai")).toEqual([])
  })
})
