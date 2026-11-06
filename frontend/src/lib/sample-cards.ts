// The built-in cards shown when the page runs in a plain browser (bun run
// dev), next to the sample roster in sample.ts. Every value is made up; the
// cards copy the shape of the Irori Cards prototype. Add ?empty to the URL
// for the empty state.

import {
  AgentRef,
  CardActivity,
  CardAsk,
  CardBlocker,
  CardBoard,
  CardDetail,
  CardHolder,
  CardOption,
  CardPR,
  CardRef,
  CardResolution,
  CardRound,
  CardSubtask,
  CardThreadItem,
  CardWhere,
  CardLeftOff,
  CardWait,
  Holding,
} from "./models"
import { LinkKind, Step, TaskState } from "./models"

const dev = "sample-device"

// NoteItem mirrors the Go constant: an input_required card's note, shown
// as its one open item.
export const NoteItem = "note"

function agent(session: string, name: string, status: string): AgentRef {
  return AgentRef({
    key: `${dev}/${session}`,
    name,
    status,
    live: status !== "gone",
    // This device by its own name, and here (card #489).
    where: "Macbook Pro (this one)",
    here: true,
    deviceId: dev,
    color: "teal",
  })
}

export const sampleAgents = {
  otter: agent("sample01-synthetic-session", "sample-otter", "busy"),
  heron: agent("sample02-synthetic-session", "sample-heron", "idle"),
  badger: agent("sample03-synthetic-session", "sample-badger", "busy"),
  // sample-mink ended its turn with a background job still running
  // (Claude Code's "shell", card #431). Not in the sample roster, so the
  // card keeps this status.
  mink: agent("sample05-synthetic-session", "sample-mink", "shell"),
  // sample-wren is on the sample's paired Desktop Computer, which is offline, so its
  // face shows the device chip greyed.
  wren: AgentRef({
    ...agent("sample04-synthetic-session", "sample-wren", "idle"),
    key: "sample-macbook/sample04-synthetic-session",
    where: "Desktop Computer",
    here: false,
    device: "Desktop Computer",
    deviceId: "sample-macbook",
    color: "orange",
    offline: true,
  }),
  // sample-lark is in sample-talk.ts's gone list, so its face offers Revive.
  lark: agent("sample99-synthetic-session", "sample-lark", "gone"),
}

export const sampleYou = AgentRef({
  key: `user:${dev}`,
  name: "You",
  status: "person",
  live: true,
  where: "Macbook Pro (this one)",
  here: true,
  deviceId: dev,
  color: "teal",
})

// sampleHolder mirrors cards_service.go's holderLine: the holder that took
// the card up, else the first assigned one.
// sampleAccent mirrors cards_service.go: the holders' devices, and their
// colour when they are all on one device (card #22).
function sampleAccent(working: AgentRef[]): {
  devices: string[]
  accent: string
} {
  const devices = [
    ...new Set(working.map((w) => w.deviceId).filter(Boolean)),
  ] as string[]
  const one =
    devices.length === 1
      ? working.find((w) => w.deviceId === devices[0])
      : undefined
  return { devices, accent: one?.color ?? "" }
}

export function sampleHolder(working: AgentRef[]): CardHolder {
  if (!working.length) return CardHolder({ state: Holding.HoldNobody })
  const who = working.find((w) => !w.assigned) ?? working[0]
  const state =
    who.status === "person"
      ? Holding.HoldPerson
      : who.waiting
        ? Holding.HoldWaiting
        : who.assigned
          ? Holding.HoldAssigned
          : who.status === "gone"
            ? Holding.HoldGone
            : who.status === "unresponsive"
              ? Holding.HoldSilent
              : who.offline
                ? Holding.HoldOffline
                : who.status === "idle"
                  ? Holding.HoldIdle
                  : Holding.HoldActive
  return CardHolder({ who, state, others: working.length - 1 })
}

type StepSeed = [text: string, done: boolean, doing?: boolean, owner?: string]

function steps(seeds: StepSeed[]): Step[] {
  return seeds.map(([text, done, doing, owner], i) =>
    Step({
      id: `s${i + 1}`,
      text,
      done,
      doing: doing || undefined,
      owner: owner || undefined,
    })
  )
}

// eventsOf is a thread made of a card's Recent lines (newest first), for
// the sample cards that seed no thread of their own. A line that quotes a
// blocker's question carries that blocker, as the Rust thread does (card
// #490), so it links to it.
function eventsOf(
  recent: CardActivity[],
  blockers: CardBlocker[] = []
): CardThreadItem[] {
  return [...recent].reverse().map((a, i) => {
    const b = blockers.find((b) => a.text.includes(`“${b.question}”`))
    return CardThreadItem({
      id: `sample-event-${i}`,
      kind: "event",
      at: a.at,
      who: a.who,
      text: a.text,
      blockerId: b?.id,
      blockerGist: b?.question,
    })
  })
}

