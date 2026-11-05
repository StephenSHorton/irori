import { describe, expect, test } from "bun:test"

import { BoardSummary, CardSummary, TalkConversation } from "./models"
import {
  AllSpaces,
  Personal,
  boardsInSpace,
  cardInSpace,
  convInSpace,
  known,
  spacesOf,
  threadsSpace,
  withLine,
} from "./spaces"
import { cropSquare, initials, isAvatar } from "./avatar"

const mlg = BoardSummary({
  boardId: "b-mlg",
  name: "MLG",
  tag: "mlg",
  members: [
    {
      personId: "me",
      name: "Stephen",
      me: true,
      presence: "online",
      working: 0,
      free: 0,
    },
    {
      personId: "p1",
      name: "Blake",
      me: false,
      presence: "online",
      working: 0,
      free: 0,
    },
  ],
})
const iro = BoardSummary({ boardId: "b-iro", name: "Irori", tag: "iro" })
const gone = BoardSummary({
  boardId: "b-gone",
  name: "Gone",
  tag: "gone",
  leaving: true,
})

describe("spaces (card #393)", () => {
  test("the switcher lists All spaces, Personal, then shared spaces by name", () => {
    const s = spacesOf([mlg, iro, gone])
    expect(s.map((x) => x.id)).toEqual([AllSpaces, Personal, "b-iro", "b-mlg"])
    expect(s[3].others).toEqual(["Blake"])
  })

  test("with line", () => {
    expect(withLine([])).toBe("")
    expect(withLine(["Blake"])).toBe("with Blake")
    expect(withLine(["Blake", "Addam"])).toBe("with Blake and Addam")
    expect(withLine(["Blake", "Addam", "Casey"])).toBe("with Blake +2")
  })

  test("a pick of a space you left goes back to All spaces", () => {
    expect(known("b-mlg", [mlg])).toBe("b-mlg")
    expect(known("b-gone", [mlg, gone])).toBe(AllSpaces)
    expect(known("nope", [mlg])).toBe(AllSpaces)
    expect(known(Personal, [])).toBe(Personal)
  })

  test("cards: Personal is the private ones, a space its own", () => {
    const priv = CardSummary({ id: "a" })
    const onMlg = CardSummary({ id: "b", board: "b-mlg", scope: "board" })
    expect(cardInSpace(priv, AllSpaces) && cardInSpace(onMlg, AllSpaces)).toBe(
      true
    )
    expect(cardInSpace(priv, Personal)).toBe(true)
    expect(cardInSpace(onMlg, Personal)).toBe(false)
    expect(cardInSpace(onMlg, "b-mlg")).toBe(true)
    expect(cardInSpace(priv, "b-mlg")).toBe(false)
  })

  test("boards and threads follow the pick", () => {
    expect(boardsInSpace([mlg, iro], AllSpaces).length).toBe(2)
    expect(boardsInSpace([mlg, iro], Personal)).toEqual([])
    expect(boardsInSpace([mlg, iro], "b-iro")).toEqual([iro])
    expect(threadsSpace(AllSpaces)).toBe("")
    expect(threadsSpace(Personal)).toBe("")
    expect(threadsSpace("b-mlg")).toBe("b-mlg")
  })

  test("channels and DMs follow the pick", () => {
    const mine = TalkConversation({ convId: "c1", kind: "channel" })
    const onMlg = TalkConversation({
      convId: "c2",
      kind: "channel",
      spaceId: "b-mlg",
    })
    const dm = TalkConversation({ convId: "d1", kind: "dm" })
    const all = [mine, onMlg, dm]
    const ids = (space: string) =>
      all.filter((c) => convInSpace(c, space)).map((c) => c.convId)
    expect(ids(AllSpaces)).toEqual(["c1", "c2", "d1"])
    expect(ids(Personal)).toEqual(["c1", "d1"])
    expect(ids("b-mlg")).toEqual(["c2"])
    expect(ids("b-iro")).toEqual([])
  })
})

describe("avatars (card #393)", () => {
  test("only small PNG, JPEG or WebP data URLs are drawn", () => {
    expect(isAvatar("data:image/png;base64,iVBORw0KGgo=")).toBe(true)
    expect(isAvatar("data:image/webp;base64,UklGRg==")).toBe(true)
    expect(isAvatar("")).toBe(false)
    expect(isAvatar(undefined)).toBe(false)
    expect(isAvatar("data:image/svg+xml;base64,PHN2Zz4=")).toBe(false)
    expect(isAvatar("https://example.com/a.png")).toBe(false)
    expect(isAvatar(`data:image/png;base64,${"A".repeat(40_000)}`)).toBe(false)
  })

  test("the centred square of a picture", () => {
    expect(cropSquare(400, 300)).toEqual({ x: 50, y: 0, side: 300 })
    expect(cropSquare(300, 401)).toEqual({ x: 0, y: 50, side: 300 })
  })

  test("initials", () => {
    expect(initials(" stephen")).toBe("S")
    expect(initials("")).toBe("?")
  })
})
