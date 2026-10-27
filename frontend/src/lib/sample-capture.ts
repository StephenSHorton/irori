// The built-in capture record shown when the page runs in a plain browser,
// next to the sample roster (sample.ts) and inbox (sample-talk.ts). Every
// value is made up and says so: no prompt, path, sha or patch here comes
// from a real session. Add ?empty to the URL for the empty states.

import type {
  CommitView,
  DecisionView,
  EditRow,
  Entry,
  TurnView,
} from "../../bindings/github.com/StephenSHorton/irori/internal/capture/models"
import { EntryKind } from "../../bindings/github.com/StephenSHorton/irori/internal/capture/models"
import { AnchorState } from "../../bindings/github.com/StephenSHorton/irori/internal/decision/models"
import type { Decision } from "../../bindings/github.com/StephenSHorton/irori/internal/decision/models"
import {
  Role,
  type Entry as PREntry,
} from "../../bindings/github.com/StephenSHorton/irori/internal/ledger/models"
import type { CaptureRepo } from "../../bindings/github.com/StephenSHorton/irori/models"

const dev = "sample-device"
export const sampleOtter = `${dev}/sample01-synthetic-session`
const heron = `${dev}/sample02-synthetic-session`
const badger = `${dev}/sample03-synthetic-session`
const goneKey = `${dev}/sample99-synthetic-session`
const repo = "github.com/sample-org/example-app"
const root = "C:\\Users\\sample\\projects\\example-app"
const localRepo = "root:5a3c1e0b9d7f2a4c6e8b0d1f3a5c7e9b1d3f5a7c"

// Made-up object ids: obviously patterned so nobody mistakes them for real.
const sha = (n: number) => `${n}`.repeat(40).slice(0, 40).replace(/^./, "c")
const blob = (n: number) => `${n}`.repeat(64).slice(0, 64).replace(/^./, "b")

export type SampleCapture = {
  timelines: Map<string, Entry[]>
  commits: CommitView[]
  decisions: DecisionView[]
  patches: Map<string, string>
  repos: CaptureRepo[]
  prs: PREntry[]
  fidelity: Record<string, string>
}

const samplePatch = (path: string, adds: string[], dels: string[]) =>
  [
    `--- a/${path}`,
    `+++ b/${path}`,
    `@@ -10,${dels.length + 3} +10,${adds.length + 3} @@`,
    " // Sample context line: nothing here is from a real file.",
    ...dels.map((l) => `-${l}`),
    ...adds.map((l) => `+${l}`),
    " ",
    " export function sampleTail() {}",
  ].join("\n")

