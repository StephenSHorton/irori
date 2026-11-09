// Huddles gateway (#654): voice calls in a channel or a person DM.
//
// The Rust side (crates/window/src/call.rs) sends the whole picture as
// call:state on every change: the huddle this device is in, the ones
// ringing here, and the ones on in conversations it can see. People come
// named, never as keys. This file keeps that picture for every component
// (one store, like voice.ts's), starts, joins and leaves huddles, and holds
// the pure parts the components need, so they are tested without a window.
// Transcripts (#656): the switch, the notice before the mic opens, live
// captions and the transcript an ended huddle leaves.

import { CallService, Events, ShareService } from "./host"
import { timedRead } from "./window-log"
import { inWails } from "./irori"
import { nameOr, unknownPerson } from "./names"
import { previewFlag } from "./preview"
import type { CallShareView, ShareSourceView } from "./share"
import { previewTitleOf } from "./huddles"
import { sampleAssistants, sampleCanvas } from "./sample-huddle"
import { allowMic, loadVoiceStatus } from "./voice"
import type {
  CallCaptionView,
  CallLiveView,
  CallMineView,
  CallPeerView,
  CallRingView,
  CallView,
  TalkTranscript,
  TalkTranscriptLine,
} from "../gen/bindings"

export type {
  CallCaptionView,
  CallLiveView,
  CallMineView,
  CallPeerView,
  CallRingView,
  CallView,
  TalkTranscript,
  TalkTranscriptLine,
}

export const EventCallState = "call:state"

// The switch is on unless the person turned it off (decision 52).
export const noCall: CallView = {
  mine: null,
  ringing: [],
  live: [],
  transcribeOn: true,
  // Unknown until the app says; the notice asks fresh anyway.
  canTranscribe: false,
}

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
    // The preview's device can write down.
    set({ ...shared, canTranscribe: true })
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

// setMuted shows the change at once; call:state confirms it. Unmuting
// while deafened undeafens too: the mic was off for the deafen, and the
// person asked for it on.
export async function setMuted(on: boolean): Promise<void> {
  const deafened = !on && !!shared.mine?.deafened
  if (shared.mine)
    set({
      ...shared,
      mine: {
        ...shared.mine,
        muted: on,
        deafened: deafened ? false : shared.mine.deafened,
      },
    })
  if (!inWails()) return
  if (deafened) await CallService.Deafen(false)
  await CallService.Mute(on)
}

// setDeafened turns the speaker and the mic off (or back) at once;
// call:state confirms it. Undeafen keeps the mute it had.
export async function setDeafened(on: boolean): Promise<void> {
  if (shared.mine)
    set({
      ...shared,
      mine: {
        ...shared.mine,
        deafened: on,
        speaking: on ? false : shared.mine.speaking,
      },
    })
  if (!inWails()) return
  await CallService.Deafen(on)
}

// micName is the mic a huddle would open, for the start sheet ("" when it
// can't say). It opens nothing.
export async function micName(): Promise<string> {
  if (!inWails()) return "MacBook Pro Microphone"
  return CallService.Mic()
}

export async function dismissRing(callId: string): Promise<void> {
  set({ ...shared, ringing: shared.ringing.filter((r) => r.callId !== callId) })
  if (!inWails()) return
  await CallService.Dismiss(callId)
}

// ---- transcripts (#656) ---------------------------------------------------------

// loadTranscribe reads "Transcribe what I say in calls" (on in the
// preview).
export async function loadTranscribe(): Promise<boolean> {
  if (!inWails()) return shared.transcribeOn
  const on = await CallService.Transcribe()
  if (on !== shared.transcribeOn) set({ ...shared, transcribeOn: on })
  return on
}

// setTranscribe turns the switch on or off on all the person's devices; a
// call in progress starts or stops writing down at once. It shows at once
// and call:state confirms it.
export async function setTranscribe(on: boolean): Promise<void> {
  const m = shared.mine
  set({
    ...shared,
    transcribeOn: on,
    // The preview's huddle stops or starts writing down, as the app's does.
    mine: m
      ? {
          ...m,
          transcribeOn: on,
          transcribing: inWails() ? m.transcribing : on && m.canTranscribe,
        }
      : null,
  })
  if (!inWails()) return
  await CallService.TranscribeSet(on)
}

