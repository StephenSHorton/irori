// The merge desks the browser preview shows (card #417): ww's stand-in
// game-repo with every state (desk-2), an app repo with no open PRs
// of ours (so it isn't listed, card #625), and a repo that takes no PRs. ?desk=unreachable makes the game-repo's
// poll fail and ?desk=truncated cuts its list short.

import type { Desk, SnapPr } from "./desk"

const GAME = "remote:github.com/you/game-repo"
const APP = "remote:github.com/you/app-repo"
const IRORI = "remote:github.com/you/irori"

function ago(now: Date, min: number): string {
  return new Date(now.getTime() - min * 60_000).toISOString()
}

export function sampleDesks(now: Date, flag: string | null): Desk[] {
  const pr = (
    n: number,
    title: string,
    author: string,
    opened: number,
    over: Partial<SnapPr> = {}
  ): SnapPr => ({
    number: n,
    url: `https://github.com/you/game-repo/pull/${n}`,
    title,
    author,
    headRefName: `pr-${n}`,
    headRefOid: `${n}f2a9c1e0b7d4`,
    baseRefName: "main",
    mergeable: "MERGEABLE",
    mergeStateStatus: "CLEAN",
    createdAt: ago(now, opened),
    updatedAt: ago(now, 5),
    ...over,
  })
  const prs = [
    pr(212, "Boat wake follows the hull", "irori-agents", 300),
    pr(218, "Shop prices from one table", "irori-agents", 180),
    pr(219, "Fix lantern flicker", "blake-dev", 170),
    pr(220, "Hookshot range +2 tiles", "irori-agents", 160, {
      checks: "pending",
    }),
    pr(215, "Enemy camp: second patrol", "irori-agents", 400, {
      mergeStateStatus: "BEHIND",
    }),
    pr(216, "Save slots: keep the last 3", "irori-agents", 350, {
      checks: "failure",
    }),
    pr(217, "Night fog tweak", "irori-agents", 320),
    pr(221, "New title screen", "irori-agents", 60, { isDraft: true }),
  ]
  const unreachable = flag === "unreachable"
  const game: Desk = {
    settings: {
      repoKey: GAME,
      enabled: true,
      takesPrs: true,
      pollDevice: "pc",
      trusted: ["you", "blake-dev"],
      perPrOk: false,
      reviewsOff: false,
    },
    prs: {
      repoKey: GAME,
      at: ago(now, unreachable ? 9 : 2),
      truncated: flag === "truncated",
      prs,
    },
    poll: unreachable
      ? { at: ago(now, 1), unreachableSince: ago(now, 6), device: "pc" }
      : { at: ago(now, 1), device: "pc" },
    authors: {
      212: { card: "c41", cardRef: "#41", agent: "primrose", name: "primrose" },
      218: { card: "c49", cardRef: "#49", agent: "scabious", name: "scabious" },
      219: { card: "c52", cardRef: "#52", name: "Blake" },
      220: { card: "c53", cardRef: "#53", agent: "primrose", name: "primrose" },
      215: { card: "c44", cardRef: "#44", agent: "kestrel", name: "kestrel" },
      216: { card: "c47", cardRef: "#47", agent: "nutmeg", name: "nutmeg" },
      217: { card: "c45", cardRef: "#45", agent: "kestrel", name: "kestrel" },
      221: { card: "c50", cardRef: "#50", agent: "nutmeg", name: "nutmeg" },
    },
    merged: [
      {
        pr: 210,
        title: "Rope bridge sway",
        sha: "e81b0c4a9d",
        at: ago(now, 3),
      },
    ],
  }
  const app: Desk = {
    settings: {
      repoKey: APP,
      enabled: true,
      takesPrs: true,
      pollDevice: "pc",
      trusted: ["you"],
      perPrOk: false,
      reviewsOff: false,
    },
    prs: { repoKey: APP, at: ago(now, 40), truncated: false, prs: [] },
    poll: { at: ago(now, 2), device: "pc" },
  }
  const irori: Desk = {
    settings: {
      repoKey: IRORI,
      enabled: false,
      takesPrs: false,
      pollDevice: "",
      trusted: [],
      perPrOk: false,
      reviewsOff: false,
    },
  }
  return [game, app, irori]
}
