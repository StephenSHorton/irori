import { beforeEach, expect, test } from "bun:test"

import { setDraft } from "./drafts"
import {
  landing,
  rewriteEdited,
  rewriteReset,
  rewriteUsed,
  StaleNote,
  startRewrite,
  UnchangedNote,
} from "./rewrite"
import * as rewrite from "./rewrite"

const key = "test/rewrite"

let seen: rewrite.RewriteState
const peek = () => rewrite.peekRewrite(key)

function deferred() {
  let answer!: (s: string) => void
  let fail!: (e: Error) => void
  const p = new Promise<string>((res, rej) => {
    answer = res
    fail = rej
  })
  return { ask: () => p, answer, fail }
}

beforeEach(() => {
  rewriteReset(key)
  setDraft(key, "hello")
})

test("landing: older text is stale, whatever came back", () => {
  expect(landing("a", "a b", "A.")).toBe("stale")
  expect(landing("a", "", "A.")).toBe("stale")
})

test("landing: the same words are unchanged, spaces aside", () => {
  expect(landing("a b", "a b", " a b\n")).toBe("unchanged")
  expect(landing("a b", "a b", "A b.")).toBe("suggestion")
})

test("an answer for the box's text shows, and the draft stays", async () => {
  const d = deferred()
  const run = startRewrite(key, "hello", [], d.ask)
  expect(peek().running).toBe(true)
  d.answer("Hello.")
  await run
  seen = peek()
  expect(seen.running).toBe(false)
  expect(seen.suggestion).toEqual({ from: "hello", text: "Hello." })
  expect(rewrite.currentDraft(key)).toBe("hello")
})

test("typing while it runs: the answer is dropped and said", async () => {
  const d = deferred()
  const run = startRewrite(key, "hello", [], d.ask)
  setDraft(key, "hello there")
  rewriteEdited(key)
  d.answer("Hello.")
  await run
  expect(peek().suggestion).toBeNull()
  expect(peek().note?.text).toBe(StaleNote)
  expect(rewrite.currentDraft(key)).toBe("hello there")
})

test("a send while it runs: the answer lands nowhere", async () => {
  const d = deferred()
  const run = startRewrite(key, "hello", [], d.ask)
  setDraft(key, "")
  rewriteReset(key)
  d.answer("Hello.")
  await run
  expect(peek()).toEqual({
    running: false,
    suggestion: null,
    undo: null,
    note: null,
  })
})

test("unchanged is said, with no suggestion", async () => {
  await startRewrite(key, "hello", [], async () => "hello")
  expect(peek().suggestion).toBeNull()
  expect(peek().note).toEqual({ text: UnchangedNote })
})

test("a failure is said and the draft stays", async () => {
  await startRewrite(key, "hello", [], async () => {
    throw new Error("The assistant needs Claude Code signed in")
  })
  expect(peek().note?.error).toBe(true)
  expect(peek().note?.text).toContain("signed in")
  expect(rewrite.currentDraft(key)).toBe("hello")
})

test("Undo holds the original across a second Use this, until an edit", () => {
  const first = { text: "orig", mentions: [{ key: "k", name: "oak" }] }
  rewriteUsed(key, first)
  rewriteUsed(key, { text: "second", mentions: [] })
  expect(peek().undo).toEqual(first)
  rewriteEdited(key)
  expect(peek().undo).toBeNull()
})
