import { expect, test } from "bun:test"

import {
  canTranscribeWith,
  convLabel,
  findLines,
  durationText,
  healthOf,
  healthWord,
  huddleNames,
  inCallHere,
  lengthText,
  namesText,
  noCall,
  personWord,
  transcriptNotice,
  transcriptOn,
  whoCanRead,
  type CallMineView,
  type CallView,
} from "./call"

const link = { phase: "live", path: "direct", rttMs: 26, lossPct: 0 }

function mineIn(convId: string): CallMineView {
  return {
    callId: "c1",
    convId,
    convName: "a",
    convKind: "channel",
    joinedAtMs: 0,
    muted: false,
    speaking: false,
    device: "",
    peers: [],
    error: null,
    transcribing: false,
    canTranscribe: true,
    transcribeOn: true,
    captions: [],
  }
}

test("healthOf grades the link, off until it's live", () => {
  expect(healthOf(link)).toBe("good")
  expect(healthOf({ ...link, rttMs: 180 })).toBe("fair")
  expect(healthOf({ ...link, lossPct: 2 })).toBe("fair")
  expect(healthOf({ ...link, rttMs: 320 })).toBe("poor")
  expect(healthOf({ ...link, lossPct: 6 })).toBe("poor")
  expect(healthOf({ ...link, phase: "connecting" })).toBe("off")
  expect(healthOf({ ...link, phase: "gone" })).toBe("off")
})

test("healthWord says the path, time and loss", () => {
  expect(healthWord(link)).toBe("direct · 26 ms")
  expect(
    healthWord({ ...link, path: "relay", rttMs: 119.6, lossPct: 3.2 })
  ).toBe("relay · 120 ms · 3% lost")
  expect(healthWord({ ...link, lossPct: 0.4 })).toBe("direct · 26 ms")
  expect(healthWord({ ...link, path: "", rttMs: 0 })).toBe("connected")
  expect(healthWord({ ...link, phase: "connecting" })).toBe("connecting…")
  expect(healthWord({ ...link, phase: "gone" })).toBe("left")
})

test("durationText is a call timer", () => {
  expect(durationText(0, 0)).toBe("0:00")
  expect(durationText(0, 252_000)).toBe("4:12")
  expect(durationText(0, 3_729_000)).toBe("1:02:09")
  // A clock behind the join never shows a negative time.
  expect(durationText(5_000, 0)).toBe("0:00")
})

test("lengthText says how long a huddle ran", () => {
  expect(lengthText(30_000)).toBe("under a minute")
  expect(lengthText(18 * 60_000 + 5_000)).toBe("18 min")
  expect(lengthText(65 * 60_000)).toBe("1 h 5 min")
  expect(lengthText(120 * 60_000)).toBe("2 h")
})

test("namesText joins names as a sentence, the person as You", () => {
  expect(namesText([])).toBe("")
  expect(namesText(["you"])).toBe("You")
  expect(namesText(["you", "Spencer"])).toBe("You and Spencer")
  expect(namesText(["Aiko", "you", "Spencer"])).toBe("Aiko, You and Spencer")
})

test("huddleNames lists each person once, You first", () => {
  const s: CallView = {
    mine: null,
    ringing: [],
    live: [
      { callId: "c1", convId: "ch-a", people: ["Spencer", "you", "Aiko"] },
      { callId: "c2", convId: "ch-b", people: ["Spencer"] },
    ],
    transcribeOn: true,
  }
  expect(huddleNames(s, "ch-a")).toEqual(["You", "Spencer", "Aiko"])
  expect(huddleNames(s, "ch-b")).toEqual(["Spencer"])
  expect(huddleNames(s, "ch-c")).toEqual([])
  expect(huddleNames(noCall, "ch-a")).toEqual([])
})

test("inCallHere is the conversation of this device's huddle", () => {
  const s: CallView = { ...noCall, mine: mineIn("ch-a") }
  expect(inCallHere(s, "ch-a")).toBe(true)
  expect(inCallHere(s, "ch-b")).toBe(false)
  expect(inCallHere(noCall, "ch-a")).toBe(false)
  // Being in it counts even before the live list has this device.
  expect(huddleNames(s, "ch-a")).toEqual(["You"])
})

test("convLabel puts # before a channel only", () => {
  expect(convLabel({ convKind: "channel", convName: "suzuri" })).toBe("#suzuri")
  expect(convLabel({ convKind: "dm", convName: "Spencer" })).toBe("Spencer")
  // A key or nothing is never shown (card #478).
  expect(convLabel({ convKind: "dm", convName: "person:0123abcdef" })).toBe(
    "another person"
  )
  expect(convLabel({ convKind: "channel", convName: "" })).toBe("#channel")
})

test("personWord: you is You, a key or nothing another person", () => {
  expect(personWord("you")).toBe("You")
  expect(personWord("Aiko")).toBe("Aiko")
  expect(personWord("")).toBe("another person")
  expect(personWord("person:0123abcdef")).toBe("another person")
})

