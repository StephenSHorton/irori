// Screen share in a huddle (#658): one device at a time shares one window
// or display, the others watch it. Rust sends H.264 over iroh and hands this
// window decoder-ready frames on a Channel (host.ts's ShareService.Attach);
// this file parses them (parseShareFrame, pure and tested), plays them into
// a canvas (SharePlayer, WebCodecs' VideoDecoder) and starts, stops and
// watches a share. In the browser preview (?call=share) a recorded fixture
// plays through the same decoder instead, and lib/call.ts's stand-in holds
// the share.

import { ShareService } from "./host"
import { inWails } from "./irori"

import type { CallShareView, ShareSourceView } from "../gen/bindings"

export type { CallShareView, ShareSourceView }

// ---- the frames ----------------------------------------------------------------

export type ShareFrame =
  | { kind: "config"; codec: string; description: Uint8Array }
  | { kind: "chunk"; key: boolean; timestampUs: number; data: Uint8Array }

// parseShareFrame reads one Channel message:
//   config: [1] [codecLen u16 BE] [codec UTF-8] [avcC description...]
//   chunk:  [2] [flags u8, bit0 key] [timestamp µs u64 BE] [AVCC access unit...]
// null for anything else (too short, an unknown kind).
export function parseShareFrame(buf: ArrayBuffer): ShareFrame | null {
  const v = new DataView(buf)
  if (v.byteLength < 1) return null
  const kind = v.getUint8(0)
  if (kind === 1) {
    if (v.byteLength < 3) return null
    const n = v.getUint16(1)
    if (v.byteLength < 3 + n) return null
    const codec = new TextDecoder().decode(new Uint8Array(buf, 3, n))
    if (!codec) return null
    return { kind: "config", codec, description: new Uint8Array(buf, 3 + n) }
  }
  if (kind === 2) {
    if (v.byteLength < 10) return null
    return {
      kind: "chunk",
      key: (v.getUint8(1) & 1) === 1,
      timestampUs: Number(v.getBigUint64(2)),
      data: new Uint8Array(buf, 10),
    }
  }
  return null
}

// splitFixture cuts the preview's recording into its Channel messages: each
// is a u32 BE length, then that many bytes. A message cut short ends it.
export function splitFixture(buf: ArrayBuffer): ArrayBuffer[] {
  const v = new DataView(buf)
  const out: ArrayBuffer[] = []
  let at = 0
  while (at + 4 <= v.byteLength) {
    const n = v.getUint32(at)
    at += 4
    if (at + n > v.byteLength) break
    out.push(buf.slice(at, at + n))
    at += n
  }
  return out
}

// ---- words ---------------------------------------------------------------------

// qualityText is the stage's quality line: "1080p · 10 fps · direct".
export function qualityText(s: CallShareView): string {
  const parts = [`${s.height}p`, `${s.fps} fps`]
  if (s.path) parts.push(s.path)
  return parts.join(" · ")
}

// whatText is what's shared, after the sharer's name: "is sharing a
// window: Windows Terminal", "is sharing their screen: Display 1".
export function whatText(s: CallShareView): string {
  if (s.mine)
    return s.kind === "display"
      ? `sharing your screen: ${s.title}`
      : `sharing a window: ${s.title}`
  return s.kind === "display"
    ? `is sharing their screen: ${s.title}`
    : `is sharing a window: ${s.title}`
}

// sourceLine is a picker row's second line: "Windows Terminal · 1920 × 1080".
export function sourceLine(s: ShareSourceView): string {
  return [s.kind === "window" ? s.app : "", `${s.width} × ${s.height}`]
    .filter(Boolean)
    .join(" · ")
}

// sortSources puts the displays first, then the windows, each kept in the
// order they came.
export function sortSources(list: readonly ShareSourceView[]) {
  return [
    ...list.filter((s) => s.kind === "display"),
    ...list.filter((s) => s.kind !== "display"),
  ]
}

