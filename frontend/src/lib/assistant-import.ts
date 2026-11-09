// Moving today's chat history (card #1105, docs/design/one-assistant.md §10).
//
// Until the chat was shared, each device kept its own history in
// localStorage (irori.assistant.history). Now the conversation is rows the
// host shares. The assistant's computer sends its own history to the host
// once; every other device keeps its old history, read-only, above a
// divider, until the person taps New chat.
//
// The host decides: it refuses unless this device is the assistant's
// computer and nothing was imported yet. The flag here only stops asking.

import { useSyncExternalStore } from "react"

import type { AssistantImportEntry } from "../gen/bindings"
import { type ChatEntry, parseHistory } from "./assistant"
import { AssistantService, Events } from "./host"

export const historyKey = "irori.assistant.history"
export const importedKey = "irori.assistant.imported"

// The host's refusal for a second import (crates/core assistant_chat).
const alreadyImported = "the history was already imported"

// The slice of assistant_home_status the import reads.
export type HomeNow = { here: boolean }

// toImportEntries is the history as the host takes it: only what its
// mapping reads (§10). Files are left behind: they were references to bytes
// the host keeps, never part of the conversation's rows.
export function toImportEntries(
  entries: readonly ChatEntry[]
): AssistantImportEntry[] {
  return entries.map((e) => ({
    role: e.role,
    text: e.text,
    note: e.note === true,
    changed: e.changed === true,
    posted: e.posted
      ? { id: e.posted.id, short: e.posted.short, tag: e.posted.tag ?? "" }
      : null,
    update: e.update ?? "",
    act: e.act
      ? {
          kind: e.act.kind,
          card: e.act.card,
          blocker: e.act.blocker,
        }
      : null,
    to: e.to ?? "",
    looked: e.looked ?? "",
  }))
}

function readOld(): ChatEntry[] {
  try {
    return parseHistory(localStorage.getItem(historyKey))
  } catch {
    return []
  }
}

function imported(): boolean {
  try {
    return localStorage.getItem(importedKey) !== null
  } catch {
    return false
  }
}

function markImported() {
  try {
    localStorage.setItem(importedKey, "1")
  } catch {
    // Blocked storage: the host still refuses a second import.
  }
}

let importing = false

// importHistory sends this computer's old history to the host, when this
// device is the assistant's computer and hasn't done it. It returns whether
// it is done (now or before); a failure leaves it to the next try.
export async function importHistory(home: HomeNow): Promise<boolean> {
  if (!home.here) return false
  if (imported()) return true
  if (importing) return false
  importing = true
  try {
    const old = readOld()
    if (old.length) {
      try {
        await AssistantService.ChatImport(toImportEntries(old))
      } catch (e) {
        // A second import is refused: it was done, here or by an earlier run
        // that didn't get to set the flag.
        if (
          !String((e as { message?: string })?.message ?? e).includes(
            alreadyImported
          )
        )
          return false
      }
    }
    markImported()
    return true
  } finally {
    importing = false
  }
}

// ---- the read-only block on the other devices --------------------------------------

type Old = { entries: ChatEntry[]; known: boolean }

let old: Old = { entries: [], known: false }
const subs = new Set<() => void>()

function setOld(next: Old) {
  old = next
  for (const f of subs) f()
}

export function subscribeOldHistory(fn: () => void): () => void {
  subs.add(fn)
  return () => {
    subs.delete(fn)
  }
}

// oldHistory is what the chat shows above the divider: this device's old
// history, on a device that isn't the assistant's computer, until New chat.
// Never sent to the model, never replicated.
export function oldHistory(): readonly ChatEntry[] {
  return old.known ? old.entries : none
}

const none: readonly ChatEntry[] = []

// clearOldHistory is New chat's part: the read-only block goes, for good.
export function clearOldHistory() {
  try {
    localStorage.removeItem(historyKey)
  } catch {
    // Nothing kept to remove.
  }
  setOld({ entries: [], known: true })
}

// applyHome follows the assistant's computer: this device's history is
// imported when it is the computer, and shown read-only when it isn't.
export async function applyHome(home: HomeNow): Promise<void> {
  if (home.here) {
    const done = await importHistory(home)
    if (done) setOld({ entries: [], known: true })
    return
  }
  setOld({ entries: imported() ? [] : readOld(), known: true })
}

let started = false

// startHistoryImport runs the import at load and whenever the assistant's
// computer changes (the first claim can land a few seconds after the window
// loads). It runs for the window's life.
export function startHistoryImport(): () => void {
  if (started) return () => {}
  started = true
  const run = (home: HomeNow) => void applyHome(home).catch(() => {})
  const off = Events.On<HomeNow>("assistant:home", (e) => run(e.data))
  AssistantService.HomeStatus().then(run, () => run({ here: false }))
  return () => {
    off()
    started = false
  }
}

// useOldHistory is the read-only block's entries; it changes when the
// assistant's computer is learned and when New chat clears it.
export function useOldHistory(): readonly ChatEntry[] {
  return useSyncExternalStore(subscribeOldHistory, oldHistory, oldHistory)
}