export function sampleCards(now: Date): CardDetail[] {
  if (new URLSearchParams(window.location.search).has("empty")) return []
  const at = (min: number) =>
    new Date(now.getTime() - min * 60000).toISOString()
  const { otter, heron, badger, wren, lark, mink } = sampleAgents
  const card = (c: Partial<CardDetail>): CardDetail => {
    const st = c.steps ?? []
    const open = (c.blockers ?? []).filter((b) => !b.resolved)
    // As cardSummary: an answered item waits on an agent, not the person,
    // and a wait on other work (card #481) or another member's question
    // is a wait.
    const asked = open.filter((b) => b.state !== "answered")
    const needs = asked.filter((b) => !b.waitsOn && !b.decider)
    const wait = asked.find((b) => b.waitsOn)
    return CardDetail({
      scope: "mine",
      scopeLabel: "My devices",
      state: open.length ? TaskState.Blocked : TaskState.Working,
      doneWhen: "",
      note: "",
      links: [],
      comments: [],
      prs: [],
      watching: [],
      short: (c.id ?? "").replace("sample-card-", ""),
      ...c,
      thread: c.thread ?? eventsOf(c.recent ?? [], c.blockers ?? []),
      steps: st,
      blockers: c.blockers ?? [],
      // A parent's bar counts its subtasks, as cardSummary does.
      progress: c.subtasks
        ? (c.progress ?? { done: 0, total: 0 })
        : { done: st.filter((s) => s.done).length, total: st.length },
      holder: sampleHolder(c.working ?? []),
      ...sampleAccent(c.working ?? []),
      talking: c.talking ?? [],
      openBlockers: needs.length,
      waiting: open.length - asked.length,
      waitingOn: c.waitingOn ?? (asked.length - needs.length || undefined),
      waitOn: c.waitOn ?? wait?.waitsOn?.label,
      waitQuestion: c.waitQuestion ?? wait?.question,
      waitTask: c.waitTask ?? wait?.waitsOn?.taskId,
      blockerQuestion: needs[0]?.question ?? "",
      ready: needs.some((b) => b.doneCheck) || undefined,
    })
  }

  // Card #222: a long "waiting on" reason and a dozen watchers, which
  // must stay inside the card's edge in the list, the canvas and a
  // parent's subtask rows.
  const crowd = [
    "sample-alder",
    "sample-birch",
    "sample-cedar",
    "sample-dace",
    "sample-eider",
    "sample-fern",
    "sample-gull",
    "sample-hazel",
    "sample-ibis",
    "sample-jay",
    "sample-kite",
    "sample-larch",
  ].map((n, i) => agent(`sample${30 + i}-synthetic-session`, n, "idle"))
  const longWait = card({
    id: "sample-card-long",
    short: "51",
    title: "App sea chart: real Great Sea map with players placed to scale",
    goal: "Each player at their place on the real chart.",
    state: TaskState.Blocked,
    working: [otter],
    watching: crowd,
    waitingOn: 1,
    waitOn:
      "external ww/dolphin-slot-1..4 leases (reed until about 21:30Z, walnut and oak until about 22:20Z); queued after pine",
    waitQuestion:
      "Waiting for one free ww/dolphin-slot lease for the live check (all four are held)",
    updatedAt: at(2),
    steps: steps([
      ["Extractor", true],
      ["Frontend", true],
      ["Live check", false, true],
    ]),
  })

  // Card #305: held, its holder idle half an hour with nothing named
  // (Home: Idle, no reason given), and the same wait named as a blocker on
  // another agent (Home: Waiting on sample-kestrel).
  const idleHolder = AgentRef({
    ...heron,
    idleSince: at(30),
  })
  const idleUnnamed = card({
    id: "sample-card-idle",
    short: "52",
    title: "Tabs, command palette and settings",
    goal: "Port Go suzuri's tabs, palette and settings.",
    working: [idleHolder],
    updatedAt: at(30),
    steps: steps([
      ["Tabs", true],
      ["Palette", true],
      ["Check on the PC", false],
    ]),
  })
  const namedWait = card({
    id: "sample-card-named",
    short: "53",
    title: "Themes and fonts",
    goal: "Go suzuri's 16 themes and font choices.",
    working: [idleHolder],
    waitingOn: 1,
    waitOn: "agent sample-kestrel",
    waitQuestion: "The PC check of themes and fonts",
    updatedAt: at(30),
    steps: steps([
      ["Themes", true],
      ["Check on the PC", false, true],
    ]),
  })

  // Card #431: held, its holder between turns with a background job
  // running (Home: "sample-mink running a job").
  const runningJob = card({
    id: "sample-card-job",
    short: "54",
    title: "Clear the old build caches",
    goal: "Free disk on the Mac: delete stale target/ dirs.",
    working: [AgentRef({ ...mink, idleSince: at(30) })],
    updatedAt: at(30),
    steps: steps([
      ["List the stale dirs", true],
      ["Delete them", false, true],
    ]),
  })

  // Card #436: ?red=1 shows main red for one test, with two cards that
  // hit it waiting on its card (PREVIEW.md).
  const red = new URLSearchParams(window.location.search).has("red")
    ? [
        card({
          id: "sample-card-red",
          short: "60",
          title: "attach_exe fails on clean main",
          goal: "attach_exe fails on clean main at 3573cf11, first reported from #61.",
          state: TaskState.Submitted,
          working: [],
          mainRed: "attach_exe",
          updatedAt: at(2),
        }),
        ...["61", "62"].map((n) =>
          card({
            id: `sample-card-red-wait-${n}`,
            short: n,
            title:
              n === "61" ? "Talk 3: agents answer now" : "Merge desk slice 2",
            goal: "Waiting on main going green.",
            state: TaskState.Blocked,
            working: [idleHolder],
            waitingOn: 1,
            waitOn: "#60",
            waitTask: "sample-card-red",
            waitQuestion: "attach_exe fails on clean main",
            updatedAt: at(2),
          })
        ),
      ]
    : []

  // Card #481: ?loop=1 shows three cards that wait on each other, as
  // #392, #394 and #396 did: each wait a grey link with nothing to
  // answer, one Needs you item for the loop, and #72's own question
  // beside its wait (PREVIEW.md).
  const waitOn = (id: string, short: string, title: string, q: string) =>
    CardBlocker({
      id: "b1",
      question: q,
      raisedBy: heron,
      at: at(90),
      resolved: null,
      state: "open",
      waitsOn: { kind: "task", label: `#${short}`, taskId: id, title },
    })
  const ring = [
    { id: "sample-card-loop-70", short: "70", tag: "", title: "Talk 4: Spaces" },
    { id: "sample-card-loop-72", short: "72", tag: "", title: "Talk 8: release Talk" },
    {
      id: "sample-card-loop-71",
      short: "71",
      tag: "",
      title: "Talk 6: person-to-person DMs",
    },
  ]
  const loop = new URLSearchParams(window.location.search).has("loop")
    ? [
        card({
          id: "sample-card-loop-70",
          short: "70",
          title: "Talk 4: Spaces",
          goal: "The space log replaces the per-board log.",
          state: TaskState.Blocked,
          working: [],
          waitLoop: ring,
          blockers: [
            waitOn(
              "sample-card-loop-72",
              "72",
              "Talk 8: release Talk",
              "s6 live check waits on the Talk release (#72)"
            ),
          ],
          updatedAt: at(80),
        }),
        card({
          id: "sample-card-loop-71",
          short: "71",
          title: "Talk 6: person-to-person DMs",
          goal: "DMs over a hidden two-person space.",
          state: TaskState.Blocked,
          working: [],
          waitLoop: ring,
          blockers: [
            waitOn(
              "sample-card-loop-70",
              "70",
              "Talk 4: Spaces",
              "Waits on Talk 4 (#70): the hidden two-person space is a space log"
            ),
          ],
          updatedAt: at(85),
        }),
        card({
          id: "sample-card-loop-72",
          short: "72",
          title: "Talk 8: release Talk",
          goal: "Cut v0.12.0 once Talk 1-7 are done.",
          state: TaskState.Blocked,
          working: [],
          waitLoop: ring,
          blockers: [
            CardBlocker({
              id: "b2",
              question:
                "Ship this release without person-to-person DMs (#71), or wait for it?",
              raisedBy: heron,
              at: at(60),
              resolved: null,
              state: "open",
              options: [
                CardOption({
                  answer: "Ship without #71; DMs go in a point release",
                  by: heron,
                  at: at(60),
                  recommended: true,
                }),
                CardOption({ answer: "Wait for #71", by: heron, at: at(60) }),
              ],
            }),
            {
              ...waitOn(
                "sample-card-loop-71",
                "71",
                "Talk 6: person-to-person DMs",
                "Release waits on Talk 6 (#71) landing on main"
              ),
              id: "b1",
            },
          ],
          updatedAt: at(60),
        }),
      ]
    : []

  // Card #448: ?spawn=states shows the spawned-agent card lines: one
  // agent working a card Irori started it for (End on its page, "for #64"
  // in its hover), one starting, one waiting on the cap, one that can't
  // start (PREVIEW.md).
  const spawn =
    new URLSearchParams(window.location.search).get("spawn") === "states"
      ? [
          card({
            id: "sample-card-spawn-on",
            short: "64",
            title: "Retry the upload on a dropped connection",
            goal: "An agent Irori started for this card is on it.",
            working: [AgentRef({ ...heron, spawned: true, status: "busy" })],
            updatedAt: at(3),
          }),
          card({
            id: "sample-card-spawn-starting",
            short: "65",
            title: "Explain the cache eviction in the README",
            goal: "Starting an agent.",
            state: TaskState.Submitted,
            working: [],
            starting: "Macbook Pro (this one)",
            updatedAt: at(1),
          }),
          card({
            id: "sample-card-spawn-cap",
            short: "66",
            title: "Bump the icon set",
            goal: "Waiting for a slot.",
            state: TaskState.Submitted,
            working: [],
            atCap: "Macbook Pro (this one)",
            updatedAt: at(1),
          }),
          // Card #456 (sa-7): Irori's own decisions on spawned cards.
          // thyme stopped on a permission prompt nobody is attached to;
          // #62's agents crashed twice, so Irori stopped starting them.
          card({
            id: "sample-card-spawn-perm",
            short: "58",
            title: "Lantern flicker on the dock",
            goal: "One writer for the lantern's alpha.",
            working: [
              AgentRef({
                ...agent(
                  "sample06-synthetic-session",
                  "sample-thyme",
                  "waiting"
                ),
                spawned: true,
              }),
            ],
            blockers: [
              CardBlocker({
                id: "b-perm",
                question:
                  "Waiting for your permission: sample-thyme is stopped on a prompt",
                context:
                  "sample-thyme, the agent Irori started for #58, is waiting for a permission nobody is there to give. Attach to it to answer the prompt, or add the command to the board's allow list in Settings › Agents and start a fresh agent.",
                raisedBy: AgentRef({
                  key: "irori",
                  name: "Irori",
                  status: "person",
                }),
                at: at(2),
                options: [],
                resolved: null,
                state: "open",
              }),
            ],
            updatedAt: at(2),
          }),
          card({
            id: "sample-card-spawn-crash",
            short: "62",
            title: "Fishing rod v2",
            goal: "Cast, bite, reel.",
            working: [],
            blockers: [
              CardBlocker({
                id: "b-crash",
                question: "Its agent crashed twice in a row: start another?",
                context:
                  "The last two agents started for this card went away before finishing. Irori won't start another until you say so: start one, or say in your own words what to try, and it goes in the next agent's brief.",
                raisedBy: AgentRef({
                  key: "irori",
                  name: "Irori",
                  status: "person",
                }),
                at: at(9),
                options: [
                  CardOption({
                    answer: "Start another agent",
                    why: "A fresh agent picks it up from Where and Left off.",
                    by: AgentRef({
                      key: "irori",
                      name: "Irori",
                      status: "person",
                    }),
                    at: at(9),
                    recommended: true,
                  }),
                ],
                resolved: null,
                state: "open",
              }),
            ],
            updatedAt: at(9),
          }),
          card({
            id: "sample-card-spawn-cant",
            short: "67",
            title: "Port the build script to Windows",
            goal: "No checkout here.",
            state: TaskState.Submitted,
            working: [],
            cantStart: "no copy of app-repo on Sample Mac",
            updatedAt: at(1),
          }),
        ]
      : []

  // Card #485: ?gone=1 shows items whose agent is gone on cards nobody
  // holds: #73 waits on kestrel (gone), an amber line with Hand to an
  // agent; #74's done check was raised by piranha (gone), and its Not
  // done goes to the agent the card goes to next (PREVIEW.md). Card #520:
  // #73 and #75 (an outside wait piranha raised) are Gone rows in Home's
  // Needs you; #74 is a done check there already.
  const kestrel = agent("sample-kestrel-session", "kestrel", "gone")
  const piranha = agent("sample-piranha-session", "piranha", "gone")
  const gone = new URLSearchParams(window.location.search).has("gone")
    ? [
        card({
          id: "sample-card-gone-73",
          short: "73",
          title: "Spawned agents part 1: the spawner",
          goal: "A first spawn with no claude daemon opens no window on the PC.",
          state: TaskState.Blocked,
          working: [],
          blockers: [
            CardBlocker({
              id: "b6",
              question: "Waiting on kestrel's no-daemon GUI spawn on the PC",
              context:
                "Every other part 1 check has passed on the Mac and the PC. Only one is left: a first spawn that also starts the claude daemon must open no window.",
              raisedBy: heron,
              at: at(600),
              resolved: null,
              state: "open",
              waitsOn: { kind: "agent", label: "kestrel", gone: true },
              goneAgent: kestrel,
            }),
          ],
          waitingOn: 1,
          waitKind: "agent",
          waitOn: "kestrel",
          waitGone: true,
          waitQuestion: "Waiting on kestrel's no-daemon GUI spawn on the PC",
          goneWait: {
            blockerId: "b6",
            agent: kestrel,
            waitedOn: true,
            question: "Waiting on kestrel's no-daemon GUI spawn on the PC",
          },
          updatedAt: at(600),
        }),
        card({
          id: "sample-card-gone-75",
          short: "75",
          title: "Release 0.13 to the testers",
          goal: "The 0.13 build reaches the testers' devices.",
          state: TaskState.Blocked,
          working: [],
          blockers: [
            CardBlocker({
              id: "b1",
              question: "Waiting on the notarization of the 0.13 build",
              raisedBy: piranha,
              at: at(400),
              resolved: null,
              state: "open",
              waitsOn: { kind: "external", label: "Apple notarization" },
              goneAgent: piranha,
            }),
          ],
          waitingOn: 1,
          waitKind: "external",
          waitOn: "Apple notarization",
          waitQuestion: "Waiting on the notarization of the 0.13 build",
          goneWait: {
            blockerId: "b1",
            agent: piranha,
            on: "Apple notarization",
            question: "Waiting on the notarization of the 0.13 build",
          },
          updatedAt: at(400),
        }),
        card({
          id: "sample-card-gone-74",
          short: "74",
          title: "Spawned agents part 9: terminal agents",
          goal: "Terminal sessions aren't free agents unless offered cards.",
          state: TaskState.Blocked,
          working: [],
          blockers: [
            CardBlocker({
              id: "b2",
              question: "Is this done?",
              context:
                "On main at 8845c11a. A terminal agent is no longer a free agent unless you turn on Offer it cards.",
              raisedBy: piranha,
              at: at(500),
              doneCheck: true,
              options: [
                CardOption({
                  answer: "Done",
                  why: "Closes the card.",
                  by: piranha,
                  at: at(500),
                  recommended: true,
                }),
              ],
              resolved: null,
              state: "open",
              goneAgent: piranha,
            }),
          ],
          updatedAt: at(500),
        }),
      ]
    : []

  return [
    ...spawn,
    ...red,
    ...loop,
    ...gone,
    longWait,
    idleUnnamed,
    namedWait,
    runningJob,
    // Asks (board.md "Asks"): the person's prompts to the room. One the
    // agents heard and nobody took yet, one sample-heron took and is
    // talking through.
    // In review (card #191): the work is done and waits on its PR's review.
    card({
      id: "sample-card-review",
      short: "16",
      title: "Mac: convert-in-Dolphin steps for RVZ discs",
      goal: "Give Mac players the convert steps, since Dolphin for macOS ships no DolphinTool.",
      state: TaskState.InReview,
      working: [otter],
      updatedAt: at(3),
      // Card #471: it waits on its PR, which agents merge; a small chip
      // on the card links it, and nothing goes to Needs you.
      waitPr: {
        repo: "sample-org/example-app",
        number: 212,
        url: "https://github.com/sample-org/example-app/pull/212",
      },
    }),
    // Card #509: an older wait on a PR's merge is the person's merge, a
    // Needs you row that opens the PR.
    card({
      id: "sample-card-merge",
      short: "18",
      title: "Asks and Talk threads get their own agent",
      goal: "Each ask and Talk thread gets its own background agent that goes when the thread closes.",
      state: TaskState.Blocked,
      working: [],
      waitingOn: 1,
      waitKind: "external",
      waitOn: "https://github.com/sample-org/example-app/pull/41",
      waitQuestion: "Review and merge PR #41 so it lands on main",
      updatedAt: at(2),
      mergePr: {
        repo: "sample-org/example-app",
        number: 41,
        url: "https://github.com/sample-org/example-app/pull/41",
      },
    }),
    // A wait on that card waits on its PR, not on the person (card #471).
    card({
      id: "sample-card-after-review",
      short: "17",
      title: "Mac: link the convert steps from the install page",
      goal: "Once the convert steps are in, point the Mac install page at them.",
      state: TaskState.Blocked,
      working: [idleHolder],
      waitingOn: 1,
      waitOn: "#16",
      waitQuestion: "After the convert steps land",
      updatedAt: at(4),
    }),
    card({
      id: "sample-ask-1",
      short: "14",
      title: "Why does the Mac build take twice as long as the PC's?",
      goal: "Why does the Mac build take twice as long as the PC's?\nIt was about the same last week. Not urgent to fix, but I want to know what changed.",
      state: TaskState.Submitted,
      working: [],
      updatedAt: at(1),
      thread: [
        CardThreadItem({
          id: "sample-ask-1-post",
          kind: "event",
          at: at(1),
          who: sampleYou,
          text: "You asked the room",
        }),
      ],
      ask: CardAsk({
        state: "open",
        told: 3,
        owner: "Macbook Pro (this one)",
        wake: false,
        taker: null,
        outcome: null,
      }),
    }),
    card({
      id: "sample-ask-2",
      short: "15",
      title: "Can someone tidy the release notes before tonight?",
      goal: "Can someone tidy the release notes before tonight? The boat fixes should come first.",
      state: TaskState.Submitted,
      working: [heron],
      updatedAt: at(4),
      thread: [
        CardThreadItem({
          id: "sample-ask-2-post",
          kind: "event",
          at: at(6),
          who: sampleYou,
          text: "You asked the room",
        }),
        CardThreadItem({
          id: "sample-ask-2-took",
          kind: "event",
          at: at(5),
          who: heron,
          text: "sample-heron took it",
        }),
        CardThreadItem({
          id: "sample-ask-2-c1",
          kind: "comment",
          at: at(4),
          who: heron,
          text: "On it. Do you want the Helmaroc fight in the notes too, or only what changed since 0.5?",
        }),
      ],
      ask: CardAsk({
        state: "taken",
        told: 3,
        owner: "Macbook Pro (this one)",
        wake: false,
        taker: heron,
        outcome: null,
      }),
    }),
    card({
      id: "sample-card-1",
      title: "Ship example-app v0.6.0",
      repoKey: "remote:github.com/sample-org/example-app",
      side: "Work",
      goal: "Tagged release with final notes and an installer on the release page.",
      working: [otter],
      watching: [heron],
      // Waits (card #87): wren is woken when this card is done; otter, on
      // it, waits for main to move.
      waits: [
        CardWait({
          id: "sample-wait-1",
          who: wren,
          text: "until done",
          onThis: true,
        }),
        CardWait({
          id: "sample-wait-2",
          who: otter,
          text: "branch main moves",
        }),
      ],
      updatedAt: at(18),
      steps: steps([
        ["Boat batch merged (#206, #207)", true],
        ["Log-flood fix merged (#209)", true],
        ["Release notes up to main", true],
        ["Two-PC co-op check in the temple level", false, true],
        ["Tag v0.6.0 and publish", false],
      ]),
      blockers: [
        CardBlocker({
          id: "b1",
          question: "Ship today, or wait for the two-PC temple check?",
          context:
            "Every automated check passes. The co-op check needs both PCs, with the game closed on the second one; it was last blocked 20 minutes ago because another game was running. Shipping now means the release notes say temple co-op is untested.",
          raisedBy: otter,
          at: at(18),
          // Card #491: the person clicks an option, so each one says who
          // acts ("sample-otter ships…", "I'll…" = the person) or the
          // outcome, never the agent's "I".
          options: [
            CardOption({
              answer: "Wait for the check; sample-otter ships tonight",
              why: "The temple is the release's headline feature; an untested headline is worse than a short delay.",
              by: otter,
              at: at(18),
              recommended: true,
            }),
            CardOption({
              answer: "I'll close the game on the second PC now",
              why: "The check runs as soon as it's closed, and sample-otter ships when it passes.",
              by: otter,
              at: at(18),
            }),
            CardOption({
              answer: "Ship now, with temple co-op marked experimental",
              why: "Players get the boat and log fixes today; a 0.6.1 can follow the check.",
              by: heron,
              at: at(15),
            }),
          ],
          resolved: null,
          state: "open",
        }),
        // Rich: something for the person to do elsewhere, with the page to
        // open, the command to copy and a file to look at.
        CardBlocker({
          id: "b5",
          question: "Request access to the gated DINOv3 weights?",
          context:
            "The probe needs the ViT-L weights, which Hugging Face gates per account. Two things: 1) open huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m and click Request access 2) type `! hf auth login` in the Mac Claude session and paste a read token.",
          raisedBy: otter,
          at: at(12),
          options: [
            CardOption({
              answer: "I requested it and logged in",
              why: "Approval is usually instant; check it under https://huggingface.co/settings/gated-repos.",
              by: otter,
              at: at(12),
              recommended: true,
              link: {
                kind: LinkKind.LinkURL,
                ref: "https://huggingface.co/settings/tokens",
                label: "huggingface.co",
              },
            }),
            CardOption({
              answer: "Use DINOv2 instead",
              why: "Ungated, but the probe scores about 3 points lower.",
              by: otter,
              at: at(12),
            }),
          ],
          links: [
            {
              kind: LinkKind.LinkURL,
              ref: "https://huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m",
              label: "huggingface.co",
            },
            {
              kind: LinkKind.LinkPath,
              ref: "docs/models.md",
              label: "docs/models.md",
            },
          ],
          attachments: [
            {
              sha256: "1".repeat(64),
              name: "gate-page.png",
              size: 48213,
              mime: "image/png",
              device: dev,
            },
          ],
          resolved: null,
          state: "open",
          missing: [],
          thread: [],
        }),
        // Replied: the person answered, the agent asked back with what
        // the answer still misses, so it is back with the person.
        CardBlocker({
          id: "b2",
          question: "What goes in the release notes' known-issues list?",
          context:
            "Two issues are open against main: the boat sometimes clips the dock, and the log viewer stutters past 50k lines.",
          raisedBy: otter,
          at: at(45),
          options: [],
          resolved: null,
          state: "replied",
          missing: [
            "Whether the dock clipping counts as known or fixed",
            "A workaround line for the log viewer",
          ],
          thread: [
            CardThreadItem({
              id: "sample-1-b2-a1",
              kind: "comment",
              at: at(38),
              who: sampleYou,
              to: otter,
              text: "Both, with a workaround each.",
              blockerId: "b2",
            }),
            CardThreadItem({
              id: "sample-1-b2-r1",
              kind: "comment",
              at: at(30),
              who: otter,
              text: "I can list the log viewer, but I don't have a workaround for it, and the dock fix (#207) may already cover the clipping.",
              blockerId: "b2",
              missing: [
                "Whether the dock clipping counts as known or fixed",
                "A workaround line for the log viewer",
              ],
            }),
          ],
        }),
        // Resolved by the agent with its note; the person's answer stands.
        CardBlocker({
          id: "b3",
          question: "Tag from main, or from the release branch?",
          context: "",
          raisedBy: otter,
          at: at(90),
          options: [],
          state: "resolved",
          resolved: CardResolution({
            answer: "From main; we don't keep release branches.",
            option: -1,
            note: "Tagging from main d48bc2f once the co-op check passes.",
            by: otter,
            at: at(70),
            closed: false,
          }),
          thread: [
            CardThreadItem({
              id: "sample-1-b3-a1",
              kind: "comment",
              at: at(80),
              who: sampleYou,
              to: otter,
              text: "From main; we don't keep release branches.",
              blockerId: "b3",
            }),
          ],
        }),
        // Closed without an answer.
        CardBlocker({
          id: "b4",
          question: "Waiting on the CI runner lease",
          context: "",
          raisedBy: heron,
          at: at(120),
          options: [],
          state: "closed",
          resolved: CardResolution({
            answer: "the lease was released",
            option: -1,
            by: AgentRef({
              key: "irori",
              name: "Irori",
              status: "irori",
              live: true,
              where: "Macbook Pro (this one)",
              here: true,
            }),
            at: at(110),
            closed: true,
          }),
        }),
      ],
      prs: [
        CardPR({
          repo: "sample-org/example-app",
          number: 208,
          url: "https://github.com/sample-org/example-app/pull/208",
          label: "GitHub #208",
          openedBy: [otter],
          reviewedBy: [heron],
          touchedBy: [],
        }),
      ],
      recent: [
        CardActivity({
          at: at(18),
          who: otter,
          text: "sample-otter raised a blocker",
        }),
        CardActivity({
          at: at(30),
          who: otter,
          text: "sample-otter replied on “What goes in the release notes' known-issues list?”",
        }),
        CardActivity({
          at: at(38),
          who: sampleYou,
          text: "You answered “What goes in the release notes' known-issues list?”",
        }),
        CardActivity({
          at: at(40),
          who: otter,
          text: "sample-otter ticked “Release notes up to main”",
        }),
        CardActivity({
          at: at(55),
          who: heron,
          text: "sample-heron started watching",
        }),
      ],
    }),
    card({
      id: "sample-card-2",
      title: "Replace the third-party VPN with a built-in LAN link",
      repoKey: "remote:github.com/sample-org/example-net",
      side: "Work",
      goal: "Two users on different networks can play LAN games together without the third-party VPN.",
      working: [badger],
      watching: [wren, lark],
      updatedAt: at(4),
      steps: steps([
        ["Research what the VPN does", true],
        ["LAN build with a virtual adapter", false, true],
        ["Two-machine test", false],
        ["Settings and docs", false],
      ]),
      blockers: [
        CardBlocker({
          id: "b1",
          question: "Which virtual adapter should the LAN build install?",
          context:
            "The VPN's source isn't public, so the app needs its own adapter. Wintun is signed and used by WireGuard, but it only carries IP, so games that find each other by LAN broadcast may not see each other. TAP-Windows carries broadcasts but is older, and installing it needs one admin (UAC) prompt.",
          raisedBy: badger,
          at: at(9),
          options: [
            CardOption({
              answer: "Wintun, with the app relaying LAN broadcasts itself",
              why: "No old driver, one modern signed one; the broadcast relay is the extra work.",
              by: badger,
              at: at(9),
              recommended: true,
            }),
            CardOption({
              answer: "TAP-Windows",
              why: "Games just work, at the cost of an older driver and a UAC prompt on install.",
              by: wren,
              at: at(7),
            }),
          ],
          resolved: null,
          // Answered: the person's question back went to the raiser, so
          // it waits on sample-badger (a grey "Waiting on agent" card).
          // Their own words set the options aside (card #90).
          state: "answered",
          sentTo: badger,
          optionsAside: true,
          thread: [
            CardThreadItem({
              id: "sample-2-b1-a1",
              kind: "comment",
              at: at(4),
              who: sampleYou,
              to: badger,
              text: "How much work is the broadcast relay? If it's under a day, Wintun.",
              blockerId: "b1",
            }),
          ],
        }),
      ],
      recent: [
        CardActivity({
          at: at(4),
          who: sampleYou,
          text: "You answered “Which virtual adapter should the LAN build install?”",
        }),
        CardActivity({
          at: at(9),
          who: badger,
          text: "sample-badger raised a blocker",
        }),
        CardActivity({
          at: at(30),
          who: wren,
          text: "sample-wren started watching",
        }),
      ],
    }),
    // A Work card made on a device that runs only Personal (card #495): it
    // waits, and is never handed to another device.
    card({
      id: "sample-card-side-wait",
      title: "Rotate the staging API keys",
      repoKey: "remote:github.com/sample-org/example-app",
      side: "Work",
      sideWait: "no device here runs Work",
      goal: "New keys in the vault, old ones revoked.",
      updatedAt: at(2),
    }),
    card({
      id: "sample-card-3",
      title: "Two-PC lobby join for the multiplayer build",
      repoKey: "remote:github.com/sample-org/example-game",
      side: "Personal",
      goal: "Two machines join the same lobby and see each other's animals.",
      working: [heron, wren],
      updatedAt: at(6),
      // input_required: the note is its question (BoardService.Card's
      // NoteItem), answered in the person's own words.
      state: TaskState.InputRequired,
      note: "Should the lobby list come from Steam, or from our own relay so it works without Steam?",
      blockers: [
        CardBlocker({
          id: NoteItem,
          question:
            "Should the lobby list come from Steam, or from our own relay so it works without Steam?",
          context: "",
          raisedBy: heron,
          at: at(6),
          options: [],
          state: "open",
        }),
      ],
      steps: steps([
        ["Lobby create and join", true],
        ["Animal sync", false, true],
        ["Two-PC run", false],
      ]),
      prs: [
        CardPR({
          repo: "sample-org/example-game",
          number: 88,
          url: "https://github.com/sample-org/example-game/pull/88",
          label: "GitHub #88",
          openedBy: [heron],
          reviewedBy: [],
          touchedBy: [wren],
        }),
      ],
      // The thread: what happened, the person's and agents' comments, and
      // messages about the card between agents, one from the paired
      // Desktop Computer, one whose recipient had ended and one the policy held.
      // This card opens first, so a plain-browser run shows them.
      thread: [
        CardThreadItem({
          id: "sample-3-e1",
          kind: "event",
          at: at(70),
          who: heron,
          text: "sample-heron took the card",
        }),
        CardThreadItem({
          id: "sample-3-e2",
          kind: "event",
          at: at(64),
          who: wren,
          device: "Desktop Computer",
          text: "sample-wren joined",
        }),
        CardThreadItem({
          id: "sample-3-c1",
          kind: "comment",
          at: at(50),
          who: sampleYou,
          text: "Test the join with the Desktop Computer as the second machine.",
          pending: ["sample-badger read it"],
        }),
        CardThreadItem({
          id: "msg_sample1",
          kind: "message",
          at: at(34),
          who: wren,
          to: heron,
          device: "Desktop Computer",
          text: "Lobby join works from the Desktop Computer build.\nAnimals spawn, but their positions lag by about a second; the capture is attached.",
          files: 1,
        }),
        CardThreadItem({
          id: "sample-3-e3",
          kind: "event",
          at: at(25),
          who: heron,
          text: "sample-heron ticked “Lobby create and join”",
        }),
        CardThreadItem({
          id: "msg_sample2",
          kind: "message",
          at: at(20),
          who: heron,
          to: lark,
          text: "Did your relay spike keep lobby lists working without Steam? If so I'll reuse it.",
          gone: true,
        }),
        CardThreadItem({
          id: "sample-3-c2",
          kind: "comment",
          at: at(14),
          who: wren,
          device: "Desktop Computer",
          text: "The lag goes away with a 20 Hz sync tick. Pushed to the PR.",
        }),
        // The person asks a busy agent (card #159): it hears it when its
        // current turn ends, and the thread says so under the comment.
        CardThreadItem({
          id: "sample-3-c3",
          kind: "comment",
          at: at(12),
          who: sampleYou,
          text: "@sample-heron does the 20 Hz tick hold on the Windows build too?",
          pending: [
            "sample-heron is busy: it hears this when its current turn ends",
          ],
        }),
        CardThreadItem({
          id: "msg_sample3",
          kind: "message",
          at: at(10),
          who: badger,
          to: heron,
          text: "",
          held: true,
        }),
        // A held message the person dismissed: Release still delivers it
        // (sample-talk's dismissed delivery).
        CardThreadItem({
          id: "msg_sample3d",
          kind: "message",
          at: at(9),
          who: badger,
          to: heron,
          text: "",
          held: true,
          dismissed: true,
          deliveryId: "sample-delivery-8",
        }),
        CardThreadItem({
          id: "sample-3-e4",
          kind: "event",
          at: at(6),
          who: wren,
          device: "Desktop Computer",
          text: "sample-wren linked GitHub #88",
        }),
        CardThreadItem({
          id: "sample-3-e5",
          kind: "event",
          at: at(6),
          who: heron,
          text: "sample-heron set the card to needs your input",
        }),
      ],
      recent: [
        CardActivity({
          at: at(6),
          who: wren,
          text: "sample-wren linked GitHub #88",
        }),
        CardActivity({
          at: at(25),
          who: heron,
          text: "sample-heron ticked “Lobby create and join”",
        }),
      ],
    }),
    card({
      // No steps, and its assignment hasn't reached the agent: the tile
      // says "Waiting for", not in progress.
      id: "sample-card-6",
      title: "Back up the out/ folder before the re-run",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "A copy of out/ somewhere safe, so the re-run can overwrite it.",
      working: [
        AgentRef({
          ...heron,
          assigned: true,
          waiting: true,
          handoff:
            "out/ is about 40 GB: copy it to the external drive, then start the re-run.",
        }),
      ],
      updatedAt: at(2),
      recent: [
        CardActivity({
          at: at(2),
          who: otter,
          text: "sample-otter assigned it to sample-heron: out/ is about 40 GB: copy it to the external drive, then start the re-run.",
        }),
      ],
    }),
    card({
      // An agent says it's done (card #112): it waits on your Done, or
      // your word on why it isn't.
      id: "sample-card-12",
      title: "Fix the boat clipping through the dock",
      repoKey: "remote:github.com/sample-org/example-game",
      goal: "Boats stop at the dock's edge instead of sliding into it.",
      working: [otter],
      updatedAt: at(1),
      steps: steps([
        ["Reproduce at the temple dock", true],
        ["Widen the dock's collider", true],
        ["Two-PC check", true],
      ]),
      blockers: [
        CardBlocker({
          id: "b1",
          question: "Is this done?",
          // Its proof (card #140): what changed, how it was checked, the
          // commit, caveats, and a screenshot.
          context:
            "Widened the dock collider by 0.4 m, so boats stop at the edge.\nChecked: cargo test, then 20 dockings in the two-PC check with no clipping (screenshot after the fix).\nCommit 4f2c9e1 on main (#207).\nCaveat: the far dock on the island wasn't tested.",
          attachments: [
            {
              sha256: "2".repeat(64),
              name: "dock-after.png",
              size: 51877,
              mime: "image/png",
              device: dev,
            },
          ],
          raisedBy: otter,
          at: at(1),
          doneCheck: true,
          options: [
            CardOption({
              answer: "Done",
              why: "Closes the card.",
              by: otter,
              at: at(1),
              recommended: true,
            }),
          ],
          resolved: null,
          state: "open",
          thread: [],
        }),
      ],
      recent: [
        CardActivity({
          at: at(1),
          who: otter,
          text: "sample-otter raised a blocker",
        }),
      ],
    }),
    // Card #476: a done check you sent back, and the one the agent raised
    // after it. The new one opens with that round: your reason, the
    // agent's answer, and the proof it replaces.
    (() => {
      const reason = CardThreadItem({
        id: "sample-round-reason",
        kind: "comment",
        at: at(42),
        who: sampleYou,
        to: heron,
        text: "Not done: the others haven't been told the next steps.",
        blockerId: "b1",
        heardBy: [heron.key],
      })
      const answer = CardThreadItem({
        id: "sample-round-answer",
        kind: "comment",
        at: at(30),
        who: heron,
        text: "Right: I posted the next steps in #game-team, and the prod check is its own card now, #108, so it doesn't rely on someone remembering.",
        blockerId: "b1",
      })
      const oldProof =
        "PR #2225 merged as 5d804f5; the feed skips legacy campaigns by default; a test in the PR covers it."
      return card({
        id: "sample-card-31",
        title: "Turn off the legacy feed for migrated clients",
        repoKey: "remote:github.com/sample-org/example-game",
        goal: "Migrated clients' legacy campaigns stop showing in the delivery feed.",
        working: [heron],
        updatedAt: at(6),
        steps: steps([
          ["Skip legacy campaigns", true],
          ["PR, review, merge", true],
          ["Tell the team the next steps", true],
        ]),
        blockers: [
          CardBlocker({
            id: "b2",
            question: "Is this done?",
            context:
              "PR #2225 merged as 5d804f5; the feed skips legacy campaigns by default; a test in the PR covers it.\nSince you sent it back: the next steps are posted in #game-team, and the prod deploy check is its own card, #108.",
            raisedBy: heron,
            at: at(6),
            doneCheck: true,
            options: [
              CardOption({
                answer: "Done",
                why: "Closes the card.",
                by: heron,
                at: at(6),
                recommended: true,
              }),
            ],
            resolved: null,
            state: "open",
            thread: [],
            previous: CardRound({
              id: "b1",
              round: 2,
              proof: oldProof,
              thread: [reason, answer],
            }),
          }),
          CardBlocker({
            id: "b1",
            question: "Is this done?",
            context: oldProof,
            raisedBy: heron,
            at: at(50),
            doneCheck: true,
            options: [
              CardOption({
                answer: "Done",
                why: "Closes the card.",
                by: heron,
                at: at(50),
                recommended: true,
              }),
            ],
            resolved: CardResolution({
              answer: reason.text,
              option: -1,
              by: heron,
              at: at(6),
              note: "asked again: ready for the person",
            }),
            state: "resolved",
            thread: [reason, answer],
          }),
        ],
      })
    })(),
    // Card #476: you sent a done check back; the agent read it but hasn't
    // answered yet.
    card({
      id: "sample-card-32",
      title: "Pin the save format before the beta",
      repoKey: "remote:github.com/sample-org/example-game",
      goal: "Old saves load after the format change.",
      working: [otter],
      updatedAt: at(3),
      blockers: [
        CardBlocker({
          id: "b1",
          question: "Is this done?",
          context: "Save format v3 with a migration from v2; cargo test green.",
          raisedBy: otter,
          at: at(20),
          doneCheck: true,
          options: [
            CardOption({
              answer: "Done",
              why: "Closes the card.",
              by: otter,
              at: at(20),
              recommended: true,
            }),
          ],
          resolved: null,
          state: "answered",
          sentTo: otter,
          optionsAside: true,
          thread: [
            CardThreadItem({
              id: "sample-sent-back",
              kind: "comment",
              at: at(3),
              who: sampleYou,
              to: otter,
              text: "Not done: a v1 save from the demo still crashes on load.",
              blockerId: "b1",
              heardBy: [otter.key],
            }),
          ],
        }),
      ],
    }),
    card({
      id: "sample-card-4",
      // Where its work is and where it left off (card #137).
      where: CardWhere({
        who: otter,
        worktree: "/Users/sample/projects/example-app-pair",
        branch: "feat/pair-devices",
        head: "4f1c2ab9e07d6c5b3a29f8e1d0c7b6a594837261",
        pushed: true,
        at: at(3),
      }),
      leftOff: CardLeftOff({
        text: "Pairing code and words work on both sides; next: the PC's firewall prompt",
        by: otter,
        at: at(6),
      }),
      title: "Pair my devices",
      // Pinned (board.md "Pinned cards"): in the Pinned group, first.
      pinned: true,
      pinRank: 1,
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "My laptop and desktop share one roster, cards and wakes.",
      // sample-heron owns the Sync step, so it's on the card until that
      // step is done (card #86).
      working: [otter, AgentRef({ ...heron, steps: ["s3"] })],
      watching: [badger],
      // sample-otter and sample-wren (on the Desktop Computer) are messaging each
      // other about it: the tile's talk bubble (card #22).
      talking: [otter, wren],
      updatedAt: at(3),
      // Busy: sample-otter is working it right now, so the tile, the page
      // and the current step show a spinner.
      doneWhen:
        "Both devices list each other's agents, and a card made on one shows on the other.",
      steps: steps([
        ["Design", true],
        ["Identity and pairing", false, true],
        ["Sync", false, false, heron.key],
        ["Cross-device wakes and files", false],
      ]),
      links: [
        {
          kind: LinkKind.LinkPath,
          ref: "docs/design/pairing.md",
          label: "docs/design/pairing.md",
        },
        {
          kind: LinkKind.LinkURL,
          ref: "https://example.com/sample-org/pairing-notes",
          label: "Pairing notes",
        },
      ],
      recent: [
        CardActivity({
          at: at(3),
          who: otter,
          text: "sample-otter ticked “Design”",
        }),
      ],
      thread: [
        CardThreadItem({
          id: "sample-4-event-1",
          kind: "event",
          at: at(40),
          who: otter,
          text: "sample-otter set 4 steps",
        }),
        CardThreadItem({
          id: "sample-4-comment-1",
          kind: "comment",
          at: at(12),
          who: otter,
          text: "The pairing code screen, as it stands. The code expires after two minutes; the other device shows the same six digits.",
          attachments: [
            {
              sha256: "4".repeat(64),
              name: "pairing-code.png",
              size: 61440,
              mime: "image/png",
              device: dev,
            },
          ],
        }),
        CardThreadItem({
          id: "sample-4-event-2",
          kind: "event",
          at: at(3),
          who: otter,
          text: "sample-otter ticked “Design”",
        }),
      ],
    }),
    // A card split into subtasks (board.md "Subtasks"): the parent's bar
    // counts them, one is blocked, and each follows its parent in the grid.
    card({
      id: "sample-card-7",
      title: "Ship the release",
      pinned: true,
      pinRank: 2,
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Version 0.6 out: notes, the build and the announcement.",
      working: [otter],
      updatedAt: at(5),
      state: TaskState.Blocked,
      steps: steps([["Pick what goes in", true]]),
      subtasks: 2,
      progress: { done: 0, total: 2 },
      subtaskBlocked: "#9: Announce on Discord as well, or only the site?",
      subtaskList: [
        CardSubtask({
          id: "sample-card-8",
          short: "8",
          title: "Release notes",
          state: TaskState.Working,
          holder: sampleHolder([heron]),
          progress: { done: 1, total: 2 },
          blocked: false,
        }),
        CardSubtask({
          id: "sample-card-9",
          short: "9",
          title: "Announcement",
          state: TaskState.Blocked,
          holder: sampleHolder([wren]),
          progress: { done: 0, total: 1 },
          blocked: true,
        }),
      ],
    }),
    card({
      id: "sample-card-8",
      title: "Release notes",
      repoKey: "remote:github.com/sample-org/example-app",
      working: [heron],
      updatedAt: at(7),
      parent: CardRef({
        id: "sample-card-7",
        short: "7",
        title: "Ship the release",
      }),
      steps: steps([
        ["Draft", true],
        ["Boat fixes first", false, true],
      ]),
    }),
    card({
      id: "sample-card-9",
      title: "Announcement",
      repoKey: "remote:github.com/sample-org/example-app",
      working: [wren],
      updatedAt: at(8),
      parent: CardRef({
        id: "sample-card-7",
        short: "7",
        title: "Ship the release",
      }),
      steps: steps([["Post it", false]]),
      blockers: [
        CardBlocker({
          id: "b1",
          question: "Announce on Discord as well, or only the site?",
          context: "",
          raisedBy: wren,
          at: at(8),
          options: [],
          state: "open",
          missing: [],
          thread: [],
          links: [],
          attachments: [],
        }),
      ],
    }),
    // A bigger family (card #113's layout mockups): five subtasks, two of
    // them done, one blocked, one nobody has picked up.
    card({
      id: "sample-card-20",
      title: "Boat physics pass",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Boats float, turn and dock the same on both PCs.",
      working: [otter],
      updatedAt: at(3),
      subtasks: 5,
      progress: { done: 2, total: 5 },
      subtaskBlocked: "#24: Clamp the roll at 30°, or let big waves flip it?",
    }),
    card({
      id: "sample-card-23",
      title: "Buoyancy from the hull shape",
      repoKey: "remote:github.com/sample-org/example-app",
      working: [otter],
      updatedAt: at(3),
      parent: CardRef({
        id: "sample-card-20",
        short: "20",
        title: "Boat physics pass",
      }),
      steps: steps([
        ["Sample points", true],
        ["Water height", true],
        ["Torque", false, true],
      ]),
    }),
    card({
      id: "sample-card-24",
      title: "Roll limits in rough water",
      repoKey: "remote:github.com/sample-org/example-app",
      // lark raised the blocker and has since finished: nobody is on it.
      working: [],
      updatedAt: at(9),
      // Its agent finished and it waits on you: your comment offers
      // "Start an agent with this" (card #452).
      thread: [
        CardThreadItem({
          id: "sample-24-c1",
          kind: "comment",
          at: at(8),
          who: sampleYou,
          text: "Clamp at 30° for now; big waves can flip it once the sails are in.",
        }),
      ],
      parent: CardRef({
        id: "sample-card-20",
        short: "20",
        title: "Boat physics pass",
      }),
      steps: steps([
        ["Wave test scene", true],
        ["Pick a limit", false],
      ]),
      blockers: [
        CardBlocker({
          id: "b1",
          question: "Clamp the roll at 30°, or let big waves flip it?",
          context: "",
          raisedBy: lark,
          at: at(9),
          options: [],
          state: "open",
          missing: [],
          thread: [],
          links: [],
          attachments: [],
        }),
      ],
    }),
    card({
      id: "sample-card-25",
      title: "Docking snaps to the pier",
      repoKey: "remote:github.com/sample-org/example-app",
      working: [],
      updatedAt: at(20),
      state: TaskState.Submitted,
      parent: CardRef({
        id: "sample-card-20",
        short: "20",
        title: "Boat physics pass",
      }),
    }),
    card({
      id: "sample-card-5",
      title: "Sample notes export",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Notes export to Markdown from the share menu.",
      state: TaskState.Completed,
      working: [heron],
      updatedAt: at(140),
      steps: steps([
        ["Export format", true],
        ["Share menu item", true],
        ["Docs", true],
      ]),
      prs: [
        CardPR({
          repo: "sample-org/example-app",
          number: 41,
          url: "https://github.com/sample-org/example-app/pull/41",
          label: "GitHub #41",
          openedBy: [heron],
          reviewedBy: [otter],
          touchedBy: [],
        }),
      ],
      recent: [
        CardActivity({
          at: at(140),
          who: heron,
          text: "sample-heron marked the card done",
        }),
      ],
    }),
    // Shared boards (v0.5 §8): two cards on the sample board MLG (lib/
    // boards.ts), shown "mlg#1" and "mlg#2", and a private card ready to
    // be shared, with a path in its goal for the share preview to drop.
    card({
      id: "sample-board-1",
      short: "1",
      title: "Lobby sync for the boat race",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Both players see the same boats when the race starts.",
      scope: "board",
      scopeLabel: "MLG",
      board: "sample-board-mlg",
      tag: "mlg",
      onBoard: CardBoard({
        id: "sample-board-mlg",
        name: "MLG",
        tag: "mlg",
        visibleTo: ["Stephen", "Blake"],
        canWithdraw: true,
        canTransfer: true,
      }),
      working: [otter],
      // sample-otter is one of your agents: Home shows it (card #199);
      // mlg#2 below, nobody's, stays on MLG's page alone.
      ours: true,
      owner: "sample-me",
      updatedAt: at(7),
      steps: steps([
        ["Mirror the boat actors", true],
        ["Settle the start timer", false],
      ]),
      recent: [
        CardActivity({
          at: at(7),
          who: otter,
          text: "sample-otter ticked “Mirror the boat actors”",
        }),
      ],
    }),
    card({
      id: "sample-board-2",
      short: "1",
      title: "Pick the race map for the demo",
      goal: "One map everyone tests on this week.",
      scope: "board",
      scopeLabel: "MLG · Netcode",
      board: "sample-board-mlg",
      tag: "net",
      onBoard: CardBoard({
        id: "sample-board-mlg",
        name: "MLG",
        tag: "mlg",
        visibleTo: ["Stephen", "Blake"],
        canWithdraw: true,
        canTransfer: true,
      }),
      state: TaskState.Submitted,
      working: [],
      updatedAt: at(40),
      // Blake made it (card #268).
      owner: "sample-person-0",
    }),
    // Blake's agent kite works Blake's card (card #268): the owner chip says
    // Blake.
    card({
      id: "sample-board-3",
      short: "3",
      title: "Tune the boost pads on the harbour track",
      goal: "Boost pads feel the same on both clients.",
      scope: "board",
      scopeLabel: "MLG",
      board: "sample-board-mlg",
      tag: "mlg",
      onBoard: CardBoard({
        id: "sample-board-mlg",
        name: "MLG",
        tag: "mlg",
        visibleTo: ["Stephen", "Blake"],
        canWithdraw: false,
        canTransfer: true,
      }),
      working: [
        AgentRef({
          key: "agent:sample-kite-card",
          name: "kite",
          status: "busy",
          live: true,
          where: "Blake's agent",
          // Blake's colour on shared boards (card #385).
          color: "indigo",
        }),
      ],
      owner: "sample-person-0",
      updatedAt: at(3),
      steps: steps([
        ["Measure the pad impulse on both", true],
        ["Match the pad timings", false],
      ]),
    }),
    // A card from someone who has since left the board: "Former member".
    card({
      id: "sample-board-4",
      short: "4",
      title: "Old lap-time leaderboard idea",
      goal: "A leaderboard of best laps per track.",
      scope: "board",
      scopeLabel: "MLG",
      board: "sample-board-mlg",
      tag: "mlg",
      onBoard: CardBoard({
        id: "sample-board-mlg",
        name: "MLG",
        tag: "mlg",
        visibleTo: ["Stephen", "Blake"],
        canWithdraw: false,
        canTransfer: true,
      }),
      state: TaskState.Submitted,
      working: [],
      owner: "sample-person-left",
      updatedAt: at(300),
    }),
    // An agent Irori couldn't start: its repo isn't trusted for agents on
    // this computer (card #449), so Needs you shows Trust and start.
    card({
      id: "sample-card-trust",
      short: "19",
      title: "Add rate limiting to the upload endpoint",
      repoKey: "remote:github.com/sample-org/example-api",
      goal: "Uploads over 20 a minute from one key get a 429.",
      state: TaskState.Submitted,
      working: [],
      cantStart: "not trusted: /Users/you/projects/example-api on this-mac",
      updatedAt: at(2),
    }),
    card({
      id: "sample-card-10",
      title: "Write up the boat desync repro",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Steps anyone can follow; the capture is in C:\\Users\\pat\\captures\\desync-03.",
      working: [wren],
      updatedAt: at(12),
      steps: steps([
        ["Record the two clients", true],
        ["Note the frame it splits on", false],
      ]),
    }),
    // More closed cards for the Done tab (?filter=done): an ask that
    // changed another card, one canceled and one failed.
    card({
      id: "sample-ask-3",
      short: "13",
      title: "Can the pairing code stay up longer than two minutes?",
      goal: "Can the pairing code stay up longer than two minutes? I keep missing it.",
      state: TaskState.Completed,
      working: [],
      updatedAt: at(55),
      thread: [
        CardThreadItem({
          id: "sample-ask-3-post",
          kind: "event",
          at: at(70),
          who: sampleYou,
          text: "You asked the room",
        }),
        CardThreadItem({
          id: "sample-ask-3-took",
          kind: "event",
          at: at(66),
          who: otter,
          text: "sample-otter took it",
        }),
      ],
      ask: CardAsk({
        state: "closed",
        told: 3,
        owner: "Macbook Pro (this one)",
        wake: false,
        taker: otter,
        outcome: {
          kind: "edit",
          task: "sample-card-4",
          note: "added a step for a five-minute code",
        },
        outcomeRef: "#4",
      }),
    }),
    card({
      id: "sample-card-7",
      title: "Move the release notes to the wiki",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Release notes live on the wiki instead of in the repo.",
      state: TaskState.Canceled,
      working: [],
      updatedAt: at(300),
      recent: [
        CardActivity({
          at: at(300),
          who: sampleYou,
          text: "You set it to canceled",
        }),
      ],
    }),
    card({
      id: "sample-card-8",
      title: "Nightly build on the old Mac mini",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "A nightly build runs on the Mac mini and posts its log.",
      state: TaskState.Failed,
      working: [badger],
      updatedAt: at(60 * 26),
      steps: steps([
        ["Install the toolchain", true],
        ["Schedule the build", false],
      ]),
      recent: [
        CardActivity({
          at: at(60 * 26),
          who: badger,
          text: "sample-badger set it to failed",
        }),
      ],
    }),
  ].map(
    (c) =>
      ({
        ...c,
        // As BoardService.Card: every link, PRs too.
        links: [
          ...c.links,
          ...c.prs.map((p) => ({
            kind: LinkKind.LinkURL,
            ref: p.url,
            label: p.label,
          })),
        ],
      }) as CardDetail
  )
}
