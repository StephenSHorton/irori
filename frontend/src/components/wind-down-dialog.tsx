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
import { runningSince, useAgentsPause } from "@/lib/agents"
import { plainBlock, plainLine, useNow } from "@/lib/irori"
import { nameOr } from "@/lib/names"
import { stopLabels as L } from "@/lib/stop-labels"
import { WhoTicks } from "@/components/stop-who-ticks"
import { stopButton, stopCounts, useStopWho } from "@/lib/stop-who"
import { devicesText, yoursOnly, type WindDown } from "@/lib/wind-down"
import type { WindDownRow } from "@/gen/bindings"

const stateVariant: Record<
  string,
  "secondary" | "warning" | "success" | "destructive"
> = {
  waiting: "secondary",
  winding: "warning",
  done: "success",
  no_answer: "destructive",
  canceled: "secondary",
}

const when = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    weekday: "short",
    hour: "numeric",
    minute: "2-digit",
  })

// WindDownDialog is Stop everywhere (cards #220, #773, #787; Wind down in
// the code), a switch for every agent on every machine. Off, it turns it
// on: nothing starts anywhere, and every agent writes its hand-off, leaves
// its card, releases its leases and stops. On, it says since when, with
// Start everywhere (any time, mid-run too), and lists each agent live,
// with Stop for one that doesn't answer. When the latest run is this
// machine's Stop here (scope "device"), it is that run's dialog: titled
// Stop here with Start here while Stop here is on, and Stop everywhere
// still on offer.
// Off, two ticks pick who it reaches (decision 71): On cards (agents Irori
// started) and Yours (the person's own sessions), both unless this
// computer kept another choice. A Yours-only run holds nothing: no switch,
// and Call off in place of Start.
// Its words are in lib/stop-labels.ts.
export function WindDownDialog({
  agents,
  windDown,
  onClose,
}: {
  // The roster on every machine, for the ticks' counts.
  agents: { spawned: boolean; here?: boolean }[]
  // App's one copy of Stop everywhere's switch (lib/wind-down.ts).
  windDown: WindDown
  onClose: () => void
}) {
  const { status: st, busy, error } = windDown
  // This machine's Stop here, for a scope "device" run (card #773).
  const here = useAgentsPause()
  const deviceRun = !st.on && st.id !== "" && st.scope === "device"
  const hereOn = deviceRun && !!here.pause?.on
  const running = st.id !== "" && !st.finished && !st.off
  const sinceMs = st.on ? st.since : hereOn ? (here.pause?.since ?? 0) : 0
  const now = useNow(30_000).getTime()
  const ago = sinceMs ? runningSince(sinceMs, Math.max(now, sinceMs)) : ""
  const done = st.rows.filter((r) => r.state === "done").length
  const shownError = error || (deviceRun ? here.error : "")
  const [who, setWho] = useStopWho()
  const counts = stopCounts(agents)
  const stopText = stopButton(who, counts)
  const yours = yoursOnly(st)
  const canCallOff = yours && !st.off && !st.finished
  const groups = rowGroups(st.rows)

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="gap-3 sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {hereOn || (yours && st.scope === "device")
              ? L.here.name
              : L.everywhere.name}
          </DialogTitle>
          <DialogDescription>{L.how}</DialogDescription>
        </DialogHeader>

        {(st.on || hereOn) && (
          <div
            role="status"
            className="flex flex-wrap items-center gap-2 rounded-md border border-warning/50 bg-warning-foreground px-3 py-2 text-sm text-warning"
          >
            <span className="min-w-0 flex-1">
              {sinceMs ? (
                <>
                  {L.since(when(sinceMs))}
                  <span className="opacity-80"> · {ago}</span>
                </>
              ) : (
                L.here.on
              )}
              {st.on && (
                <span className="opacity-80"> · {devicesText(st.devices)}</span>
              )}
            </span>
            {st.on ? (
              <Button
                size="sm"
                variant="outline"
                disabled={busy === "off"}
                onClick={() => void windDown.off()}
                title={L.everywhere.startTitle}
              >
                {L.everywhere.start}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="outline"
                disabled={here.busy}
                onClick={() => here.set(false)}
                title={L.here.startTitle}
              >
                {L.here.start}
              </Button>
            )}
          </div>
        )}

        {!st.on && (
          <p className="text-sm text-muted-foreground">
            {L.everywhere.explain}
          </p>
        )}

        {!st.on && <WhoTicks who={who} counts={counts} onChange={setWho} />}

        {st.id !== "" && (
          <div className="flex flex-col gap-1.5">
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <span className="min-w-0 flex-1">
                {L.summary(done, st.rows.length)}
                {st.off && ` · ${yours ? L.calledOff : L.startedAgain}`}
              </span>
              {canCallOff && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy === "callOff"}
                  onClick={() => void windDown.callOff()}
                  title={L.callOffTitle}
                >
                  {L.callOff}
                </Button>
              )}
            </p>
            <div className="flex max-h-[50vh] flex-col gap-2 overflow-y-auto">
              {groups.map((g) => (
                <ul
                  key={g.label}
                  className="flex flex-col gap-1"
                  aria-label={g.label || "Agents"}
                >
                  {g.label && (
                    <li className="px-1 text-xs font-medium text-muted-foreground">
                      {g.label}
                    </li>
                  )}
                  {g.rows.map((r) => (
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
                          {L.rows[r.state] ?? r.state}
                        </Badge>
                        {(r.state === "waiting" || r.state === "winding") && (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={busy === r.key}
                            title={L.stopRowTitle}
                            onClick={() => void windDown.stop(r.key)}
                          >
                            {L.stopRow}
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
              ))}
            </div>
          </div>
        )}

        {shownError && (
          <p className="text-sm text-destructive" role="alert">
            {shownError}
          </p>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            {running ? "Hide" : "Close"}
          </Button>
          {/* Off: turns it on; with 0 agents it just holds starts (card #787). */}
          {!st.on && (
            <Button
              disabled={busy === "on" || !stopText}
              title={stopText ? undefined : L.who.none}
              onClick={() => void windDown.on(who)}
            >
              {stopText || L.who.none}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// rowGroups splits a run's rows into On cards and Yours when it has both
// (decision 71); a run of one kind is one list with no heading.
function rowGroups(
  rows: WindDownRow[]
): { label: string; rows: WindDownRow[] }[] {
  const cards = rows.filter((r) => r.spawned)
  const yours = rows.filter((r) => !r.spawned)
  if (cards.length === 0 || yours.length === 0) return [{ label: "", rows }]
  return [
    { label: L.who.cards(cards.length), rows: cards },
    { label: L.who.yours(yours.length), rows: yours },
  ]
}
