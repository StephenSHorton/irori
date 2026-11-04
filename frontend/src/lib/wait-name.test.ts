import { describe, expect, test } from "bun:test"

import { leaseNames, waitLine, waitName } from "./wait-name"

// Card #487: a lease wait in plain words, not the raw names.
describe("waitName", () => {
  test("a lease family is a free one of it", () => {
    expect(
      leaseNames(["ww/dolphin-slot-1", "ww/dolphin-slot-2", "ww/dolphin-slot-3", "ww/dolphin-slot-4"])
    ).toBe("a free Dolphin slot")
    expect(leaseNames(["check/macbook/slot-1", "check/macbook/slot-2"])).toBe(
      "a free check slot on macbook"
    )
  })
  test("one lease is itself", () => {
    expect(leaseNames(["ww/dolphin-slot-2"])).toBe("Dolphin slot 2")
    expect(leaseNames(["gpu/qwen"])).toBe("gpu/qwen")
    expect(leaseNames(["port:8080"])).toBe("port 8080")
    expect(leaseNames(["desk/irori"])).toBe("the merge desk")
    expect(leaseNames(["path:irori:crates/coord/**"])).toBe("crates/coord/** in irori")
  })
  test("two families name the first and count the rest", () => {
    expect(leaseNames(["ww/dolphin-slot-1", "ww/dolphin-slot-2", "port:9246"])).toBe(
      "a free Dolphin slot and 1 more"
    )
  })
  test("an external wait naming a lease family reads the same", () => {
    const on = "external ww/dolphin-slot-1..4 leases (reed until the netem run ends)"
    expect(waitName({ waitKind: "external", waitOn: on })).toBe("a free Dolphin slot")
    expect(waitName({ waitOn: on })).toBe("a free Dolphin slot")
    expect(waitLine({ waitKind: "external", waitOn: on })).toBe("Waiting for a free Dolphin slot")
  })
  test("anything else keeps its words", () => {
    expect(waitLine({ waitKind: "agent", waitOn: "kestrel" })).toBe("Waiting on kestrel")
    expect(waitLine({ waitKind: "task", waitOn: "#12" })).toBe("Waiting on #12")
    expect(waitLine({ waitKind: "external", waitOn: "external CI on main" })).toBe("Waiting on CI on main")
    expect(waitLine({ waitKind: "lease", waitOn: "ww/dolphin-slot-1 ww/dolphin-slot-2" })).toBe(
      "Waiting for a free Dolphin slot"
    )
    expect(waitLine({})).toBe("Waiting on something else")
  })
})
