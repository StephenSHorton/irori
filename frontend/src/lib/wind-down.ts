// Stop everywhere (cards #220, #484, #773, #787; "Wind down" in the code):
// an on/off switch for every agent on every machine. On holds agent starts
// everywhere and asks every agent to write its hand-off and leave its card;
// off lifts the holds it set, at any time, mid-run too. Stop here (one
// machine, Pause agents in the code) shows its run here too, with scope
// "device". Either Stop reaches On cards, Yours or both (decision 71,
// stop-who.ts); a Yours-only run holds nothing and ends with Call off.
// Home has one line for a whole run, in place of a DM per agent. Words:
// stop-labels.ts.

import { useCallback, useEffect, useState } from "react"

import type { WindDownRow, WindDownStatus } from "@/gen/bindings"
import { WindDownStatus as emptyStatus } from "@/gen/models"

import { Events, WindDownService } from "./host"
import { inWails, plainLine } from "./irori"
import { previewFlag } from "./preview"
import { stopLabels as L } from "./stop-labels"
import type { StopWho } from "./stop-who"

// EVENT_WIND_DOWN is windDown:changed: the switch or a run moved (also when
// another device flips it). Its payload is the status.
export const EVENT_WIND_DOWN = "windDown:changed"

// windDownLine is Home's line for a run: how many agents it reached and
// how many handed off. Empty when there is no run.
export function windDownLine(st: WindDownStatus): string {
  if (!st.id || st.rows.length === 0) return ""
  const n = st.rows.length
  const done = st.rows.filter((r) => r.state === "done").length
  // A Yours-only run (decision 71) stopped nothing and held nothing: it
  // asked, and Call off is its only end besides the answers.
  if (yoursOnly(st)) {
    const tail = st.off ? ` · ${L.calledOff}` : ""
    if (st.rows.every((r) => !r.deliveryId))
      return L.line.yoursNoneReached(n) + tail
    if (!st.finished) return L.line.yoursRunning(n, done) + tail
    return L.line.yoursDone(n, done) + tail
  }
  // Card #787: the switch's state after the run; an agent already handing
  // off may still answer after off, so "started again" goes on either
  // branch.
  const tail = st.off
    ? ` · ${L.startedAgain}`
    : st.on && st.finished
      ? ` · ${L.stoppedEverywhere}`
      : ""
  // Every send failed: the run never starts, so it never finishes either.
  if (st.rows.every((r) => !r.deliveryId)) return L.line.noneReached(n) + tail
  if (!st.finished) return L.line.running(n, done) + tail
  return L.line.done(n, done) + tail
}

// yoursOnly is a run that reached only the person's own sessions: no
// switch, Call off instead of Start (decision 71).
export function yoursOnly(st: WindDownStatus): boolean {
  return st.id !== "" && !!st.who && !st.who.cards && st.who.yours
}

// canCallOff is whether the dialog offers Call off (decision 71): a
// Yours-only run still going, or any Yours ask not reached yet
// (yoursWaiting), also one whose run a later Stop replaced or a restart
// forgot.
export function canCallOff(st: WindDownStatus): boolean {
  return (yoursOnly(st) && !st.off && !st.finished) || st.yoursWaiting > 0
}

// devicesText names the machines Stop everywhere holds: "this Mac and Stephen's PC".
export function devicesText(devices: string[]): string {
  return L.machines(devices.map(plainLine))
}

// windDownSample is the browser preview's Stop everywhere (`?winddown`): a
// finished run of 43 agents, 23 of them handed off, still on.
// `?winddown=running` is mid-run; `?winddown=off` was started again
// mid-run (its unreached agents "Called off"); `?winddown=on` is on from
// another machine, with no run here; `?winddown=device` is this machine's
// Stop here mid-run (with `&agentspause=on` its switch is on);
// `?winddown=yours` is a Yours-only run mid-run (decision 71).
export function windDownSample(): WindDownStatus | null {
  if (typeof window === "undefined") return null
  const flag = previewFlag("winddown")
  if (flag === null) return null
  const since = Date.now() - 42 * 60_000
  if (flag === "on") {
    return emptyStatus({
      on: true,
      since,
      devices: ["this Mac", "Stephen's PC"],
    })
  }
  const state = (i: number) => {
    if (i < 23) return "done"
    if (flag === "running" || flag === "device" || flag === "yours")
      return i < 30 ? "winding" : "waiting"
    if (flag === "off") return i < 30 ? "winding" : "canceled"
    return "no_answer"
  }
  const yours = flag === "yours"
  const rows: WindDownRow[] = Array.from({ length: 43 }, (_, i) => {
    const done = i < 23
    return {
      key: `sample-device/sample-wd-${i + 1}`,
      name: `card-${440 + i}-1`,
      deviceId: "sample-device",
      device: "this Mac",
      deliveryId: `sample-wd-dlv-${i + 1}`,
      state: state(i),
      reply: done
        ? `Handed off. Left #${440 + i} with a hand-off; no leases held.`
        : "",
      error: "",
      spawned: yours ? false : i % 4 !== 0,
    }
  })
  const off = flag === "off"
  // This machine's Stop here (card #773): a run here, Stop everywhere off.
  const device = flag === "device"
  const idle = off || device || yours
  return {
    on: !idle,
    since: idle ? 0 : since,
    devices: idle ? [] : ["this Mac", "Stephen's PC"],
    id: "wd_sample",
    scope: device ? "device" : "all",
    finished: flag !== "running" && !idle,
    off,
    who: { cards: !yours, yours: true },
    rows,
    yoursWaiting: 0,
  }
}

