// Card terminals (card #369; docs/design/inline-terminals.md): the gateway
// between the <Terminal> component and the term_* commands. The view
// package (suzuri's ui/view) knows nothing of Tauri; it calls a TermHost,
// and termHost wires that to the commands here.

import { TermService } from "./host"
import { inHost } from "./irori"
import type * as B from "../gen/bindings"
import type { TermHost } from "suzuri-view"

export type TermInfo = B.TermInfo
export type TermMsg = B.TermMsg
export type Folder = B.Folder

// termsHere says whether this window can open terminals: in the app, not
// in a plain browser (the sample mode) or on a phone.
export function termsHere(): boolean {
  if (!inHost()) return false
  return !/Android|iPhone|iPad/i.test(navigator.userAgent)
}

export function cardFolders(card: string): Promise<Folder[]> {
  return TermService.Folders(card)
}

// cardTerms lists a card's shells on this device. It never starts one.
export function cardTerms(card: string): Promise<TermInfo[]> {
  return TermService.List(card)
}

export function openCardTerm(
  card: string,
  cols: number,
  rows: number,
  cwd: string | null
): Promise<TermInfo> {
  return TermService.Open({ kind: "card", id: card }, cols, rows, cwd)
}

export function closeTerm(id: number): Promise<void> {
  return TermService.Close(id)
}

// The folder the person picked for a card on this device (design 6.1, step
// 3, and the picker when there are several). It stays on this device: the
// path means nothing on another one.
const FolderKey = "irori.term.folder."

export function rememberedFolder(card: string): string | null {
  try {
    return localStorage.getItem(FolderKey + card)
  } catch {
    return null
  }
}

export function rememberFolder(card: string, path: string): void {
  try {
    localStorage.setItem(FolderKey + card, path)
  } catch {
    // Private mode: the picker asks again next time.
  }
}

// pickFolder chooses the folder to open in with no question, when it can:
// the remembered one if it is still offered, else the only one offered.
// null means the person has to choose (several, or none).
export function pickFolder(card: string, offered: Folder[]): string | null {
  const kept = rememberedFolder(card)
  if (kept && offered.some((f) => f.path === kept)) return kept
  if (offered.length === 1) return offered[0].path
  return null
}

// TermHost is what the view package calls (inline-terminals.md section 4):
// the view sends what the person types and how big it is; this sends it on.
export type { TermHost }

const decoder = new TextDecoder()

// termHost is the TermHost for terminal id. Selection, copy, mouse
// reports and links are later slices; they do nothing yet.
export function termHost(
  id: number,
  opts: { onError: (e: Error) => void; openUrl: (url: string) => void }
): TermHost {
  const quiet = (p: Promise<void>) => {
    p.catch(opts.onError)
  }
  return {
    write: (data) =>
      quiet(
        TermService.Write(
          id,
          typeof data === "string" ? data : decoder.decode(data)
        )
      ),
    paste: (text) => {
      if (text) quiet(TermService.Paste(id, text))
    },
    resize: (cols, rows) => quiet(TermService.Resize(id, cols, rows)),
    cellPx: (w, h) => quiet(TermService.CellPx(id, w, h)),
    scroll: (delta) => quiet(TermService.Scroll(id, delta)),
    // An ack after the shell exited is harmless; never shown.
    ack: (seq) => {
      TermService.Ack(id, seq).catch(() => {})
    },
    redraw: () => quiet(TermService.Redraw(id)),
    focus: (on) => {
      TermService.Focus(id, on).catch(() => {})
    },
    select: () => {},
    copy: () => Promise.resolve(false),
    mouse: () => {},
    link: () => Promise.resolve(null),
    openUrl: opts.openUrl,
  }
}

// attachTerm starts the view's streams: the cached state, then a full
// frame. detach is called when the view goes; the shell keeps running.
//
// A shell has one local view, and the Core forgets it on any detach. When
// one view of a shell goes and another comes in the same render (the card
// page's section handing it to the bottom panel, card #734, or a remount),
// the two calls are separate invokes that may land in either order; a
// detach landing last would leave the new view blank. So each shell's
// attach and detach go out one after another, each after the last ended.
const pending = new Map<number, Promise<unknown>>()

function inTurn<T>(id: number, call: () => Promise<T>): Promise<T> {
  const prev = pending.get(id) ?? Promise.resolve()
  const next = prev.catch(() => {}).then(call)
  const tail = next.catch(() => {})
  pending.set(id, tail)
  void tail.then(() => {
    if (pending.get(id) === tail) pending.delete(id)
  })
  return next
}

export function attachTerm(
  id: number,
  onFrame: (frame: ArrayBuffer) => void,
  onMsg: (msg: TermMsg) => void
): Promise<void> {
  return inTurn(id, () => TermService.Attach(id, onFrame, onMsg))
}

export function detachTerm(id: number): void {
  inTurn(id, () => TermService.Detach(id)).catch(() => {})
}

// The person's own shells on this device (the device scope, card #734):
// the terminal panel's on pages other than a card's.
export function deviceTerms(): Promise<TermInfo[]> {
  return TermService.DeviceList()
}

// openDeviceTerm starts one of the person's own shells in cwd, or at home
// when cwd is null. A folder that has gone since (a removed worktree)
// opens at home instead of failing.
export async function openDeviceTerm(
  cols: number,
  rows: number,
  cwd: string | null
): Promise<TermInfo> {
  const scope: B.Scope = { kind: "device" }
  if (!cwd) return TermService.Open(scope, cols, rows, null)
  try {
    return await TermService.Open(scope, cols, rows, cwd)
  } catch {
    return TermService.Open(scope, cols, rows, null)
  }
}

// firstDeviceTerm is what the panel shows when it opens off a card: the
// shells already there, and a new one in cwd when none of them runs. Opening the panel is the
// person asking for a shell. Calls made while one is under way share it,
// so a double mount (StrictMode, a quick toggle) never starts two.
let starting: Promise<TermInfo[]> | null = null

export function firstDeviceTerm(
  cols: number,
  rows: number,
  cwd: string | null
): Promise<TermInfo[]> {
  starting ??= deviceTerms()
    // Shells that ended stay listed (with Restart) until closed; with
    // none still running, a new one is started beside them.
    .then(async (ts) =>
      ts.some((t) => t.running)
        ? ts
        : [...ts, await openDeviceTerm(cols, rows, cwd)]
    )
    .finally(() => {
      starting = null
    })
  return starting
}
