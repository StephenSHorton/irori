// Huddles as places (card #1044; docs/design/call-agents.md §3): the list
// Talk shows and the data the huddle page reads. Each huddle is one row of
// `talk_huddles` (title, place, people, length, whether it's over); the
// page's lines are the transcript (lib/call.ts). Pure helpers sit up top so
// the tests feed them directly; the hook and the preview's stand-in follow.

import { useCallback, useEffect, useState } from "react"

import { subscribeCall, sharedCall, personWord, lengthText } from "./call"
import { TalkService } from "./host"
import { onChange } from "./changes"
import { timedRead } from "./window-log"
import { inWails } from "./irori"
import { errorText } from "./talk"
import type { TalkCanvasItem, TalkHuddle } from "../gen/bindings"
import { sampleAdd, sampleAsk, sampleSet } from "./sample-huddle"

export type { TalkHuddle }

export const EventTalkChanged = "talk:changed"

// huddleName is what a huddle is called: its title, else where it was.
export function huddleName(h: {
  title: string
  convKind: string
  convName: string
}): string {
  const t = h.title.trim()
  if (t) return t
  return h.convKind === "channel"
    ? `Huddle in #${h.convName}`
    : `Huddle with ${personWord(h.convName)}`
}

// placeText is where a huddle was: "#suzuri", or "Spencer" for a DM.
export function placeText(h: { convKind: string; convName: string }): string {
  return h.convKind === "channel" ? `#${h.convName}` : personWord(h.convName)
}

// isLive is whether the huddle is on as this device can tell. Not knowing
// counts as live (call-agents.md §3: the after-call steps stay off).
export function isLive(h: { over: boolean; ended: boolean }): boolean {
  return !h.over && !h.ended
}

// byNewest orders huddles newest first, by their start (else their newest
// row), a live one before an older one that ended later.
export function byNewest(hs: readonly TalkHuddle[]): TalkHuddle[] {
  const at = (h: TalkHuddle) => h.startedAtMs ?? h.lastAtMs
  return [...hs].sort(
    (a, b) => at(b) - at(a) || a.callId.localeCompare(b.callId)
  )
}

// lengthOf is a huddle's length as a row says it: how long it ran, or
// "live" while it is on.
export function lengthOf(h: TalkHuddle): string {
  if (isLive(h)) return "live"
  if (h.ended) return lengthText(h.durationMs)
  // Over with no end mark (a crash): the span its rows cover, if known.
  const span = h.startedAtMs != null ? h.lastAtMs - h.startedAtMs : 0
  return span > 0 ? lengthText(span) : "ended"
}

// peopleText is who was in it: "You, Spencer and Aiko".
export function peopleText(names: readonly string[]): string {
  const n = names.map(personWord)
  if (n.length <= 1) return n[0] ?? ""
  return `${n.slice(0, -1).join(", ")} and ${n[n.length - 1]}`
}

// ---- reading and writing ----------------------------------------------------------

export async function loadHuddles(): Promise<TalkHuddle[]> {
  if (!inWails()) return previewHuddles()
  return byNewest(await timedRead("huddles", () => TalkService.Huddles()))
}

// setHuddleTitle sets the title as the person; the title now shown comes
// back. A space that isn't ready refuses it, in words.
export async function setHuddleTitle(
  callId: string,
  title: string
): Promise<string> {
  if (!inWails()) return previewSetTitle(callId, title)
  return TalkService.CallTitleSet(callId, title)
}

// useHuddles keeps the list: read on mount, again on every Talk change and
// whenever the huddles on now change.
export function useHuddles(): {
  huddles: TalkHuddle[]
  loading: boolean
  error: string | null
} {
  const [state, setState] = useState<{
    huddles: TalkHuddle[]
    loading: boolean
    error: string | null
  }>({ huddles: [], loading: true, error: null })
  const reload = useCallback(async () => {
    try {
      const huddles = await loadHuddles()
      setState({ huddles, loading: false, error: null })
    } catch (err) {
      setState((s) => ({ ...s, loading: false, error: errorText(err) }))
    }
  }, [])
  useEffect(() => {
    void reload()
    const offCall = subscribeCall(() => void reload())
    previewListeners.add(reload)
    const off = inWails()
      ? onChange(EventTalkChanged, () => void reload())
      : () => {}
    return () => {
      offCall()
      previewListeners.delete(reload)
      off()
    }
  }, [reload])
  return state
}

// ---- the browser preview's stand-in -----------------------------------------------

const previewListeners = new Set<() => Promise<void>>()
const previewTitles = new Map<string, string>()