// canTranscribeHere is whether this device could write down its person's
// voice before a huddle starts: the app's own answer (CallView's
// canTranscribe: a speech engine and its model), read fresh. If that read
// fails, the voice status says the same thing. The preview says yes.
export async function canTranscribeHere(): Promise<boolean> {
  if (!inWails()) return true
  try {
    return (await CallService.State()).canTranscribe
  } catch {
    return canTranscribeWith((await loadVoiceStatus()).state)
  }
}

// agentsReady is whether a huddle in `convId` reaches agents live (card
// #1047); false in a space with a device too old for huddle rows. The
// preview says yes.
export async function agentsReady(convId: string): Promise<boolean> {
  if (!inWails()) return true
  return CallService.AgentsReady(convId)
}

// canTranscribeWith is canTranscribeHere for a voice status: no engine or
// no model can't; a mic that's denied still has the model.
export function canTranscribeWith(voiceState: string): boolean {
  return voiceState !== "unsupported" && voiceState !== "needs-model"
}

// transcriptNotice is what the window says before the mic opens: who reads
// what you say, or that your voice isn't written down and why. Present
// tense (call-agents.md section 10): agents hear the huddle. `agentsOff` is
// a channel of a space where some device is too old for huddle rows: the
// lines reach it only at the end, and agents and the canvas are off.
export function transcriptNotice(c: {
  convKind: string
  convName: string
  transcribeOn: boolean
  canTranscribe: boolean
  agentsOff?: boolean
}): string {
  const place = convLabel(c)
  const personal = c.convKind === "personal"
  // Your own voice not written down doesn't stop the others' being.
  const others = personal
    ? ""
    : c.convKind === "channel"
      ? ` Others in the huddle may still be written down in ${place}.`
      : " The other person may still be written down."
  if (!c.canTranscribe)
    return `Your voice isn't transcribed on this device.${others}`
  if (!c.transcribeOn)
    return `Your voice isn't transcribed (Settings > App).${others}`
  const said = "What you say is written down as you speak"
  if (personal)
    return `Agents hear this huddle. ${said}, for you and your agents.`
  if (c.convKind === "channel") {
    if (c.agentsOff)
      return `Agents and the canvas are off: someone's Irori in this space needs updating. What you say is written down in ${place} at the end, for its members and their agents.`
    return `Agents hear this huddle. ${said}, for ${place}'s members and all their agents.`
  }
  return `Agents hear this huddle. ${said}, for the two of you and your agents.`
}

// transcriptOn is whether anyone in the huddle is writing down: the
// "Transcript on" badge.
export function transcriptOn(m: CallMineView): boolean {
  return (
    m.transcribing ||
    m.peers.some((p) => p.phase !== "gone" && p.transcribing === true)
  )
}

// whoCanRead says who reads a huddle's transcript.
// A person DM's lines carry `agents` when said under the notice that lets
// agents read them; older ones stay with the two people (card #1047).
export function whoCanRead(t: {
  convKind: string
  convName: string
  agents?: boolean
}): string {
  if (t.convKind === "channel")
    return `Who can read it: the people and agents of ${convLabel(t)}.`
  if (t.convKind === "personal") return "Who can read it: you and your agents."
  return t.agents
    ? "Who can read it: the two of you and your agents."
    : "Who can read it: the two of you."
}

// clockText is a line's time of day: "10:02".
export function clockText(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  })
}

// findLines is the transcript's lines that have every word of `q` in
// what was said or who said it, any case.
export function findLines(
  lines: readonly TalkTranscriptLine[],
  q: string
): TalkTranscriptLine[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return [...lines]
  return lines.filter((l) => {
    const hay = `${personWord(l.name)} ${l.text}`.toLowerCase()
    return words.every((w) => hay.includes(w))
  })
}

// loadTranscript reads an ended (or running) huddle's transcript.
export async function loadTranscript(callId: string): Promise<TalkTranscript> {
  if (!inWails()) return previewTranscript(callId)
  return timedRead("transcript", () => CallService.Transcript(callId))
}

// ---- screen share (#658) --------------------------------------------------------

// shareSources is what this device can share: its displays and windows.
// It can fail in plain words (a Mac without the Screen Recording grant).
export async function shareSources(): Promise<ShareSourceView[]> {
  if (!inWails()) return previewSources
  return ShareService.Sources()
}

// startShare shares `sourceId` in the huddle you're in; anyone else's share
// stops (they're told). call:state shows it.
export async function startShare(sourceId: string): Promise<void> {
  if (!inWails()) return previewShare(sourceId)
  await ShareService.Start(sourceId)
}

