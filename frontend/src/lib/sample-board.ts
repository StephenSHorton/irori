// The built-in Board shown when the page runs in a plain browser, next to the
// sample roster in sample.ts. Every value is made up and says so. Add ?empty
// to the URL for the empty state.

import {
  BlockerKind,
  LinkKind,
  TaskState,
  type Comment,
  type LeaseCard,
  type Link,
  type Task,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"

const dev = "sample-device"
const otter = `${dev}/sample01-synthetic-session`
const heron = `${dev}/sample02-synthetic-session`
const badger = `${dev}/sample03-synthetic-session`
const you = `user:${dev}`
const repo = "github.com/sample-org/example-app"
const local = "root:5a3c1e0b9d7f2a4c6e8b0d1f3a5c7e9b1d3f5a7c"

export type SampleBoard = {
  you: string
  tasks: Task[]
  comments: Comment[]
  leases: LeaseCard[]
}

// sampleLink labels a link roughly the way coord.LinkLabel does, enough for
// the sample's own links and ones typed while trying it.
export function sampleLink(ref: string): Link {
  ref = ref.trim()
  if (/^[0-9a-f]{7,40}$/i.test(ref)) {
    return {
      kind: LinkKind.LinkCommit,
      ref: ref.toLowerCase(),
      label: `commit ${ref.slice(0, 7).toLowerCase()}`,
    }
  }
  if (!ref.includes("://")) {
    const segs = ref.replace(/\\/g, "/").split("/").filter(Boolean)
    return {
      kind: LinkKind.LinkPath,
      ref,
      label: segs.length > 2 ? `.../${segs.slice(-2).join("/")}` : ref,
    }
  }
  let label = ref
  try {
    const u = new URL(ref)
    const host = u.hostname.replace(/^www\./, "")
    const gh = u.pathname.match(/^\/[^/]+\/[^/]+\/(?:issues|pull)\/(\d+)/)
    const lin = u.pathname.match(/^\/[^/]+\/issue\/([A-Za-z][A-Za-z0-9]*-\d+)/)
    label =
      host === "github.com" && gh
        ? `GitHub #${gh[1]}`
        : host === "linear.app" && lin
          ? `Linear ${lin[1].toUpperCase()}`
          : host
  } catch {
    // keep the ref as its own label
  }
  return { kind: LinkKind.LinkURL, ref, label }
}

// sampleBoard builds the synthetic Board relative to now.
export function sampleBoard(now: Date): SampleBoard {
  const at = (agoMin: number) =>
    new Date(now.getTime() - agoMin * 60000).toISOString()
  const until = (inMin: number) =>
    new Date(now.getTime() + inMin * 60000).toISOString()
  if (new URLSearchParams(window.location.search).has("empty")) {
    return { you, tasks: [], comments: [], leases: [] }
  }

  const task = (t: Partial<Task> & Pick<Task, "id" | "title">): Task =>
    ({
      objective: "",
      state: TaskState.Submitted,
      createdBy: you,
      createdAt: at(300),
      updatedAt: at(60),
      ...t,
      holder: t.holders?.[0],
    }) as Task

  const tasks: Task[] = [
    task({
      id: "sample-t1",
      title: "Sample: which name wins for the made-up recording setting?",
      repoKey: repo,
      state: TaskState.Blocked,
      reason:
        "Settings, tray and README use three names for one sample feature; I need your pick before renaming.",
      blockedBy: { kind: BlockerKind.BlockedOnPerson },
      holders: [otter],
      createdBy: otter,
      updatedAt: at(22),
      links: [
        sampleLink("https://github.com/sample-org/example-app/pull/56"),
        sampleLink("frontend/src/routes/Settings.tsx"),
      ],
    }),
    task({
      id: "sample-t2",
      title: "Sample: port 8799 is held by a stale dev server",
      repoKey: repo,
      state: TaskState.Blocked,
      reason: "The live test needs the port; the lease holder's server exited.",
      blockedBy: { kind: BlockerKind.BlockedOnLease, ref: "port:8799" },
      holders: [badger],
      createdBy: badger,
      updatedAt: at(12),
    }),
    task({
      id: "sample-t3",
      title: "Sample: ship the example app's next release",
      objective: "Tag, release notes and installer for the made-up release.",
      doneCriteria: "Tag pushed, notes final, installer on the release page",
      repoKey: repo,
      state: TaskState.Working,
      holders: [otter],
      updatedAt: at(40),
      links: [
        sampleLink("https://linear.app/sample/issue/EX-123/next-release"),
      ],
    }),
    task({
      id: "sample-t4",
      title: "Sample: review the made-up capture loop",
      repoKey: local,
      state: TaskState.Working,
      shared: true,
      holders: [heron, badger],
      createdBy: heron,
      updatedAt: at(9),
      links: [sampleLink("internal/capture/tailer.go")],
    }),
    task({
      id: "sample-t5",
      title: "Sample: two-machine join test",
      doneCriteria: "Both sample machines see each other in the lobby",
      repoKey: repo,
      shared: true,
      updatedAt: at(180),
      links: [sampleLink("docs/mp-test.md")],
    }),
    task({
      id: "sample-t6",
      title: "Sample: merge the placeholder batch",
      repoKey: repo,
      state: TaskState.Completed,
      holders: [otter],
      createdBy: otter,
      updatedAt: at(95),
      links: [sampleLink("https://github.com/sample-org/example-app/pull/206")],
    }),
  ]

  const c = (
    n: number,
    taskId: string,
    by: string,
    agoMin: number,
    text: string,
    wake = false
  ): Comment => ({
    id: `sample-comment-${n}`,
    taskId,
    by,
    text,
    wake,
    at: at(agoMin),
  })
  const comments: Comment[] = [
    c(
      1,
      "sample-t1",
      otter,
      41,
      "Sample text. Settings says one name, the tray another, the README a third. Which one should all three use?"
    ),
    c(
      2,
      "sample-t1",
      heron,
      30,
      "Sample text. The short one is what people search for."
    ),
    c(
      3,
      "sample-t3",
      you,
      180,
      "Sample text. Once the batch is merged, cut the release."
    ),
    c(
      4,
      "sample-t3",
      otter,
      60,
      "Sample text. Batch merged. Release notes next."
    ),
    c(
      5,
      "sample-t4",
      heron,
      15,
      "Sample text. Looking for a second pair of eyes on the loop."
    ),
    c(
      6,
      "sample-t4",
      badger,
      9,
      "Sample text. Joined. Reading the tailer first."
    ),
    c(
      7,
      "sample-t5",
      you,
      300,
      "Sample text. Whoever has a free slot: needs both machines."
    ),
  ]

  const leases: LeaseCard[] = [
    {
      resource: "port:8799",
      holder: heron,
      until: until(24),
      acquiredAt: at(70),
    } as LeaseCard,
  ]

  return { you, tasks, comments, leases }
}
