// Huddles gateway (#654): voice calls in a channel or a person DM.
//
// The Rust side (crates/window/src/call.rs) sends the whole picture as
// call:state on every change: the huddle this device is in, the ones
// ringing here, and the ones on in conversations it can see. People come
// named, never as keys. This file keeps that picture for every component
// (one store, like voice.ts's), starts, joins and leaves huddles, and holds
// the pure parts the components need, so they are tested without a window.

import { CallService, Events } from "./host"
import { inWails } from "./irori"
import { nameOr, unknownPerson } from "./names"
import { previewFlag } from "./preview"
import { allowMic } from "./voice"
import type {
  CallLiveView,
  CallMineView,
  CallPeerView,
  CallRingView,
  CallView,
} from "../gen/bindings"

export type { CallLiveView, CallMineView, CallPeerView, CallRingView, CallView }

export const EventCallState = "call:state"

export const noCall: CallView = { mine: null, ringing: [], live: [] }

// ---- the shared picture ---------------------------------------------------------

let shared: CallView = noCall
const subs = new Set<() => void>()

function set(next: CallView) {
  shared = next
  for (const f of subs) f()
}

export function sharedCall(): CallView {
  return shared
}

export function subscribeCall(f: () => void): () => void {
  subs.add(f)
  return () => subs.delete(f)
}

let started = false

// startCallState follows call:state; App calls it once. The first read
// waits for the subscription, so no change falls between the two. In a
// plain browser (the design preview) the stand-in below fills it from
// ?call=.
export function startCallState(): () => void {
  if (started) return () => {}
  started = true
  if (!inWails()) {
    previewSeed(previewFlag("call"))
    return () => {
      started = false
      previewTick(false)
    }
  }
  const off = Events.On<CallView>(
    EventCallState,
    (e) => set(e.data),
    () =>
      void CallService.State()
        .then(set)
        .catch(() => {})
  )
  return () => {
    started = false
    off()
  }
}

// Whoever holds the mic for something else (dictation's MicButton) lets go
// of it here before a huddle opens it, so the two never hold it at once.
const micHolders = new Set<() => Promise<void> | void>()

// onHuddleMic registers `release`, run before a huddle takes the mic.
export function onHuddleMic(release: () => Promise<void> | void): () => void {
  micHolders.add(release)
  return () => {
    micHolders.delete(release)
  }
}

async function releaseMic(): Promise<void> {
  await Promise.all(
    [...micHolders].map((f) => Promise.resolve(f()).catch(() => {}))
  )
}

// startHuddle starts one in channel or person DM `convId` (`name` is what
// the preview's stand-in calls it). It opens the mic, so dictation lets go
// of it and Android's prompt come first.
export async function startHuddle(convId: string, name = ""): Promise<void> {
  await releaseMic()
  if (!inWails()) return previewStart(convId, name)
  await allowMic()
  await CallService.Start(convId)
}

export async function joinHuddle(callId: string): Promise<void> {
  await releaseMic()
  if (!inWails()) return previewJoin(callId)
  await allowMic()
  await CallService.Join(callId)
}

export async function leaveHuddle(): Promise<void> {
  if (!inWails()) return previewLeave()
  await CallService.Leave()
}

// setMuted shows the change at once; call:state confirms it.
export async function setMuted(on: boolean): Promise<void> {
  if (shared.mine) set({ ...shared, mine: { ...shared.mine, muted: on } })
  if (!inWails()) return
  await CallService.Mute(on)
}

export async function dismissRing(callId: string): Promise<void> {
  set({ ...shared, ringing: shared.ringing.filter((r) => r.callId !== callId) })
  if (!inWails()) return
  await CallService.Dismiss(callId)
}

// ---- pure parts -----------------------------------------------------------------

export type Health = "good" | "fair" | "poor" | "off"

type Link = Pick<CallPeerView, "phase" | "path" | "rttMs" | "lossPct">