export async function stopShare(): Promise<void> {
  if (!inWails()) return previewShare("")
  await ShareService.Stop()
}

// dismissShareNote clears the share's note line ("Aiko started sharing, so
// yours stopped") at once; call:state confirms it.
export async function dismissShareNote(): Promise<void> {
  if (shared.mine?.shareNote)
    set({ ...shared, mine: { ...shared.mine, shareNote: null } })
  if (!inWails()) return
  await ShareService.NoteSeen()
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
  return c.convKind === "channel" || c.convKind === "personal"
    ? `#${nameOr(c.convName, "channel")}`
    : personWord(c.convName)
}

// ---- the browser preview's stand-in ---------------------------------------------

// The preview has no calls (card #571's ?call=): there a stand-in fills the
// picture with sample people, and starting, joining, muting and leaving
// change it in memory, while speaking and the links move a little each
// tick so the dots and rings can be seen. The app never runs it.
//   ?call=live: you're in #suzuri's huddle
//   ?call=share: and Spencer shares a window there (#658); sharing: you do
//   ?call=start: Spencer's huddle is on in #netcode and rings here
//   ?call=sheet: the open channel's Huddle start sheet (conversation.tsx)
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
    transcribing: true,
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
    // Aiko turned the switch off.
    transcribing: false,
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
    transcribing: null,
  },
]

const previewNames: Record<string, string> = {
  "sample-ch-suzuri": "suzuri",
  "sample-ch-netcode": "netcode",
}

// The share ?call=share starts with: Spencer's terminal.
export const previewTheirShare: CallShareView = {
  shareId: "preview-share-1",
  mine: false,
  name: "Spencer",
  device: "",
  kind: "window",
  title: "Windows Terminal",
  app: "Windows Terminal",
  // As the recorded fixture (public/preview/share-fixture.bin).
  width: 1280,
  height: 720,
  fps: 10,
  kbps: 1800,
  path: "direct",
}

const previewSources: ShareSourceView[] = [
  {
    id: "preview-win-1",
    kind: "window",
    title: "irori · #658 share stage",
    app: "Visual Studio Code",
    width: 1680,
    height: 1050,
  },
  {
    id: "preview-display-1",
    kind: "display",
    title: "Built-in Retina Display",
    app: "",
    width: 3024,
    height: 1964,
  },
  {
    id: "preview-win-2",
    kind: "window",
    title: "Terminal: spike-call",
    app: "Terminal",
    width: 1280,
    height: 800,
  },
]

// previewShare shares a sample source as yours ("" stops it).
function previewShare(sourceId: string) {
  const m = shared.mine
  if (!m) return
  const src = previewSources.find((s) => s.id === sourceId)
  const theirs = m.share && !m.share.mine ? m.share : null
  set({
    ...shared,
    mine: {
      ...m,
      share: src
        ? {
            shareId: `preview-share-${src.id}`,
            mine: true,
            name: "you",
            device: "",
            kind: src.kind,
            title: src.title,
            app: src.app,
            width: src.width,
            height: src.height,
            fps: 10,
            kbps: 1800,
            path: "",
          }
        : null,
      shareNote: theirs
        ? `${theirs.name}'s share stopped, and they were told.`
        : null,
    },
  })
}

function previewMine(convId: string, name: string, ago: number): CallMineView {
  return {
    share: null,
    canShare: true,
    shareNote: null,
    callId: `preview-call-${convId}`,
    convId,
    convName: name || previewNames[convId] || "",
    convKind: convId.includes("-dm-") ? "dm" : "channel",
    joinedAtMs: Date.now() - ago,
    muted: false,
    deafened: false,
    speaking: false,
    device: "MacBook Pro",
    peers: previewPeers,
    error: null,
    transcribing: shared.transcribeOn,
    canTranscribe: true,
    transcribeOn: shared.transcribeOn,
    captions: [],
  }
}

// What the preview's huddle hears, a line every few ticks.
const previewSaid: { name: string; text: string }[] = [
  {
    name: "you",
    text: "Okay, quick one: are we tagging 0.10.0 today or waiting on #8?",
  },
  {
    name: "Spencer",
    text: "#8 merged an hour ago. The only thing left is the PC transfer, it still needs the Firewall click.",
  },
  {
    name: "you",
    text: "Then let's tag at two, right after the click.",
  },
  {
    name: "Spencer",
    text: "Someone should rerun the latency test on the PC once it's idle, the spikes looked like load.",
  },
  { name: "you", text: "I'll ask heron to make a card for that rerun." },
]

