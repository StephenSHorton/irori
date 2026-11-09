import { useEffect, useState } from "react"

import { RosterService } from "@/lib/host"
import { inWails } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import type { SlotHeld } from "@/gen/bindings"

// The browser preview's slots (?slots, lib/preview.ts).
const SAMPLE: SlotHeld[] = [
  { pool: "test-server", run: "fix-login", slot: 1, limit: 2, until: "" },
  { pool: "test-server", run: "add-export", slot: 2, limit: 2, until: "" },
  { pool: "build", run: "release", slot: 1, limit: 1, until: "" },
]

// How often the sidebar re-reads the slots: a run renews its lease every
// so often, so one that ended shows within a poll.
const POLL_MS = 10_000

// slotLine is a held slot as a row reads: "1 of 2".
export function slotLine(s: Pick<SlotHeld, "slot" | "limit">): string {
  return `${s.slot} of ${s.limit}`
}

// Slots is the sidebar's list of slots runs on this device hold now, in
// any pool (card #1167, POST /v1/slots). Nothing shows while none is held.
export function Slots() {
  const [slots, setSlots] = useState<SlotHeld[]>(() =>
    !inWails() && previewFlag("slots") !== null ? SAMPLE : []
  )
  useEffect(() => {
    if (!inWails()) return
    let live = true
    const read = () =>
      RosterService.Slots()
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
      <div className="pb-0.5 font-medium">Slots</div>
      <ul>
        {slots.map((s) => (
          <li
            key={`${s.pool}/${s.slot}`}
            className="truncate"
            title={`${s.pool}: slot ${slotLine(s)}, held by ${s.run}`}
          >
            {s.pool}
            {" · "}
            {s.run} <span className="tabular-nums">{slotLine(s)}</span>
          </li>
        ))}
      </ul>
    </div>
  )
}
