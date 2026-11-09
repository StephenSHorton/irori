// RunsList is the card page's Running section (card #1249, live-tasks.md
// §1, §5.3): its agents' background runs (a shell run with Show tab and
// Stop, an agent or workflow run with neither: their output isn't a
// terminal), the runs that ended, its foreground shell calls with time
// and CPU, and runs on other devices with where they run.

import { ActivityIcon, SquareTerminalIcon } from "lucide-react"

import { Tick } from "@/components/common/ago"
import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import { attachAgent } from "@/lib/agents"
import { displayName } from "@/lib/cards"
import { plainLine } from "@/lib/irori"
import {
  endText,
  fgText,
  remote,
  runChipText,
  runId,
  running,
  stopRun,
  useRunLoads,
  withLoad,
  type CardRun,
} from "@/lib/runs"
import { showRunTab, termsHere } from "@/lib/term"
import { cn } from "@/lib/utils"

// runsOrder: running first, newest first within each.
function order(runs: readonly CardRun[]): CardRun[] {
  return [...runs].sort(
    (a, b) => Number(running(b)) - Number(running(a)) || b.started - a.started
  )
}

export function RunsList({
  runs,
  cardId,
}: {
  runs: readonly CardRun[]
  cardId: string
}) {
  const loads = useRunLoads()
  return (
    <div className="grid gap-2">
      {order(runs.map((r) => withLoad(r, loads))).map((r) => (
        <RunRow key={runId(r)} run={r} cardId={cardId} />
      ))}
    </div>
  )
}

function RunRow({ run: r, cardId }: { run: CardRun; cardId: string }) {
  const live = running(r)
  const away = remote(r)
  const fg = r.kind === "fg"
  const shell = r.kind === "bash"
  // What it is, after the command: who ran it and, for an agent or a
  // workflow run, which.
  const what = [
    displayName(r.who),
    r.kind === "agent" ? "agent run" : r.kind === "workflow" ? "workflow" : "",
    fg ? "in its turn" : "",
  ]
    .filter(Boolean)
    .join(" · ")
  return (
    <div className="grid grid-cols-[14px_minmax(0,1fr)] gap-x-2 text-[13px]">
      {fg || !shell ? (
        <ActivityIcon
          className="mt-0.5 size-3.5 text-muted-foreground"
          aria-hidden
        />
      ) : (
        <SquareTerminalIcon
          className="mt-0.5 size-3.5 text-muted-foreground"
          aria-hidden
        />
      )}
      <span
        className={cn("min-w-0 break-words", !live && "text-muted-foreground")}
      >
        <Tick of={(now) => rowText(r, now.getTime())} />
      </span>
      <span className="col-start-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <span>
          {what}
          {!live && !fg && !away && ` · ${endText(r)}`}
        </span>
        {live && shell && !away && r.who.here && (
          <>
            {termsHere() && (
              <Button
                size="xs"
                variant="ghost"
                className="h-5 px-1.5"
                onClick={() => showRunTab(cardId, r.run)}
                title="Its output in the card's Terminal, read only"
              >
                Show tab
              </Button>
            )}
            <ConfirmButton
              label="Stop"
              variant="destructive"
              question={`Stop ${plainLine(r.cmd) || "this run"}?`}
              confirmLabel="Stop it"
              onConfirm={() => stopRun(r)}
            />
          </>
        )}
        {live && fg && r.who.spawned && r.who.here && (
          <Button
            size="xs"
            variant="ghost"
            className="h-5 px-1.5"
            onClick={() => void attachAgent(r.who.key, cardId)}
            title="Open the agent in this card's terminal tab"
          >
            Attach
          </Button>
        )}
      </span>
    </div>
  )
}

function rowText(r: CardRun, now: number): string {
  if (r.kind === "fg") return fgText(r, now)
  if (remote(r)) return runChipText(r, now)
  if (!running(r)) {
    const took = Math.max(0, (r.ended ?? now) - r.started)
    const min = Math.round(took / 60_000)
    return `${plainLine(r.cmd) || "a run"} · ${min < 1 ? "under a minute" : `${min} min`}`
  }
  return runChipText(r, now)
}