// healthOf is how well a device reaches this one: off until it's live,
// poor past 5% lost or 300 ms, fair past 1% or 150 ms.
export function healthOf(p: Link): Health {
  if (p.phase !== "live") return "off"
  // A time or loss Rust couldn't say (NaN) comes as null.
  const [rtt, loss] = [p.rttMs ?? 0, p.lossPct ?? 0]
  if (loss >= 5 || rtt >= 300) return "poor"
  if (loss >= 1 || rtt >= 150) return "fair"
  return "good"
}

// healthWord says the link in words: "direct · 26 ms", "relay · 120 ms ·
// 3% lost", "connecting…".
export function healthWord(p: Link): string {
  if (p.phase === "connecting") return "connecting…"
  if (p.phase === "gone") return "left"
  const [rtt, loss] = [p.rttMs ?? 0, p.lossPct ?? 0]
  const parts = [p.path || "connected"]
  if (rtt > 0) parts.push(`${Math.round(rtt)} ms`)
  if (loss >= 1) parts.push(`${Math.round(loss)}% lost`)
  return parts.join(" · ")
}

// durationText is how long since `sinceMs`, as a call timer: "4:12",
// "1:02:09".
export function durationText(sinceMs: number, nowMs: number): string {
  const s = Math.max(0, Math.floor((nowMs - sinceMs) / 1000))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const ss = String(s % 60).padStart(2, "0")
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`
}

// lengthText is how long a huddle ran, for its end post: "under a
// minute", "18 min", "1 h 5 min".
export function lengthText(ms: number): string {
  const min = Math.floor(ms / 60_000)
  if (min < 1) return "under a minute"
  if (min < 60) return `${min} min`
  const h = Math.floor(min / 60)
  return min % 60 ? `${h} h ${min % 60} min` : `${h} h`
}

// personWord is a name as a sentence starts it: the Core's "you" is "You",
// and a key or nothing is "another person" (card #478).
export function personWord(name: string): string {
  return name === "you" ? "You" : nameOr(name, unknownPerson)
}

// namesText joins names as a sentence does: "You, Spencer and Aiko".
export function namesText(names: readonly string[]): string {
  const n = names.map(personWord)
  if (n.length <= 1) return n[0] ?? ""
  return `${n.slice(0, -1).join(", ")} and ${n[n.length - 1]}`
}

// liveIn is the huddle on in `convId`, if one is.
export function liveIn(s: CallView, convId: string): CallLiveView | undefined {
  return s.live.find((l) => l.convId === convId)
}

// huddleNames is who is in `convId`'s huddle, each once, "You" first.
export function huddleNames(s: CallView, convId: string): string[] {
  const names = new Set<string>()
  if (inCallHere(s, convId)) names.add("You")
  for (const n of liveIn(s, convId)?.people ?? []) names.add(personWord(n))
  if (names.has("You")) {
    names.delete("You")
    return ["You", ...names]
  }
  return [...names]
}

// inCallHere is whether this device is in `convId`'s huddle.
export function inCallHere(s: CallView, convId: string): boolean {
  return s.mine?.convId === convId
}

// convLabel is a huddle's conversation as a line says it: "#suzuri", or
// the DM's person.
export function convLabel(c: { convKind: string; convName: string }): string {
  return c.convKind === "channel"
    ? `#${nameOr(c.convName, "channel")}`
    : personWord(c.convName)
}

// ---- the browser preview's stand-in ---------------------------------------------

// The preview has no calls (card #571's ?call=): there a stand-in fills the
// picture with sample people, and starting, joining, muting and leaving
// change it in memory, while speaking and the links move a little each
// tick so the dots and rings can be seen. The app never runs it.
//   ?call=live (and share): you're in #suzuri's huddle
//   ?call=start: Spencer's huddle is on in #netcode and rings here
export const previewPeers: CallPeerView[] = [
  {
    name: "Spencer",
    device: "",
    phase: "live",
    path: "direct",
    rttMs: 26,
    lossPct: 0,
    speaking: true,
    muted: false,
  },
  {
    name: "Aiko",
    device: "",
    phase: "live",
    path: "relay",
    rttMs: 120,
    lossPct: 1.4,
    speaking: false,
    muted: true,
  },
  {
    name: "you",
    device: "Pixel",
    phase: "connecting",
    path: "",
    rttMs: 0,
    lossPct: 0,
    speaking: false,
    muted: false,
  },
]