// ---- the player ----------------------------------------------------------------

// canDecode is whether this window has WebCodecs' VideoDecoder at all.
export function canDecode(): boolean {
  return typeof VideoDecoder !== "undefined"
}

type PlayerHooks = {
  // The first picture (and each after) is on the canvas.
  onPicture?: () => void
  // This device can't decode the share (no VideoDecoder, or the codec isn't
  // supported).
  onUnsupported?: () => void
  // Ask the sharer for a keyframe (throttled here to once per 2 s).
  onAskKeyframe?: () => void
}

// SharePlayer decodes the frames into a canvas, latency over smoothness:
// nothing queues, a backed-up decoder drops deltas until the next keyframe,
// and each picture is drawn the moment it comes out.
export class SharePlayer {
  private decoder: VideoDecoder | null = null
  private config: VideoDecoderConfig | null = null
  private needKey = true
  private lastAsk = 0
  private closed = false
  private checking = 0
  // Messages wait here only while a config's support check is out, so a
  // chunk never reaches the decoder before the config it is for.
  private chain: Promise<void> = Promise.resolve()
  private canvas: HTMLCanvasElement
  private hooks: PlayerHooks

  constructor(canvas: HTMLCanvasElement, hooks: PlayerHooks = {}) {
    this.canvas = canvas
    this.hooks = hooks
  }

  // push takes one Channel message.
  push(buf: ArrayBuffer) {
    const f = parseShareFrame(buf)
    if (f) this.feed(f)
  }

  // feed takes one parsed frame.
  feed(f: ShareFrame) {
    if (this.closed) return
    if (f.kind === "config") {
      this.checking++
      this.chain = this.chain
        .then(() => this.configure(f))
        .finally(() => this.checking--)
      return
    }
    // A config is being checked: keep the order.
    if (this.checking > 0) {
      this.chain = this.chain.then(() => this.chunk(f))
      return
    }
    this.chunk(f)
  }

  close() {
    this.closed = true
    this.drop()
  }

  private drop() {
    const d = this.decoder
    this.decoder = null
    if (d && d.state !== "closed") {
      try {
        d.close()
      } catch {
        // Already closed by an error.
      }
    }
  }

  private async configure(f: Extract<ShareFrame, { kind: "config" }>) {
    if (this.closed) return
    if (!canDecode()) {
      this.hooks.onUnsupported?.()
      return
    }
    const config: VideoDecoderConfig = {
      codec: f.codec,
      description: f.description,
      optimizeForLatency: true,
      hardwareAcceleration: "no-preference",
    }
    const ok = await VideoDecoder.isConfigSupported(config).then(
      (r) => !!r.supported,
      () => false
    )
    if (this.closed) return
    if (!ok) {
      this.drop()
      this.config = null
      this.hooks.onUnsupported?.()
      return
    }
    this.config = config
    this.open()
  }

  // open makes (or remakes) the decoder for the current config, then waits
  // for a keyframe.
  private open() {
    if (!this.config) return
    if (this.decoder && this.decoder.state !== "closed") {
      this.decoder.reset()
    } else {
      this.decoder = new VideoDecoder({
        output: (frame) => this.draw(frame),
        error: () => this.failed(),
      })
    }
    this.decoder.configure(this.config)
    this.needKey = true
  }

  private chunk(f: Extract<ShareFrame, { kind: "chunk" }>) {
    const d = this.decoder
    if (this.closed || !d || d.state !== "configured") return
    if (!f.key) {
      if (d.decodeQueueSize > 2) this.needKey = true
      if (this.needKey) {
        this.askKeyframe()
        return
      }
    }
    try {
      d.decode(
        new EncodedVideoChunk({
          type: f.key ? "key" : "delta",
          timestamp: f.timestampUs,
          data: f.data,
        })
      )
      if (f.key) this.needKey = false
    } catch {
      this.failed()
    }
  }

