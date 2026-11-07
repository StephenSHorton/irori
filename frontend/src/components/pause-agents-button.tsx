import { useEffect, useState } from "react"
import { PauseIcon, PlayIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { runningSince, useAgentsPause } from "@/lib/agents"
import { cn } from "@/lib/utils"

// PauseAgentsButton is the top nav's Pause agents switch (card #665): one
// click stops Irori starting or restarting any agent on every one of the
// person's devices, and while it is on the nav says so and since when, and
// one click turns it off. Running agents keep running and keep getting
// messages. Icon only on a phone.
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
    const title =
      `Agents paused since ${at}${pause.by ? ` by ${pause.by}` : ""}. ` +
      "Irori starts no agent on any of your devices until you turn this off; running agents keep running. Click to start agents again." +
      (error ? `\n${error}` : "")
    return (
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => set(false)}
        title={title}
        aria-label={`Agents paused since ${at}. Start agents again`}
        aria-pressed
        className={cn(
          "border-warning/50 bg-warning-foreground text-warning hover:bg-warning-foreground/80 hover:text-warning",
          error && "ring-1 ring-destructive"
        )}
      >
        <PlayIcon aria-hidden className="size-3.5" />
        <span className="hidden sm:inline">
          Agents paused
          <span className="ml-1 font-normal opacity-80">
            · {ago === "just now" ? "just now" : `${ago}`}
          </span>
        </span>
        <span className="sm:hidden">
          Paused<span className="ml-1 font-normal opacity-80">· {ago}</span>
        </span>
      </Button>
    )
  }
  return (
    <Button
      size="sm"
      variant="ghost"
      disabled={busy}
      onClick={() => set(true)}
      title={
        "Pause agents: Irori stops starting or restarting agents on all your devices (new cards, threads, retries, the fresh agent after End). Running agents keep running." +
        (error ? `\n${error}` : "")
      }
      aria-label="Pause agents"
      aria-pressed={false}
      className={cn(error && "ring-1 ring-destructive")}
    >
      <PauseIcon aria-hidden className="size-3.5" />
      <span className="hidden sm:inline">Pause agents</span>
    </Button>
  )
}
