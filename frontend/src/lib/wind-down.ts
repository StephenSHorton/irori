// Wind down's summary (card #484): one line on Home for a whole run, in
// place of a DM per agent.

import type { WindDownRow, WindDownStatus } from "@/gen/bindings"

const agents = (n: number) => `${n} agent${n === 1 ? "" : "s"}`

// windDownLine is Home's line for a run: how many agents it reached and
// how many answered "wound down". Empty when there is no run.
export function windDownLine(st: WindDownStatus): string {
  if (!st.id || st.rows.length === 0) return ""
  const n = st.rows.length
  const done = st.rows.filter((r) => r.state === "done").length
  if (!st.finished) return `Winding down ${agents(n)}, ${done} confirmed so far`
  const tail = st.paused ? " · Pause all is on" : ""
  return `Wound down ${agents(n)}, ${done} confirmed${tail}`
}

// windDownSample is the browser preview's finished run (`?winddown`): 43
// agents, 23 of them answered.
export function windDownSample(): WindDownStatus | null {
  if (typeof window === "undefined" || !new URLSearchParams(window.location.search).has("winddown")) {
    return null
  }
  const rows: WindDownRow[] = Array.from({ length: 43 }, (_, i) => {
    const done = i < 23
    return {
      key: `sample-device/sample-wd-${i + 1}`,
      name: `card-${440 + i}-1`,
      deviceId: "sample-device",
      device: "this Mac",
      deliveryId: `sample-wd-dlv-${i + 1}`,
      state: done ? "done" : "no_answer",
      reply: done ? `Wound down. Left #${440 + i} with a hand-off; no leases held.` : "",
      error: "",
    }
  })
  return { id: "wd_sample", scope: "this", pauseAfter: true, paused: true, finished: true, rows }
}
