import { useEffect, useState } from "react"

import { RosterService } from "@/lib/host"
import { inWails } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import type { CheckSlot } from "@/gen/bindings"

// The browser preview's slots (?slots, lib/preview.ts).
const SAMPLE: CheckSlot[] = [
  { resource: "check/mac/slot-1", run: "check-irori-437-19174", until: "" },
  { resource: "check/mac/slot-2", run: "check-irori-391-20311", until: "" },
]

// How often the sidebar re-reads the slots: check.sh renews its lease every
// 30 s, so a run that ended shows within a poll.
const POLL_MS = 10_000

// slotNumber is the N of check/<host>/slot-N.
export function slotNumber(resource: string): string {
  return resource.match(/\/slot-(\d+)$/)?.[1] ?? "?"
}

// CheckSlots is the sidebar's line of check.sh runs holding this device's
// check slots (card #437): at most two full checks build at once, the rest
// wait their turn. Nothing shows while no run holds one.
export function CheckSlots() {
  const [slots, setSlots] = useState<CheckSlot[]>(() =>
    !inWails() && previewFlag("slots") !== null ? SAMPLE : []
  )
  useEffect(() => {
    if (!inWails()) return
    let live = true
    const read = () =>
      RosterService.CheckSlots()
        .then((s) => live && setSlots(s))
        .catch(() => {})
    read()
    const t = setInterval(read, POLL_MS)
    return () => {
      live = false
      clearInterval(t)
    }
  }, [])
  if (!slots.length) return null
  return (
    <div className="px-2.5 pt-2 text-xs text-muted-foreground">
      <div className="pb-0.5 font-medium">Check slots</div>
      <ul>
        {slots.map((s) => (
          <li
            key={s.resource}
            className="truncate"
            title={`${s.resource}: held by ${s.run} (scripts/check.sh)`}
          >
            <span className="tabular-nums">{slotNumber(s.resource)}</span>
            {" · "}
            {s.run}
          </li>
        ))}
      </ul>
    </div>
  )
}
