// The built-in cards shown when the page runs in a plain browser (bun run
// dev), next to the sample roster in sample.ts. Every value is made up; the
// cards copy the shape of the Irori Cards prototype. Add ?empty to the URL
// for the empty state.

import {
  AgentRef,
  CardActivity,
  CardBlocker,
  CardDetail,
  CardOption,
  CardPR,
  CardResolution,
  CardThreadItem,
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
      ...c,
      thread: c.thread ?? eventsOf(c.recent ?? []),
      steps: st,
      blockers: c.blockers ?? [],
      progress: { done: st.filter((s) => s.done).length, total: st.length },
      openBlockers: needs.length,
      waiting: open.length - needs.length,
      blockerQuestion: needs[0]?.question ?? "",
    })
  }

  return [
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
      id: "sample-card-4",
      title: "Pair my devices",
      repoKey: "remote:github.com/sample-org/example-app",
      goal: "My laptop and desktop share one roster, cards and wakes.",
      working: [otter],
      watching: [badger],
      updatedAt: at(3),
      steps: steps([
        ["Design", false, true],
        ["Identity and pairing", false],
        ["Sync", false],
        ["Cross-device wakes and files", false],
      ]),
      recent: [
        new CardActivity({
          at: at(3),
          who: otter,
          text: "sample-otter set 4 steps",
        }),
      ],
    }),
  ].map(
    (c) =>
      ({
        ...c,
        links: c.prs.map((p) => ({
          kind: LinkKind.LinkURL,
          ref: p.url,
          label: p.label,
        })),
      }) as CardDetail
  )
}
