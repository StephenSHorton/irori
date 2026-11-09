// GivenBatch (decision 69, docs/design/task-proposals.md §6.1 and §7): one
// block in Needs you for the cards one person gave you, one batch at a time.
// Each card has Start an agent / Not now / Give back. Only the person's own
// press counts (decision 50): Start selected and Start all name how many
// agents they start and ask once more, then say Start once per card.

import { useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Textarea } from "@/components/ui/textarea"
import { cards } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import { GIVEN_NOT_NOW, GIVEN_START, type GivenGroup } from "@/lib/needs"
import type { CardSummary } from "@/lib/cards"

// givenHeader is the block's title: "Spencer gave you 4 tasks".
export function givenHeader(g: GivenGroup): string {
  const n = g.cards.length
  return `${g.giver.name} gave you ${n === 1 ? "a task" : `${n} tasks`}`
}

// startAllText is the second ask before Start selected / Start all.
export function startAllText(n: number): string {
  return `That's ${n} ${n === 1 ? "yes" : "yeses"}, one per card: ${n} ${
    n === 1 ? "agent starts" : "agents start"
  }.`
}

type Said = "start" | "notnow" | "back"

const refOf = (c: CardSummary) => (c.short ? `#${plainLine(c.short)}` : "")

// canStart: the card has its question open, or its Not now can be taken
// back (an agent starts under today's rules until S4 routes Start).
const canStart = (c: CardSummary, said: Said | undefined) =>
  !said &&
  c.givenSaid !== "start" &&
  (!!c.givenQuestion || c.givenSaid === "notnow")

