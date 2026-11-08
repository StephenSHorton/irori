// ELK in-process for bun test (card #776). elkjs's worker file picks how
// to run when it loads: where `self` exists and `document` doesn't (bun),
// it takes over self.onmessage as if it were a real worker. With a
// `document` there for the moment it loads, it hands back its FakeWorker
// instead, which runs the layout on this thread. ELK asks `typeof
// document`, so a `document` key left behind as undefined (a test that
// put back a global it never had) counts as none: the stand-in goes in
// whenever the value is missing, not only when the key is (card #1085).
// The first load is cached for the whole run, so a wrong pick here breaks
// every later layout. The stand-in is removed again at once, so no other
// test sees it. It also picks its global (`$wnd`) once, at load: `window`
// if there is one. Another test file may leave a stand-in `window` behind
// (bun runs every file in one process), and ELK would then fail on
// `$wnd.Error`, so `window` is hidden while it loads and put back after.

import ELK from "elkjs/lib/elk-api"

import type { Elk } from "./elk"

type FakeWorkerClass = new (url?: string) => Worker

async function loadFakeWorker(): Promise<FakeWorkerClass> {
  const g = globalThis as { document?: unknown; window?: unknown }
  const doc = Object.getOwnPropertyDescriptor(globalThis, "document")
  const win = Object.getOwnPropertyDescriptor(globalThis, "window")
  const stand = g.document === undefined
  if (stand) g.document = {}
  if (win) delete g.window
  try {
    // A path in a variable: the file has no types of its own.
    const file = "elkjs/lib/elk-worker.min.js"
    const mod = (await import(file)) as { Worker?: FakeWorkerClass }
    if (!mod.Worker) {
      throw new Error(
        "elk-worker loaded as a real worker (no document when it first loaded), so it has no in-process Worker"
      )
    }
    return mod.Worker
  } finally {
    if (stand) {
      if (doc) Object.defineProperty(globalThis, "document", doc)
      else delete g.document
    }
    if (win) Object.defineProperty(globalThis, "window", win)
  }
}

let elk: Elk | null = null

// testElk is an ELK that lays out on this thread.
export async function testElk(): Promise<Elk> {
  if (!elk) {
    const FakeWorker = await loadFakeWorker()
    elk = new ELK({
      workerUrl: "in-process",
      workerFactory: (url) => new FakeWorker(url),
    })
  }
  return elk
}
