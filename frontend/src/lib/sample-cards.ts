// The built-in cards shown when the page runs in a plain browser (bun run
// dev), next to the sample roster in sample.ts. Every value is made up; the
// cards copy the shape of the Irori Cards prototype. Add ?empty to the URL
// for the empty state.

import {
  AgentRef,
  CardActivity,
  CardAsk,
  CardBlocker,
  CardDetail,
  CardHolder,
  CardOption,
  CardPR,
  CardRef,
  CardResolution,
  CardSubtask,
  CardThreadItem,
  Holding,
} from "../../bindings/github.com/StephenSHorton/irori/models"
import {
  LinkKind,
  Step,
  TaskState,
} from "../../bindings/github.com/StephenSHorton/irori/internal/coord/models"

const dev = "sample-device"

// NoteItem mirrors the Go constant: an input_required card's note, shown
// as its one open item.
export const NoteItem = "note"

function agent(session: string, name: string, status: string): AgentRef {
  return new AgentRef({
    key: `${dev}/${session}`,
    name,
    status,
    live: status !== "gone",
    where: "This PC",
    deviceId: dev,
    color: "teal",
  })
}

export const sampleAgents = {
  otter: agent("sample01-synthetic-session", "sample-otter", "busy"),
  heron: agent("sample02-synthetic-session", "sample-heron", "idle"),
  badger: agent("sample03-synthetic-session", "sample-badger", "busy"),
  // sample-wren is on the sample's paired MacBook, which is offline, so its
  // face shows the device chip greyed.
  wren: new AgentRef({
    ...agent("sample04-synthetic-session", "sample-wren", "idle"),
    key: "sample-macbook/sample04-synthetic-session",
    where: "MacBook",
    device: "MacBook",
    deviceId: "sample-macbook",
    color: "orange",
    offline: true,
  }),
  // sample-lark is in sample-talk.ts's gone list, so its face offers Revive.
  lark: agent("sample99-synthetic-session", "sample-lark", "gone"),
}

export const sampleYou = new AgentRef({
  key: `user:${dev}`,
  name: "You",
  status: "person",
  live: true,
  where: "This PC",
})

// sampleHolder mirrors cards_service.go's holderLine: the holder that took
// the card up, else the first assigned one.
// sampleAccent mirrors cards_service.go: the holders' devices, and their
// colour when they are all on one device (card #22).
function sampleAccent(working: AgentRef[]): { devices: string[]; accent: string } {
  const devices = [...new Set(working.map((w) => w.deviceId).filter(Boolean))] as string[]
  const one = devices.length === 1 ? working.find((w) => w.deviceId === devices[0]) : undefined
  return { devices, accent: one?.color ?? "" }
}

export function sampleHolder(working: AgentRef[]): CardHolder {
  if (!working.length) return new CardHolder({ state: Holding.HoldNobody })
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
            : who.offline
              ? Holding.HoldOffline
              : who.status === "idle"
                ? Holding.HoldIdle
                : Holding.HoldActive
  return new CardHolder({ who, state, others: working.length - 1 })
}

type StepSeed = [text: string, done: boolean, doing?: boolean]

function steps(seeds: StepSeed[]): Step[] {
  return seeds.map(
    ([text, done, doing], i) =>
      new Step({ id: `s${i + 1}`, text, done, doing: doing || undefined })
  )
}

// eventsOf is a thread made of a card's Recent lines (newest first), for
// the sample cards that seed no thread of their own.
function eventsOf(recent: CardActivity[]): CardThreadItem[] {
  return [...recent].reverse().map(
    (a, i) =>
      new CardThreadItem({
        id: `sample-event-${i}`,
        kind: "event",
        at: a.at,
        who: a.who,
        text: a.text,
      })
  )
}