export function sampleCapture(now: Date): SampleCapture {
  const empty = new URLSearchParams(window.location.search).has("empty")
  const at = (agoSec: number) =>
    new Date(now.getTime() - agoSec * 1000).toISOString()

  const patches = new Map<string, string>()
  const edit = (
    n: number,
    key: string,
    turnId: string,
    path: string,
    agoSec: number,
    adds: string[],
    dels: string[],
    fidelity = "exact"
  ): EditRow => {
    patches.set(blob(n), samplePatch(path, adds, dels))
    return {
      id: `e-sample${n}`,
      key,
      turnId,
      toolUseId: `toolu_sample${n}`,
      tool: fidelity === "exact" ? "Edit" : "Bash",
      repoKey: repo,
      root,
      path,
      patch: blob(n),
      adds: adds.length,
      dels: dels.length,
      fidelity,
      at: at(agoSec),
    }
  }

  const turn = (
    key: string,
    turnId: string,
    agoSec: number,
    durSec: number,
    prompt: string,
    reply: string,
    tools: Array<[string, string]>,
    edits: EditRow[],
    open = false
  ): TurnView => ({
    id: `t-sample-${turnId}`,
    key,
    repoKey: repo,
    root,
    turnId,
    sessionId: key.split("/")[1],
    origin: "human",
    prompt,
    reply,
    cwd: root,
    branch: "main",
    startedAt: at(agoSec),
    endedAt: open ? undefined : at(agoSec - durSec),
    tools: tools.map(([name, target], i) => ({
      id: `toolu_${turnId}_${i}`,
      name,
      target,
      done: !open || i < tools.length - 1,
      at: at(agoSec - i * 5),
    })),
    editPaths: edits.map((e) => `${root}\\${e.path.replaceAll("/", "\\")}`),
    endReason: open ? undefined : "turn_duration",
    open,
    edits,
  })

  const e1 = edit(
    1,
    sampleOtter,
    "turn-o3",
    "src/widget/parser.ts",
    50,
    [
      "export function parseSampleWidget(input: string) {",
      "  return input.split(',').map((s) => s.trim())",
      "}",
    ],
    ["export function parseSampleWidget() {}"]
  )
  const e2 = edit(
    2,
    sampleOtter,
    "turn-o3",
    "src/widget/parser.test.ts",
    40,
    [
      "test('sample: splits on commas', () => {",
      "  expect(parseSampleWidget('a, b')).toEqual(['a', 'b'])",
      "})",
    ],
    []
  )
  const e3 = edit(
    3,
    sampleOtter,
    "turn-o2",
    "README.md",
    60 * 30,
    ["Sample: the widget parser reads comma lists."],
    ["Sample: TODO."]
  )
  const e4 = edit(
    4,
    sampleOtter,
    "turn-o2",
    "scripts/sample-gen.sh",
    60 * 29,
    ["echo sample"],
    [],
    "heuristic"
  )

  const otterTurns: TurnView[] = [
    turn(
      sampleOtter,
      "turn-o4",
      8,
      0,
      "Sample prompt. Run the example test suite and tell me which sample cases fail.",
      "",
      [
        ["Bash", "bun test src/widget"],
        ["Read", "src/widget/parser.test.ts"],
      ],
      [],
      true
    ),
    turn(
      sampleOtter,
      "turn-o3",
      60,
      40,
      "Sample prompt. Make the widget parser split on commas and add a test for it.",
      "Sample reply. The parser now splits on commas and trims each part; one new test covers it.",
      [
        ["Read", "src/widget/parser.ts"],
        ["Edit", "src/widget/parser.ts"],
        ["Write", "src/widget/parser.test.ts"],
        ["Bash", "bun test src/widget"],
        ["Bash", 'git commit -am "Sample: split widget input on commas"'],
      ],
      [e1, e2]
    ),
    turn(
      sampleOtter,
      "turn-o2",
      60 * 30,
      120,
      "Sample prompt. Document what the widget parser accepts.",
      "Sample reply. README now describes the comma list format.",
      [
        ["Grep", "parseSampleWidget"],
        ["Edit", "README.md"],
        ["Bash", "sh scripts/sample-gen.sh"],
      ],
      [e3, e4]
    ),
    turn(
      sampleOtter,
      "turn-o1",
      60 * 60 * 2,
      300,
      "Sample prompt. Look around the example app and tell me where widgets are parsed.",
      "Sample reply. Widgets are parsed in src/widget/parser.ts; nothing else reads them.",
      [
        ["Glob", "src/**/*.ts"],
        ["Read", "src/widget/parser.ts"],
        ["Agent", "Sample: survey the widget folder"],
      ],
      []
    ),
  ]

  const c1: CommitView = {
    repoKey: repo,
    root,
    op: "commit",
    branch: "main",
    sha: sha(1),
    parents: [sha(2)],
    at: at(25),
    author: "Sample Person",
    subject: "Sample: split widget input on commas",
    files: [
      { path: "src/widget/parser.ts", blob: blob(11), status: "M" },
      { path: "src/widget/parser.test.ts", blob: blob(12), status: "A" },
      { path: "package.json", blob: blob(13), status: "M" },
    ],
    link: {
      repoKey: repo,
      sha: sha(1),
      committer: { key: sampleOtter, turnId: "turn-o3" },
      files: [
        { path: "src/widget/parser.ts", link: "exact", edits: [e1.id] },
        { path: "src/widget/parser.test.ts", link: "partial", edits: [e2.id] },
        { path: "package.json", link: "unattributed" },
      ],
      sessions: [sampleOtter],
      turns: ["turn-o3"],
      at: at(24),
    },
  }
  const c2: CommitView = {
    repoKey: repo,
    root,
    op: "commit",
    branch: "main",
    sha: sha(2),
    parents: [sha(3)],
    at: at(60 * 28),
    author: "Sample Person",
    subject: "Sample: document the comma list format",
    files: [
      { path: "README.md", blob: blob(14), status: "M" },
      { path: "scripts/sample-gen.sh", blob: blob(15), status: "A" },
    ],
    link: {
      repoKey: repo,
      sha: sha(2),
      files: [
        { path: "README.md", link: "exact", edits: [e3.id] },
        { path: "scripts/sample-gen.sh", link: "heuristic", edits: [e4.id] },
      ],
      sessions: [sampleOtter],
      turns: ["turn-o2"],
      at: at(60 * 28),
    },
  }
  const c3: CommitView = {
    repoKey: repo,
    root,
    op: "merge sample-branch",
    branch: "main",
    sha: sha(3),
    parents: [sha(4), sha(5)],
    at: at(60 * 60 * 26),
    author: "Sample Person",
    subject: "Sample: merge a made-up branch nobody recorded",
    files: [{ path: "src/legacy/sample.ts", blob: blob(16), status: "M" }],
    link: {
      repoKey: repo,
      sha: sha(3),
      files: [{ path: "src/legacy/sample.ts", link: "unattributed" }],
      at: at(60 * 60 * 26),
    },
  }

  const pr: PREntry = {
    key: "pr:sample-org/example-app#7",
    repo: "sample-org/example-app",
    number: 7,
    url: "https://github.com/sample-org/example-app/pull/7",
    session: sampleOtter,
    role: Role.Author,
    source: "gh",
    at: at(15),
  }

  const otterEntries: Entry[] = [
    {
      kind: EntryKind.EntryTurn,
      at: otterTurns[0].startedAt,
      turn: otterTurns[0],
    },
    { kind: EntryKind.EntryPR, at: pr.at, pr },
    { kind: EntryKind.EntryCommit, at: c1.at, commit: c1 },
    {
      kind: EntryKind.EntryTurn,
      at: otterTurns[1].startedAt,
      turn: otterTurns[1],
    },
    {
      kind: EntryKind.EntrySummary,
      at: at(60 * 10),
      summary: {
        id: "s-sample1",
        key: sampleOtter,
        sessionId: "sample01-synthetic-session",
        kind: "away_summary",
        text: "Sample summary. While you were away: the parser was documented and committed.",
        at: at(60 * 10),
      },
    },
    { kind: EntryKind.EntryCommit, at: c2.at, commit: c2 },
    {
      kind: EntryKind.EntryTurn,
      at: otterTurns[2].startedAt,
      turn: otterTurns[2],
    },
    {
      kind: EntryKind.EntryTurn,
      at: otterTurns[3].startedAt,
      turn: otterTurns[3],
    },
  ]

  const heronTurn = turn(
    heron,
    "turn-h1",
    95,
    30,
    "Sample prompt. <b>Tags</b> in a prompt stay literal text.",
    "Sample reply. Nothing to change; the sample check passed.",
    [["Bash", "bun run lint"]],
    []
  )

  const d = (
    id: string,
    statement: string,
    why: string,
    agoSec: number,
    by: Decision["by"],
    state: AnchorState | null,
    extra: Partial<DecisionView> = {}
  ): DecisionView => ({
    id,
    by,
    statement,
    why,
    trust: by.kind === "user" ? "user" : "untrusted",
    at: at(agoSec),
    status: state ? { state, start: 10, end: 14 } : undefined,
    ...extra,
  })
  const youParty = { key: `user:${dev}`, name: "you", kind: "user" }
  const otterParty = { key: sampleOtter, name: "sample-otter", kind: "claude" }
  const anchor = (path: string, start = 10, end = 14) => ({
    repoKey: repo,
    root,
    path,
    sha: sha(1),
    start,
    end,
    hash: blob(20),
  })
  const decisions: DecisionView[] = [
    d(
      "d-sample-3",
      "Sample decision. The widget parser trims every part.",
      "Sample reason. Made-up inputs carry spaces after commas.",
      30,
      otterParty,
      AnchorState.OK,
      {
        anchor: anchor("src/widget/parser.ts"),
        supersedes: ["d-sample-1"],
        source: { turnId: "turn-o3" },
      }
    ),
    d(
      "d-sample-2",
      "Sample decision. README examples use only made-up data.",
      "",
      60 * 40,
      youParty,
      AnchorState.Moved,
      { anchor: anchor("README.md", 3, 3) }
    ),
    d(
      "d-sample-4",
      "Sample decision. Legacy sample code stays frozen.",
      "Sample reason. Nobody owns it.",
      60 * 60 * 30,
      youParty,
      AnchorState.Gone,
      { anchor: anchor("src/legacy/old-sample.ts", 0, 0) }
    ),
    d(
      "d-sample-5",
      "Sample decision. Prefer small commits in this made-up repo.",
      "",
      60 * 60 * 40,
      otterParty,
      null
    ),
    d(
      "d-sample-1",
      "Sample decision. The widget parser keeps spaces as they are.",
      "",
      60 * 60 * 50,
      otterParty,
      AnchorState.Changed,
      { anchor: anchor("src/widget/parser.ts"), supersededBy: ["d-sample-3"] }
    ),
  ]

  return {
    timelines: empty
      ? new Map()
      : new Map([
          [sampleOtter, otterEntries],
          [
            heron,
            [
              {
                kind: EntryKind.EntryTurn,
                at: heronTurn.startedAt,
                turn: heronTurn,
              },
            ],
          ],
          [badger, []],
          [goneKey, []],
        ]),
    commits: empty ? [] : [c1, c2, c3],
    decisions: empty ? [] : decisions,
    patches,
    repos: empty
      ? []
      : [
          { repoKey: repo, roots: [root], live: true },
          {
            repoKey: localRepo,
            roots: ["/home/sample/src/placeholder-service"],
            live: true,
          },
        ],
    prs: empty ? [] : [pr],
    fidelity: {
      [sampleOtter]: "mod",
      [heron]: "passive",
      [badger]: "passive",
      [`${dev}/sample04-synthetic-session`]: "passive",
    },
  }
}
