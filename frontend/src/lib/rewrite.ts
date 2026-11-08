// Rewrite in the Start a thread box (card #981): the office assistant
// rewords the draft when the person presses the button. The wording shows
// under the box; the draft is untouched until they take it.
//
// State lives here by draft key, like lib/drafts.ts, so an answer that
// lands while the box is off screen (or a later Undo) still finds it.

import { useSyncExternalStore } from "react"

import { draft } from "./drafts"
import { AssistantService } from "./host"
import { errorText } from "./talk"
import { inWails } from "./irori"
import type { Mention } from "./mentions"
import { previewFlag } from "./preview"

export type Suggestion = { from: string; text: string }
export type UndoTo = { text: string; mentions: Mention[] }
export type RewriteNote = { text: string; error?: boolean }

export type RewriteState = {
  running: boolean
  suggestion: Suggestion | null
  undo: UndoTo | null
  note: RewriteNote | null
}

const idle: RewriteState = {
  running: false,
  suggestion: null,
  undo: null,
  note: null,
}

export const UnchangedNote = "It already reads clearly: nothing to change."
export const StaleNote =
  "You kept typing, so that rewrite was dropped. Press Rewrite for the new text."

export type Landing = "stale" | "unchanged" | "suggestion"

// landing says what an answer for the draft `from` is, now that the box
// holds `current`. Stale first: an answer for older text is never shown.
export function landing(
  from: string,
  current: string,
  answer: string
): Landing {
  if (current !== from) return "stale"
  return answer.trim() === from.trim() ? "unchanged" : "suggestion"
}

const states = new Map<string, RewriteState>()
const runs = new Map<string, number>()
const listeners = new Set<() => void>()

function get(key: string): RewriteState {
  return states.get(key) ?? idle
}

function put(key: string, next: Partial<RewriteState>) {
  const s = { ...get(key), ...next }
  if (!s.running && !s.suggestion && !s.undo && !s.note) states.delete(key)
  else states.set(key, s)
  listeners.forEach((l) => l())
}

function subscribe(l: () => void) {
  listeners.add(l)
  return () => {
    listeners.delete(l)
  }
}

export const peekRewrite = get
export const currentDraft = draft

export function useRewrite(key: string): RewriteState {
  return useSyncExternalStore(subscribe, () => get(key))
}

// Ends any run's say over the box: its answer lands nowhere.
function forget(key: string) {
  runs.set(key, (runs.get(key) ?? 0) + 1)
}

export type Ask = (text: string, mentions: string[]) => Promise<string>

// askAssistant is the Rust side's turn; outside the app a stand-in, with
// ?rewrite=fail, signedout, same, lost or slow for those paths.
export const askAssistant: Ask = async (text, mentions) => {
  if (inWails() && previewFlag("rewrite") === null)
    return AssistantService.Rewrite(text, mentions)
  const how = previewFlag("rewrite")
  await new Promise((r) => setTimeout(r, how === "slow" ? 4000 : 700))
  if (how === "fail") throw new Error("The assistant took too long. Try again.")
  if (how === "signedout")
    throw new Error(
      "The assistant needs Claude Code signed in on this computer."
    )
  if (how === "lost")
    throw new Error("The rewrite changed #12, so it wasn't shown. Try again.")
  if (how === "same") return text
  const t = text.trim()
  return `Could you please help with this: ${t[0].toLowerCase()}${t.slice(1)}`
}

// startRewrite asks for wording for the box's text and, when it lands,
// shows it only if the box still holds that text.
export async function startRewrite(
  key: string,
  text: string,
  mentions: readonly Mention[],
  ask: Ask = askAssistant
) {
  if (get(key).running || !text.trim()) return
  forget(key)
  const run = runs.get(key)
  put(key, { running: true, suggestion: null, note: null })
  let answer: string
  try {
    answer = await ask(
      text,
      mentions.map((m) => m.name)
    )
  } catch (err) {
    if (runs.get(key) !== run) return
    // A failure is said even after typing on: nothing of theirs moved.
    put(key, { running: false, note: { text: errorText(err), error: true } })
    return
  }
  if (runs.get(key) !== run) return
  const what = landing(text, draft(key), answer)
  put(key, {
    running: false,
    suggestion: what === "suggestion" ? { from: text, text: answer } : null,
    note:
      what === "unchanged"
        ? { text: UnchangedNote }
        : what === "stale"
          ? { text: StaleNote }
          : null,
  })
}

// rewriteUsed records taking the wording. A second Use this keeps the
// first Undo, so Undo always goes back to what the person wrote.
export function rewriteUsed(key: string, undo: UndoTo) {
  put(key, {
    suggestion: null,
    note: null,
    undo: get(key).undo ?? undo,
  })
}

export function rewriteDismissed(key: string) {
  put(key, { suggestion: null })
}

// rewriteEdited is the person changing the text: the wording was for the
// old text and Undo ends. A run going on will find itself stale.
export function rewriteEdited(key: string) {
  if (get(key) !== idle) put(key, { suggestion: null, undo: null, note: null })
}

// rewriteReset is a send or an Undo: nothing is left to say, and an answer
// still coming lands nowhere.
export function rewriteReset(key: string) {
  forget(key)
  put(key, { running: false, suggestion: null, undo: null, note: null })
}