// previewCaptions is the first `n` lines, as if said `n` lines ago.
function previewCaptions(n: number, now: number): CallCaptionView[] {
  return previewSaid.slice(0, n).map((l, i) => {
    const at = now - (n - i) * 9_000
    return {
      name: l.name,
      mine: l.name === "you",
      text: l.text,
      atMs: at,
      endMs: at + 4_000,
    }
  })
}

// previewTranscript is the stand-in for a huddle's transcript
// (?call=transcript: #suzuri's earlier huddle). A huddle the stand-in has
// on (preview-call-…) is live, and gains a line every few seconds, so the
// huddle page can be seen following its transcript (card #1044).
function previewTranscript(callId: string): TalkTranscript {
  const live = callId.startsWith("preview-call-")
  const convId = live
    ? callId.slice("preview-call-".length)
    : callId === "sample-call-2"
      ? "sample-ch-netcode"
      : "sample-ch-suzuri"
  const convName = convId.split("-").pop() ?? ""
  const started = live ? previewLiveStart : Date.now() - 90 * 60_000
  const said = [
    ...previewSaid,
    { name: "Spencer", text: "Sounds good. I'll be at the PC around two." },
  ]
  const n = live
    ? Math.min(said.length, 1 + Math.floor((Date.now() - started) / 6_000))
    : callId === "sample-call-3"
      ? 0
      : said.length
  return {
    callId,
    convId,
    convName,
    convKind: "channel",
    agents: true,
    startedAtMs: started,
    ended: !live,
    durationMs: live ? 0 : 18 * 60_000,
    title: previewTitleOf(callId) ?? "",
    titleByAssistant: false,
    over: !live,
    assistants: sampleAssistants(live),
    canvas: sampleCanvas(callId, Date.now()),
    lines: said.slice(0, n).map((l, i) => ({
      name: l.name,
      mine: l.name === "you",
      agents: true,
      atMs: started + 20_000 + i * 75_000,
      text: l.text,
    })),
  }
}

const previewLiveStart = Date.now() - 4_000

function previewSeed(flag: string | null) {
  if (flag === "live" || flag === "share" || flag === "sharing") {
    const mine = {
      ...previewMine("sample-ch-suzuri", "", 252_000),
      captions: previewCaptions(3, Date.now()),
    }
    set({
      transcribeOn: shared.transcribeOn,
      canTranscribe: shared.canTranscribe,
      mine:
        flag === "share"
          ? { ...mine, share: previewTheirShare }
          : flag === "sharing"
            ? {
                ...mine,
                share: {
                  ...previewTheirShare,
                  shareId: "preview-share-mine",
                  mine: true,
                  name: "you",
                  title: "irori · #658 share stage",
                  app: "Visual Studio Code",
                  path: "",
                },
              }
            : mine,
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
      transcribeOn: shared.transcribeOn,
      canTranscribe: shared.canTranscribe,
      mine: null,
      ringing: [
        {
          callId: "preview-call-sample-ch-netcode",
          convId: "sample-ch-netcode",
          convName: "netcode",
          convKind: "channel",
          by: "Spencer",
          byDevice: "",
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
        speaking: !m.muted && !m.deafened && n % 5 === 2,
        // A line is written down every few ticks, while anyone is.
        captions:
          n % 6 === 0 && transcriptOn(m)
            ? [...m.captions, captionAt(m.captions.length, Date.now())].slice(
                -40
              )
            : m.captions,
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

// captionAt is the preview's `i`th line, said just now.
function captionAt(i: number, now: number): CallCaptionView {
  const l = previewSaid[i % previewSaid.length]
  return {
    name: l.name,
    mine: l.name === "you",
    text: l.text,
    atMs: now - 4_000,
    endMs: now,
  }
}

function previewStart(convId: string, name: string) {
  set({
    transcribeOn: shared.transcribeOn,
    canTranscribe: shared.canTranscribe,
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
    transcribeOn: shared.transcribeOn,
    canTranscribe: shared.canTranscribe,
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
    transcribeOn: shared.transcribeOn,
    canTranscribe: shared.canTranscribe,
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
