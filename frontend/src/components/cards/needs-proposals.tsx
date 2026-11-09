// PROTOTYPE (card #1205, from #1204): two asks in Needs you, on mock data
// only. No backend, no rows. Shown in the browser preview with `?proposals`.
//
// 1. "OK to start an agent?" asks grouped by the person who gave the cards:
//    Start all / Not now for all, plus a Start / Not now on each row.
// 2. An agent's batch proposal of several tasks as one list: a checkbox on
//    each task, Accept selected, Accept all, and Reject with an optional
//    reason.
//
// Decision 50 and spawned-agents.md §9 hold throughout: only the person's
// own press counts, and one yes starts one agent. So "Start all" names how
// many agents it starts and asks once more before it does; it is still one
// yes per card, written for each card, never one yes for the group.

import { useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Textarea } from "@/components/ui/textarea"
import { previewFlag } from "@/lib/preview"

export function proposalsPreview(): boolean {
  return previewFlag("proposals") !== null
}

type Given = { id: string; ref: string; title: string; goal: string }
type Giver = { name: string; space: string; cards: Given[] }

const GIVERS: Giver[] = [
  {
    name: "Spencer",
    space: "MLG",
    cards: [
      {
        id: "g1",
        ref: "mlg#71",
        title: "Add a retry to the upload step",
        goal: "Uploads over 50 MB retry twice before failing.",
      },
      {
        id: "g2",
        ref: "mlg#74",
        title: "Show upload progress in the status bar",
        goal: "A percentage and the file name while an upload runs.",
      },
      {
        id: "g3",
        ref: "mlg#75",
        title: "Fix the clip counter after a cancelled upload",
        goal: "The counter drops back by one when an upload is cancelled.",
      },
    ],
  },
  {
    name: "Blake",
    space: "MLG",
    cards: [
      {
        id: "g4",
        ref: "mlg#80",
        title: "Rename Highlights to Reels in the menu",
        goal: "Menu label and the page title only; the URL stays.",
      },
    ],
  },
]

type Task = { id: string; title: string; goal: string; weight: string }

const PROPOSAL = {
  agent: "sample-heron",
  from: "#1180 Talk search",
  why: "Search is slow on long channels. I split it so each part can go to a different agent.",
  tasks: [
    {
      id: "t1",
      title: "Index message text when it arrives",
      goal: "Write each new post to a search index instead of scanning on every query.",
      weight: "normal",
    },
    {
      id: "t2",
      title: "Backfill the index for existing channels",
      goal: "One pass over old posts, resumable, with a progress line.",
      weight: "light",
    },
    {
      id: "t3",
      title: "Search box: results as you type",
      goal: "Debounced, shows the first 20, keeps the keyboard focus.",
      weight: "normal",
    },
    {
      id: "t4",
      title: "Decide how search treats edited and deleted posts",
      goal: "A design question for you before anything is built.",
      weight: "heavy",
    },
  ] as Task[],
}

export function ProposalsPreview() {
  return (
    <div className="grid min-w-0 gap-2.5">
      <p className="px-1 text-[12px] text-muted-foreground">
        Prototype · mock data · nothing here is saved or started
      </p>
      {GIVERS.map((g) => (
        <GiverAsk key={g.name} giver={g} />
      ))}
      <BatchProposal />
    </div>
  )
}

type Pick = "start" | "later"

function GiverAsk({ giver }: { giver: Giver }) {
  const [picked, setPicked] = useState<Record<string, Pick>>({})
  const [confirm, setConfirm] = useState(false)
  const open = giver.cards.filter((c) => !picked[c.id])
  const n = giver.cards.length
  const mark = (ids: string[], p: Pick) => {
    setPicked((s) => ({ ...s, ...Object.fromEntries(ids.map((i) => [i, p])) }))
    setConfirm(false)
  }
  const started = giver.cards.filter((c) => picked[c.id] === "start").length
  return (
    <article className="grid min-w-0 gap-2 px-4 py-3 glass-panel">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
        <Badge variant="info">Given to you</Badge>
        <span className="min-w-0 truncate">{giver.space}</span>
      </div>
      <h3 className="min-w-0 text-[15px] leading-snug font-semibold break-words">
        OK to start an agent?
        <span className="font-medium text-muted-foreground">
          {" "}
          {giver.name} gave you {n === 1 ? "a card" : `${n} cards`}
        </span>
      </h3>
      {open.length > 0 && (
        <p className="text-[13px] text-muted-foreground">
          Irori stopped because {giver.name} wrote {n === 1 ? "it" : "them"}. An
          agent runs on this computer under your own settings and cap, and
          treats the card as {giver.name}&apos;s request.
        </p>
      )}
      <ul className="grid min-w-0 divide-y divide-border/60">
        {giver.cards.map((c) => {
          const p = picked[c.id]
          return (
            <li
              key={c.id}
              className="grid min-w-0 gap-1.5 py-2 first:pt-0 last:pb-0"
            >
              <div className="min-w-0">
                <p className="text-sm break-words">
                  <span className="text-muted-foreground tabular-nums">
                    {c.ref}
                  </span>{" "}
                  {c.title}
                </p>
                <p className="text-[13px] break-words text-muted-foreground">
                  {c.goal}
                </p>
              </div>
              {p ? (
                <p role="status" className="text-[13px] text-muted-foreground">
                  {p === "start"
                    ? "Starting an agent on this computer…"
                    : "Not now. The card stays yours, with no agent."}{" "}
                  <button
                    type="button"
                    className="underline underline-offset-2"
                    onClick={() =>
                      setPicked((s) => {
                        const x = { ...s }
                        delete x[c.id]
                        return x
                      })
                    }
                  >
                    {p === "start" ? "Stop it" : "Ask again"}
                  </button>
                </p>
              ) : (
                <span className="flex flex-wrap gap-1.5">
                  <Button size="sm" onClick={() => mark([c.id], "start")}>
                    Start an agent
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => mark([c.id], "later")}
                  >
                    Not now
                  </Button>
                </span>
              )}
            </li>
          )
        })}
      </ul>
      {open.length > 1 && !confirm && (
        <span className="flex flex-wrap items-center gap-1.5 border-t border-border/60 pt-2">
          <Button size="sm" onClick={() => setConfirm(true)}>
            Start all {open.length}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              mark(
                open.map((c) => c.id),
                "later"
              )
            }
          >
            Not now for all
          </Button>
        </span>
      )}
      {open.length > 1 && confirm && (
        <div
          role="group"
          aria-label="Confirm starting all"
          className="grid gap-1.5 border-t border-border/60 pt-2"
        >
          <p className="text-[13px]">
            That is {open.length} yeses, one per card: {open.length} agents
            start on this computer, each under your model and cap. Cards over
            the cap queue.
          </p>
          <span className="flex flex-wrap gap-1.5">
            <Button
              size="sm"
              onClick={() =>
                mark(
                  open.map((c) => c.id),
                  "start"
                )
              }
            >
              Yes, start {open.length} agents
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setConfirm(false)}
            >
              Cancel
            </Button>
          </span>
        </div>
      )}
      {open.length === 0 && started > 0 && (
        <p className="sr-only">All answered.</p>
      )}
    </article>
  )
}

