// The window's desktop shortcuts (card #733): Back and Forward, Cmd+1..5
// for the sections, Cmd+B to fold the sidebar, Cmd+, for Settings, Cmd+N
// for a new card and Cmd+F to search cards (Ctrl for Cmd on a PC). Each
// runs a command from the registry (lib/commands.ts), so the palette lists
// it with its keys and the shortcut does exactly what the palette's row
// does. They wait while the person types in a field or a terminal has the
// keyboard, and while a dialog is open.

import { isMac, runCommand } from "@/lib/commands"
import { editable } from "@/lib/mic-shortcut"

// The keys as the palette shows them (shortcutKeys). Back and Forward are
// Cmd+[ and Cmd+] on a Mac; on a PC Alt+Left and Alt+Right, since Ctrl+[
// is Esc in a terminal.
export function backShortcut(mac = isMac()): string {
  return mac ? "Mod+[" : "Alt+←"
}
export function forwardShortcut(mac = isMac()): string {
  return mac ? "Mod+]" : "Alt+→"
}
export const SectionShortcuts = [
  "Mod+1",
  "Mod+2",
  "Mod+3",
  "Mod+4",
  "Mod+5",
] as const
export const SidebarShortcut = "Mod+B"
export const SettingsShortcut = "Mod+,"
export const NewCardShortcut = "Mod+N"
export const SearchShortcut = "Mod+F"

// Cmd+1..5, in the sidebar's order.
const sections = [
  "nav.home",
  "nav.talk",
  "nav.notes",
  "nav.boards",
  "nav.notifications",
]

const letters: Record<string, string> = {
  b: "sidebar.fold",
  ",": "settings.open",
  n: "card.new",
  f: "search.cards",
}

type Key = Pick<
  KeyboardEvent,
  "key" | "code" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"
>

// appShortcut is the id of the command a keydown runs, or null. Mod is Cmd
// on a Mac and Ctrl elsewhere, with no Shift or Alt, as paletteOpenKey
// (on a Mac, Ctrl+B and Ctrl+N stay the text caret's).
export function appShortcut(e: Key, mac: boolean): string | null {
  // A keyboard's or mouse's own Back and Forward keys.
  if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
    if (e.key === "BrowserBack") return "nav.back"
    if (e.key === "BrowserForward") return "nav.forward"
  }
  if (!mac && e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
    if (e.key === "ArrowLeft") return "nav.back"
    if (e.key === "ArrowRight") return "nav.forward"
    return null
  }
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey
  if (!mod || e.shiftKey || e.altKey) return null
  if (mac && e.key === "[") return "nav.back"
  if (mac && e.key === "]") return "nav.forward"
  // The digit by its key, or by where it sits (Cmd+& is Cmd+1 on AZERTY).
  const digit = /^[1-5]$/.test(e.key)
    ? Number(e.key)
    : /^Digit[1-5]$/.test(e.code ?? "")
      ? Number(e.code.slice(5))
      : 0
  if (digit) return sections[digit - 1]
  return letters[e.key.toLowerCase()] ?? null
}

// mouseShortcut is the command a mouse's side button runs: button 3 is
// Back, button 4 Forward.
export function mouseShortcut(button: number): string | null {
  if (button === 3) return "nav.back"
  if (button === 4) return "nav.forward"
  return null
}

// What the focus is in, as inField sees it.
type Focus = Pick<Element, "tagName" | "closest"> & {
  type?: string
  isContentEditable?: boolean
}

// inField says whether the keys belong where the focus is: a text field,
// a select, or a terminal (.rsz-view), whose keys are its program's.
export function inField(el: Focus | null): boolean {
  if (!el) return false
  if (el.tagName === "SELECT") return true
  if (editable(el as unknown as Element)) return true
  return !!el.closest(".rsz-view")
}

// A dialog (Settings, New card, the palette) has the keys while it is open.
const dialogs = '[role="dialog"],[role="alertdialog"]'

// listenAppShortcuts adds the window's listeners and returns what takes
// them out. Bubble phase, so a field, a menu or a dialog that handles a
// key first (and prevents it) keeps it.
export function listenAppShortcuts(): () => void {
  const mac = isMac()
  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.isComposing) return
    const id = appShortcut(e, mac)
    if (!id) return
    const t = e.target instanceof Element ? e.target : null
    if (inField(t) || inField(document.activeElement)) return
    if (document.querySelector(dialogs)) return
    // Holding Back or Forward steps on, like a browser; the others once.
    const nav = id === "nav.back" || id === "nav.forward"
    if (e.repeat && !nav) {
      e.preventDefault()
      return
    }
    // Only a command that ran takes the key: Cmd+F with no search box
    // here leaves it be.
    if (runCommand(id)) e.preventDefault()
  }
  // A side button: on mouseup, where preventDefault stops the web view's
  // own back, which would go back a second time.
  const onMouse = (e: MouseEvent) => {
    const id = mouseShortcut(e.button)
    if (!id) return
    e.preventDefault()
    if (document.querySelector(dialogs)) return
    runCommand(id)
  }
  window.addEventListener("keydown", onKey)
  window.addEventListener("mouseup", onMouse)
  return () => {
    window.removeEventListener("keydown", onKey)
    window.removeEventListener("mouseup", onMouse)
  }
}
