import { describe, expect, test } from "bun:test"

import { DeviceView } from "./models"
import { devices, reachLine, readDevices, shownDevices } from "./devices"

const dev = (p: Partial<DeviceView>) =>
  DeviceView({ deviceId: "d", name: "D", ...p })

describe("reachLine", () => {
  test("direct, relay and unreachable", () => {
    expect(reachLine(dev({ reach: "direct" }))).toBe("direct")
    expect(reachLine(dev({ reach: "relay" }))).toBe("via relay")
    expect(reachLine(dev({ reach: "unreachable" }))).toBe("unreachable")
  })
  test("what a device is behind on", () => {
    expect(
      reachLine(
        dev({ reach: "unreachable", version: "0.11.0", behind: "0.12.1" })
      )
    ).toBe("unreachable · runs 0.11.0, behind 0.12.1")
    expect(reachLine(dev({ reach: "direct", behind: "0.12.1" }))).toBe(
      "direct · behind 0.12.1"
    )
  })
  test("nothing for this device or a removed one", () => {
    expect(reachLine(dev({ this: true, reach: "direct" }))).toBe("")
    expect(reachLine(dev({ revoked: true, reach: "unreachable" }))).toBe("")
    expect(reachLine(dev({}))).toBe("")
  })
})

describe("reachLine through another device", () => {
  test("rows still arrive: no direct link, not unreachable", () => {
    expect(reachLine(dev({ reach: "unreachable", via: "Phone" }))).toBe(
      "no direct link"
    )
  })
})

describe("a removed device is forgotten (card #1240)", () => {
  // The sample runs outside the host; bun has no window.
  const g = globalThis as unknown as { window?: unknown }
  g.window ??= { location: { search: "" } }

  test("a settled removal has no row, a pending one keeps its row", () => {
    const pending = dev({
      deviceId: "p",
      revoked: true,
      holdouts: [{ deviceId: "n", name: "Node" }],
    })
    const shown = shownDevices([
      dev({ deviceId: "a" }),
      dev({ deviceId: "gone", revoked: true }),
      dev({ deviceId: "gone2", revoked: true, holdouts: [] }),
      pending,
    ])
    expect(shown.map((d) => d.deviceId)).toEqual(["a", "p"])
  })
  test("the sample's Remove takes the device off the list and its holdouts", async () => {
    const before = await readDevices()
    expect(before.some((d) => d.deviceId === "sample-laptop")).toBe(true)
    await devices.remove("sample-laptop")
    const ids = (await readDevices()).map((d) => d.deviceId)
    expect(ids).not.toContain("sample-laptop")
    // The pending removal waits on Node: it stays until Node is removed.
    expect(ids).toContain("sample-old")
    await devices.remove("sample-node")
    const after = await readDevices()
    expect(after.map((d) => d.deviceId)).not.toContain("sample-node")
    expect(after.map((d) => d.deviceId)).not.toContain("sample-old")
    expect(after.some((d) => d.revoked)).toBe(false)
  })
})
