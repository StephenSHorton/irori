import { expect, test } from "bun:test"

import { draft, setDraft } from "./drafts"

test("a draft is kept until it is cleared", () => {
  expect(draft("c1/comment")).toBe("")
  setDraft("c1/comment", "half a thought")
  expect(draft("c1/comment")).toBe("half a thought")
  expect(draft("c2/comment")).toBe("")
  setDraft("c1/comment", "")
  expect(draft("c1/comment")).toBe("")
})
