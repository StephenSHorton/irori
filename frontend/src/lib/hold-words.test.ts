// Card #488: why a message is held, in plain words that name who.

import { describe, expect, test } from "bun:test"

import { HoldReason, holdText, repoShort, State, type Delivery } from "./talk"

const held = (d: Partial<Delivery>, from = { key: "dev/otter", name: "otter", kind: "claude" }, hop = 0) =>
  ({
    id: "d",
    messageId: "m",
    to: "dev/heron",
    state: State.Held,
    reminded: false,
    createdAt: "2026-11-02T10:00:00Z",
    updatedAt: "2026-11-02T10:00:00Z",
    ...d,
    message: {
      id: "m",
      from,
      to: { kind: "agent", value: d.to ?? "dev/heron" },
      text: "Hello",
      hop,
      trust: "untrusted",
      sentAt: "2026-11-02T10:00:00Z",
    },
  }) as Delivery

const name = (k: string) => ({ "dev/otter": "otter", "dev/heron": "heron" })[k] ?? ""
const repo = (k: string) => (k === "root:5a3c1e0b9d7f" ? "ww-multiplayer" : repoShort(k))

describe("holdText", () => {
  test("a chain between two agents names them and counts", () => {
    expect(holdText(held({ hold: HoldReason.HoldHopLimit }, undefined, 8), { name, repo })).toBe(
      "Otter and heron have messaged each other 8 times without stopping."
    )
    expect(holdText(held({ hold: HoldReason.HoldHopLimit }), { name })).toBe(
      "Otter and heron kept messaging each other without stopping."
    )
  })
  test("your message to a repo nobody is in names the repo, never its key", () => {
    const you = { key: "user:dev", name: "Stephen", kind: "user" }
    const d = held({ to: "", repoKey: "root:5a3c1e0b9d7f", hold: HoldReason.HoldUnowned }, you)
    expect(holdText(d, { name, repo })).toBe(
      "No agent is working in ww-multiplayer to take your message."
    )
    // Without a checkout here: its short form, not the 40-character key.
    expect(holdText(d, { name })).toBe("No agent is working in local repo 5a3c1e0 to take your message.")
  })
  test("Pause all, the lists, and an older Irori's rule", () => {
    expect(holdText(held({ hold: HoldReason.HoldPaused }))).toBe("You paused wakes.")
    expect(holdText(held({ hold: HoldReason.HoldNotAllowed }), { name })).toBe(
      "Otter isn't on your allow list."
    )
    expect(holdText(held({ hold: HoldReason.HoldBlocked }), { name })).toBe(
      "Otter is on your block list."
    )
    expect(holdText(held({ hold: HoldReason.HoldBypassReceiver }))).not.toContain("permission")
  })
  test("no internal words leak", () => {
    for (const h of Object.values(HoldReason)) {
      if (!h) continue
      const t = holdText(held({ hold: h }, undefined, 8), { name, repo })
      expect(t).not.toMatch(/wake policy|hop|_/)
      expect(t.length).toBeGreaterThan(0)
    }
    expect(holdText(held({ hold: "something_new" as HoldReason }))).toBe("Irori held it.")
    // History keeps why it was held.
    expect(holdText(held({ state: State.Dismissed, heldFor: HoldReason.HoldPaused }))).toBe(
      "You paused wakes."
    )
  })
})

describe("repoShort", () => {
  test("a repo key as a person reads it", () => {
    expect(repoShort("root:5a3c1e0b9d7f2a4c")).toBe("local repo 5a3c1e0")
    expect(repoShort("remote:github.com/sample-org/ww-multiplayer.git")).toBe("ww-multiplayer")
    expect(repoShort("github.com/sample-org/example-app")).toBe("example-app")
  })
})
