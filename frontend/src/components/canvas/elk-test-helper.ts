// ELK in-process for bun test (card #776). elkjs's worker file picks how
// to run when it loads: where `self` exists and `document` doesn't (bun),
// it takes over self.onmessage as if it were a real worker. With a
// `document` there for the moment it loads, it hands back its FakeWorker
// instead, which runs the layout on this thread. The stand-in is removed
// again at once, so no other test sees it. It also picks its global
// (`$wnd`) once, at load: `window` if there is one. Another test file may
// leave a stand-in `window` behind (bun runs every file in one process),
// and ELK would then fail on `$wnd.Error`, so `window` is hidden while it
// loads and put back after.

import ELK from "elkjs/lib/elk-api"

import type { Elk } from "./elk"

type FakeWorkerClass = new (url?: string) => Worker

async function loadFakeWorker(): Promise<FakeWorkerClass> {
  const g = globalThis as { document?: unknown }
  const had = "document" in g
  const win = Object.getOwnPropertyDescriptor(globalThis, "window")
  if (!had) g.document = {}
  if (win) delete (globalThis as { window?: unknown }).window
  try {
    // A path in a variable: the file has no types of its own.
    const file = "elkjs/lib/elk-worker.min.js"
    const mod = (await import(file)) as { Worker: FakeWorkerClass }
    return mod.Worker
  } finally {
    if (!had) delete g.document
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
