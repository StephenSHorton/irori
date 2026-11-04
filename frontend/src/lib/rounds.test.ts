import { describe, expect, test } from "bun:test"

import { AgentRef, CardThreadItem } from "./models"
import { sentBack } from "./rounds"

const you = AgentRef({ key: "person", name: "You", status: "person" })
const agent = AgentRef({ key: "mac/card-81-1", name: "card-81-1", status: "idle" })
const irori = AgentRef({ key: "irori", name: "Irori", status: "irori" })

const reason = CardThreadItem({
  id: "c1",
  kind: "comment",
  who: you,
  text: "the others haven't been told the next steps",
  at: "2026-11-04T10:00:00Z",
  blockerId: "b1",
})

describe("a done check the person sent back (card #476)", () => {
  test("nothing said on it: no round", () => {
    expect(sentBack([])).toBeNull()
    expect(sentBack([CardThreadItem({ kind: "event", who: irori })])).toBeNull()
  })

  test("sent, not read yet: the pending line, neither sign", () => {
    const r = sentBack([{ ...reason, pending: ["sent to card-81-1, not read yet"] }])
    expect(r?.reason).toBe("the others haven't been told the next steps")
    expect(r?.heard).toBe(false)
    expect(r?.answered).toBe(false)
    expect(r?.pending).toEqual(["sent to card-81-1, not read yet"])
  })

  test("read: heard, not answered", () => {
    const r = sentBack([{ ...reason, heardBy: [agent.key] }])
    expect(r?.heard).toBe(true)
    expect(r?.answered).toBe(false)
  })

  test("answered: the agent's words after the reason, and heard", () => {
    const reply = CardThreadItem({
      id: "c2",
      kind: "comment",
      who: agent,
      text: "made #108 for the deploy check and told the team",
    })
    const early = CardThreadItem({ id: "c0", kind: "comment", who: agent, text: "earlier" })
    const r = sentBack([early, reason, reply])
    expect(r?.replies.map((x) => x.id)).toEqual(["c2"])
    expect(r?.heard).toBe(true)
    expect(r?.answered).toBe(true)
    expect(r?.pending).toEqual([])
  })

  test("the newest reason counts when they sent it back twice", () => {
    const again = { ...reason, id: "c3", text: "still no note to the team" }
    const reply = CardThreadItem({ id: "c2", kind: "comment", who: agent, text: "on it" })
    const r = sentBack([reason, reply, again])
    expect(r?.reason).toBe("still no note to the team")
    expect(r?.answered).toBe(false)
  })

  test("an older card: the reason only in the check's answer", () => {
    const r = sentBack([], "also the tests")
    expect(r?.reason).toBe("also the tests")
    expect(r?.answered).toBe(false)
    expect(sentBack([], "  ")).toBeNull()
  })
})