// turnedOff is the preview's Off: rows not reached yet close, the switch
// goes off (what the app does, card #787).
function turnedOff(st: WindDownStatus): WindDownStatus {
  return {
    ...st,
    on: false,
    since: 0,
    devices: [],
    off: st.id !== "" && !st.finished,
    rows: st.rows.map((r) =>
      r.state === "waiting" ? { ...r, state: "canceled" } : r
    ),
  }
}

// useWindDown is Stop everywhere's switch and this device's latest run (card
// #787): read on mount, every 10s, on focus and on windDown:changed (another
// device may flip it), like Pause agents. App holds the one copy.
export function useWindDown() {
  const [st, setSt] = useState<WindDownStatus>(() =>
    inWails() ? emptyStatus() : (windDownSample() ?? emptyStatus())
  )
  const [error, setError] = useState("")
  const [busy, setBusy] = useState("")
  const reload = useCallback(() => {
    if (!inWails()) return
    void WindDownService.Status().then(
      (v) => (setSt(v), setError("")),
      (e: Error) => setError(e.message)
    )
  }, [])
  useEffect(() => {
    if (!inWails()) return
    let live = true
    const load = () =>
      WindDownService.Status().then(
        (v) => live && (setSt(v), setError("")),
        (e: Error) => live && setError(e.message)
      )
    void load()
    const id = window.setInterval(load, 10_000)
    const off = Events.On<WindDownStatus>(EVENT_WIND_DOWN, (ev) => {
      if (live && ev.data) setSt(ev.data)
    })
    window.addEventListener("focus", load)
    return () => {
      live = false
      window.clearInterval(id)
      window.removeEventListener("focus", load)
      off()
    }
  }, [])
  // run does one of the switch's calls, busy as `what` until it answers.
  const run = useCallback(
    async (what: string, call: () => Promise<WindDownStatus | void>) => {
      setBusy(what)
      try {
        const v = await call()
        if (v) setSt(v)
        setError("")
        return true
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e))
        return false
      } finally {
        setBusy("")
      }
    },
    []
  )
  // Stop everywhere is every machine (card #787): no scope of machines.
  // `who` is which kind of agent (decision 71): On cards, Yours or both.
  const on = useCallback(
    (who?: StopWho) => {
      if (!inWails()) {
        // Yours alone holds nothing: the preview lights no switch.
        if (who && !who.cards) return Promise.resolve(true)
        setSt(
          emptyStatus({
            on: true,
            since: Date.now(),
            devices: ["this Mac", "Stephen's PC"],
          })
        )
        return Promise.resolve(true)
      }
      return run("on", () => WindDownService.All(who))
    },
    [run]
  )
  const off = useCallback(() => {
    if (!inWails()) {
      setSt(turnedOff)
      return Promise.resolve(true)
    }
    return run("off", () => WindDownService.Off())
  }, [run])
  // Call off ends a Yours-only run (decision 71): every Yours ask not
  // reached yet closes, and nothing else moves.
  const callOff = useCallback(() => {
    if (!inWails()) {
      setSt((s) => ({
        ...s,
        off: s.id !== "" && !s.finished,
        yoursWaiting: 0,
        rows: s.rows.map((r) =>
          r.state === "waiting" ? { ...r, state: "canceled" } : r
        ),
      }))
      return Promise.resolve(true)
    }
    return run("callOff", () => WindDownService.CallOff())
  }, [run])
  const stop = useCallback(
    (key: string) => run(key, () => WindDownService.Stop(key)),
    [run]
  )
  return { status: st, error, busy, reload, on, off, callOff, stop }
}

export type WindDown = ReturnType<typeof useWindDown>