export function sampleCards(now: Date): CardDetail[] {
  if (new URLSearchParams(window.location.search).has("empty")) return []
  const at = (min: number) =>
    new Date(now.getTime() - min * 60000).toISOString()
  const { otter, heron, badger, wren, lark } = sampleAgents
  const card = (c: Partial<CardDetail>): CardDetail => {
    const st = c.steps ?? []
    const open = (c.blockers ?? []).filter((b) => !b.resolved)
    // As cardSummary: an answered item waits on an agent, not the person.
    const needs = open.filter((b) => b.state !== "answered")
    return new CardDetail({
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
      thread: c.thread ?? eventsOf(c.recent ?? []),
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
      waiting: open.length - needs.length,
      blockerQuestion: needs[0]?.question ?? "",
    })
  }

  return [
    // Asks (board.md "Asks"): the person's prompts to the room. One the
    // agents heard and nobody took yet, one sample-heron took and is
    // talking through.
    card({
      id: "sample-ask-1",
      short: "14",
      title: "Why does the Mac build take twice as long as the PC's?",
      goal: "Why does the Mac build take twice as long as the PC's?\nIt was about the same last week. Not urgent to fix, but I want to know what changed.",
      state: TaskState.Submitted,
      working: [],
      updatedAt: at(1),
      thread: [
        new CardThreadItem({ id: "sample-ask-1-post", kind: "event", at: at(1), who: sampleYou, text: "You asked the room" }),
      ],
      ask: new CardAsk({ state: "open", told: 3, owner: "This PC", wake: false, taker: null, outcome: null }),
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
        new CardThreadItem({ id: "sample-ask-2-post", kind: "event", at: at(6), who: sampleYou, text: "You asked the room" }),
        new CardThreadItem({ id: "sample-ask-2-took", kind: "event", at: at(5), who: heron, text: "sample-heron took it" }),
        new CardThreadItem({
          id: "sample-ask-2-c1",
          kind: "comment",
          at: at(4),
          who: heron,
          text: "On it. Do you want the Helmaroc fight in the notes too, or only what changed since 0.5?",
        }),
      ],
      ask: new CardAsk({ state: "taken", told: 3, owner: "This PC", wake: false, taker: heron, outcome: null }),
    }),
    card({
      id: "sample-card-1",
      title: "Ship example-app v0.6.0",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "Tagged release with final notes and an installer on the release page.",
      working: [otter],
      watching: [heron],
      updatedAt: at(18),
      steps: steps([
        ["Boat batch merged (#206, #207)", true],
        ["Log-flood fix merged (#209)", true],
        ["Release notes up to main", true],
        ["Two-PC co-op check in the temple level", false, true],
        ["Tag v0.6.0 and publish", false],
      ]),
      blockers: [
        new CardBlocker({
          id: "b1",
          question: "Ship today, or wait for the two-PC temple check?",
          context:
            "Every automated check passes on main d48bc2f. The co-op check needs both PCs, with the game closed on the second one; it was last blocked 20 minutes ago because another game was running. Shipping now means the release notes say temple co-op is untested.",
          raisedBy: otter,
          at: at(18),
          options: [
            new CardOption({
              answer: "Wait for the check, ship tonight",
              why: "The temple is the release's headline feature; an untested headline is worse than a short delay.",
              by: otter,
              at: at(18),
              recommended: true,
            }),
            new CardOption({
              answer: "Ship now, mark temple co-op as experimental",
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
        new CardBlocker({
          id: "b5",
          question: "Request access to the gated DINOv3 weights?",
          context:
            "The probe needs the ViT-L weights, which Hugging Face gates per account. Two things: 1) open huggingface.co/facebook/dinov3-vitl16-pretrain-lvd1689m and click Request access 2) type `! hf auth login` in the Mac Claude session and paste a read token.",
          raisedBy: otter,
          at: at(12),
          options: [
            new CardOption({
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
            new CardOption({
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
            { kind: LinkKind.LinkPath, ref: "docs/models.md", label: "docs/models.md" },
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
        new CardBlocker({
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
            new CardThreadItem({
              id: "sample-1-b2-a1",
              kind: "comment",
              at: at(38),
              who: sampleYou,
              to: otter,
              text: "Both, with a workaround each.",
              blockerId: "b2",
            }),
            new CardThreadItem({
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
        new CardBlocker({
          id: "b3",
          question: "Tag from main, or from the release branch?",
          context: "",
          raisedBy: otter,
          at: at(90),
          options: [],
          state: "resolved",
          resolved: new CardResolution({
            answer: "From main; we don't keep release branches.",
            option: -1,
            note: "Tagging from main d48bc2f once the co-op check passes.",
            by: otter,
            at: at(70),
            closed: false,
          }),
          thread: [
            new CardThreadItem({
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
        new CardBlocker({
          id: "b4",
          question: "Waiting on the CI runner lease",
          context: "",
          raisedBy: heron,
          at: at(120),
          options: [],
          state: "closed",
          resolved: new CardResolution({
            answer: "the lease was released",
            option: -1,
            by: new AgentRef({
              key: "irori",
              name: "Irori",
              status: "irori",
              live: true,
              where: "This PC",
            }),
            at: at(110),
            closed: true,
          }),
        }),
      ],
      prs: [
        new CardPR({
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
        new CardActivity({
          at: at(18),
          who: otter,
          text: "sample-otter raised a blocker",
        }),
        new CardActivity({
          at: at(30),
          who: otter,
          text: "sample-otter replied on blocker b2",
        }),
        new CardActivity({
          at: at(38),
          who: sampleYou,
          text: "You answered blocker b2",
        }),
        new CardActivity({
          at: at(40),
          who: otter,
          text: "sample-otter ticked “Release notes up to main”",
        }),
        new CardActivity({
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
        new CardBlocker({
          id: "b1",
          question: "Which virtual adapter should the LAN build install?",
          context:
            "The VPN's source isn't public, so the app needs its own adapter. Wintun is signed and used by WireGuard, but it only carries IP, so games that find each other by LAN broadcast may not see each other. TAP-Windows carries broadcasts but is older, and installing it needs one admin (UAC) prompt.",
          raisedBy: badger,
          at: at(9),
          options: [
            new CardOption({
              answer: "Wintun, with the app relaying LAN broadcasts itself",
              why: "No old driver, one modern signed one; the broadcast relay is the extra work.",
              by: badger,
              at: at(9),
              recommended: true,
            }),
            new CardOption({
              answer: "TAP-Windows",
              why: "Games just work, at the cost of an older driver and a UAC prompt on install.",
              by: wren,
              at: at(7),
            }),
          ],
          resolved: null,
          // Answered: the person's question back went to the raiser, so
          // it waits on sample-badger (a grey "Waiting on agent" card).
          state: "answered",
          sentTo: badger,
          thread: [
            new CardThreadItem({
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
        new CardActivity({
          at: at(4),
          who: sampleYou,
          text: "You answered blocker b1",
        }),
        new CardActivity({
          at: at(9),
          who: badger,
          text: "sample-badger raised a blocker",
        }),
        new CardActivity({
          at: at(30),
          who: wren,
          text: "sample-wren started watching",
        }),
      ],
    }),
    card({
      id: "sample-card-3",
      title: "Two-PC lobby join for the multiplayer build",
      repoKey: "remote:github.com/sample-org/example-game",
      goal: "Two machines join the same lobby and see each other's animals.",
      working: [heron, wren],
      updatedAt: at(6),
      // input_required: the note is its question (BoardService.Card's
      // NoteItem), answered in the person's own words.
      state: TaskState.InputRequired,
      note: "Should the lobby list come from Steam, or from our own relay so it works without Steam?",
      blockers: [
        new CardBlocker({
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
        new CardPR({
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
      // MacBook, one whose recipient had ended and one the policy held.
      // This card opens first, so a plain-browser run shows them.
      thread: [
        new CardThreadItem({
          id: "sample-3-e1",
          kind: "event",
          at: at(70),
          who: heron,
          text: "sample-heron took the card",
        }),
        new CardThreadItem({
          id: "sample-3-e2",
          kind: "event",
          at: at(64),
          who: wren,
          device: "MacBook",
          text: "sample-wren joined",
        }),
        new CardThreadItem({
          id: "sample-3-c1",
          kind: "comment",
          at: at(50),
          who: sampleYou,
          text: "Test the join with the MacBook as the second machine.",
        }),
        new CardThreadItem({
          id: "msg_sample1",
          kind: "message",
          at: at(34),
          who: wren,
          to: heron,
          device: "MacBook",
          text: "Lobby join works from the MacBook build.\nAnimals spawn, but their positions lag by about a second; the capture is attached.",
          files: 1,
        }),
        new CardThreadItem({
          id: "sample-3-e3",
          kind: "event",
          at: at(25),
          who: heron,
          text: "sample-heron ticked “Lobby create and join”",
        }),
        new CardThreadItem({
          id: "msg_sample2",
          kind: "message",
          at: at(20),
          who: heron,
          to: lark,
          text: "Did your relay spike keep lobby lists working without Steam? If so I'll reuse it.",
          gone: true,
        }),
        new CardThreadItem({
          id: "sample-3-c2",
          kind: "comment",
          at: at(14),
          who: wren,
          device: "MacBook",
          text: "The lag goes away with a 20 Hz sync tick. Pushed to the PR.",
        }),
        new CardThreadItem({
          id: "msg_sample3",
          kind: "message",
          at: at(10),
          who: badger,
          to: heron,
          text: "",
          held: true,
        }),
        new CardThreadItem({
          id: "sample-3-e4",
          kind: "event",
          at: at(6),
          who: wren,
          device: "MacBook",
          text: "sample-wren linked GitHub #88",
        }),
        new CardThreadItem({
          id: "sample-3-e5",
          kind: "event",
          at: at(6),
          who: heron,
          text: "sample-heron set the card to needs your input",
        }),
      ],
      recent: [
        new CardActivity({
          at: at(6),
          who: wren,
          text: "sample-wren linked GitHub #88",
        }),
        new CardActivity({
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
      working: [new AgentRef({ ...heron, assigned: true, waiting: true })],
      updatedAt: at(2),
      recent: [
        new CardActivity({
          at: at(2),
          who: otter,
          text: "sample-otter assigned it to sample-heron",
        }),
      ],
    }),
    card({
      id: "sample-card-4",
      title: "Pair my devices",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "My laptop and desktop share one roster, cards and wakes.",
      working: [otter],
      watching: [badger],
      // sample-otter and sample-wren (on the MacBook) are messaging each
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
        ["Sync", false],
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
        new CardActivity({
          at: at(3),
          who: otter,
          text: "sample-otter ticked “Design”",
        }),
      ],
      thread: [
        new CardThreadItem({
          id: "sample-4-event-1",
          kind: "event",
          at: at(40),
          who: otter,
          text: "sample-otter set 4 steps",
        }),
        new CardThreadItem({
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
        new CardThreadItem({
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
        new CardSubtask({ id: "sample-card-8", short: "8", title: "Release notes", state: TaskState.Working,
          holder: sampleHolder([heron]), progress: { done: 1, total: 2 }, blocked: false }),
        new CardSubtask({ id: "sample-card-9", short: "9", title: "Announcement", state: TaskState.Blocked,
          holder: sampleHolder([wren]), progress: { done: 0, total: 1 }, blocked: true }),
      ],
    }),
    card({
      id: "sample-card-8",
      title: "Release notes",
      repoKey: "remote:github.com/sample-org/example-app",
      working: [heron],
      updatedAt: at(7),
      parent: new CardRef({ id: "sample-card-7", short: "7", title: "Ship the release" }),
      steps: steps([["Draft", true], ["Boat fixes first", false, true]]),
    }),
    card({
      id: "sample-card-9",
      title: "Announcement",
      repoKey: "remote:github.com/sample-org/example-app",
      working: [wren],
      updatedAt: at(8),
      parent: new CardRef({ id: "sample-card-7", short: "7", title: "Ship the release" }),
      steps: steps([["Post it", false]]),
      blockers: [
        new CardBlocker({
          id: "b1", question: "Announce on Discord as well, or only the site?", context: "", raisedBy: wren, at: at(8),
          options: [], state: "open", missing: [], thread: [], links: [], attachments: [],
        }),
      ],
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
        new CardPR({
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
        new CardActivity({
          at: at(140),
          who: heron,
          text: "sample-heron marked the card done",
        }),
      ],
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
        new CardThreadItem({ id: "sample-ask-3-post", kind: "event", at: at(70), who: sampleYou, text: "You asked the room" }),
        new CardThreadItem({ id: "sample-ask-3-took", kind: "event", at: at(66), who: otter, text: "sample-otter took it" }),
      ],
      ask: new CardAsk({
        state: "closed",
        told: 3,
        owner: "This PC",
        wake: false,
        taker: otter,
        outcome: { kind: "edit", task: "sample-card-4", note: "added a step for a five-minute code" },
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
        new CardActivity({ at: at(300), who: sampleYou, text: "You set it to canceled" }),
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
        new CardActivity({ at: at(60 * 26), who: badger, text: "sample-badger set it to failed" }),
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
