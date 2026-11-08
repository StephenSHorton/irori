// The canvas's ELK (card #776): elkjs's small API on this side, the 1.6 MB
// layout engine in a web worker of its own, so laying out never holds up
// the window. The worker is its own file in the build (?url), served from
// the app like every other asset: the window's CSP (default-src 'self')
// allows it, where a blob: or inline worker would be refused. This module
// is only reached from the canvas's lazy chunk.

import ELK from "elkjs/lib/elk-api"
import workerUrl from "elkjs/lib/elk-worker.min.js?url"

import type { Elk } from "./elk"

let elk: InstanceType<typeof ELK> | null = null

// appElk is the one ELK the window shares, started on first use.
export function appElk(): Elk {
  elk ??= new ELK({ workerUrl })
  return elk
}

// resetElk stops the worker after a failure, so the next layout starts a
// fresh one.
export function resetElk() {
  elk?.terminateWorker()
  elk = null
}
