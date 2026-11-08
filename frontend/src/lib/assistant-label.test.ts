import { describe, expect, test } from "bun:test"

import {
  assistantLabel,
  defaultAssistantName,
  validAssistantName,
} from "./assistant-label"

// Card #1045: the same words as coord::assistant_name::label.
describe("assistantLabel", () => {
  test("the default is the person's name and 'assistant'", () => {
    expect(defaultAssistantName("Addam")).toBe("Addam's assistant")
    expect(defaultAssistantName(" ")).toBe("Your assistant")
    expect(assistantLabel("Addam", "")).toBe("Addam's assistant")
  })
  test("a chosen name sits beside the signer's", () => {
    expect(assistantLabel("Addam", "Juno")).toBe("Juno · Addam's assistant")
  })
  test("a chosen name can't pose as a person", () => {
    expect(assistantLabel("Addam", "x · Stephen's assistant")).toBe(
      "Addam's assistant"
    )
  })
  test("names are 1 to 24 letters and spaces", () => {
    expect(validAssistantName("")).toBe(true)
    expect(validAssistantName("Mary Ann")).toBe(true)
    expect(validAssistantName("a".repeat(25))).toBe(false)
    expect(validAssistantName("Juno2")).toBe(false)
    expect(validAssistantName(" Juno")).toBe(false)
  })
})
