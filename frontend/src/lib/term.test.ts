import { describe, expect, test } from "bun:test"

import {
  inTurn,
  mergeTails,
  newestShell,
  tailEnded,
  type TermInfo,
} from "./term"

// A promise resolved by hand.
function gate() {
  let open = () => {}
  const p = new Promise<void>((r) => {
    open = r
  })
  return { p, open }
}

describe("inTurn", () => {
  test("a shell's next call waits for the one before it", async () => {
    // The card section's detach still in flight when the panel attaches
    // (card #734): the attach must not go out before the detach ended.
    const order: string[] = []
    const detach = gate()
    const first = inTurn(1, async () => {
      order.push("detach start")
      await detach.p
      order.push("detach end")
    })
    const second = inTurn(1, async () => {
      order.push("attach")
    })
    await Promise.resolve()
    await Promise.resolve()
    expect(order).toEqual(["detach start"])
    detach.open()
    await Promise.all([first, second])
    expect(order).toEqual(["detach start", "detach end", "attach"])
  })

  test("a failed call doesn't stop the next one", async () => {
    const bad = inTurn(2, () => Promise.reject(new Error("gone")))
    const good = inTurn(2, () => Promise.resolve("ok"))
    await expect(bad).rejects.toThrow("gone")
    expect(await good).toBe("ok")
  })

  test("other shells don't wait", async () => {
    const held = gate()
    const slow = inTurn(3, () => held.p)
    expect(await inTurn(4, () => Promise.resolve(4))).toBe(4)
    held.open()
    await slow
  })
})

// Run tabs (card #1249, live-tasks.md §5.2).
const shell = (id: number): TermInfo => ({
  id,
  scope: { kind: "card", id: "c" },
  cwd: "/r",
  title: "",
  running: true,
})
const tail = (id: number, end?: Partial<TermInfo>): TermInfo => ({
  ...shell(id),
  tail: { label: "cargo test", run: `b${id}`, stopped: false },
  ...end,
})

describe("run tabs", () => {
  test("a card shows its newest shell on mount, never a run tab", () => {
    expect(newestShell([shell(1), shell(2), tail(3)])).toBe(2)
    expect(newestShell([tail(3), tail(4)])).toBeNull()
    expect(newestShell([])).toBeNull()
  })

  test("a fresh list adds new run tabs and leaves shells alone", () => {
    const shown = [shell(1), tail(3)]
    const fresh = [shell(1), shell(9), tail(3), tail(4)]
    expect(mergeTails(shown, fresh).map((t) => t.id)).toEqual([1, 3, 4])
  })

  test("a run tab takes its end from the fresh list", () => {
    const ended = tail(3, { running: false })
    ended.tail = { ...ended.tail!, exit: 1 }
    const got = mergeTails([shell(1), tail(3)], [shell(1), ended])
    expect(got[1].running).toBe(false)
    expect(got[1].tail?.exit).toBe(1)
  })

  test("a run tab the Core closed goes; a shell stays", () => {
    const got = mergeTails([shell(1), tail(3)], [])
    expect(got.map((t) => t.id)).toEqual([1])
  })

  test("an end heard on the tab stays on it", () => {
    const got = tailEnded([shell(1), tail(3)], 3, 101)
    expect(got[0]).toEqual(shell(1))
    expect(got[1].running).toBe(false)
    expect(got[1].tail?.exit).toBe(101)
    // Stopped: no code, and the list's own end is kept.
    expect(tailEnded([tail(3)], 3, null)[0].tail?.exit).toBeNull()
  })
})
