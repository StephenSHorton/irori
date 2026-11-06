import { expect, test } from "bun:test"

import {
  anchorAt,
  compose,
  errorLine,
  joinWords,
  newSession,
  settingsFix,
  spoken,
  step,
  type VoiceEvent,
} from "./voice"

const ev = (
  session: number,
  kind: string,
  text = "",
  code = ""
): VoiceEvent => ({
  session,
  kind,
  text,
  code,
})

test("dictation goes in at the caret and keeps the text either side", () => {
  const a = anchorAt("Ship it after the review", 8, 8)
  expect(a).toEqual({ before: "Ship it ", after: "after the review" })
  expect(compose(a, "today")).toEqual({
    text: "Ship it today after the review",
    caret: "Ship it today".length,
  })
})

test("a selection is replaced", () => {
  const a = anchorAt("call it foo now", 8, 11)
  expect(compose(a, "the parser").text).toBe("call it the parser now")
})

test("spaces only where words would touch", () => {
  expect(compose(anchorAt("Hello", 5, 5), "world").text).toBe("Hello world")
  expect(compose(anchorAt("", 0, 0), "First words").text).toBe("First words")
  expect(compose(anchorAt("end.", 0, 0), "Start").text).toBe("Start end.")
  expect(compose(anchorAt("so,", 2, 2), "and").text).toBe("so and,")
  expect(compose(anchorAt("a\n", 2, 2), "b").text).toBe("a\nb")
  // Nothing said yet: the box is as it was, caret at the anchor.
  expect(compose(anchorAt("abc", 1, 2), "")).toEqual({ text: "ac", caret: 1 })
})

test("joinWords", () => {
  expect(joinWords("", "x")).toBe("x")
  expect(joinWords("x", "")).toBe("x")
  expect(joinWords("x ", "y")).toBe("x y")
  expect(joinWords("x", "y")).toBe("x y")
})

test("a partial is replaced, a final settles, the next phrase follows", () => {
  let s = newSession(7)
  s = step(s, ev(7, "partial", "Please"))
  s = step(s, ev(7, "partial", "Please review the"))
  expect(spoken(s)).toBe("Please review the")
  s = step(s, ev(7, "final", "Please review the card."))
  expect(spoken(s)).toBe("Please review the card.")
  s = step(s, ev(7, "partial", "Then send"))
  expect(spoken(s)).toBe("Please review the card. Then send")
  s = step(s, ev(7, "final", "Then send it."))
  s = step(s, ev(7, "ended"))
  expect(s.ended).toBe(true)
  expect(spoken(s)).toBe("Please review the card. Then send it.")
})

test("an empty final drops the partial; ended drops an unsettled one", () => {
  let s = step(newSession(1), ev(1, "partial", "uh"))
  s = step(s, ev(1, "final", ""))
  expect(spoken(s)).toBe("")
  s = step(s, ev(1, "partial", "half a"))
  s = step(s, ev(1, "ended"))
  expect(spoken(s)).toBe("")
})

test("another session's events are ignored, and nothing after ended", () => {
  let s = newSession(2)
  s = step(s, ev(3, "partial", "not mine"))
  expect(spoken(s)).toBe("")
  s = step(s, ev(2, "ended"))
  s = step(s, ev(2, "final", "late"))
  expect(spoken(s)).toBe("")
})

test("errors keep their code and text", () => {
  const s = step(
    newSession(4),
    ev(4, "error", "Allow the microphone", "mic-denied")
  )
  expect(s.error).toEqual({ code: "mic-denied", text: "Allow the microphone" })
  expect(settingsFix("mic-denied")).toBe(true)
  expect(settingsFix("no-model")).toBe(false)
  expect(errorLine("no-mic", "")).toBe("No microphone was found.")
  expect(errorLine("other", "Custom")).toBe("Custom")
})
