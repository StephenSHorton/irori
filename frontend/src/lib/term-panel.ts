// The terminal panel (card #734): a shell docked at the bottom of the
// window, toggled by Ctrl+` on both platforms, as in VS Code. On a card's
// page it shows that card's shells; anywhere else, the person's own shell
// on this device (the device scope), opened in the folder it was last in.
//
// Whether it is open lives here, outside React, so the card page can tell
// its Terminal section to step aside while the panel shows the same
// shells (a terminal has one local view). Only the height is remembered
// across starts: a panel that reopened by itself would start a shell
// nobody asked for (inline-terminals.md section 8).

import { useSyncExternalStore } from "react"

import { previewFlag } from "./preview"

const HeightKey = "irori.term.panelHeight"
const LastFolderKey = "irori.term.lastFolder"

export const DefaultHeight = 280
export const MinHeight = 120
// What the panel always leaves above it for the page.
export const PageRoom = 160

type State = { open: boolean; height: number }

let state: State = {
  // ?term=open in the preview or a window check (lib/preview.ts). The
  // test runner has no window.
  open: typeof window !== "undefined" && previewFlag("term") === "open",
  height: storedHeight(),
}
const listeners = new Set<() => void>()

function set(next: Partial<State>) {
  state = { ...state, ...next }
  for (const fn of listeners) fn()
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  return () => {
    listeners.delete(fn)
  }
}

const snapshot = () => state

// useTermPanel is whether the panel is open and how tall it is.
export function useTermPanel(): State {
  return useSyncExternalStore(subscribe, snapshot, snapshot)
}

export function termPanelOpen(): boolean {
  return state.open
}

export function setTermPanelOpen(open: boolean): void {
  if (open !== state.open) set({ open })
}

export function toggleTermPanel(): void {
  set({ open: !state.open })
}

// clampHeight keeps the panel between its least height and what leaves
// the page PageRoom above it.
export function clampHeight(h: number, viewport: number): number {
  const most = Math.max(MinHeight, viewport - PageRoom)
  return Math.round(Math.min(most, Math.max(MinHeight, h)))
}

// setTermPanelHeight sets the height while dragging; save remembers it
// (once, when the drag ends).
export function setTermPanelHeight(h: number, save = false): void {
  const height = clampHeight(h, window.innerHeight)
  if (height !== state.height) set({ height })
  if (!save) return
  try {
    localStorage.setItem(HeightKey, String(height))
  } catch {
    // Blocked storage only loses the memory.
  }
}

function storedHeight(): number {
  try {
    const n = Number(localStorage.getItem(HeightKey))
    return Number.isFinite(n) && n >= MinHeight ? n : DefaultHeight
  } catch {
    return DefaultHeight
  }
}

// TerminalShortcut is how the palette shows the key: Ctrl on the Mac too,
// as in VS Code (Cmd+` there switches windows).
export const TerminalShortcut = "Ctrl+`"

// isTerminalToggle says whether a keydown is Ctrl+`. It goes by the key's
// place (code), not its character, so it works on layouts where that key
// types something else, and Shift or Alt with it is left alone.
export function isTerminalToggle(e: {
  code: string
  ctrlKey: boolean
  metaKey: boolean
  altKey: boolean
  shiftKey: boolean
}): boolean {
  return (
    e.code === "Backquote" &&
    e.ctrlKey &&
    !e.metaKey &&
    !e.altKey &&
    !e.shiftKey
  )
}

// The folder the person's own shell was last in on this device, so the
// next one opens there. It stays on this device: the path means nothing
// on another one.
export function lastFolder(): string | null {
  try {
    return localStorage.getItem(LastFolderKey) || null
  } catch {
    return null
  }
}

export function rememberLastFolder(path: string): void {
  if (!path) return
  try {
    localStorage.setItem(LastFolderKey, path)
  } catch {
    // Blocked storage: the next shell opens at home.
  }
}
