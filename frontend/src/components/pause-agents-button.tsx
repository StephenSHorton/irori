import { useEffect, useState } from "react"
import { ChevronDownIcon, PauseIcon, PlayIcon } from "lucide-react"

import { WhoTicks } from "@/components/stop-who-ticks"
import { Button } from "@/components/ui/button"
import { ButtonGroup } from "@/components/ui/button-group"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { runningSince, useAgentsPause } from "@/lib/agents"
import { stopLabels as L } from "@/lib/stop-labels"
import {
  stopCounts,
  stopHereBlocked,
  stopHereTitle,
  useStopWho,
} from "@/lib/stop-who"
import { cn } from "@/lib/utils"

// PauseAgentsButton is the top nav's Stop here switch (cards #665, #773): one
// click stops work on this machine (each machine has its own switch,
// Stephen's pick): its agents write their hand-offs and leave their cards,
// the other machines take them, and nothing starts here. While it is on
// the nav says so and since when, and one click starts it again. Its words are in
// lib/stop-labels.ts.
// Off, one click stops whom this computer last chose (decision 71: On
// cards, Yours or both, both unless chosen); the chevron beside it shows
// the two ticks. Yours alone lights nothing: its run shows in the dialog
// and on Home, and Call off ends it.
export function PauseAgentsButton({
  agents = [],
  onYours,
}: {
  // The roster, for the ticks' counts on this machine.
  agents?: { spawned: boolean; here?: boolean }[]
  // Opens the dialog: Yours alone lights nothing here, so its run shows
  // there.
  onYours?: () => void
}) {
  const { pause, error, busy, set } = useAgentsPause()
  const [who, setWho] = useStopWho()
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!pause?.on) return
    const id = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(id)
  }, [pause?.on])
  if (!pause) return null
  if (pause.on) {
    const sinceMs = pause.since ?? now
    const since = new Date(sinceMs)
    const at = since.toLocaleString(undefined, {
      weekday: "short",
      hour: "numeric",
      minute: "2-digit",
    })
    const ago = runningSince(sinceMs, Math.max(now, sinceMs))
    const title = L.here.onTitle(at, pause.by) + (error ? `\n${error}` : "")
    return (
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => set(false)}
        title={title}
        aria-label={L.here.onAria(at)}
        aria-pressed
        data-spot="stop-here"
        className={cn(
          "border-warning/50 bg-warning-foreground text-warning hover:bg-warning-foreground/80 hover:text-warning",
          error && "ring-1 ring-destructive"
        )}
      >
        <PlayIcon aria-hidden className="size-3.5" />
        <span className="hidden sm:inline">
          {L.here.on}
          <span className="ml-1 font-normal opacity-80">· {ago}</span>
        </span>
        <span className="sm:hidden">{L.here.onShort}</span>
      </Button>
    )
  }
  const counts = stopCounts(agents, true)
  // Nothing ticked, or Yours alone with none here: nothing to do, and
  // the hover says why.
  const blocked = stopHereBlocked(who, counts)
  return (
    <ButtonGroup>
      <Button
        size="sm"
        variant="ghost"
        disabled={busy || !!blocked}
        onClick={() => {
          set(true, who)
          if (!who.cards) onYours?.()
        }}
        title={(blocked || stopHereTitle(who)) + (error ? `\n${error}` : "")}
        aria-label={L.here.name}
        aria-pressed={false}
        data-spot="stop-here"
        className={cn(error && "ring-1 ring-destructive")}
      >
        <PauseIcon aria-hidden className="size-3.5" />
        <span className="hidden sm:inline">{L.here.name}</span>
      </Button>
      <Popover>
        <PopoverTrigger
          render={
            <Button
              size="sm"
              variant="ghost"
              className="px-1"
              aria-label={L.here.choose}
              title={L.here.choose}
            >
              <ChevronDownIcon aria-hidden className="size-3.5" />
            </Button>
          }
        />
        <PopoverContent align="end" className="w-72">
          <WhoTicks who={who} counts={counts} onChange={setWho} />
        </PopoverContent>
      </Popover>
    </ButtonGroup>
  )
}
