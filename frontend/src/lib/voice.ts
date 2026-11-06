// Voice input gateway (card #240): the mic button on the text boxes.
//
// Speech is transcribed on this device by Whisper (crates/voice); the Rust
// side sends the words as voice:event and the model download as
// voice:download. This file holds the pure parts the button needs, so they
// are tested without a window:
// - compose: where the words go. Dictation starts at the caret (or replaces
//   the selection) and the text either side stays put; a space is added
//   where a word would otherwise run into the text around it.
// - step: one session's state from its events (settled phrases plus the
//   live partial; the partial is replaced, never appended).
// - the shared status and download progress (one model, one download, for
//   every box).
// Nothing here sends anything: the person edits the text and presses Send.

import { Events, VoiceService } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"
import { errorText } from "./talk"
import type { VoiceDownload, VoiceEvent, VoiceStatus } from "../gen/bindings"

export type { VoiceDownload, VoiceEvent, VoiceStatus }

export const EventVoice = "voice:event"
export const EventVoiceDownload = "voice:download"

// ---- composing text -----------------------------------------------------------

// Anchor is the text either side of where dictation started.
export type Anchor = { before: string; after: string }

// anchorAt splits the box's text at the selection; a selection is replaced.
export function anchorAt(value: string, start: number, end: number): Anchor {
  const a = Math.max(0, Math.min(start, value.length))
  const b = Math.max(a, Math.min(end, value.length))
  return { before: value.slice(0, a), after: value.slice(b) }
}

// joinWords puts b after a with one space between, unless either side is
// empty or a already ends in whitespace.
export function joinWords(a: string, b: string): string {
  if (!a) return b
  if (!b) return a
  return /\s$/.test(a) ? a + b : `${a} ${b}`
}

// compose is the box's text and caret for the words so far: the anchor's
// text either side, with the spoken words between. The caret sits right
// after the words.
export function compose(
  anchor: Anchor,
  spoken: string
): { text: string; caret: number } {
  if (!spoken) {
    return { text: anchor.before + anchor.after, caret: anchor.before.length }
  }
  const head = joinWords(anchor.before, spoken)
  // A word running into the text after gets a space; punctuation does not.
  const tail =
    anchor.after && !/^[\s.,;:!?)\]}'"]/.test(anchor.after)
      ? ` ${anchor.after}`
      : anchor.after
  return { text: head + tail, caret: head.length }
}

// ---- one session ----------------------------------------------------------------

export type Session = {
  id: number
  // The settled phrases, joined.
  committed: string
  // The phrase being spoken; replaced by each partial.
  partial: string
  ended: boolean
  error: { code: string; text: string } | null
}

export function newSession(id: number): Session {
  return { id, committed: "", partial: "", ended: false, error: null }
}

// spoken is everything said so far, as the box shows it.
export function spoken(s: Session): string {
  return joinWords(s.committed, s.partial)
}

// step applies one voice:event; events of another session leave s as it is.
export function step(s: Session, e: VoiceEvent): Session {
  if (e.session !== s.id || s.ended) return s
  switch (e.kind) {
    case "partial":
      return { ...s, partial: e.text.trim() }
    case "final":
      // An empty final (not understood) just drops the partial.
      return {
        ...s,
        committed: joinWords(s.committed, e.text.trim()),
        partial: "",
      }
    case "error":
      return { ...s, error: { code: e.code, text: e.text } }
    case "ended":
      // Whatever partial was left was never settled: drop it.
      return { ...s, partial: "", ended: true }
    default:
      return s
  }
}

// ---- messages -------------------------------------------------------------------

// settingsFix is whether an error code has an OS settings page to open.
export function settingsFix(code: string): boolean {
  return code === "mic-denied" || code === "no-mic"
}

// errorLine is the line the box shows for a failed session or start.
export function errorLine(code: string, text: string): string {
  if (text) return text
  switch (code) {
    case "mic-denied":
      return "Microphone access is off for Irori."
    case "no-mic":
      return "No microphone was found."
    case "no-model":
      return "The speech model isn't downloaded yet."
    default:
      return "Voice input stopped."
  }
}

