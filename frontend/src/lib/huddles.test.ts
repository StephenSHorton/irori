import { describe, expect, test } from "bun:test"

import { TalkHuddle } from "../gen/models"
import { huddleName, byNewest, isLive, lengthOf, placeText } from "./huddles"
import { huddlesFor } from "../components/talk/huddles-list"

const h = (p: Partial<ReturnType<typeof TalkHuddle>>) =>
  TalkHuddle({ convKind: "channel", convName: "suzuri", ...p })

describe("huddles", () => {
  test("a huddle is called its title, else where it was", () => {
    expect(huddleName(h({ title: " Signing plan " }))).toBe("Signing plan")
    expect(huddleName(h({}))).toBe("Huddle in #suzuri")
    expect(huddleName(h({ convKind: "dm", convName: "Spencer" }))).toBe(
      "Huddle with Spencer"
    )
    expect(placeText(h({}))).toBe("#suzuri")
  })

  test("not knowing counts as live; an end mark or over does not", () => {
    expect(isLive({ over: false, ended: false })).toBe(true)
    expect(isLive({ over: true, ended: false })).toBe(false)
    expect(isLive({ over: false, ended: true })).toBe(false)
    expect(
      lengthOf(h({ ended: true, over: true, durationMs: 18 * 60_000 }))
    ).toBe("18 min")
    expect(lengthOf(h({}))).toBe("live")
  })

  test("newest first, by start", () => {
    const a = h({ callId: "a", startedAtMs: 100, lastAtMs: 900 })
    const b = h({ callId: "b", startedAtMs: 200, lastAtMs: 300 })
    const c = h({ callId: "c", startedAtMs: null, lastAtMs: 150 })
    expect(byNewest([a, c, b]).map((x) => x.callId)).toEqual(["b", "c", "a"])
  })

  test("the list is the picked space's huddles, found by search", () => {
    const a = h({ callId: "a", convId: "ch1", title: "Signing plan" })
    const b = h({ callId: "b", convId: "ch2", convName: "netcode" })
    const convs = [
      { convId: "ch1", kind: "channel", spaceId: "s1" },
      { convId: "ch2", kind: "channel", spaceId: "s2" },
    ]
    expect(huddlesFor([a, b], convs, "").map((x) => x.callId)).toEqual([
      "a",
      "b",
    ])
    expect(huddlesFor([a, b], convs, "s1").map((x) => x.callId)).toEqual(["a"])
    expect(
      huddlesFor([a, b], convs, "", "netcode").map((x) => x.callId)
    ).toEqual(["b"])
    expect(
      huddlesFor([a, b], convs, "", "signing").map((x) => x.callId)
    ).toEqual(["a"])
  })
})