const previewNames: Record<string, string> = {
  "sample-ch-suzuri": "suzuri",
  "sample-ch-netcode": "netcode",
}

function previewMine(convId: string, name: string, ago: number): CallMineView {
  return {
    callId: `preview-call-${convId}`,
    convId,
    convName: name || previewNames[convId] || "",
    convKind: convId.includes("-dm-") ? "dm" : "channel",
    joinedAtMs: Date.now() - ago,
    muted: false,
    speaking: false,
    device: "MacBook Pro",
    peers: previewPeers,
    error: null,
  }
}

function previewSeed(flag: string | null) {
  if (flag === "live" || flag === "share") {
    set({
      mine: previewMine("sample-ch-suzuri", "", 252_000),
      ringing: [],
      live: [
        {
          callId: "preview-call-sample-ch-suzuri",
          convId: "sample-ch-suzuri",
          people: ["you", "Spencer", "Aiko"],
        },
      ],
    })
    previewTick(true)
  } else if (flag === "start") {
    set({
      mine: null,
      ringing: [
        {
          callId: "preview-call-sample-ch-netcode",
          convId: "sample-ch-netcode",
          convName: "netcode",
          convKind: "channel",
          by: "Spencer",
          atMs: Date.now() - 20_000,
        },
      ],
      live: [
        {
          callId: "preview-call-sample-ch-netcode",
          convId: "sample-ch-netcode",
          people: ["Spencer", "Aiko"],
        },
      ],
    })
  }
}

let ticker = 0

function previewTick(on: boolean) {
  window.clearInterval(ticker)
  ticker = 0
  if (!on) return
  let n = 0
  ticker = window.setInterval(() => {
    const m = shared.mine
    if (!m) return
    n++
    set({
      ...shared,
      mine: {
        ...m,
        speaking: !m.muted && n % 5 === 2,
        peers: m.peers.map((p, i) =>
          p.phase !== "live"
            ? n > 3
              ? { ...p, phase: "live", path: "direct", rttMs: 4 }
              : p
            : {
                ...p,
                speaking: !p.muted && (n + i) % 3 === 0,
                rttMs: Math.max(
                  1,
                  (p.rttMs ?? 0) + (((n * 7 + i * 3) % 9) - 4)
                ),
              }
        ),
      },
    })
  }, 700)
}

function previewStart(convId: string, name: string) {
  set({
    mine: previewMine(convId, name, 0),
    ringing: shared.ringing.filter((r) => r.convId !== convId),
    live: [
      ...shared.live.filter((l) => l.convId !== convId),
      { callId: `preview-call-${convId}`, convId, people: ["you"] },
    ],
  })
  previewTick(true)
}

function previewJoin(callId: string) {
  const l = shared.live.find((x) => x.callId === callId)
  const r = shared.ringing.find((x) => x.callId === callId)
  const convId = l?.convId ?? r?.convId
  if (!convId) return
  set({
    mine: { ...previewMine(convId, r?.convName ?? "", 0), callId },
    ringing: shared.ringing.filter((x) => x.callId !== callId),
    live: shared.live.map((x) =>
      x.callId === callId ? { ...x, people: ["you", ...x.people] } : x
    ),
  })
  previewTick(true)
}

function previewLeave() {
  const m = shared.mine
  previewTick(false)
  set({
    mine: null,
    ringing: shared.ringing,
    live: shared.live
      .map((l) =>
        l.callId === m?.callId
          ? { ...l, people: l.people.filter((p) => p !== "you") }
          : l
      )
      .filter((l) => l.people.length > 0),
  })
}