// megabytes is a byte count as the download line shows it.
export function megabytes(n: number): string {
  return `${Math.round(n / 1_000_000)} MB`
}

// ---- the shared status and download ---------------------------------------------

export type Shared = {
  status: VoiceStatus | null
  // The model download, while it runs.
  download: VoiceDownload | null
  downloadError: string
}

let shared: Shared = { status: null, download: null, downloadError: "" }
const subs = new Set<() => void>()

function set(next: Partial<Shared>) {
  shared = { ...shared, ...next }
  for (const f of subs) f()
}

export function sharedVoice(): Shared {
  return shared
}

export function subscribeVoice(f: () => void): () => void {
  subs.add(f)
  return () => subs.delete(f)
}

const unsupported: VoiceStatus = {
  state: "unsupported",
  reason: "Voice input needs the Irori app.",
  fix: "",
  modelBytes: 0,
}

// loadVoiceStatus asks the Rust side once (and again after a download).
// In a plain browser (the design preview) there is no engine.
export async function loadVoiceStatus(force = false): Promise<VoiceStatus> {
  if (shared.status && !force) return shared.status
  if (!inWails()) {
    // ?voice=ready (or needs-model, denied) shows the mic in the preview.
    const flag = previewFlag("voice")
    const st: VoiceStatus =
      flag === "ready" || flag === "needs-model" || flag === "denied"
        ? {
            state: flag,
            reason:
              flag === "denied"
                ? "No microphone was found."
                : flag === "needs-model"
                  ? "Voice input needs a one-time download of the speech model (147 MB)."
                  : "",
            fix: flag === "denied" ? "mic-denied" : "",
            modelBytes: 147_964_211,
          }
        : unsupported
    set({ status: st })
    return st
  }
  try {
    const st = await VoiceService.Status()
    set({ status: st })
    return st
  } catch (err) {
    const st = { ...unsupported, reason: errorText(err) }
    set({ status: st })
    return st
  }
}

let downloading: Promise<boolean> | null = null

// downloadModel fetches the model once for every box; true when it's ready.
export function downloadModel(): Promise<boolean> {
  if (downloading) return downloading
  set({
    download: { done: 0, total: shared.status?.modelBytes ?? 0 },
    downloadError: "",
  })
  const off = Events.On<VoiceDownload>(EventVoiceDownload, (e) =>
    set({ download: e.data })
  )
  downloading = VoiceService.DownloadModel()
    .then(async () => {
      await loadVoiceStatus(true)
      return true
    })
    .catch((err: unknown) => {
      set({ downloadError: errorText(err) })
      return false
    })
    .finally(() => {
      off()
      downloading = null
      set({ download: null })
    })
  return downloading
}

// onVoiceEvent subscribes to every session's events.
export function onVoiceEvent(f: (e: VoiceEvent) => void): () => void {
  return Events.On<VoiceEvent>(EventVoice, (e) => f(e.data))
}

// Android (card #240): the native engine records with AAudio, which needs
// the RECORD_AUDIO runtime permission. Tauri's WebView asks Android for it
// when the page asks for the mic, so the first press opens the mic here
// once (Android shows its Allow prompt) and closes it again before the
// engine starts. Elsewhere the engine's own status and errors cover it.
const onAndroid =
  typeof navigator !== "undefined" && /Android/i.test(navigator.userAgent)
let micAllowed = false

export async function allowMic(): Promise<void> {
  if (!onAndroid || micAllowed || !navigator.mediaDevices?.getUserMedia) return
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
    for (const t of stream.getTracks()) t.stop()
    micAllowed = true
  } catch {
    throw new Error(
      "Allow Irori to use the microphone in Settings > Apps > Irori > Permissions, then try again."
    )
  }
}

export const startVoice = async () => {
  await allowMic()
  return VoiceService.Start()
}
export const stopVoice = (session: number) => VoiceService.Stop(session)
export const openVoiceSettings = (code: string) =>
  VoiceService.OpenSettings(code)
