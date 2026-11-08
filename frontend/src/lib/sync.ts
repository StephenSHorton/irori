// Sync state (card #1089): whether this device is exchanging rows or blobs
// with a peer right now, for the header's badge. The host holds it a second
// past the last movement (crates/mesh activity.rs) and sends `sync:changed`
// with true or false when it flips. Outside the app nothing syncs, so it is
// false; the preview's ?sync=on shows the badge for a look.

import { useSyncExternalStore } from "react"

import { Events } from "./host"
import { inWails } from "./irori"
import { previewFlag } from "./preview"

export const EventSyncChanged = "sync:changed"

const state = {
  value: false,
  readers: new Set<() => void>(),
  off: null as (() => void) | null,
}

export function setSyncing(next: boolean) {
  if (state.value === next) return
  state.value = next
  for (const fn of state.readers) fn()
}

function follow(): () => void {
  if (!inWails()) {
    setSyncing(previewFlag("sync") === "on")
    return () => {}
  }
  return Events.On<boolean>(EventSyncChanged, (ev) =>
    setSyncing(ev.data === true)
  )
}

export function subscribeSyncing(reader: () => void): () => void {
  state.readers.add(reader)
  state.off ??= follow()
  return () => {
    state.readers.delete(reader)
    if (state.readers.size === 0) {
      state.off?.()
      state.off = null
    }
  }
}

export const syncingNow = () => state.value
const snapshot = syncingNow

// useSyncing is true while this device is syncing with a peer.
export function useSyncing(): boolean {
  return useSyncExternalStore(subscribeSyncing, snapshot, snapshot)
}
