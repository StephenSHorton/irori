// WindDownLine is Home's one line for a Wind down (card #484): "Wound down
// 43 agents, 23 confirmed". A Wind down once opened a DM with every agent it
// reached, and each "wound down" waited there unread; now it opens none,
// and this line stands for the whole run. It opens the Wind down dialog,
// with each agent's answer, and goes once dismissed, until the next run.
// It reads the run the window keeps, so a restart of Irori clears it.

import { useCallback, useEffect, useState } from "react"
import { PowerIcon, XIcon } from "lucide-react"

import { EventWindDown } from "@/components/wind-down-dialog"
import type { WindDownStatus } from "@/gen/bindings"
import { Events, WindDownService } from "@/lib/host"
import { inWails } from "@/lib/irori"
import { useLatest } from "@/lib/talk"
import { windDownLine, windDownSample } from "@/lib/wind-down"

const seenKey = "irori.windDown.dismissed"

function readSeen(): string {
  try {
    return localStorage.getItem(seenKey) ?? ""
  } catch {
    return ""
  }
}

const none: WindDownStatus = {
  id: "",
  scope: "this",
  pauseAfter: true,
  paused: false,
  finished: false,
  rows: [],
}

export function WindDownLine({ onOpen }: { onOpen: () => void }) {
  const load = useCallback(
    async () => (inWails() ? WindDownService.Status() : (windDownSample() ?? none)),
    []
  )
  const { value: st, reload } = useLatest<WindDownStatus>(load, none)
  useEffect(() => {
    void reload()
    if (!inWails()) return
    return Events.On(EventWindDown, () => void reload())
  }, [reload])
  const [seen, setSeen] = useState(readSeen)
  const text = windDownLine(st)
  if (!text || seen === st.id) return null
  const dismiss = () => {
    setSeen(st.id)
    try {
      localStorage.setItem(seenKey, st.id)
    } catch {
      // No storage: it stays dismissed until the window reloads.
    }
  }
  return (
    <div role="status" className="flex items-center gap-2 px-1 text-[12.5px] text-muted-foreground">
      <PowerIcon className="size-3.5 shrink-0" aria-hidden />
      <button
        type="button"
        onClick={onOpen}
        title="Show each agent's answer"
        className="min-w-0 truncate rounded text-left hover:text-ink hover:underline"
      >
        {text}
      </button>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss"
        title="Dismiss"
        className="ml-auto shrink-0 rounded p-0.5 hover:bg-secondary hover:text-ink"
      >
        <XIcon className="size-3.5" aria-hidden />
      </button>
    </div>
  )
}
