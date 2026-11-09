// Card #1134: whether anyone can see the window. document.hidden is enough
// in a browser and in WKWebView on the Mac, but WebView2 under Tauri keeps
// reporting the page visible when the window is minimised or closed to the
// tray, so on the PC every "pause while hidden" (whileShown, useNow) never
// paused: the window kept re-reading lists and re-rendering every second
// with nobody looking. appPage adds the Tauri window's own state: hidden
// when it is minimised or not visible. Checked on WebView2 (Tauri 2):
// minimising sends resize and blur; closing to the tray sends only
// close-requested (the shell then hides the window, and no event follows);
// showing it sends focus only when Windows grants focus. So it re-checks on
// focus, blur and resize, counts close-requested as hidden at once, and
// while the window is unfocused re-checks every couple of seconds, which
// finds a show that came without focus.

import { getCurrentWindow } from "@tauri-apps/api/window"

// Page is the shape whileShown and useNow read: document's, so a plain
// document still works where there is no Tauri window.
export type Page = {
  readonly hidden: boolean
  addEventListener(type: "visibilitychange", fn: () => void): void
  removeEventListener(type: "visibilitychange", fn: () => void): void
}

// WindowState is what appPage asks the native window.
export type WindowState = {
  // Minimised or not visible (closed to the tray).
  hidden(): Promise<boolean>
  // Subscribes fn to the window's focus, blur, resize and close-requested
  // events; returns the unsubscribe.
  onChange(fn: (change: WindowChange) => void): () => void
}

export type WindowChange = "focus" | "blur" | "resize" | "close"

// RecheckMs is how often an unfocused window re-checks its state.
export const RecheckMs = 2000

type Timers = {
  set(fn: () => void, ms: number): unknown
  clear(id: unknown): void
}

// combinedPage is appPage with its parts given (tests pass fakes).
export function combinedPage(
  doc: Page,
  win: WindowState,
  timers: Timers = {
    set: (fn, ms) => setInterval(fn, ms),
    clear: (id) => clearInterval(id as ReturnType<typeof setInterval>),
  },
  every = RecheckMs
): Page & { refresh(): Promise<void> } {
  let winHidden = false
  let wasHidden = doc.hidden
  const listeners = new Set<() => void>()
  let timer: unknown = null

  const notify = () => {
    const now = doc.hidden || winHidden
    if (now === wasHidden) return
    wasHidden = now
    for (const fn of [...listeners]) fn()
  }
  const refresh = async () => {
    try {
      winHidden = await win.hidden()
    } catch {
      winHidden = false
    }
    notify()
  }
  const recheck = (change: WindowChange) => {
    if (change === "focus") {
      if (timer !== null) timers.clear(timer)
      timer = null
    } else if (change !== "resize" && timer === null) {
      timer = timers.set(() => void refresh(), every)
    }
    if (change === "close") {
      // The shell hides the window right after this event, so asking now
      // would still answer visible: count it hidden, and the timer confirms.
      winHidden = true
      notify()
      return
    }
    void refresh()
  }

  doc.addEventListener("visibilitychange", notify)
  win.onChange(recheck)
  void refresh()

  return {
    get hidden() {
      return doc.hidden || winHidden
    },
    addEventListener(_type, fn) {
      listeners.add(fn)
    },
    removeEventListener(_type, fn) {
      listeners.delete(fn)
    },
    refresh,
  }
}

function tauriWindow(): WindowState {
  const w = getCurrentWindow()
  return {
    async hidden() {
      const [min, vis] = await Promise.all([w.isMinimized(), w.isVisible()])
      return min || !vis
    },
    onChange(fn) {
      // A plain listener on close-requested, not onCloseRequested: that
      // helper destroys the window unless prevented, and the shell already
      // turns a close into a hide.
      const offs: Array<Promise<() => void>> = [
        w.onFocusChanged((e) => fn(e.payload ? "focus" : "blur")),
        w.onResized(() => fn("resize")),
        w.listen("tauri://close-requested", () => fn("close")),
      ]
      return () => {
        for (const p of offs) void p.then((off) => off()).catch(() => {})
      }
    },
  }
}

let shared: Page | null = null

// appPage is the window's one visibility: document's, plus the native
// window's state inside the app. Outside the app it is document itself.
export function appPage(): Page | null {
  if (typeof document === "undefined") return null
  // Inside the app (irori.ts inHost; not imported here: irori.ts imports
  // host.ts, which imports shown.ts, which imports this).
  const app = Boolean(
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  )
  if (!app) return document
  shared ??= combinedPage(document, tauriWindow())
  return shared
}
