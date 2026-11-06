import { describe, expect, test } from "bun:test"

import {
  BYPASS,
  choiceOf,
  LIMITED,
  rulesOf,
  sampleAgentPermissions,
} from "./agent-permissions"

describe("agent permissions (card #449)", () => {
  test("an empty setting shows as bypass", () => {
    expect(choiceOf({ ...sampleAgentPermissions, permission: "" })).toBe(BYPASS)
    expect(choiceOf({ ...sampleAgentPermissions, permission: LIMITED })).toBe(
      LIMITED
    )
  })
  test("rules are one per line, trimmed, blanks and repeats dropped", () => {
    expect(
      rulesOf(" Bash(cargo test:*) \n\nBash(cargo test:*)\nBash(bun test:*)\n")
    ).toEqual(["Bash(cargo test:*)", "Bash(bun test:*)"])
  })
})
