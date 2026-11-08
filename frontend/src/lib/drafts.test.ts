import { expect, test } from "bun:test"

import {
  draft,
  dropMakingCard,
  keepMakingCard,
  makingCard,
  setDraft,
  type CardForm,
} from "./drafts"

test("a draft is kept until it is cleared", () => {
  expect(draft("c1/comment")).toBe("")
  setDraft("c1/comment", "half a thought")
  expect(draft("c1/comment")).toBe("half a thought")
  expect(draft("c2/comment")).toBe("")
  setDraft("c1/comment", "")
  expect(draft("c1/comment")).toBe("")
})

const form = (title: string): CardForm => ({
  title,
  objective: "the goal",
  doneWhen: "",
  repoKey: "",
  assignTo: "",
  picked: null,
  files: [],
})

// Card #1071: Make a card's Settings link keeps the form and its post
// while Settings has the window, and the conversation finds them back.
test("a kept Make a card is its conversation's until dropped", () => {
  expect(makingCard("conv-1")).toBeUndefined()
  keepMakingCard({ convId: "conv-1", post: { postId: "p1" }, form: form("A") })
  // Read twice, as a strict-mode state initialiser does: still there.
  expect(makingCard<{ postId: string }>("conv-1")?.post.postId).toBe("p1")
  expect(makingCard("conv-1")?.form.title).toBe("A")
  // Another conversation neither finds it nor drops it.
  expect(makingCard("conv-2")).toBeUndefined()
  dropMakingCard("conv-2")
  expect(makingCard("conv-1")?.form.title).toBe("A")
  dropMakingCard("conv-1")
  expect(makingCard("conv-1")).toBeUndefined()
})

test("keeping another Make a card replaces the one kept", () => {
  keepMakingCard({ convId: "conv-1", post: {}, form: form("A") })
  keepMakingCard({ convId: "conv-2", post: {}, form: form("B") })
  expect(makingCard("conv-1")).toBeUndefined()
  expect(makingCard("conv-2")?.form.title).toBe("B")
  dropMakingCard("conv-2")
})
