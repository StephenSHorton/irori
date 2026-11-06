import { describe, expect, test } from "bun:test"

import { MaxSides, cleanSides, sameSide } from "@/lib/devices"
import { sidesLine } from "./devices-panel"

describe("card #494: sides", () => {
  test("cleanSides trims, drops blanks and repeats in any case, keeps order", () => {
    expect(
      cleanSides(["  Work ", "work", "", " ", "Home  lab", "Personal"])
    ).toEqual(["Work", "Home lab", "Personal"])
  })
  test("cleanSides keeps at most MaxSides", () => {
    const many = Array.from({ length: MaxSides + 3 }, (_, i) => `s${i}`)
    expect(cleanSides(many)).toHaveLength(MaxSides)
  })
  test("sameSide ignores case and spacing", () => {
    expect(sameSide("Work", " work ")).toBe(true)
    expect(sameSide("Work", "Personal")).toBe(false)
  })
  test("no pin runs every side", () => {
    expect(sidesLine([])).toStartWith("This device runs every side.")
  })
  test("a pin names its sides and where new cards go", () => {
    expect(sidesLine(["Work"])).toBe(
      "This device runs Work, and cards with no side. New cards typed here go on Work. Every device still shows every card."
    )
    expect(sidesLine(["Personal", "Garden", "Work"])).toContain(
      "runs Personal, Garden and Work"
    )
  })
})
