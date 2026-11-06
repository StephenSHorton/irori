// The dictation shortcut (card #634): Cmd+D on a Mac, Ctrl+D elsewhere,
// toggles voice input, like Claude Desktop. Every mic button registers
// here; one window listener picks the one mic that answers a press.

import { isMac } from "@/lib/commands"

// As the command palette shows shortcuts (shortcutKeys).
export const DictationShortcut = "Mod+D"

// dictationKey says whether a keydown is the dictation shortcut: Cmd+D on
// a Mac, Ctrl+D elsewhere, with no Shift or Alt. On a Mac, Ctrl+D stays
// the text field's delete-forward. Mod handling matches paletteOpenKey.
export function dictationKey(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
  mac: boolean
): boolean {
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey
  if (!mod || e.shiftKey || e.altKey) return false
  return e.key.toLowerCase() === "d"
}

// A mic as the shortcut sees it.
export type Mic = {
  // Its text box.
  box: () => HTMLElement | null
  // The main composer of its page or dialog (the Ask box, a card's
  // comment box, a message reply), which answers when no box has focus.
  main: boolean
  // Listening now (or starting).
  live: () => boolean
  // Can start: not disabled, voice is there.
  usable: () => boolean
  toggle: () => void
  // When it was registered and when its box last had focus; higher is
  // later. Set by the registry.
  mounted: number
  focused: number
}

// pickMic is the one mic a press toggles, or null:
//  1. one that is listening, so the press stops it wherever focus went;
//  2. the focused box's mic;
//  3. with focus in some other text field, none: the words never land
//     somewhere the person isn't typing;
//  4. else the main composer in view, the one whose box had focus last,
//     then the newest (a dialog over a page).
export function pickMic(
  mics: readonly Mic[],
  active: Element | null,
  inView: (box: HTMLElement) => boolean
): Mic | null {
  const live = mics.find((m) => m.live())
  if (live) return live
  if (active) {
    const own = mics.find((m) => m.box() === active)
    if (own) return own.usable() ? own : null
    if (editable(active)) return null
  }
  let best: Mic | null = null
  for (const m of mics) {
    if (!m.main || !m.usable()) continue
    const b = m.box()
    if (!b || !inView(b)) continue
    if (
      !best ||
      m.focused > best.focused ||
      (m.focused === best.focused && m.mounted > best.mounted)
    )
      best = m
  }
  return best
}

// editable says whether focus is in a text field the person types in.
export function editable(el: Element): boolean {
  const tag = el.tagName
  if (tag === "TEXTAREA") return true
  if (tag === "INPUT") {
    const type = (el as HTMLInputElement).type
    return ![
      "button",
      "checkbox",
      "radio",
      "submit",
      "reset",
      "range",
      "color",
      "file",
    ].includes(type)
  }
  return (el as HTMLElement).isContentEditable === true
}

// inView says whether a box can be seen: part of it is in the window and
// nothing (a dialog, the palette) covers its middle.
export function inView(box: HTMLElement): boolean {
  if (!box.isConnected) return false
  const r = box.getBoundingClientRect()
  const x0 = Math.max(r.left, 0)
  const x1 = Math.min(r.right, window.innerWidth)
  const y0 = Math.max(r.top, 0)
  const y1 = Math.min(r.bottom, window.innerHeight)
  if (x1 - x0 < 1 || y1 - y0 < 1) return false
  const hit = document.elementFromPoint((x0 + x1) / 2, (y0 + y1) / 2)
  if (!hit) return false
  return hit === box || box.contains(hit) || !!box.parentElement?.contains(hit)
}

const mics: Mic[] = []
let tick = 0
let off: (() => void) | null = null

// registerMic adds a mic until the returned function removes it. The
// window listeners live while any mic does.
export function registerMic(m: Omit<Mic, "mounted" | "focused">): () => void {
  const entry: Mic = { ...m, mounted: ++tick, focused: 0 }
  mics.push(entry)
  if (!off) off = listen()
  return () => {
    const i = mics.indexOf(entry)
    if (i >= 0) mics.splice(i, 1)
    if (mics.length === 0 && off) {
      off()
      off = null
    }
  }
}

function listen(): () => void {
  const mac = isMac()
  // Capture, so it comes before a field or a dialog takes the key.
  const onKey = (e: KeyboardEvent) => {
    if (e.isComposing || !dictationKey(e, mac)) return
    const m = pickMic(mics, document.activeElement, inView)
    if (!m) return
    // Ctrl+D would bookmark in a browser engine; one mic, one press.
    e.preventDefault()
    e.stopPropagation()
    if (e.repeat) return
    m.toggle()
  }
  const onFocus = (e: FocusEvent) => {
    const m = mics.find((x) => x.box() === e.target)
    if (m) m.focused = ++tick
  }
  window.addEventListener("keydown", onKey, true)
  window.addEventListener("focusin", onFocus, true)
  return () => {
    window.removeEventListener("keydown", onKey, true)
    window.removeEventListener("focusin", onFocus, true)
  }
}
