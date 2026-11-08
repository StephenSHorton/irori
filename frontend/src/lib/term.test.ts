import { describe, expect, test } from "bun:test"

import { inTurn } from "./term"

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
