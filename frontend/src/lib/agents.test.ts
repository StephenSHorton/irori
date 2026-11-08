import { describe, expect, test } from "bun:test"

import {
  effortLabel,
  idleLabel,
  modelLabel,
  runningSince,
  shortModel,
  weightBy,
  weightLabel,
} from "./agents"
import type { CardWeight } from "../gen/bindings"

describe("Settings › Agents labels", () => {
  test("the default is marked, the rest named", () => {
    expect(modelLabel("sonnet", "sonnet")).toBe("Sonnet (default)")
    expect(modelLabel("opus", "sonnet")).toBe("Opus")
    expect(effortLabel("xhigh", "medium")).toBe("Extra high")
    expect(effortLabel("medium", "medium")).toBe("Medium (default)")
  })
  test("idle limits in minutes or hours, 30 marked", () => {
    expect(idleLabel(10)).toBe("10 minutes")
    expect(idleLabel(30)).toBe("30 minutes (default)")
    expect(idleLabel(60)).toBe("1 hour")
    expect(idleLabel(240)).toBe("4 hours")
  })
  test("an unknown value shows as itself", () => {
    expect(modelLabel("fable", "sonnet")).toBe("fable")
  })
})

describe("runningSince", () => {
  test("minutes, then hours", () => {
    expect(runningSince(0, 30_000)).toBe("just now")
    expect(runningSince(0, 6 * 60_000)).toBe("6 min")
    expect(runningSince(0, 130 * 60_000)).toBe("2 h")
  })
})

describe("a card's weight (card #963)", () => {
  const w = (p: Partial<CardWeight>): CardWeight => ({
    level: "heavy",
    reason: "",
    setBy: "agent",
    byName: "sample-otter",
    model: "opus",
    effort: "high",
    at: "",
    ...p,
  })
  test("level, model and effort, then the reason when there is one", () => {
    expect(weightLabel(w({ reason: "planning the split" }))).toBe(
      "heavy · Opus high: planning the split"
    )
    expect(weightLabel(w({}))).toBe("heavy · Opus high")
    expect(weightLabel(w({ reason: undefined }))).toBe("heavy · Opus high")
    expect(shortModel("sonnet", "xhigh")).toBe("Sonnet extra high")
  })
  test("says who set it; the person's is locked", () => {
    expect(weightBy(w({}))).toBe("sample-otter set it")
    expect(weightBy(w({ setBy: "assistant", byName: "Irori" }))).toBe(
      "Irori's assistant set it"
    )
    expect(weightBy(w({ setBy: "person", byName: "you" }))).toBe(
      "You set it; agents can't change it"
    )
    expect(weightBy(w({ setBy: "person", byName: "Blake" }))).toBe(
      "Blake set it; agents can't change it"
    )
  })
})