  private draw(frame: VideoFrame) {
    try {
      if (this.closed) return
      const c = this.canvas
      const [w, h] = [frame.displayWidth, frame.displayHeight]
      if (c.width !== w || c.height !== h) {
        c.width = w
        c.height = h
      }
      c.getContext("2d")?.drawImage(frame, 0, 0, w, h)
      this.hooks.onPicture?.()
    } finally {
      frame.close()
    }
  }

  // failed is a decode error: the decoder is closed by it, so a new one
  // takes the same config and waits for a keyframe, which is asked for.
  private failed() {
    if (this.closed) return
    this.decoder = null
    try {
      this.open()
    } catch {
      this.drop()
    }
    this.askKeyframe()
  }

  private askKeyframe() {
    const now = Date.now()
    if (now - this.lastAsk < 2000) return
    this.lastAsk = now
    this.hooks.onAskKeyframe?.()
  }
}

// ---- watching ------------------------------------------------------------------

// Attach and Detach go out one after another, so a quick remount's Detach
// can't land after the Attach that follows it.
let ops: Promise<unknown> = Promise.resolve()

function queued<T>(f: () => Promise<T>): Promise<T> {
  const p = ops.then(f, f)
  ops = p.catch(() => {})
  return p
}

// watchShare starts watching the call's share into `player` and returns
// the stop. In the browser preview the fixture plays instead, looped;
// `onNoFixture` says there is none, so the stage shows its stand-in picture.
export function watchShare(
  player: SharePlayer,
  onNoFixture: () => void
): () => void {
  if (!inWails()) return playFixture(player, onNoFixture)
  let on = true
  void queued(() =>
    ShareService.Attach((buf) => {
      if (on) player.push(buf)
    })
  ).catch(() => {})
  return () => {
    on = false
    void queued(() => ShareService.Detach()).catch(() => {})
  }
}

// askKeyframe asks the sharer for a keyframe (the app only).
export function askKeyframe() {
  if (!inWails()) return
  void ShareService.Keyframe().catch(() => {})
}

const fixtureUrl = `${import.meta.env.BASE_URL}preview/share-fixture.bin`

// playFixture loops the recording at its own timestamps: each pass feeds
// its config again (which resets the decoder), then the chunks as their
// times come.
function playFixture(player: SharePlayer, onNoFixture: () => void) {
  let on = true
  let timer = 0
  void fetch(fixtureUrl)
    .then(async (r) => {
      // A dev server answers a missing file with its index page.
      const type = r.headers.get("content-type") ?? ""
      if (!r.ok || type.includes("html")) return null
      return splitFixture(await r.arrayBuffer())
        .map(parseShareFrame)
        .filter((f): f is ShareFrame => f !== null)
    })
    .catch(() => null)
    .then((frames) => {
      if (!on) return
      if (!frames?.some((f) => f.kind === "config")) {
        onNoFixture()
        return
      }
      const chunks = frames.filter((f) => f.kind === "chunk")
      const first = chunks[0]?.timestampUs ?? 0
      const last = chunks[chunks.length - 1]?.timestampUs ?? first
      // One frame's time after the last, before it starts over.
      const gap = chunks.length > 1 ? (last - first) / (chunks.length - 1) : 0
      const pass = () => {
        if (!on) return
        const start = performance.now()
        let i = 0
        const step = () => {
          if (!on) return
          const now = (performance.now() - start) * 1000
          while (i < frames.length) {
            const f = frames[i]
            if (f.kind === "chunk" && f.timestampUs - first > now) break
            player.feed(f)
            i++
          }
          if (i < frames.length) {
            const f = frames[i] as Extract<ShareFrame, { kind: "chunk" }>
            const wait = (f.timestampUs - first) / 1000 - now / 1000
            timer = window.setTimeout(step, Math.max(0, wait))
          } else {
            timer = window.setTimeout(pass, Math.max(16, gap / 1000))
          }
        }
        step()
      }
      pass()
    })
  return () => {
    on = false
    window.clearTimeout(timer)
  }
}
