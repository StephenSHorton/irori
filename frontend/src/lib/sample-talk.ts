// The built-in inbox, tasks, leases and gone sessions shown when the page
// runs in a plain browser, next to the sample roster in sample.ts. Every
// value is made up and says so. Add ?empty to the URL for the empty states.

import { GoneSession } from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  Lease,
  Task,
  TaskState,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"
import {
  AddressKind,
  Delivery,
  HoldReason,
  Message,
  Mode,
  Policy,
  Sender,
  State,
  Trust,
} from "../../bindings/github.com/StephenSHorton/irori/internal/inbox/models"

const dev = "sample-device"
const otter = `${dev}/sample01-synthetic-session`
const heron = `${dev}/sample02-synthetic-session`
const badger = `${dev}/sample03-synthetic-session`
const gone = `${dev}/sample99-synthetic-session`
const grok = `${dev}/grok-sample7`
const repo = "github.com/sample-org/example-app"

export type SampleTalk = {
  deliveries: Delivery[]
  policy: Policy
  tasks: Task[]
  leases: Lease[]
  gone: GoneSession[]
}

type Seed = {
  from: Sender
  to: string
  repoKey?: string
  state: State
  hold?: HoldReason
  agoSec: number
  text: string
}

const sender = (key: string, name: string, kind = "claude") =>
  new Sender({ key, name, kind })

const you = sender(`user:${dev}`, "you", "user")

const seeds: Seed[] = [
  {
    from: sender(grok, "sample-grok", "grok"),
    to: otter,
    state: State.Held,
    hold: HoldReason.HoldBypassReceiver,
    agoSec: 40,
    text: "Sample text. Could you look at the sample parser's failing case?\nIt is the third one in the made-up suite.",
  },
  {
    from: sender(heron, "sample-heron"),
    to: badger,
    state: State.Answered,
    agoSec: 60 * 6,
    text: "Sample text. The example docs build passes on my side. <script>alert(1)</script> and **bold** stay literal.",
  },
  {
    from: sender(badger, "sample-badger"),
    to: heron,
    state: State.Engaged,
    agoSec: 60 * 9,
    text: "Sample text. Taking the placeholder review now.\n\n- first made-up point\n- second made-up point",
  },
  {
    from: you,
    to: "",
    repoKey: "root:5a3c1e0b9d7f2a4c6e8b0d1f3a5c7e9b1d3f5a7c",
    state: State.Held,
    hold: HoldReason.HoldUnowned,
    agoSec: 60 * 14,
    text: "Sample text. Whoever picks up the placeholder service next: the sample config moved.",
  },
  {
    from: sender(otter, "sample-otter"),
    to: heron,
    state: State.Queued,
    agoSec: 60 * 20,
    text: "Sample text. Lease on the sample parser is yours after this run.",
  },
  {
    from: sender(heron, "sample-heron"),
    to: otter,
    state: State.Held,
    hold: HoldReason.HoldPaused,
    agoSec: 60 * 31,
    text: "Sample text. Ping while wakes were paused.",
  },
]

// sampleTalk builds the synthetic inbox and work state relative to now.
export function sampleTalk(now: Date): SampleTalk {
  const at = (agoSec: number) =>
    new Date(now.getTime() - agoSec * 1000).toISOString()
  const empty = new URLSearchParams(window.location.search).has("empty")
  const policy = new Policy({
    mode: Mode.Everyone,
    block: [],
    allow: [],
    paused: false,
  })
  if (empty) return { deliveries: [], policy, tasks: [], leases: [], gone: [] }

  const deliveries = seeds.map((s, i) => {
    const id = `sample-delivery-${i + 1}`
    return new Delivery({
      id,
      messageId: `sample-message-${i + 1}`,
      to: s.to,
      repoKey: s.repoKey,
      state: s.state,
      hold: s.hold,
      via: s.state === State.Held ? undefined : "mod",
      reminded: false,
      createdAt: at(s.agoSec),
      updatedAt: at(Math.max(0, s.agoSec - 20)),
      message: new Message({
        id: `sample-message-${i + 1}`,
        from: s.from,
        to: s.repoKey
          ? { kind: AddressKind.ToRepo, value: s.repoKey }
          : { kind: AddressKind.ToAgent, value: s.to },
        text: s.text,
        hop: 0,
        trust: s.from.kind === "user" ? Trust.UserTrust : Trust.Untrusted,
        sentAt: at(s.agoSec),
      }),
    })
  })

  const tasks = [
    new Task({
      id: "sample-task-1",
      title: "Sample: fix the example parser's third case",
      objective: "Sample objective.",
      repoKey: repo,
      state: TaskState.Working,
      holder: otter,
      createdBy: heron,
      createdAt: at(3600),
      updatedAt: at(120),
    }),
    new Task({
      id: "sample-task-2",
      title: "Sample: review the placeholder docs",
      objective: "Sample objective.",
      repoKey: "root:5a3c1e0b9d7f2a4c6e8b0d1f3a5c7e9b1d3f5a7c",
      state: TaskState.Submitted,
      createdBy: you.key,
      createdAt: at(1800),
      updatedAt: at(1800),
    }),
    new Task({
      id: "sample-task-3",
      title: "Sample: rename the made-up module",
      objective: "Sample objective.",
      repoKey: repo,
      state: TaskState.Completed,
      holder: heron,
      createdBy: heron,
      createdAt: at(7200),
      updatedAt: at(5400),
    }),
  ]

  const until = (sec: number) =>
    new Date(now.getTime() + sec * 1000).toISOString()
  const leases = [
    new Lease({
      resource: `path:${repo}:src/parser/**`,
      holder: otter,
      until: until(60 * 24),
      acquiredAt: at(360),
    }),
    new Lease({
      resource: "port:8799",
      holder: heron,
      until: until(60 * 3 + 20),
      acquiredAt: at(60 * 27),
    }),
  ]

  const goneList = [
    new GoneSession({
      key: gone,
      sessionId: "sample99-synthetic-session",
      name: "sample-lark",
      title: "Sample: an earlier made-up session",
      cwd: "C:\\Users\\sample\\projects\\example-app",
      repoKey: repo,
      goneAt: at(60 * 20),
      reason: "gone",
      transcript: true,
      trusted: true,
    }),
  ]

  return { deliveries, policy, tasks, leases, gone: goneList }
}