const MIN = 60_000

// Two huddles that came and went: #suzuri's (card #656's sample, no title)
// and #netcode's, titled.
function previewEnded(now: number): TalkHuddle[] {
  const base = {
    titleByAssistant: false,
    ended: true,
    over: true,
  }
  return [
    {
      ...base,
      callId: "sample-call-1",
      convId: "sample-ch-suzuri",
      convName: "suzuri",
      convKind: "channel",
      title: "",
      startedAtMs: now - 90 * MIN,
      lastAtMs: now - 72 * MIN,
      durationMs: 18 * MIN,
      people: ["you", "Spencer"],
      lines: 6,
    },
    {
      ...base,
      callId: "sample-call-2",
      convId: "sample-ch-netcode",
      convName: "netcode",
      convKind: "channel",
      title: "Signing plan for 0.10.0",
      startedAtMs: now - 26 * 60 * MIN,
      lastAtMs: now - 25 * 60 * MIN,
      durationMs: 41 * MIN,
      people: ["you", "Spencer", "Aiko"],
      lines: 22,
    },
    {
      ...base,
      callId: "sample-call-3",
      convId: "sample-dm-spencer",
      convName: "Spencer",
      convKind: "dm",
      title: "",
      startedAtMs: now - 3 * 24 * 60 * MIN,
      lastAtMs: now - 3 * 24 * 60 * MIN + 7 * MIN,
      durationMs: 7 * MIN,
      people: ["you", "Spencer"],
      lines: 0,
    },
  ]
}

// previewHuddles is the sample list: the ended ones, and each huddle the
// stand-in has on (lib/call.ts's previewSeed), live.
function previewHuddles(): TalkHuddle[] {
  const now = Date.now()
  const live = sharedCall().live.map((l): TalkHuddle => ({
    callId: l.callId,
    convId: l.convId,
    convName: l.convId.split("-").pop() ?? "",
    convKind: l.convId.includes("-dm-") ? "dm" : "channel",
    title: "",
    titleByAssistant: false,
    startedAtMs: now - 6 * MIN,
    lastAtMs: now,
    ended: false,
    durationMs: 0,
    over: false,
    people: l.people,
    lines: 0,
  }))
  const all = [...live, ...previewEnded(now)]
  return byNewest(
    all.map((h) => {
      const t = previewTitles.get(h.callId)
      return t === undefined ? h : { ...h, title: t }
    })
  )
}

async function previewSetTitle(callId: string, title: string): Promise<string> {
  const t = title.trim()
  previewTitles.set(callId, t)
  await Promise.all([...previewListeners].map((f) => f()))
  return t
}

// previewTitleOf is a title the preview's person set, for the transcript.
export function previewTitleOf(callId: string): string | undefined {
  return previewTitles.get(callId)
}

// ---- the canvas: a person's writes (card #1052) ---------------------------------

// addCanvasItem adds an item of the person's own: a note, question,
// decision or task. A space that isn't ready refuses it, in words.
export async function addCanvasItem(
  callId: string,
  kind: string,
  text: string
): Promise<TalkCanvasItem> {
  if (!inWails()) return sampleAdd(callId, kind, text)
  return TalkService.HuddleItemAdd(callId, kind, text)
}

// setCanvasItem changes an item: its text (your own), its status
// (`settled`, `dropped`, `open`) or its place. After the call (card #1054)
// also `carded` with the ref of the card it became.
export async function setCanvasItem(
  callId: string,
  itemId: string,
  change: {
    text?: string
    status?: string
    x?: number
    y?: number
    card?: string
  }
): Promise<TalkCanvasItem> {
  const c = (v: number | string | undefined) => (v === undefined ? null : v)
  if (!inWails())
    return sampleSet(
      callId,
      itemId,
      c(change.text) as string | null,
      c(change.status) as string | null,
      c(change.x) as number | null,
      c(change.y) as number | null,
      c(change.card) as string | null
    )
  return TalkService.HuddleItemSet(
    callId,
    itemId,
    c(change.text) as string | null,
    c(change.status) as string | null,
    c(change.x) as number | null,
    c(change.y) as number | null,
    c(change.card) as string | null
  )
}

// askAssistant puts a question to the viewer's own assistant. It comes
// back as an item of theirs; the answer arrives as another.
export async function askAssistant(
  callId: string,
  text: string
): Promise<TalkCanvasItem> {
  if (!inWails()) return sampleAsk(callId, text)
  return TalkService.HuddleAsk(callId, text)
}
