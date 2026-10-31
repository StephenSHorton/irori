import { describe, expect, test } from "bun:test"

import { InviteState, InviteStatus, inviteOpen, suggestTag, validTag } from "./boards"

describe("board tags (identity.ValidBoardTag)", () => {
  test("a lowercase letter, then up to 15 letters, digits or '-'", () => {
    for (const t of ["mlg", "a", "site-2", "abcdefghijklmnop"]) expect(validTag(t)).toBe(true)
    for (const t of ["", "MLG", "1mlg", "-x", "has space", "abcdefghijklmnopq", "m#1"]) expect(validTag(t)).toBe(false)
  })
  test("New board suggests one from the name's first word", () => {
    expect(suggestTag("MLG")).toBe("mlg")
    expect(suggestTag("Website refresh")).toBe("website")
    expect(suggestTag("2026 plans")).toBe("")
    expect(suggestTag("")).toBe("")
  })
})

describe("invites", () => {
  test("inviteOpen is true only while an invite is in progress", () => {
    expect(inviteOpen(null)).toBe(false)
    expect(inviteOpen(InviteStatus({}))).toBe(false)
    for (const st of [InviteState.Waiting, InviteState.Words, InviteState.Joining])
      expect(inviteOpen(InviteStatus({ state: st }))).toBe(true)
    for (const st of [InviteState.Done, InviteState.Failed])
      expect(inviteOpen(InviteStatus({ state: st }))).toBe(false)
  })

})
