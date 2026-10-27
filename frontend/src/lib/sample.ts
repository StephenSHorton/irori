// The built-in roster shown when the page runs in a plain browser (bun run
// dev), so the UI can be built and screenshotted without the Go app. Every
// value is made up and says so; none of it comes from a real session.
// Add ?empty to the URL to preview the empty state instead.

import { Identity } from "../../bindings/github.com/StephenSHorton/irori/internal/device/models"
import { AgentCard } from "../../bindings/github.com/StephenSHorton/irori/internal/presence/models"
import { Event as LogEvent } from "../../bindings/github.com/StephenSHorton/irori/internal/store/models"

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
  },
  {
    session: "sample03-synthetic-session",
    name: "sample-badger",
    title: "Sample: draft the placeholder docs",
    cwd: "/home/sample/src/placeholder-service",
    status: "idle",
    kind: "interactive",
    entry: "desktop",
    agoSec: 60 * 42,
    text: "Sample text. The placeholder docs are drafted; waiting for a made-up review.",
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
          new AgentCard({
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
          })
      )

  const hlc = (agoSec: number) => at(agoSec).getTime() * 65536
  const rows: Array<[string, string, number]> = [
    ["device.started", "irori", 60 * 60 * 6],
    ...agents.map((a, i): [string, string, number] => [
      "agent.seen",
      `agent:${a.key}`,
      seeds[i].agoSec + 3600,
    ]),
    ["agent.status", `agent:${deviceId}/${seeds[0].session}`, 3],
    ["agent.status", `agent:${deviceId}/${seeds[1].session}`, 95],
    ["agent.gone", `agent:${deviceId}/sample99-synthetic-session`, 60 * 20],
  ]
  const recent = empty
    ? []
    : rows
        .map(
          ([kind, actor, ago], i) =>
            new LogEvent({
              deviceId,
              seq: i + 1,
              hlc: hlc(ago),
              actor,
              kind,
              payload: null,
            })
        )
        .sort((a, b) => b.hlc - a.hlc)

  return {
    device: new Identity({
      id: deviceId,
      name: "sample-device",
      os: "sample",
    }),
    version: "sample",
    agents,
    recent,
  }
}
