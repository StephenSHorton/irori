import { describe, expect, it } from "bun:test"

import { agentsNowDefault } from "./agents-now"

// The same cases as coord's the_default_is_off_only_when_just_people_are_named.
describe("agentsNowDefault", () => {
  const m = (key: string) => ({ key })
  it("is on when nobody is named", () => {
    expect(agentsNowDefault([])).toBe(true)
  })
  it("is on when an agent is named", () => {
    expect(agentsNowDefault([m("dev-a/alice-session")])).toBe(true)
    expect(agentsNowDefault([m("agent:c1")])).toBe(true)
  })
  it("is off when only people are named", () => {
    expect(agentsNowDefault([m("person:p2")])).toBe(false)
    expect(agentsNowDefault([m("person:p2"), m("user:u1")])).toBe(false)
  })
  it("is on when a person and an agent are named", () => {
    expect(agentsNowDefault([m("person:p2"), m("agent:c1")])).toBe(true)
  })
})