test("null health numbers (NaN from Rust) read as zero", () => {
  const p = { ...link, rttMs: null, lossPct: null }
  expect(healthOf(p)).toBe("good")
  expect(healthWord(p)).toBe("direct")
})

test("transcriptNotice says who reads it, or why it isn't written", () => {
  const on = { transcribeOn: true, canTranscribe: true }
  expect(
    transcriptNotice({ convKind: "channel", convName: "suzuri", ...on })
  ).toBe(
    "This huddle is transcribed. What you say is written down in #suzuri for its members and their agents."
  )
  expect(transcriptNotice({ convKind: "dm", convName: "Spencer", ...on })).toBe(
    "This huddle is transcribed. What you say is written down for the two of you."
  )
  // A Personal channel: you and your agents.
  expect(
    transcriptNotice({ convKind: "personal", convName: "us", ...on })
  ).toBe(
    "This huddle is transcribed. What you say is written down in #us for you and your agents."
  )
  expect(
    transcriptNotice({
      convKind: "personal",
      convName: "us",
      ...on,
      transcribeOn: false,
    })
  ).toBe("Your voice isn't transcribed (Settings > App).")
  // Your own voice off still says the others' may be written down.
  const off = { ...on, transcribeOn: false }
  expect(
    transcriptNotice({ convKind: "channel", convName: "suzuri", ...off })
  ).toBe(
    "Your voice isn't transcribed (Settings > App). Others in the huddle may still be written down in #suzuri."
  )
  expect(
    transcriptNotice({ convKind: "dm", convName: "Spencer", ...off })
  ).toBe(
    "Your voice isn't transcribed (Settings > App). The other person may still be written down."
  )
  // No model or no engine here: the switch can't help, so it isn't named.
  for (const transcribeOn of [true, false]) {
    expect(
      transcriptNotice({
        convKind: "channel",
        convName: "suzuri",
        transcribeOn,
        canTranscribe: false,
      })
    ).toBe(
      "Your voice isn't transcribed on this device. Others in the huddle may still be written down in #suzuri."
    )
    expect(
      transcriptNotice({
        convKind: "dm",
        convName: "Spencer",
        transcribeOn,
        canTranscribe: false,
      })
    ).toBe(
      "Your voice isn't transcribed on this device. The other person may still be written down."
    )
  }
  // A key is never shown (card #478).
  expect(
    transcriptNotice({
      convKind: "channel",
      convName: "person:0123abcdef",
      ...on,
    })
  ).toContain("in #channel for")
  expect(
    transcriptNotice({
      convKind: "channel",
      convName: "person:0123abcdef",
      ...off,
    })
  ).toContain("written down in #channel.")
})

test("canTranscribeWith: no engine or no model can't", () => {
  expect(canTranscribeWith("ready")).toBe(true)
  expect(canTranscribeWith("denied")).toBe(true)
  expect(canTranscribeWith("needs-model")).toBe(false)
  expect(canTranscribeWith("unsupported")).toBe(false)
})

test("transcriptOn is anyone in the huddle written down", () => {
  const peer = {
    name: "Aiko",
    device: "",
    ...link,
    speaking: false,
    muted: false,
    transcribing: null as boolean | null,
  }
  const m = mineIn("ch-a")
  expect(transcriptOn(m)).toBe(false)
  expect(transcriptOn({ ...m, transcribing: true })).toBe(true)
  expect(transcriptOn({ ...m, peers: [peer] })).toBe(false)
  expect(
    transcriptOn({ ...m, peers: [{ ...peer, transcribing: false }] })
  ).toBe(false)
  expect(transcriptOn({ ...m, peers: [{ ...peer, transcribing: true }] })).toBe(
    true
  )
  // One who left doesn't count.
  expect(
    transcriptOn({
      ...m,
      peers: [{ ...peer, phase: "gone", transcribing: true }],
    })
  ).toBe(false)
})

test("whoCanRead names the channel, or the two of you", () => {
  expect(whoCanRead({ convKind: "channel", convName: "suzuri" })).toBe(
    "Who can read it: the people and agents of #suzuri."
  )
  expect(whoCanRead({ convKind: "dm", convName: "Spencer" })).toBe(
    "Who can read it: the two of you."
  )
})

test("findLines keeps lines with every word, by text or speaker", () => {
  const lines = [
    { name: "you", mine: true, atMs: 1, text: "Tag at two" },
    { name: "Spencer", mine: false, atMs: 2, text: "Rerun the latency TEST" },
    { name: "Aiko", mine: false, atMs: 3, text: "I can do the click" },
  ]
  expect(findLines(lines, "")).toEqual(lines)
  expect(findLines(lines, "  ")).toEqual(lines)
  expect(findLines(lines, "test").map((l) => l.atMs)).toEqual([2])
  expect(findLines(lines, "spencer latency").map((l) => l.atMs)).toEqual([2])
  expect(findLines(lines, "you tag").map((l) => l.atMs)).toEqual([1])
  expect(findLines(lines, "click two")).toEqual([])
})
