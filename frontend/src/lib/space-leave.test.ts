import { describe, expect, test } from "bun:test"

import { Home, type Section } from "./nav"
import {
  leftHomeFilter,
  startLeave,
  stepLeave,
  type LeaveState,
  type SpaceView,
} from "./space-leave"
import { AllSpaces, Personal } from "./spaces"

const threads = [
  { id: "mine" },
  { id: "shared", spaceId: "b-a" },
  { id: "other", spaceId: "b-b" },
]
const conversations = [
  { convId: "chan", kind: "channel", spaceId: "b-a" },
  {
    convId: "person-dm",
    kind: "dm",
    members: [
      { kind: "person", key: "user:me" },
      { kind: "person", key: "person:blake" },
    ],
  },
]

const view = (over: Partial<SpaceView>): SpaceView => ({
  space: "b-a",
  spaceKnown: true,
  section: Home,
  open: "",
  card: null,
  cardFailed: false,
  threads,
  conversations,
  threadsLoading: false,
  conversationsLoading: false,
  ...over,
})

const talk = (id: string): Section => ({ kind: "talk", id })
const Landing = talk("")

// switched is the decision on the render a pick from -> to shows up.
const switched = (from: string, over: Partial<SpaceView>) =>
  stepLeave(startLeave(from), view(over)).to

describe("a pick that changes", () => {
  test("a thread of the space left goes to Talk's landing", () => {
    expect(switched(Personal, { space: "b-a", section: talk("mine") })).toEqual(
      Landing
    )
  })

  test("a channel of the space left goes too", () => {
    expect(switched("b-a", { space: "b-b", section: talk("chan") })).toEqual(
      Landing
    )
  })

  test("a card of the space left closes to the section behind it", () => {
    const notes: Section = { kind: "notes" }
    expect(
      switched(Personal, { space: "b-a", open: "c", card: {}, section: notes })
    ).toEqual(notes)
    expect(
      switched(Personal, { space: "b-a", open: "c", card: {}, section: Home })
    ).toEqual(Home)
  })

  test("a closed card (from its detail) closes the same way", () => {
    expect(
      switched("b-a", { space: "b-b", open: "c", card: { board: "b-a" } })
    ).toEqual(Home)
  })

  test("a card over a thread of the space left lands on Talk in one hop", () => {
    expect(
      switched(Personal, {
        space: "b-a",
        open: "c",
        card: {},
        section: talk("mine"),
      })
    ).toEqual(Landing)
  })

  test("a card over a thread still in the pick keeps the thread", () => {
    expect(
      switched(Personal, {
        space: "b-a",
        open: "c",
        card: {},
        section: talk("shared"),
      })
    ).toEqual(talk("shared"))
  })

  test("a card over a board follows the pick to its boards", () => {
    expect(
      switched("b-a", {
        space: "b-b",
        open: "c",
        card: { board: "b-a" },
        section: { kind: "board", id: "b-a" },
      })
    ).toEqual({ kind: "board", id: "b-b" })
  })

  test("All spaces stays put", () => {
    expect(
      switched(Personal, { space: AllSpaces, section: talk("mine") })
    ).toBeNull()
    expect(
      switched("b-a", { space: AllSpaces, open: "c", card: { board: "b-a" } })
    ).toBeNull()
  })

  test("a person-to-person DM stays put", () => {
    expect(
      switched(Personal, { space: "b-a", section: talk("person-dm") })
    ).toBeNull()
  })

  test("what is also in the new pick stays put", () => {
    expect(
      switched(AllSpaces, { space: "b-a", section: talk("shared") })
    ).toBeNull()
    expect(
      switched(AllSpaces, { space: "b-a", open: "c", card: { board: "b-a" } })
    ).toBeNull()
  })

  test("pages no space holds stay put", () => {
    for (const section of [
      { kind: "notes" },
      { kind: "profile" },
      { kind: "notifications" },
      { kind: "desk", repo: "" },
      Home,
      Landing,
    ] as Section[])
      expect(switched(Personal, { space: "b-a", section })).toBeNull()
  })

  test("a conversation neither list has stays, once they've loaded", () => {
    expect(
      switched(Personal, { space: "b-a", section: talk("unknown") })
    ).toBeNull()
  })
})

