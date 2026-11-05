import { describe, expect, test } from "bun:test"

import { latestReads, StuckReadMs, type Answer } from "./talk"

// Card #551: useLatest's rule, with reads finished by hand. A read here is
// a promise the test settles, so "slow" means "not settled yet".
type Read = {
  resolve: (v: string) => void
  reject: (e: unknown) => void
}

function rig() {
  const shown: Answer<string>[] = []
  const clock = { now: 0 }
  const reads: Read[] = []
  const load = () =>
    new Promise<string>((resolve, reject) => reads.push({ resolve, reject }))
  const other = () =>
    new Promise<string>((resolve, reject) => reads.push({ resolve, reject }))
  const read = latestReads<string>(
    (a) => shown.push(a),
    () => clock.now
  )
  const values = () => shown.map((a) => ("value" in a ? a.value : "error"))
  return { values, clock, reads, load, other, read }
}

const settled = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

describe("latestReads: events faster than the read (card #551)", () => {
  test("the list updates while events keep coming", async () => {
    const r = rig()
    let answered = 0
    // Each read takes 3 events: 12 events, none of them a quiet gap.
    for (let tick = 1; tick <= 12; tick++) {
      void r.read(r.load)
      if (tick % 3 === 0) {
        r.reads[answered].resolve(`list at ${tick}`)
        answered++
        await settled()
      }
    }
    expect(r.values()).toEqual([
      "list at 3",
      "list at 6",
      "list at 9",
      "list at 12",
    ])
  })

  test("a burst of events costs one read and one re-run, not one each", async () => {
    const r = rig()
    for (let i = 0; i < 20; i++) void r.read(r.load)
    expect(r.reads).toHaveLength(1)
    r.reads[0].resolve("a")
    await settled()
    expect(r.reads).toHaveLength(2)
    r.reads[1].resolve("b")
    await settled()
    expect(r.reads).toHaveLength(2)
    expect(r.values()).toEqual(["a", "b"])
  })

  test("a quiet window reads once", async () => {
    const r = rig()
    void r.read(r.load)
    r.reads[0].resolve("a")
    await settled()
    expect(r.reads).toHaveLength(1)
  })

  test("an awaited reload sees a read begun after it was called", async () => {
    const r = rig()
    void r.read(r.load)
    let done = false
    void r.read(r.load).then(() => (done = true))
    r.reads[0].resolve("before the action")
    await settled()
    expect(done).toBe(false)
    r.reads[1].resolve("after the action")
    await settled()
    expect(done).toBe(true)
    expect(r.values()).toEqual(["before the action", "after the action"])
  })
})

describe("latestReads: an older answer never replaces a newer one", () => {
  test("a stuck read doesn't freeze the list, and can't undo it when it wakes", async () => {
    const r = rig()
    void r.read(r.load)
    r.clock.now += StuckReadMs - 1
    void r.read(r.load)
    expect(r.reads).toHaveLength(1)
    r.clock.now += 1
    void r.read(r.load)
    expect(r.reads).toHaveLength(2)
    r.reads[1].resolve("newer")
    await settled()
    r.reads[0].resolve("older")
    await settled()
    expect(r.values()).toEqual(["newer"])
  })

  test("a failure that comes back late can't replace a newer answer", async () => {
    const r = rig()
    void r.read(r.load)
    r.clock.now += StuckReadMs
    void r.read(r.load)
    r.reads[1].resolve("newer")
    await settled()
    r.reads[0].reject(new Error("late"))
    await settled()
    expect(r.values()).toEqual(["newer"])
  })

  test("two reads answering in order both show, the newer last", async () => {
    const r = rig()
    void r.read(r.load)
    r.clock.now += StuckReadMs
    void r.read(r.load)
    r.reads[0].resolve("older")
    await settled()
    r.reads[1].resolve("newer")
    await settled()
    expect(r.values()).toEqual(["older", "newer"])
  })
})

describe("latestReads: the list changes (another board or conversation)", () => {
  test("the new list reads at once, and the old one's answer is dropped", async () => {
    const r = rig()
    void r.read(r.load)
    void r.read(r.other)
    expect(r.reads).toHaveLength(2)
    r.reads[0].resolve("old board")
    await settled()
    expect(r.values()).toEqual([])
    r.reads[1].resolve("new board")
    await settled()
    expect(r.values()).toEqual(["new board"])
    expect(r.reads).toHaveLength(2)
  })

  test("a re-run that was waiting is covered by the new list's read", async () => {
    const r = rig()
    void r.read(r.load)
    let waited = false
    void r.read(r.load).then(() => (waited = true))
    void r.read(r.other)
    r.reads[1].resolve("new board")
    await settled()
    expect(waited).toBe(true)
    r.reads[0].resolve("old board")
    await settled()
    expect(r.values()).toEqual(["new board"])
    expect(r.reads).toHaveLength(2)
  })
})

describe("latestReads: failures", () => {
  test("a failed read shows, and the re-run behind it still goes", async () => {
    const r = rig()
    void r.read(r.load)
    void r.read(r.load)
    r.reads[0].reject(new Error("no"))
    await settled()
    expect(r.values()).toEqual(["error"])
    r.reads[1].resolve("back")
    await settled()
    expect(r.values()).toEqual(["error", "back"])
  })

  test("after a failure with nothing waiting, the next reload reads", async () => {
    const r = rig()
    void r.read(r.load)
    r.reads[0].reject(new Error("no"))
    await settled()
    void r.read(r.load)
    expect(r.reads).toHaveLength(2)
  })
})
