import { expect, test } from "bun:test"

import {
  parseShareFrame,
  qualityText,
  sortSources,
  sourceLine,
  splitFixture,
  whatText,
  type CallShareView,
  type ShareSourceView,
} from "./share"

function config(codec: string, desc: number[]): ArrayBuffer {
  const c = new TextEncoder().encode(codec)
  const b = new Uint8Array(3 + c.length + desc.length)
  b[0] = 1
  new DataView(b.buffer).setUint16(1, c.length)
  b.set(c, 3)
  b.set(desc, 3 + c.length)
  return b.buffer
}

function chunk(key: boolean, ts: bigint, data: number[]): ArrayBuffer {
  const b = new Uint8Array(10 + data.length)
  const v = new DataView(b.buffer)
  b[0] = 2
  b[1] = key ? 1 : 0
  v.setBigUint64(2, ts)
  b.set(data, 10)
  return b.buffer
}

test("parseShareFrame reads a config: the codec, then the avcC after it", () => {
  const f = parseShareFrame(config("avc1.640028", [1, 0x64, 0, 0x28, 0xff]))
  expect(f?.kind).toBe("config")
  if (f?.kind !== "config") return
  expect(f.codec).toBe("avc1.640028")
  expect([...f.description]).toEqual([1, 0x64, 0, 0x28, 0xff])
  expect(f.description.byteOffset).toBe(3 + "avc1.640028".length)
})

test("parseShareFrame reads a chunk: the key bit, the time, the access unit", () => {
  // Past 2^32 µs (about 71 minutes), so the high word counts.
  const ts = 5_000_000_123n
  const f = parseShareFrame(chunk(true, ts, [0, 0, 0, 2, 9, 0x10]))
  expect(f?.kind).toBe("chunk")
  if (f?.kind !== "chunk") return
  expect(f.key).toBe(true)
  expect(f.timestampUs).toBe(5_000_000_123)
  expect(f.data.byteOffset).toBe(10)
  expect([...f.data]).toEqual([0, 0, 0, 2, 9, 0x10])

  const d = parseShareFrame(chunk(false, 33_333n, [7]))
  expect(d).toMatchObject({ kind: "chunk", key: false, timestampUs: 33_333 })
  // Only bit 0 is the key.
  const b = new Uint8Array(chunk(false, 1n, [7]))
  b[1] = 0b10
  expect(parseShareFrame(b.buffer)).toMatchObject({ key: false })
})

test("parseShareFrame refuses what's cut short or unknown", () => {
  expect(parseShareFrame(new ArrayBuffer(0))).toBeNull()
  expect(parseShareFrame(new Uint8Array([3, 0, 0]).buffer)).toBeNull()
  expect(parseShareFrame(new Uint8Array([2, 1, 0, 0]).buffer)).toBeNull()
  // A codec length past the end.
  expect(parseShareFrame(new Uint8Array([1, 0, 9, 97]).buffer)).toBeNull()
  // No codec at all.
  expect(parseShareFrame(new Uint8Array([1, 0, 0]).buffer)).toBeNull()
})

test("splitFixture cuts the recording at its u32 BE lengths", () => {
  const parts = [config("avc1.42e01f", [1]), chunk(true, 0n, [1, 2, 3])]
  const total = parts.reduce((n, p) => n + 4 + p.byteLength, 0)
  const b = new Uint8Array(total + 3)
  const v = new DataView(b.buffer)
  let at = 0
  for (const p of parts) {
    v.setUint32(at, p.byteLength)
    b.set(new Uint8Array(p), at + 4)
    at += 4 + p.byteLength
  }
  // A last message cut short ends it.
  b.set([0, 0, 0], at)
  const out = splitFixture(b.buffer)
  expect(out.length).toBe(2)
  expect(parseShareFrame(out[0])).toMatchObject({ codec: "avc1.42e01f" })
  expect(parseShareFrame(out[1])).toMatchObject({ key: true, timestampUs: 0 })
})

const theirs: CallShareView = {
  shareId: "s1",
  mine: false,
  name: "Spencer",
  device: "",
  kind: "window",
  title: "Windows Terminal",
  app: "Windows Terminal",
  width: 1920,
  height: 1080,
  fps: 10,
  kbps: 1800,
  path: "direct",
}

test("the stage's words: what's shared and how well", () => {
  expect(qualityText(theirs)).toBe("1080p · 10 fps · direct")
  expect(qualityText({ ...theirs, path: "" })).toBe("1080p · 10 fps")
  expect(whatText(theirs)).toBe("is sharing a window: Windows Terminal")
  expect(whatText({ ...theirs, kind: "display", title: "Display 1" })).toBe(
    "is sharing their screen: Display 1"
  )
  expect(whatText({ ...theirs, mine: true })).toBe(
    "sharing a window: Windows Terminal"
  )
})

test("the picker: displays first, then windows with their app", () => {
  const w = (id: string, kind: string): ShareSourceView => ({
    id,
    kind,
    title: id,
    app: kind === "window" ? "Terminal" : "",
    width: 1280,
    height: 800,
  })
  const list = [w("a", "window"), w("b", "display"), w("c", "window")]
  expect(sortSources(list).map((s) => s.id)).toEqual(["b", "a", "c"])
  expect(sourceLine(list[0])).toBe("Terminal · 1280 × 800")
  expect(sourceLine(list[1])).toBe("1280 × 800")
})
