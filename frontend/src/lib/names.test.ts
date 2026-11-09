import { describe, expect, test } from "bun:test"

import { memberName } from "./names"

describe("memberName and assistants (assistants talk, card 3)", () => {
  test("a verified assistant reads through the one label helper", () => {
    expect(
      memberName({
        kind: "agent",
        name: "ignored",
        owner: "Addam",
        assistant: true,
        assistantName: "Juno",
      })
    ).toBe("Juno · Addam's assistant")
    expect(memberName({ kind: "agent", owner: "Addam", assistant: true })).toBe(
      "Addam's assistant"
    )
  })
  test("a card merely named like one stays an ordinary agent", () => {
    expect(
      memberName({
        kind: "agent",
        name: "Addam's assistant · Eve",
        owner: "Eve",
      })
    ).toBe("Addam's assistant · Eve")
  })
})
