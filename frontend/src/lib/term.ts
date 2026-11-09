// Card terminals (card #369; docs/design/inline-terminals.md): the gateway
// between the <Terminal> component and the term_* commands. The view
// package (suzuri's ui/view) knows nothing of Tauri; it calls a TermHost,
// and termHost wires that to the commands here.

import { onChange, names } from "./changes"
import { TermService } from "./host"
import { inHost } from "./irori"
import { previewFlag } from "./preview"
import {
  onSampleExit,
  sampleRunsOn,
  sampleTail,
  type TailInfo,
  type TermWithTail,
} from "./runs"
import type * as B from "../gen/bindings"
import type { TermHost } from "suzuri-view"

// TermInfo is a terminal; tail is set on one that follows a run's output
// (card #1249, live-tasks.md §5.1).
export type TermInfo = TermWithTail
export type { TailInfo }
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

// sampleTailTerms is a card's stand-in run tabs (card #1249): one per
// sample shell run on this device, from sample-cards.ts.
const sampleTailIds = new Map<string, number>()
const sampleAttached = new Map<number, (msg: TermMsg) => void>()
let nextTailId = 100

function sampleTailTerms(card: string): TermInfo[] {
  return (
    sampleRunsOn(card)
      .filter((r) => r.kind === "bash" && r.who.here !== false)
      // An older run's tab the sample takes as closed.
      .filter((r) => r.started > Date.now() - 20 * 60_000)
      .map((r) => {
        let id = sampleTailIds.get(r.run)
        if (id === undefined) {
          id = nextTailId++
          sampleTailIds.set(r.run, id)
        }
        const tail = sampleTail(r)
        return {
          id,
          scope: { kind: "card", id: card },
          cwd: sampleFolder,
          title: "",
          running: tail.exit === undefined,
          tail,
        }
      })
  )
}

// A sample run's Stop ends its stand-in tab as the Core's tail would.
onSampleExit((run) => {
  const id = sampleTailIds.get(run)
  if (id !== undefined) sampleAttached.get(id)?.({ t: "exit", code: null })
})

const SampleTermService = {
  Folders: () => Promise.resolve([{ path: sampleFolder, why: "repo" }]),
  DeviceList: () => Promise.resolve([]),
  List: (card: string) =>
    Promise.resolve<TermInfo[]>([
      {
        id: 1,
        scope: { kind: "card", id: card },
        cwd: sampleFolder,
        title: "",
        running: true,
      },
      ...sampleTailTerms(card),
    ]),
  Open: (scope: B.Scope, _cols: number, _rows: number, cwd: string | null) =>
    Promise.resolve({
      id: nextSampleId++,
      scope,
      cwd: cwd ?? sampleFolder,
      title: "",
      running: true,
    }),
  Attach: (
    id: number,
    _onFrame: (frame: ArrayBuffer) => void,
    onMsg: (msg: TermMsg) => void
  ) => {
    sampleAttached.set(id, onMsg)
    return nothing()
  },
  Detach: (id: number) => {
    sampleAttached.delete(id)
    return nothing()
  },
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

// The stand-in keeps the real service's shape; a list may carry tails.
type Svc = Omit<typeof TermService, "List"> & {
  List: (card: string) => Promise<TermInfo[]>
}
SampleTermService satisfies Svc

function svc(): Svc {
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

// cardTerms lists a card's shells on this device, its run tabs among
// them. It never starts one.
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
//
// A shell has one local view, and the Core forgets it on any detach. When
// one view of a shell goes and another comes in the same render (the card
// page's section handing it to the bottom panel, card #734, or a remount),
// the two calls are separate invokes that may land in either order; a
// detach landing last would leave the new view blank. So each shell's
// attach and detach go out one after another, each after the last ended
// (exported for its test).
const pending = new Map<number, Promise<unknown>>()

export function inTurn<T>(id: number, call: () => Promise<T>): Promise<T> {
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
  return inTurn(id, () => svc().Attach(id, onFrame, onMsg))
}

export function detachTerm(id: number): void {
  inTurn(id, () => svc().Detach(id)).catch(() => {})
}

// The person's own shells on this device (the device scope, card #734):
// the terminal panel's on pages other than a card's.
export function deviceTerms(): Promise<TermInfo[]> {
  return svc().DeviceList()
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
  if (!cwd) return svc().Open(scope, cols, rows, null)
  try {
    return await svc().Open(scope, cols, rows, cwd)
  } catch {
    return svc().Open(scope, cols, rows, null)
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

// Run tabs (card #1249, live-tasks.md §5.2) ------------------------------------

// ShowRunTab is the event the Running section's Show tab raises: the
// card's Terminal shows that run's tab. The person asked, so it may
// change the shown tab; it never focuses the terminal.
export const ShowRunTab = "irori:show-run-tab"
export type ShowRunTabDetail = { cardId: string; run: string }

export function showRunTab(cardId: string, run: string): void {
  window.dispatchEvent(
    new CustomEvent<ShowRunTabDetail>(ShowRunTab, { detail: { cardId, run } })
  )
}

// isTail says a terminal follows a run's output (read only).
export function isTail(t: TermInfo): t is TermInfo & { tail: TailInfo } {
  return !!t.tail
}

// newestShell is the tab a card's Terminal shows on mount: the newest
// shell that isn't a run tab, else none (a run tab never opens itself in
// front of the person).
export function newestShell(ts: readonly TermInfo[]): number | null {
  const shells = ts.filter((t) => !isTail(t))
  return shells.length ? shells[shells.length - 1].id : null
}

// mergeTails adds the run tabs a fresh list has to the ones shown, takes
// their newer end, and drops the ones the Core closed (the card went Done,
// §5.2); shells and the order stay as they are.
export function mergeTails(
  shown: readonly TermInfo[],
  fresh: readonly TermInfo[]
): TermInfo[] {
  const byId = new Map(fresh.map((t) => [t.id, t]))
  const kept = shown.flatMap((t) => (isTail(t) ? (byId.get(t.id) ?? []) : [t]))
  const have = new Set(shown.map((t) => t.id))
  return [...kept, ...fresh.filter((t) => isTail(t) && !have.has(t.id))]
}

// tailEnded is a run tab with its run's end, as its Terminal heard it, so
// a remount shows "Exited with code N" before the next list.
export function tailEnded(
  ts: readonly TermInfo[],
  id: number,
  code: number | null
): TermInfo[] {
  return ts.map((t) =>
    t.id !== id
      ? t
      : {
          ...t,
          running: false,
          tail: t.tail && { ...t.tail, exit: code ?? t.tail.exit ?? null },
        }
  )
}

// onCardChanged runs fn when the card changes (a run started or ended on
// it); nothing in a plain browser, where the sample tabs are listed once.
export function onCardChanged(card: string, fn: () => void): () => void {
  if (!inHost()) return () => {}
  return onChange("board:changed", (n) => names(n, "cards", card) && fn(), {
    follow: ["cards"],
  })
}
