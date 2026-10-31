// The built-in inbox and gone sessions shown when the page
// runs in a plain browser, next to the sample roster in sample.ts. Every
// value is made up and says so. Add ?empty to the URL for the empty states.

import { GoneSession } from "./models"
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
} from "./models"

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
  // heldFor and decidedAgoSec: a held message the person already decided
  // (the Held list's History).
  heldFor?: HoldReason
  decidedAgoSec?: number
  ref?: string
}

const sender = (key: string, name: string, kind = "claude") =>
  Sender({ key, name, kind })

const you = sender(`user:${dev}`, "you", "user")

const seeds: Seed[] = [
  {
    from: sender(grok, "sample-grok", "grok"),
    to: otter,
    state: State.Held,
    hold: HoldReason.HoldNotAllowed,
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
    to: you.key,
    state: State.Queued,
    agoSec: 50,
    text: "Sample text. The lobby build is on the second PC and both machines see each other's lobby. Before I wire animal sync: the made-up save format keeps animals per player, so a joiner's animals would vanish when the host leaves.\n\nShould a joiner keep their own animals in their own save, or should the host's world own every animal while they're connected?",
  },
  {
    from: sender(heron, "sample-heron"),
    to: otter,
    state: State.Held,
    hold: HoldReason.HoldPaused,
    agoSec: 60 * 31,
    text: "Sample text. Ping while wakes were paused.",
  },
  {
    from: sender(grok, "sample-grok", "grok"),
    to: heron,
    state: State.Dismissed,
    hold: HoldReason.HoldNotAllowed,
    heldFor: HoldReason.HoldNotAllowed,
    agoSec: 60 * 45,
    decidedAgoSec: 60 * 40,
    ref: "task:sample-card-1",
    text: "Sample text. Run the made-up cleanup script on the sample repo.",
  },
  {
    from: sender(grok, "sample-grok", "grok"),
    to: otter,
    state: State.Engaged,
    heldFor: HoldReason.HoldNotAllowed,
    agoSec: 60 * 70,
    decidedAgoSec: 60 * 66,
    text: "Sample text. The sample fixture needs a new made-up case.",
  },
]

// sampleTalk builds the synthetic inbox relative to now.
export function sampleTalk(now: Date): SampleTalk {
  const at = (agoSec: number) =>
    new Date(now.getTime() - agoSec * 1000).toISOString()
  const empty = new URLSearchParams(window.location.search).has("empty")
  const policy = Policy({
    mode: Mode.Everyone,
    block: [],
    allow: [],
    paused: false,
  })
  if (empty) return { deliveries: [], policy, gone: [] }

  const deliveries = seeds.map((s, i) => {
    const id = `sample-delivery-${i + 1}`
    return Delivery({
      id,
      messageId: `sample-message-${i + 1}`,
      to: s.to,
      repoKey: s.repoKey,
      state: s.state,
      hold: s.hold,
      heldFor: s.heldFor ?? s.hold,
      decidedBy: s.decidedAgoSec !== undefined ? you.key : undefined,
      decidedAt:
        s.decidedAgoSec !== undefined ? at(s.decidedAgoSec) : undefined,
      via:
        s.state === State.Held || s.state === State.Dismissed
          ? undefined
          : "mod",
      reminded: false,
      createdAt: at(s.agoSec),
      updatedAt: at(Math.max(0, s.agoSec - 20)),
      message: Message({
        id: `sample-message-${i + 1}`,
        from: s.from,
        to: s.repoKey
          ? { kind: AddressKind.ToRepo, value: s.repoKey }
          : { kind: AddressKind.ToAgent, value: s.to },
        text: s.text,
        ref: s.ref,
        hop: 0,
        trust: s.from.kind === "user" ? Trust.UserTrust : Trust.Untrusted,
        sentAt: at(s.agoSec),
      }),
    })
  })

  const goneList = [
    GoneSession({
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

  return { deliveries, policy, gone: goneList }
}
