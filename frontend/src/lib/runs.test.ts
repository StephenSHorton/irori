import { describe, expect, test } from "bun:test"

import { AgentRef } from "../gen/models"
import {
  busyLine,
  busyList,
  cardLoad,
  endText,
  fgText,
  fmtCores,
  heavyRuns,
  runChipText,
  runsOf,
  withLoad,
  withRuns,
  runId,
  type CardRun,
} from "./runs"

const GB = 1024 ** 3
const now = 1_762_700_000_000
const here = AgentRef({ key: "dev/a", name: "a", here: true })
const away = AgentRef({ key: "mac/b", name: "b", here: false, where: "Mac" })

const run = (r: Partial<CardRun>): CardRun => ({
  who: here,
  run: "b1",
  kind: "bash",
  cmd: "cargo test",
  started: now - 4 * 60_000,
  exit: null,
  stopped: false,
  device: "",
  ...r,
})

describe("run chips", () => {
  test("a run here says its load and time", () => {
    const r = run({ cpuCores: 11.8, memBytes: 5.8 * GB })
    expect(runChipText(r, now)).toBe("cargo test · 12 cores · 5.8 GB · 4 min")
    expect(runChipText(r, now, true)).toBe("12 cores · 5.8 GB")
  })
  test("before its second reading it shows what is known", () => {
    expect(runChipText(run({ cpuCores: null }), now)).toBe("cargo test · 4 min")
  })
  test("a run on another device says where, then its end and peaks", () => {
    const r = run({ who: away, device: "Mac" })
    expect(runChipText(r, now)).toBe("cargo test · 4 min · on Mac")
    const over = {
      ...r,
      ended: now,
      exit: 1,
      peakCores: 12,
      peakMemBytes: 2 * GB,
    }
    expect(runChipText(over, now)).toBe(
      "cargo test · Exited with code 1 · peak 12 cores · 2.0 GB · on Mac"
    )
  })
  test("cores read plainly", () => {
    expect(fmtCores(1)).toBe("1 core")
    expect(fmtCores(0.42)).toBe("0.4 cores")
    expect(fmtCores(2.5)).toBe("2.5 cores")
    expect(fmtCores(15.6)).toBe("16 cores")
  })
  test("an end reads Stopped over its code", () => {
    expect(endText({ exit: 1, stopped: true })).toBe("Stopped")
    expect(endText({ exit: 0, stopped: false })).toBe("Exited with code 0")
    expect(endText({ exit: null, stopped: false })).toBe("Ended")
  })
})

describe("heavy runs", () => {
  test("only background runs still going, heaviest first", () => {
    const list = heavyRuns([
      run({ run: "light", cpuCores: 1 }),
      run({ run: "fg", kind: "fg", cpuCores: 9 }),
      run({ run: "over", exit: 0, cpuCores: 9 }),
      run({ run: "stopped", stopped: true, cpuCores: 9 }),
      run({ run: "big", cpuCores: 3, memBytes: 4 * GB }),
    ])
    expect(list.map((r) => r.run)).toEqual(["big", "light"])
  })
  test("a live reading replaces the card's", () => {
    const r = run({ cpuCores: 1 })
    const loads = new Map([
      [
        runId(r),
        {
          key: "dev/a",
          run: "b1",
          cores: 7,
          memBytes: GB,
          at: now,
          device: "",
        },
      ],
    ])
    expect(withLoad(r, loads).cpuCores).toBe(7)
    expect(withLoad(r, new Map()).cpuCores).toBe(1)
  })
})

describe("Busy now", () => {
  const card = (id: string, short: string, runs: CardRun[]) =>
    withRuns({ id, short }, runs)
  test("cards by load, a remote run left out", () => {
    const list = busyList([
      card("c12", "12", [run({ cmd: "bun test", cpuCores: 1.5 })]),
      card("c54", "54", [run({ cpuCores: 11.8, memBytes: 5.8 * GB })]),
      card("c9", "9", [run({ who: away, device: "Mac" })]),
      card("c3", "3", []),
    ])
    expect(list.map((c) => c.short)).toEqual(["54", "12"])
    expect(busyLine(list)).toBe(
      "Busy now: #54 cargo test 12 cores · 5.8 GB, #12 bun test 1.5 cores"
    )
  })
  test("nothing heavy is no line", () => {
    expect(busyLine(busyList([card("c1", "1", [])]))).toBe("")
  })
  test("a card's runs add up", () => {
    const l = cardLoad({ id: "c", short: "1" }, [
      run({ run: "a", cpuCores: 2, memBytes: GB }),
      run({ run: "b", cpuCores: 3, memBytes: GB }),
    ])
    expect(l).toMatchObject({ cores: 5, memBytes: 2 * GB, cmd: "cargo test" })
  })
})

describe("the seam", () => {
  test("no runs leaves the field out", () => {
    expect("runs" in withRuns({ id: "x" }, [])).toBe(false)
    expect(runsOf({ id: "x" })).toEqual([])
    expect(runsOf(withRuns({ id: "x" }, [run({})]))).toHaveLength(1)
  })
})

describe("foreground rows", () => {
  test("time and CPU", () => {
    const r = run({
      kind: "fg",
      cmd: "cargo check",
      started: now - 40_000,
      ended: now,
      cpuCores: 2.1,
    })
    expect(fgText(r, now)).toBe("cargo check · 40s · 2.1 cores")
  })
})