function BatchProposal() {
  const P = PROPOSAL
  const [sel, setSel] = useState<Set<string>>(
    () => new Set(P.tasks.map((t) => t.id))
  )
  const [result, setResult] = useState("")
  const [rejecting, setRejecting] = useState(false)
  const [reason, setReason] = useState("")
  const count = sel.size
  const toggle = (id: string, on: boolean) =>
    setSel((s) => {
      const x = new Set(s)
      if (on) x.add(id)
      else x.delete(id)
      return x
    })
  const accept = (ids: string[]) => {
    const left = P.tasks.length - ids.length
    setResult(
      `Accepted ${ids.length} of ${P.tasks.length}: ${ids.length === 1 ? "a card" : `${ids.length} cards`} made on your board.` +
        (left > 0 ? ` ${left} left out, and ${P.agent} was told.` : "")
    )
  }
  return (
    <article className="grid min-w-0 gap-2 px-4 py-3 glass-panel">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
        <Badge variant="info">Proposal</Badge>
        <span className="min-w-0 truncate">
          {P.agent} · from {P.from}
        </span>
      </div>
      <h3 className="min-w-0 text-[15px] leading-snug font-semibold break-words">
        {P.agent} proposes {P.tasks.length} tasks
      </h3>
      {result ? (
        <p role="status" className="text-[13px] text-muted-foreground">
          {result}
        </p>
      ) : (
        <>
          <p className="text-[13px] break-words text-muted-foreground">
            {P.why}
          </p>
          <ul className="grid min-w-0 divide-y divide-border/60">
            {P.tasks.map((t) => (
              <li key={t.id} className="min-w-0 py-1.5 first:pt-0 last:pb-0">
                <label className="flex min-w-0 cursor-pointer items-start gap-2.5">
                  <Checkbox
                    className="mt-0.5"
                    checked={sel.has(t.id)}
                    onCheckedChange={(v) => toggle(t.id, !!v)}
                    aria-label={t.title}
                  />
                  <span className="grid min-w-0 flex-1 gap-0.5">
                    <span className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-sm">
                      <span className="break-words">{t.title}</span>
                      <Badge variant="slate">{t.weight}</Badge>
                    </span>
                    <span className="text-[13px] break-words text-muted-foreground">
                      {t.goal}
                    </span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
          {rejecting ? (
            <div className="grid gap-1.5 border-t border-border/60 pt-2">
              <Textarea
                aria-label="Why you are rejecting it (optional)"
                placeholder="Why not? (optional) It goes back to the agent."
                value={reason}
                onChange={(e) => setReason(e.target.value)}
                rows={2}
              />
              <span className="flex flex-wrap gap-1.5">
                <Button
                  size="sm"
                  variant="destructive"
                  onClick={() =>
                    setResult(
                      reason.trim()
                        ? `Rejected. ${P.agent} was told: “${reason.trim()}”`
                        : `Rejected. ${P.agent} was told, with no reason.`
                    )
                  }
                >
                  Reject
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setRejecting(false)}
                >
                  Cancel
                </Button>
              </span>
            </div>
          ) : (
            <span className="flex flex-wrap gap-1.5 border-t border-border/60 pt-2">
              <Button
                size="sm"
                disabled={count === 0}
                onClick={() =>
                  accept(P.tasks.filter((t) => sel.has(t.id)).map((t) => t.id))
                }
              >
                Accept selected
                {count > 0 && count < P.tasks.length ? ` (${count})` : ""}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => accept(P.tasks.map((t) => t.id))}
              >
                Accept all
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setRejecting(true)}
              >
                Reject…
              </Button>
            </span>
          )}
          <p className="text-[12px] text-muted-foreground">
            Accepting makes cards. It starts no agent: each card follows your
            usual start rules.
          </p>
        </>
      )}
    </article>
  )
}
