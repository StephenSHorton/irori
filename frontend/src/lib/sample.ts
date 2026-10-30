// The built-in roster shown when the page runs in a plain browser (bun run
// dev), so the UI can be built and screenshotted without the Go app. Every
// value is made up and says so; none of it comes from a real session.
// Add ?empty to the URL to preview the empty state instead.

import { Identity } from "./models"
import { AgentCard } from "./models"
import { Event as LogEvent } from "./models"

const deviceId = "sample-device"

type Seed = {
  session: string
  name: string
  title: string
  cwd: string
  status: string
  kind: string
  entry: string
  agoSec: number
  text: string
  repoKey: string
  mode: string
}

const seeds: Seed[] = [
  {
    session: "sample01-synthetic-session",
    name: "sample-otter",
    title: "Sample: wire up the widget parser",
    cwd: "C:\\Users\\sample\\projects\\example-app",
    status: "busy",
    kind: "interactive",
    entry: "cli",
    agoSec: 3,
    text: "Sample text. Running the example test suite; two of five sample cases pass so far.",
    repoKey: "github.com/sample-org/example-app",
    mode: "bypassPermissions",
  },
  {
    session: "sample02-synthetic-session",
    name: "sample-heron",
    title: "",
    cwd: "C:\\Users\\sample\\projects\\example-app",
    status: "idle",
    kind: "bg",
    entry: "cli",
    agoSec: 95,
    text: "Sample text. <b>Tags</b> and **markdown** stay literal here because agent text is untrusted.",
    repoKey: "github.com/sample-org/example-app",
    mode: "bypassPermissions",
  },
  {
    session: "sample03-synthetic-session",
    name: "sample-badger",
    title: "Sample: draft the placeholder docs",
    cwd: "/home/sample/src/placeholder-service",
    status: "busy",
    kind: "interactive",
    entry: "desktop",
    agoSec: 60 * 42,
    text: "Sample text. The placeholder docs are drafted; waiting for a made-up review.",
    repoKey: "root:5a3c1e0b9d7f2a4c6e8b0d1f3a5c7e9b1d3f5a7c",
    mode: "default",
  },
  {
    session: "sample04-synthetic-session",
    name: "sample-wren",
    title: "",
    cwd: "C:\\Users\\sample\\projects\\demo\\very\\deep\\nested\\workspace",
    status: "waiting",
    kind: "interactive",
    entry: "cli",
    agoSec: 60 * 60 * 5,
    text: "",
    repoKey: "",
    mode: "",
  },
]

// sampleRoster builds the synthetic roster with activity relative to now so
// the relative times tick like real ones.
export function sampleRoster(now: Date) {
  const at = (agoSec: number) => new Date(now.getTime() - agoSec * 1000)
  const empty = new URLSearchParams(window.location.search).has("empty")

  const agents = empty
    ? []
    : seeds.map(
        (s, i) =>
          AgentCard({
            key: `${deviceId}/${s.session}`,
            deviceId,
            agent: "claude",
            sessionId: s.session,
            pid: 10001 + i,
            name: s.name,
            title: s.title,
            cwd: s.cwd,
            status: s.status,
            sessionKind: s.kind,
            entrypoint: s.entry,
            version: "0.0.0-sample",
            peerFeatures: [],
            startedAt: at(s.agoSec + 3600).toISOString(),
            updatedAt: at(s.agoSec).toISOString(),
            lastActivity: at(s.agoSec).toISOString(),
            lastText: s.text,
            transcript: "",
            repoKey: s.repoKey,
            permissionMode: s.mode,
          })
      )

  const hlc = (agoSec: number) => at(agoSec).getTime() * 65536
  // Core writes every row with actor "irori"; the agent a row is about is
  // in payload.key, as here.
  const rows: Array<[string, number, unknown]> = [
    ["device.started", 60 * 60 * 6, null],
    ...agents.map((a, i): [string, number, unknown] => [
      "agent.seen",
      seeds[i].agoSec + 3600,
      a,
    ]),
    [
      "agent.status",
      3,
      { key: agents[0]?.key, status: seeds[0].status, name: seeds[0].name },
    ],
    [
      "agent.status",
      95,
      { key: agents[1]?.key, status: seeds[1].status, name: seeds[1].name },
    ],
    [
      "agent.gone",
      60 * 20,
      { key: `${deviceId}/sample99-synthetic-session`, reason: "gone" },
    ],
  ]
  const recent = empty
    ? []
    : rows
        .map(
          ([kind, ago, payload], i) =>
            LogEvent({
              deviceId,
              seq: i + 1,
              hlc: hlc(ago),
              actor: "irori",
              kind,
              payload: payload as LogEvent["payload"],
            })
        )
        .sort((a, b) => b.hlc - a.hlc)

  return {
    device: Identity({
      id: deviceId,
      name: "sample-device",
      os: "sample",
    }),
    version: "sample",
    agents,
    recent,
  }
}
