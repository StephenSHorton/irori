import { expect, test } from "bun:test"

import { type AgentCard, sameAgents } from "./irori"

const card = (key: string, text: string) =>
  ({ key, lastText: text }) as unknown as AgentCard

test("sameAgents: equal rosters match, a changed field does not", () => {
  const a = [card("a", "x"), card("b", "y")]
  expect(sameAgents(a, a)).toBe(true)
  expect(sameAgents(a, [card("a", "x"), card("b", "y")])).toBe(true)
  expect(sameAgents(a, [card("a", "x"), card("b", "z")])).toBe(false)
  expect(sameAgents(a, [card("a", "x")])).toBe(false)
})
