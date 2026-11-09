// Card #1261: the joining device's address box, on the Join form and in the
// Looking view, and the retry that keeps the code.
import { describe, expect, test } from "bun:test"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"

import { PairStatus } from "@/lib/models"
import { devices, PairState, skipPairing } from "@/lib/devices"
import { AddressLink, Looking, LookingAskMs, Start } from "./devices-panel"

// The sample runs outside the host; bun has no window. Nothing reads it
// before a test runs.
const g = globalThis as unknown as { window?: Record<string, unknown> }
g.window ??= { location: { search: "" } }
g.window.setTimeout ??= setTimeout
g.window.clearTimeout ??= clearTimeout

const looking = (p: Partial<PairStatus> = {}) =>
  PairStatus({ mode: "join", state: PairState.Waiting, ...p })

const renderLooking = (
  props: Partial<Parameters<typeof Looking>[0]> = {}
): string =>
  renderToStaticMarkup(
    createElement(Looking, {
      status: looking(),
      cancel: null,
      canRetry: true,
      onRetry: async () => {},
      ...props,
    })
  )

describe("the Join form's address link", () => {
  test("reads plainly, not as away from home", () => {
    expect(AddressLink).toBe("Can't find it? Enter the other device's address")
    const html = renderToStaticMarkup(
      createElement(Start, { onStarted: () => {}, join: true })
    )
    expect(html).toContain(
      "Can&#x27;t find it? Enter the other device&#x27;s address"
    )
    expect(html).not.toContain("outside your home")
    expect(html).not.toContain('aria-label="Server address"')
  })
  test("a failed retry opens the form again with the code, address and why", () => {
    const html = renderToStaticMarkup(
      createElement(Start, {
        onStarted: () => {},
        rejoin: {
          code: "ABCD-EFGH-JKLM-NPQR",
          server: "10.0.0.227:25817",
          error: "no such host",
        },
      })
    )
    expect(html).toContain('value="ABCD-EFGH-JKLM-NPQR"')
    expect(html).toContain('value="10.0.0.227:25817"')
    expect(html).toContain("no such host")
  })
})

describe("the Looking view (card #1261)", () => {
  test("waits about 15 seconds before asking", () => {
    expect(LookingAskMs).toBe(15_000)
    const html = renderLooking()
    expect(html).toContain("Looking for the other device…")
    expect(html).not.toContain("Server address")
    expect(html).not.toContain("Try again")
  })
  test("then offers the address box and Try again", () => {
    const html = renderLooking({ askAfterMs: 0 })
    expect(html).toContain("Looking for the other device…")
    expect(html).toContain('aria-label="Server address"')
    expect(html).toContain("Try again")
    expect(html).toContain("Your code is kept.")
  })
  test("a join through an address says so and fills the box with it", () => {
    const html = renderLooking({
      askAfterMs: 0,
      status: looking({ via: "10.0.0.227:25817" }),
    })
    expect(html).toContain("through")
    expect(html).toContain('value="10.0.0.227:25817"')
  })
  test("without the code here it says how, with no dead box", () => {
    const html = renderLooking({ askAfterMs: 0, canRetry: false })
    expect(html).not.toContain("Try again")
    expect(html).toContain("join again with the code")
  })
  test("the adding device never shows it", () => {
    const html = renderLooking({
      askAfterMs: 0,
      status: PairStatus({ mode: "add", state: PairState.Waiting }),
    })
    expect(html).not.toContain("Server address")
  })
})

describe("retryJoin keeps the code (card #1261)", () => {
  test("cancels the open join and joins again through the address", async () => {
    await devices.cancel()
    const first = await devices.join("ABCD-EFGH-JKLM-NPQR")
    expect(first.state).toBe(PairState.Waiting)
    expect(devices.keptJoinCode()).toBe("ABCD-EFGH-JKLM-NPQR")
    const again = await devices.retryJoin("10.0.0.227:25817")
    expect(again.mode).toBe("join")
    expect(again.state).toBe(PairState.Waiting)
    expect(again.via).toBe("10.0.0.227:25817")
    expect(devices.keptJoinCode()).toBe("ABCD-EFGH-JKLM-NPQR")
    // The canceled status on the way is skipped; the person's own isn't.
    expect(skipPairing(PairStatus({ state: PairState.Canceled }))).toBe(true)
    expect(skipPairing(PairStatus({ state: PairState.Waiting }))).toBe(false)
    await devices.cancel()
    expect(devices.keptJoinCode()).toBe("")
    expect(skipPairing(PairStatus({ state: PairState.Canceled }))).toBe(false)
  })
  test("with no code kept it asks for the code again", async () => {
    await devices.cancel()
    await expect(devices.retryJoin("10.0.0.227:25817")).rejects.toThrow(
      "Type the code again"
    )
  })
})