describe("no change of the pick", () => {
  test("another space's thread opens and stays: a link still works", () => {
    const s = startLeave("b-a")
    expect(stepLeave(s, view({ section: talk("other") })).to).toBeNull()
  })

  test("a reload on a stored pick moves nothing, loaded or not", () => {
    let s = startLeave("b-a")
    for (const over of [
      { spaceKnown: false, threads: [], threadsLoading: true },
      { spaceKnown: true, threads: [], threadsLoading: true },
      {},
    ]) {
      const r = stepLeave(s, view({ section: talk("mine"), ...over }))
      expect(r.to).toBeNull()
      expect(r.changed).toBe(false)
      s = r.state
    }
  })
})

describe("a switch before the lists are in", () => {
  const steps = (start: LeaveState, ...views: Partial<SpaceView>[]) => {
    let s = start
    return views.map((over) => {
      const r = stepLeave(s, view(over))
      s = r.state
      return r.to
    })
  }

  test("waits for the threads, then moves once", () => {
    const section = talk("mine")
    expect(
      steps(
        startLeave(Personal),
        { section, threads: [], threadsLoading: true },
        { section, threads: [], threadsLoading: true },
        { section },
        { section }
      )
    ).toEqual([null, null, Landing, null])
  })

  test("waits for an open card's detail, then closes it", () => {
    expect(
      steps(
        startLeave(Personal),
        { open: "c", card: null },
        { open: "c", card: {} },
        { open: "", card: null }
      )
    ).toEqual([null, Home, null])
  })

  test("a card that can't be read stays", () => {
    expect(
      steps(startLeave(Personal), { open: "c", cardFailed: true })
    ).toEqual([null])
  })

  test("waits until the boards hold a shared pick", () => {
    const section = talk("mine")
    expect(
      steps(
        startLeave(Personal),
        { section, spaceKnown: false },
        { section, spaceKnown: true }
      )
    ).toEqual([null, Landing])
  })

  test("a left space's id back from an emptied list moves nothing", () => {
    const section = talk("mine")
    expect(
      steps(
        startLeave(AllSpaces),
        { section, spaceKnown: false, space: "b-gone" },
        { section, spaceKnown: false, space: "b-gone" }
      )
    ).toEqual([null, null])
  })

  test("a shared pick that turns out stale falls to All: stays", () => {
    const section = talk("mine")
    expect(
      steps(
        startLeave(Personal),
        { section, spaceKnown: false, space: "b-gone" },
        { section, spaceKnown: true, space: AllSpaces }
      )
    ).toEqual([null, null])
  })

  test("going elsewhere meanwhile calls it off: a link still opens", () => {
    expect(
      steps(
        startLeave(Personal),
        { section: talk("mine"), threads: [], threadsLoading: true },
        { section: talk("other") },
        { section: talk("other") }
      )
    ).toEqual([null, null, null])
  })

  test("a pick undone meanwhile moves nothing", () => {
    const section = talk("mine")
    expect(
      steps(
        startLeave(Personal),
        { section, threads: [], threadsLoading: true },
        { section, space: Personal },
        { section, space: Personal }
      )
    ).toEqual([null, null, null])
  })
})

describe("leftHomeFilter", () => {
  test("a board filter outside the new pick is reset", () => {
    expect(leftHomeFilter("board:b-a", "b-b")).toBe(true)
    expect(leftHomeFilter("board:b-a", Personal)).toBe(true)
  })

  test("everything else is kept", () => {
    expect(leftHomeFilter("board:b-a", "b-a")).toBe(false)
    expect(leftHomeFilter("board:b-a", AllSpaces)).toBe(false)
    expect(leftHomeFilter("private", "b-a")).toBe(false)
    expect(leftHomeFilter("all", "b-a")).toBe(false)
  })
})
