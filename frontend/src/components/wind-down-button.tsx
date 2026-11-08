import { useEffect, useState } from "react"
import { PlayIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { runningSince } from "@/lib/agents"
import { stopLabels as L } from "@/lib/stop-labels"
import { devicesText, type WindDown } from "@/lib/wind-down"
import { cn } from "@/lib/utils"

// WindDownButton is the top nav's Stop everywhere switch (cards #773,
// #787; Wind down in the code), like Stop here: off, it opens the dialog,
// where the person confirms; on, it says so and since when, and one click
// starts everywhere again (any time, mid-run too). A short label on a phone. Words: lib/stop-labels.ts.
export function WindDownButton({
  windDown,
  onOpen,
}: {
  windDown: WindDown
  // Opens the dialog; without it (the sample roster) the off button hides.
  onOpen?: () => void
}) {
  const { status: st, error, busy, off } = windDown
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!st.on) return
    const id = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(id)
  }, [st.on])
  if (st.on) {
    const sinceMs = st.since || now
    const at = new Date(sinceMs).toLocaleString(undefined, {
      weekday: "short",
      hour: "numeric",
      minute: "2-digit",
    })
    const ago = runningSince(sinceMs, Math.max(now, sinceMs))
    const title =
      L.everywhere.onTitle(at, devicesText(st.devices)) +
      (error ? `\n${error}` : "")
    return (
      <Button
        size="sm"
        variant="outline"
        disabled={busy === "off"}
        onClick={() => void off()}
        title={title}
        aria-label={L.everywhere.onAria(at)}
        data-spot="stop-everywhere"
        aria-pressed
        className={cn(
          "border-warning/50 bg-warning-foreground text-warning hover:bg-warning-foreground/80 hover:text-warning",
          error && "ring-1 ring-destructive"
        )}
      >
        <PlayIcon aria-hidden className="size-3.5" />
        <span className="hidden sm:inline">
          {L.everywhere.on}
          <span className="ml-1 font-normal opacity-80">· {ago}</span>
        </span>
        <span className="sm:hidden">{L.everywhere.onShort}</span>
      </Button>
    )
  }
  if (!onOpen) return null
  return (
    <Button
      size="sm"
      variant="ghost"
      onClick={onOpen}
      aria-pressed={false}
      title={L.everywhere.title}
      data-spot="stop-everywhere"
    >
      {L.everywhere.name}
    </Button>
  )
}
