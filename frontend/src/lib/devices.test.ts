import { describe, expect, test } from "bun:test"

import { DeviceView } from "./models"
import { reachLine } from "./devices"

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
