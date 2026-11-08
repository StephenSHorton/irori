import { useEffect, useState } from "react"
import { PauseIcon, PlayIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { runningSince, useAgentsPause } from "@/lib/agents"
import { stopLabels as L } from "@/lib/stop-labels"
import { cn } from "@/lib/utils"

// PauseAgentsButton is the top nav's Stop here switch (cards #665, #773): one
// click stops work on this machine (each machine has its own switch,
// Stephen's pick): its agents write their hand-offs and leave their cards,
// the other machines take them, and nothing starts here. While it is on
// the nav says so and since when, and one click starts it again. Its words are in
// lib/stop-labels.ts.
export function PauseAgentsButton() {
  const { pause, error, busy, set } = useAgentsPause()
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
        data-spot="pause-agents"
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
  return (
    <Button
      size="sm"
      variant="ghost"
      disabled={busy}
      onClick={() => set(true)}
      title={L.here.title + (error ? `\n${error}` : "")}
      aria-label={L.here.name}
      aria-pressed={false}
      data-spot="pause-agents"
      className={cn(error && "ring-1 ring-destructive")}
    >
      <PauseIcon aria-hidden className="size-3.5" />
      <span className="hidden sm:inline">{L.here.name}</span>
    </Button>
  )
}
