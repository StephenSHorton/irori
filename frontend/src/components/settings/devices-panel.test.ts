import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { HomeServerView, PairStatus } from "@/lib/models"
import { inviteReachLine } from "../boards/parts"
import { homeLine, PairAddrs, pairAddrLines } from "./devices-panel"

const CGNAT =
  "Your internet provider shares your public address (CGNAT), so devices away from home can't reach this PC, and a port forward won't help."
const FORWARD =
  "Away from home your devices can't reach this PC. Turn on UPnP on your router, or forward TCP and UDP port 25817 to 192.168.1.20 and enter your public address below."

const home = (p: Partial<HomeServerView>) =>
  HomeServerView({ can: true, on: true, running: true, outside: true, ...p })

describe("issue 9: warnings with public relays", () => {
  test("invite: no warning behind CGNAT with public relays on", () => {
    expect(
      inviteReachLine(home({ reach: "cgnat", line: CGNAT, publicRelays: true }))
    ).toBe("")
  })
  test("invite: no warning with the home server off and public relays on", () => {
    expect(
      inviteReachLine(home({ on: false, running: false, publicRelays: true }))
    ).toBe("")
  })
  test("invite: today's lines with public relays off", () => {
    expect(
      inviteReachLine(
        home({ reach: "cgnat", line: CGNAT, publicRelays: false })
      )
    ).toContain("can't be reached from outside your home yet")
    expect(
      inviteReachLine(home({ on: false, running: false, publicRelays: false }))
    ).toContain("Your home server is off")
    expect(
      inviteReachLine(home({ reach: "outside", publicRelays: false }))
    ).toBe("")
  })
  test("devices: CGNAT says public relays cover it when on", () => {
    const l = homeLine(
      home({ reach: "cgnat", line: CGNAT, publicRelays: true })
    )
    expect(l).toContain("public relays connect your devices")
    expect(l).not.toContain("can't reach this PC")
  })
  test("devices: port-forward keeps the router steps after the relays line", () => {
    const l = homeLine(
      home({ reach: "forward", line: FORWARD, publicRelays: true })
    )
    expect(l).toContain("public relays connect your devices")
    expect(l).toContain("For a direct connection: Turn on UPnP")
    expect(l).not.toContain("can't reach this PC")
  })
  test("devices: lines unchanged with public relays off, and other lines always", () => {
    expect(
      homeLine(home({ reach: "cgnat", line: CGNAT, publicRelays: false }))
    ).toBe(CGNAT)
    const moved = "Port 25817 was taken, so the server moved to port 25818."
    expect(
      homeLine(home({ reach: "home", line: moved, publicRelays: true }))
    ).toBe(moved)
  })
})

// Card #1260: the Add window says where a joiner that can't find this
// device types its way in, instead of only the code.
describe("the address under the pairing code", () => {
  const LAN = "192.168.1.20:24817"
  const OUT = "203.0.113.7:24817"
  const addr = (p: Partial<PairStatus>) =>
    renderToStaticMarkup(
      createElement(PairAddrs, {
        status: PairStatus({ mode: "add", state: "waiting", ...p }),
      })
    )

  test("home only: the LAN address with its own Copy", () => {
    expect(pairAddrLines({ lanAddr: LAN })).toEqual([
      { lead: "If the other device can't find this one, enter", addr: LAN },
    ])
    const html = addr({ lanAddr: LAN })
    expect(html).toContain("If the other device can&#x27;t find this one")
    expect(html).toContain(LAN)
    expect(html).toContain(`aria-label="Copy ${LAN}"`)
    expect(html).not.toContain("outside your home")
  })
  test("reachable from outside too: both lines", () => {
    const l = pairAddrLines({ lanAddr: LAN, serverAddr: OUT })
    expect(l.map((x) => x.addr)).toEqual([LAN, OUT])
    expect(addr({ lanAddr: LAN, serverAddr: OUT })).toContain(
      `aria-label="Copy ${OUT}"`
    )
  })
  test("only an outside address (another device's server)", () => {
    expect(pairAddrLines({ serverAddr: OUT })).toEqual([
      { lead: "If it's outside your home, enter", addr: OUT },
    ])
  })
  test("one address shows once", () => {
    expect(pairAddrLines({ lanAddr: LAN, serverAddr: LAN })).toHaveLength(1)
  })
  test("no server: nothing under the code", () => {
    expect(pairAddrLines({})).toEqual([])
    expect(addr({})).toBe("")
  })
})
