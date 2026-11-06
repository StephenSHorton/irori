import { useCallback, useEffect, useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import type { WindDownRow, WindDownStatus } from "@/gen/bindings"
import { Events, WindDownService } from "@/lib/host"
import { inWails, plainBlock, plainLine } from "@/lib/irori"
import { nameOr } from "@/lib/names"
import { errorText, useLatest } from "@/lib/talk"
import { windDownSample } from "@/lib/wind-down"

// EventWindDown is windDown:changed: a run's status moved.
export const EventWindDown = "windDown:changed"

const empty: WindDownStatus = {
  id: "",
  scope: "this",
  pauseAfter: true,
  paused: false,
  finished: false,
  rows: [],
}

const stateText: Record<string, string> = {
  waiting: "Not reached yet",
  winding: "Winding down",
  done: "Wound down",
  no_answer: "No answer",
}

const stateVariant: Record<
  string,
  "secondary" | "warning" | "success" | "destructive"
> = {
  waiting: "secondary",
  winding: "warning",
  done: "success",
  no_answer: "destructive",
}

// WindDownDialog is Wind down all (card #220): every agent of the person's
// saves its place, hands back its cards, releases its leases and stops.
// Before a run it asks where (this device or all of them) and whether to
// turn Pause all on after; during one it lists each agent live, with Stop
// for one that doesn't answer.
export function WindDownDialog({
  agents,
  onClose,
}: {
  // How many agents run here and on every device, for the confirm.
  agents: { here: number; all: number }
  onClose: () => void
}) {
  const loadStatus = useCallback(
    async () =>
      inWails() ? WindDownService.Status() : (windDownSample() ?? empty),
    []
  )
  const live = useLatest<WindDownStatus>(loadStatus, empty)
  const [started, setStarted] = useState<WindDownStatus | null>(null)
  const st = live.value.id ? live.value : (started ?? live.value)
  const [scope, setScope] = useState<"this" | "all">("this")
  const [pauseAfter, setPauseAfter] = useState(true)
  const [busy, setBusy] = useState("")
  const [error, setError] = useState<string | null>(null)

  const reload = live.reload
  useEffect(() => {
    void reload()
    if (!inWails()) return
    return Events.On(EventWindDown, () => void reload())
  }, [reload])

  const running = st.id !== "" && !st.finished
  const start = async () => {
    setBusy("start")
    setError(null)
    try {
      setStarted(await WindDownService.All(scope, pauseAfter))
      await reload()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy("")
    }
  }
  const stop = async (r: WindDownRow) => {
    setBusy(r.key)
    setError(null)
    try {
      await WindDownService.Stop(r.key)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy("")
    }
  }

  const n = scope === "all" ? agents.all : agents.here
  const done = st.rows.filter((r) => r.state === "done").length

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="gap-3 sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Wind down all agents</DialogTitle>
          <DialogDescription>
            Each agent finishes the step in hand, pushes its work, posts a
            hand-off on each card it holds and leaves it, releases its leases
            and answers you. Nothing is cut: a busy agent stops at its next safe
            point.
          </DialogDescription>
        </DialogHeader>

        {(!st.id || st.finished) && (
          <div className="flex flex-col gap-2 text-sm">
            <div
              className="flex flex-wrap items-center gap-1"
              role="group"
              aria-label="Which agents"
            >
              <Button
                size="sm"
                aria-pressed={scope === "this"}
                variant={scope === "this" ? "default" : "ghost"}
                onClick={() => setScope("this")}
              >
                This device ({agents.here})
              </Button>
              <Button
                size="sm"
                aria-pressed={scope === "all"}
                variant={scope === "all" ? "default" : "ghost"}
                onClick={() => setScope("all")}
              >
                All my devices ({agents.all})
              </Button>
            </div>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={pauseAfter}
                onChange={(e) => setPauseAfter(e.target.checked)}
              />
              Pause all after, so nothing wakes them until you resume
              {scope === "all" && (
                <span className="text-xs text-muted-foreground">
                  (this device's wakes)
                </span>
              )}
            </label>
          </div>
        )}

        {st.id !== "" && (
          <div className="flex flex-col gap-1.5">
            <p className="text-sm text-muted-foreground">
              {done} of {st.rows.length} wound down
              {st.finished && st.paused && " · Pause all is on"}
              {st.finished && !st.pauseAfter && " · Pause all left as it was"}
            </p>
            <ul
              className="flex max-h-[50vh] flex-col gap-1 overflow-y-auto"
              aria-label="Agents"
            >
              {st.rows.map((r) => (
                <li
                  key={r.key || r.name}
                  className="flex flex-col gap-1 px-3 py-2 text-sm glass-panel"
                >
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 truncate font-medium">
                      {nameOr(r.name)}
                    </span>
                    {r.device && (
                      <span className="text-xs text-muted-foreground">
                        {plainLine(r.device)}
                      </span>
                    )}
                    <Badge
                      variant={stateVariant[r.state] ?? "secondary"}
                      className="ml-auto"
                    >
                      {stateText[r.state] ?? r.state}
                    </Badge>
                    {(r.state === "waiting" || r.state === "winding") && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={busy === r.key}
                        title="Cut its running turn and ask it to wind down now"
                        onClick={() => void stop(r)}
                      >
                        Stop
                      </Button>
                    )}
                  </div>
                  {r.reply && (
                    <p className="text-xs whitespace-pre-wrap text-muted-foreground">
                      {plainBlock(r.reply)}
                    </p>
                  )}
                  {r.error && (
                    <p className="text-xs text-destructive">{r.error}</p>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}

        {error && (
          <p className="text-sm text-destructive" role="alert">
            {error}
          </p>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {running ? "Hide" : "Close"}
          </Button>
          {!running && (
            <Button
              disabled={busy === "start" || n === 0}
              onClick={() => void start()}
            >
              {st.id
                ? "Wind down again"
                : `Wind down ${n} agent${n === 1 ? "" : "s"}`}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
