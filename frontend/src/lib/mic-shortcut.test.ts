import { describe, expect, test } from "bun:test"

import { dictationKey, pickMic, type Mic } from "./mic-shortcut"

const k = (
  key: string,
  mods: Partial<
    Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "shiftKey" | "altKey">
  > = {}
) => ({
  key,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
})

describe("dictationKey", () => {
  test("Cmd+D on a Mac", () => {
    expect(dictationKey(k("d", { metaKey: true }), true)).toBe(true)
    expect(dictationKey(k("D", { metaKey: true }), true)).toBe(true)
  })
  test("Ctrl+D on a Mac is the field's delete-forward, not dictation", () => {
    expect(dictationKey(k("d", { ctrlKey: true }), true)).toBe(false)
    expect(dictationKey(k("d", { ctrlKey: true, metaKey: true }), true)).toBe(
      false
    )
  })
  test("Ctrl+D elsewhere, never Cmd/Win+D", () => {
    expect(dictationKey(k("d", { ctrlKey: true }), false)).toBe(true)
    expect(dictationKey(k("D", { ctrlKey: true }), false)).toBe(true)
    expect(dictationKey(k("d", { metaKey: true }), false)).toBe(false)
  })
  test("no Shift or Alt", () => {
    expect(dictationKey(k("d", { metaKey: true, shiftKey: true }), true)).toBe(
      false
    )
    expect(dictationKey(k("d", { ctrlKey: true, altKey: true }), false)).toBe(
      false
    )
  })
  test("only D, and only with the modifier", () => {
    expect(dictationKey(k("d"), true)).toBe(false)
    expect(dictationKey(k("d"), false)).toBe(false)
    expect(dictationKey(k("k", { metaKey: true }), true)).toBe(false)
  })
})

// Boxes stand in for elements: the picker only compares them, reads
// tagName for "is this a text field", and asks inView.
const box = (name: string, tagName = "TEXTAREA") =>
  ({ name, tagName }) as unknown as HTMLElement

function mic(
  b: HTMLElement,
  o: Partial<{
    main: boolean
    live: boolean
    usable: boolean
    mounted: number
    focused: number
  }> = {}
): Mic {
  return {
    box: () => b,
    main: o.main ?? false,
    live: () => o.live ?? false,
    usable: () => o.usable ?? true,
    toggle: () => {},
    mounted: o.mounted ?? 1,
    focused: o.focused ?? 0,
  }
}

const all = () => true

describe("pickMic", () => {
  const ask = box("ask")
  const reply = box("reply")
  const comment = box("comment")

  test("the focused box's mic, even when it isn't the main one", () => {
    const a = mic(ask, { main: true })
    const r = mic(reply)
    expect(pickMic([a, r], reply, all)).toBe(r)
  })

  test("no focus: the main composer in view", () => {
    const a = mic(ask, { main: true })
    const r = mic(reply)
    expect(pickMic([r, a], null, all)).toBe(a)
    expect(pickMic([r, a], { tagName: "BODY" } as Element, all)).toBe(a)
    expect(pickMic([r, a], { tagName: "BUTTON" } as Element, all)).toBe(a)
  })

  test("a main composer out of view or covered doesn't answer", () => {
    const a = mic(ask, { main: true })
    expect(pickMic([a], null, () => false)).toBe(null)
  })

  test("of two main composers in view, the one focused last, then the newest", () => {
    const a = mic(ask, { main: true, mounted: 1 })
    const c = mic(comment, { main: true, mounted: 2 })
    expect(pickMic([a, c], null, all)).toBe(c)
    a.focused = 5
    expect(pickMic([a, c], null, all)).toBe(a)
    expect(pickMic([a, c], null, (b) => b !== ask)).toBe(c)
  })

  test("a listening mic answers first, so the press stops it", () => {
    const a = mic(ask, { main: true })
    const r = mic(reply, { live: true })
    expect(pickMic([a, r], null, all)).toBe(r)
    expect(pickMic([a, r], ask, all)).toBe(r)
  })

  test("focus in another text field: no mic", () => {
    const a = mic(ask, { main: true })
    expect(pickMic([a], box("search", "INPUT"), all)).toBe(null)
    expect(pickMic([a], box("other"), all)).toBe(null)
  })

  test("a focused box whose mic can't start: no mic, not the main one", () => {
    const a = mic(ask, { main: true })
    const r = mic(reply, { usable: false })
    expect(pickMic([a, r], reply, all)).toBe(null)
  })

  test("a main composer that can't start is skipped", () => {
    const a = mic(ask, { main: true, usable: false })
    expect(pickMic([a], null, all)).toBe(null)
  })

  test("only one mic per press", () => {
    const mics = [
      mic(ask, { main: true }),
      mic(comment, { main: true }),
      mic(reply),
    ]
    const picked = pickMic(mics, null, all)
    expect(mics.filter((m) => m === picked)).toHaveLength(1)
  })
})
