import { describe, expect, test } from "bun:test"

import { waitLabel, waitTip } from "@/components/cards/bits"

import { leaseNames, waitLine, waitName } from "./wait-name"

// Card #487: a lease wait in plain words, not the raw names.
describe("waitName", () => {
  test("a lease family is a free one of it", () => {
    expect(
      leaseNames([
        "ww/dolphin-slot-1",
        "ww/dolphin-slot-2",
        "ww/dolphin-slot-3",
        "ww/dolphin-slot-4",
      ])
    ).toBe("a free Dolphin slot")
    expect(leaseNames(["check/macbook/slot-1", "check/macbook/slot-2"])).toBe(
      "a free check slot on macbook"
    )
    expect(leaseNames(["heavy/3dc30c94/slot-1", "heavy/3dc30c94/slot-2"])).toBe(
      "a free heavy slot"
    )
  })
  test("one lease is itself", () => {
    expect(leaseNames(["ww/dolphin-slot-2"])).toBe("Dolphin slot 2")
    expect(leaseNames(["gpu/qwen"])).toBe("gpu/qwen")
    expect(leaseNames(["port:8080"])).toBe("port 8080")
    expect(leaseNames(["desk/irori"])).toBe("the merge desk")
    expect(
      leaseNames(["path:remote:github.com/sh/irori:crates/coord/**"])
    ).toBe("crates/coord/** in irori")
    expect(leaseNames(["path:root:abc1234def:src/*.rs"])).toBe(
      "src/*.rs in repo abc1234"
    )
    expect(leaseNames(["check/mac/slot-3"])).toBe("check slot 3 on mac")
    expect(leaseNames(["heavy/3dc30c94/slot-2"])).toBe("heavy slot 2")
  })
  test("free text that isn't a lease keeps its words", () => {
    expect(
      waitLine({
        waitKind: "external",
        waitOn: "external deploy to aws/us-east-1 finishing",
      })
    ).toBe("Waiting on deploy to aws/us-east-1 finishing")
    expect(
      waitLine({
        waitKind: "external",
        waitOn: "external CI at https://ci.example.com/job/build-42",
      })
    ).toBe("Waiting on CI at https://ci.example.com/job/build-42")
    expect(
      waitLine({ waitKind: "external", waitOn: "the ww/dolphin-slot-2 lease" })
    ).toBe("Waiting for Dolphin slot 2")
  })
  test("two families name the first and count the rest", () => {
    expect(
      leaseNames(["ww/dolphin-slot-1", "ww/dolphin-slot-2", "port:9246"])
    ).toBe("a free Dolphin slot and 1 more")
  })
  test("an external wait naming a lease family reads the same", () => {
    const on =
      "external ww/dolphin-slot-1..4 leases (reed until the netem run ends)"
    expect(waitName({ waitKind: "external", waitOn: on })).toBe(
      "a free Dolphin slot"
    )
    expect(waitName({ waitOn: on })).toBe("a free Dolphin slot")
    expect(waitLine({ waitKind: "external", waitOn: on })).toBe(
      "Waiting for a free Dolphin slot"
    )
  })
  test("anything else keeps its words", () => {
    expect(waitLine({ waitKind: "agent", waitOn: "kestrel" })).toBe(
      "Waiting on kestrel"
    )
    expect(waitLine({ waitKind: "task", waitOn: "#12" })).toBe("Waiting on #12")
    expect(
      waitLine({ waitKind: "external", waitOn: "external CI on main" })
    ).toBe("Waiting on CI on main")
    expect(
      waitLine({
        waitKind: "lease",
        waitOn: "ww/dolphin-slot-1 ww/dolphin-slot-2",
      })
    ).toBe("Waiting for a free Dolphin slot")
    expect(waitLine({})).toBe("Waiting on something else")
  })
})

// Card #553: a version wait's pill names the version, its tooltip the
// devices too.
describe("a version wait", () => {
  const card = {
    waitingOn: 1,
    waitKind: "version",
    waitOn: "Irori 0.12.4",
    waitFull: "Irori 0.12.4 on Desktop and another device",
  }
  test("the pill says the version", () => {
    expect(waitLabel(card)).toBe("Waiting on Irori 0.12.4")
    expect(waitLabel({ ...card, waitingOn: 2 })).toBe(
      "Waiting on Irori 0.12.4 +1"
    )
  })
  test("the tooltip names the devices, then the question", () => {
    expect(waitTip(card)).toBe(
      "Waiting on Irori 0.12.4 on Desktop and another device"
    )
    expect(waitTip({ ...card, waitQuestion: "After the update" })).toBe(
      "Waiting on Irori 0.12.4 on Desktop and another device: After the update"
    )
  })
  test("any other wait's tooltip is its label and question", () => {
    expect(
      waitTip({
        waitingOn: 1,
        waitKind: "agent",
        waitOn: "kestrel",
        waitQuestion: "Soon",
      })
    ).toBe("Waiting on kestrel: Soon")
  })
})