export function GivenBatch({
  group,
  onOpen,
}: {
  group: GivenGroup
  onOpen: (id: string) => void
}) {
  const g = group
  const [said, setSaid] = useState<Record<string, Said>>({})
  const [sel, setSel] = useState<Set<string>>(
    () => new Set(g.cards.map((c) => c.id))
  )
  const [confirm, setConfirm] = useState(false)
  const [backing, setBacking] = useState(false)
  const [reason, setReason] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const statusOf = (c: CardSummary): Said | undefined =>
    said[c.id] ??
    (c.givenSaid === "start"
      ? "start"
      : c.givenSaid === "notnow"
        ? "notnow"
        : undefined)
  const open = g.cards.filter(
    (c) => statusOf(c) !== "start" && statusOf(c) !== "back"
  )
  const picked = open.filter((c) => sel.has(c.id))
  const startable = (cs: CardSummary[]) =>
    cs.filter((c) => canStart(c, said[c.id]))

  // run goes one card at a time, so one failure doesn't hide the others,
  // and reports what happened the way HeldQuiet.releaseAll does.
  async function run(
    cs: CardSummary[],
    to: Said,
    act: (c: CardSummary) => Promise<void>
  ) {
    setBusy(true)
    setError("")
    setConfirm(false)
    const failed: string[] = []
    let ok = 0
    for (const c of cs) {
      try {
        await act(c)
        ok++
        setSaid((s) => ({ ...s, [c.id]: to }))
      } catch (e) {
        failed.push(
          `${refOf(c) || plainLine(c.title)}: ${(e as Error).message}`
        )
      }
    }
    setBusy(false)
    if (failed.length)
      setError(`${ok} of ${cs.length} done. ${failed.join(" · ")}`)
  }
  const start = (c: CardSummary) =>
    c.givenQuestion ? cards.sayGiven(c.id, "start") : cards.startAgent(c.id, "")
  const notNow = (c: CardSummary) => cards.sayGiven(c.id, "notnow")
  const giveBack = (c: CardSummary) => cards.giveBack(c.id, reason.trim())

  const toStart = startable(picked)
  const setOne = (id: string, on: boolean) =>
    setSel((s) => {
      const x = new Set(s)
      if (on) x.add(id)
      else x.delete(id)
      return x
    })
  const allStart = startable(open)

  return (
    <article className="grid min-w-0 gap-2 px-4 py-3 glass-panel">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
        <Badge variant="info">Given to you</Badge>
        {g.viaAssistant && (
          <span className="min-w-0 truncate">
            via {g.giver.name}&apos;s assistant
          </span>
        )}
      </div>
      <h3 className="min-w-0 text-[15px] leading-snug font-semibold break-words">
        {givenHeader(g)}
      </h3>
      {g.note && (
        <blockquote
          aria-label={`${g.giver.name}'s note`}
          className="min-w-0 border-l-2 border-border pl-2.5 text-[13px] break-words text-muted-foreground"
        >
          {plainLine(g.note)}
        </blockquote>
      )}
      <ul className="grid min-w-0 divide-y divide-border/60">
        {g.cards.map((c) => {
          const st = statusOf(c)
          return (
            <li
              key={c.id}
              className="grid min-w-0 gap-1.5 py-2 first:pt-0 last:pb-0"
            >
              <div className="flex min-w-0 items-start gap-2.5">
                {st !== "start" && st !== "back" && (
                  <Checkbox
                    className="mt-0.5"
                    checked={sel.has(c.id)}
                    onCheckedChange={(v) => setOne(c.id, !!v)}
                    aria-label={plainLine(c.title)}
                  />
                )}
                <div className="min-w-0 flex-1">
                  <p className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-sm">
                    <button
                      type="button"
                      className="min-w-0 text-left break-words underline-offset-2 hover:underline"
                      onClick={() => onOpen(c.id)}
                    >
                      <span className="text-muted-foreground tabular-nums">
                        {refOf(c)}
                      </span>{" "}
                      {plainLine(c.title)}
                    </button>
                    {c.weight && (
                      <Badge variant="slate">{c.weight.level}</Badge>
                    )}
                  </p>
                </div>
              </div>
              {st === "start" ? (
                <p role="status" className="text-[13px] text-muted-foreground">
                  Started.{" "}
                  <button
                    type="button"
                    className="underline underline-offset-2"
                    onClick={() => onOpen(c.id)}
                  >
                    Stop it
                  </button>
                </p>
              ) : st === "back" ? (
                <p role="status" className="text-[13px] text-muted-foreground">
                  Given back to {g.giver.name}.
                </p>
              ) : st === "notnow" ? (
                <p role="status" className="text-[13px] text-muted-foreground">
                  {GIVEN_NOT_NOW}.{" "}
                  <button
                    type="button"
                    disabled={busy}
                    className="underline underline-offset-2"
                    onClick={() => void run([c], "start", start)}
                  >
                    {GIVEN_START}
                  </button>
                </p>
              ) : c.givenQuestion ? (
                <span className="flex flex-wrap gap-1.5 pl-7">
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => void run([c], "start", start)}
                  >
                    {GIVEN_START}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() => void run([c], "notnow", notNow)}
                  >
                    {GIVEN_NOT_NOW}
                  </Button>
                </span>
              ) : (
                <p className="pl-7 text-[13px] text-muted-foreground">
                  Not asked yet: it asks on the computer with its repo.
                </p>
              )}
            </li>
          )
        })}
      </ul>
      {open.length > 0 && !confirm && !backing && (
        <span className="flex flex-wrap items-center gap-1.5 border-t border-border/60 pt-2">
          <Button
            size="sm"
            disabled={busy || toStart.length === 0}
            onClick={() => setConfirm(true)}
          >
            Start selected ({toStart.length})
          </Button>
          {allStart.length > 1 && (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => {
                setSel(new Set(allStart.map((c) => c.id)))
                setConfirm(true)
              }}
            >
              Start all {allStart.length}
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={
              busy || picked.filter((c) => c.givenQuestion).length === 0
            }
            onClick={() =>
              void run(
                picked.filter((c) => c.givenQuestion),
                "notnow",
                notNow
              )
            }
          >
            Not now for these
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={busy || picked.length === 0}
            onClick={() => setBacking(true)}
          >
            Give back…
          </Button>
        </span>
      )}
      {confirm && (
        <div
          role="group"
          aria-label="Confirm starting"
          className="grid gap-1.5 border-t border-border/60 pt-2"
        >
          <p className="text-[13px]">
            {startAllText(toStart.length)} Each runs under your model and cap;
            cards over the cap queue.
          </p>
          <span className="flex flex-wrap gap-1.5">
            <Button
              size="sm"
              disabled={busy}
              onClick={() => void run(toStart, "start", start)}
            >
              Yes, start {toStart.length}{" "}
              {toStart.length === 1 ? "agent" : "agents"}
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
      {backing && (
        <div className="grid gap-1.5 border-t border-border/60 pt-2">
          <Textarea
            aria-label={`Why you are giving ${picked.length === 1 ? "it" : "them"} back (optional)`}
            placeholder={`Why? (optional) ${g.giver.name} sees it.`}
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            rows={2}
          />
          <span className="flex flex-wrap gap-1.5">
            <Button
              size="sm"
              variant="destructive"
              disabled={busy}
              onClick={() => {
                setBacking(false)
                void run(picked, "back", giveBack)
              }}
            >
              Give back {picked.length}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setBacking(false)}>
              Cancel
            </Button>
          </span>
        </div>
      )}
      {error && (
        <p role="alert" className="text-[12.5px] break-words text-destructive">
          {error}
        </p>
      )}
      <p className="text-[12px] text-muted-foreground">
        {g.giver.name}&apos;s assistant will tell them.
      </p>
    </article>
  )
}
