// Card terminals (card #369; docs/design/inline-terminals.md): the gateway
// between the <Terminal> component and the term_* commands. The view
// package (suzuri's ui/view) knows nothing of Tauri; it calls a TermHost,
// and termHost wires that to the commands here.

import { TermService } from "./host"
import { inHost } from "./irori"
import { previewFlag } from "./preview"
import type * as B from "../gen/bindings"
import type { TermHost } from "suzuri-view"

export type TermInfo = B.TermInfo
export type TermMsg = B.TermMsg
export type Folder = B.Folder

// In the browser preview, ?term gives every card page one blank stand-in
// shell (card #730), so the Terminal section's layout can be looked at; no
// shell runs and nothing is drawn in it.
function sampleTerms(): boolean {
  return !inHost() && previewFlag("term") !== null
}

const sampleFolder = "/Users/sample/projects/irori"
const nothing = () => Promise.resolve()
// Each stand-in shell New terminal opens gets its own id.
let nextSampleId = 2

const SampleTermService: typeof TermService = {
  Folders: () => Promise.resolve([{ path: sampleFolder, why: "repo" }]),
  List: (card) =>
    Promise.resolve([
      {
        id: 1,
        scope: { kind: "card", id: card },
        cwd: sampleFolder,
        title: "",
        running: true,
      },
    ]),
  Open: (scope, _cols, _rows, cwd) =>
    Promise.resolve({
      id: nextSampleId++,
      scope,
      cwd: cwd ?? sampleFolder,
      title: "",
      running: true,
    }),
  Attach: nothing,
  Detach: nothing,
  Write: nothing,
  Paste: nothing,
  Resize: nothing,
  Scroll: nothing,
  CellPx: nothing,
  Focus: nothing,
  Redraw: nothing,
  Ack: nothing,
  Close: nothing,
}

function svc(): typeof TermService {
  return sampleTerms() ? SampleTermService : TermService
}

// termsHere says whether this window can open terminals: in the app, not
// in a plain browser (the sample mode, but for ?term) or on a phone.
export function termsHere(): boolean {
  if (!inHost()) return sampleTerms()
  return !/Android|iPhone|iPad/i.test(navigator.userAgent)
}

export function cardFolders(card: string): Promise<Folder[]> {
  return svc().Folders(card)
}

// cardTerms lists a card's shells on this device. It never starts one.
export function cardTerms(card: string): Promise<TermInfo[]> {
  return svc().List(card)
}

export function openCardTerm(
  card: string,
  cols: number,
  rows: number,
  cwd: string | null
): Promise<TermInfo> {
  return svc().Open({ kind: "card", id: card }, cols, rows, cwd)
}

export function closeTerm(id: number): Promise<void> {
  return svc().Close(id)
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
        svc().Write(id, typeof data === "string" ? data : decoder.decode(data))
      ),
    paste: (text) => {
      if (text) quiet(svc().Paste(id, text))
    },
    resize: (cols, rows) => quiet(svc().Resize(id, cols, rows)),
    cellPx: (w, h) => quiet(svc().CellPx(id, w, h)),
    scroll: (delta) => quiet(svc().Scroll(id, delta)),
    // An ack after the shell exited is harmless; never shown.
    ack: (seq) => {
      svc()
        .Ack(id, seq)
        .catch(() => {})
    },
    redraw: () => quiet(svc().Redraw(id)),
    focus: (on) => {
      svc()
        .Focus(id, on)
        .catch(() => {})
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
export function attachTerm(
  id: number,
  onFrame: (frame: ArrayBuffer) => void,
  onMsg: (msg: TermMsg) => void
): Promise<void> {
  return svc().Attach(id, onFrame, onMsg)
}

export function detachTerm(id: number): void {
  svc()
    .Detach(id)
    .catch(() => {})
}
