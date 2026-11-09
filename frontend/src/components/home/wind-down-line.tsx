// WindDownLine is Home's one line for a Stop here or Stop everywhere run
// (cards #484, #773): "Stopped 43 agents, 23 handed off". A run once opened
// a DM with every agent it reached, and each answer waited there unread;
// now it opens none, and this line stands for the whole run. It opens the
// run's dialog,
// with each agent's answer, and goes once dismissed, until the next run.
// It reads the run the window keeps, so a restart of Irori clears it.

import { useState } from "react"
import { PowerIcon, XIcon } from "lucide-react"

import type { WindDownStatus } from "@/gen/bindings"
import { stopLabels as L } from "@/lib/stop-labels"
import { windDownLine } from "@/lib/wind-down"

const seenKey = "irori.windDown.dismissed"

function readSeen(): string {
  try {
    return localStorage.getItem(seenKey) ?? ""
  } catch {
    return ""
  }
}

// It reads App's one copy of the switch (card #787).
export function WindDownLine({
  status: st,
  onOpen,
}: {
  status: WindDownStatus
  onOpen: () => void
}) {
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
    <div
      role="status"
      className="flex items-center gap-2 px-1 text-[12.5px] text-muted-foreground"
    >
      <PowerIcon className="size-3.5 shrink-0" aria-hidden />
      <button
        type="button"
        onClick={onOpen}
        title={L.line.openTitle}
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
