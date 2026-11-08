import { describe, expect, test } from "bun:test"

import { Bone, endlessSweeper } from "./endless-animations"

function fakeTimers() {
  const t = { fn: null as null | (() => void), sets: 0, clears: 0 }
  return {
    t,
    timers: {
      set(fn: () => void) {
        t.fn = fn
        t.sets++
        return t.sets
      },
      clear() {
        t.fn = null
        t.clears++
      },
    },
  }
}

const el = (bone = true) => ({
  isConnected: true,
  hasAttribute: (n: string) => bone && n === Bone,
})

function anim(target: ReturnType<typeof el>) {
  const a = {
    effect: { target },
    playState: "running",
    cancelled: 0,
    cancel() {
      a.cancelled++
      a.playState = "idle"
    },
  }
  return a
}

describe("endlessSweeper", () => {
  test("cancels an endless shimmer once its bone leaves the page", () => {
    const { t, timers } = fakeTimers()
    const s = endlessSweeper(timers, 10)
    const bone = el()
    const a = anim(bone)
    s.watch(bone, { iterations: Infinity }, a)
    expect(s.size()).toBe(1)
    expect(t.sets).toBe(1)

    t.fn!()
    expect(a.cancelled).toBe(0)

    bone.isConnected = false
    t.fn!()
    expect(a.cancelled).toBe(1)
    expect(s.size()).toBe(0)
    // Nothing left to watch: the check stops.
    expect(t.clears).toBe(1)
    expect(t.fn).toBeNull()
  })

  test("leaves finite animations and other elements alone", () => {
    const { t, timers } = fakeTimers()
    const s = endlessSweeper(timers, 10)
    s.watch(el(), { duration: 200 }, anim(el()))
    s.watch(el(), 300, anim(el()))
    s.watch(el(false), { iterations: Infinity }, anim(el(false)))
    expect(s.size()).toBe(0)
    expect(t.sets).toBe(0)
  })

  test("forgets an animation skullet cancelled itself, without cancelling again", () => {
    const { t, timers } = fakeTimers()
    const s = endlessSweeper(timers, 10)
    const bone = el()
    const a = anim(bone)
    s.watch(bone, { iterations: Infinity }, a)
    a.playState = "idle"
    bone.isConnected = false
    t.fn!()
    expect(a.cancelled).toBe(0)
    expect(s.size()).toBe(0)
  })

  test("one check runs for many bones and starts again after it stopped", () => {
    const { t, timers } = fakeTimers()
    const s = endlessSweeper(timers, 10)
    const bones = [el(), el(), el()]
    const anims = bones.map((b) => {
      const a = anim(b)
      s.watch(b, { iterations: Infinity }, a)
      return a
    })
    expect(t.sets).toBe(1)
    bones[0].isConnected = false
    expect(s.sweep()).toBe(1)
    expect(s.size()).toBe(2)
    for (const b of bones) b.isConnected = false
    expect(s.sweep()).toBe(2)
    expect(anims.every((a) => a.cancelled === 1)).toBe(true)
    expect(t.clears).toBe(1)

    const b = el()
    s.watch(b, { iterations: Infinity }, anim(b))
    expect(t.sets).toBe(2)
  })
})
