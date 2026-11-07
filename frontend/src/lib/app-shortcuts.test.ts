import { describe, expect, test } from "bun:test"

import {
  appShortcut,
  backShortcut,
  forwardShortcut,
  inField,
  mouseShortcut,
} from "./app-shortcuts"
import { register, runCommand, shortcutKeys } from "./commands"

const k = (
  key: string,
  o: Partial<{
    code: string
    metaKey: boolean
    ctrlKey: boolean
    shiftKey: boolean
    altKey: boolean
  }> = {}
) => ({
  key,
  code: "",
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...o,
})
const cmd = { metaKey: true }
const ctrl = { ctrlKey: true }

describe("appShortcut", () => {
  test("Back and Forward: Cmd+[ and Cmd+] on a Mac", () => {
    expect(appShortcut(k("[", cmd), true)).toBe("nav.back")
    expect(appShortcut(k("]", cmd), true)).toBe("nav.forward")
    // Ctrl+[ is Esc in a terminal; not Back on either side.
    expect(appShortcut(k("[", ctrl), true)).toBeNull()
    expect(appShortcut(k("[", ctrl), false)).toBeNull()
    // Alt+Left is a word back on a Mac.
    expect(appShortcut(k("ArrowLeft", { altKey: true }), true)).toBeNull()
  })
  test("Back and Forward: Alt+Left and Alt+Right on a PC", () => {
    expect(appShortcut(k("ArrowLeft", { altKey: true }), false)).toBe(
      "nav.back"
    )
    expect(appShortcut(k("ArrowRight", { altKey: true }), false)).toBe(
      "nav.forward"
    )
    expect(
      appShortcut(k("ArrowLeft", { altKey: true, shiftKey: true }), false)
    ).toBeNull()
    expect(
      appShortcut(k("ArrowLeft", { altKey: true, ctrlKey: true }), false)
    ).toBeNull()
    expect(appShortcut(k("ArrowLeft"), false)).toBeNull()
    expect(appShortcut(k("b", { altKey: true }), false)).toBeNull()
  })
  test("a keyboard's own Back and Forward keys", () => {
    for (const mac of [true, false]) {
      expect(appShortcut(k("BrowserBack"), mac)).toBe("nav.back")
      expect(appShortcut(k("BrowserForward"), mac)).toBe("nav.forward")
    }
  })
  test("Cmd+1..5 go to the sections", () => {
    const want = [
      "nav.home",
      "nav.talk",
      "nav.notes",
      "nav.boards",
      "nav.notifications",
    ]
    want.forEach((id, i) => {
      const d = String(i + 1)
      expect(appShortcut(k(d, cmd), true)).toBe(id)
      expect(appShortcut(k(d, ctrl), false)).toBe(id)
    })
    expect(appShortcut(k("6", cmd), true)).toBeNull()
    expect(appShortcut(k("0", ctrl), false)).toBeNull()
    // AZERTY: Cmd+& is the 1 key.
    expect(appShortcut(k("&", { ...cmd, code: "Digit1" }), true)).toBe(
      "nav.home"
    )
    // No modifier: a plain 1 is typing.
    expect(appShortcut(k("1"), true)).toBeNull()
  })
  test("Cmd+B, Cmd+, Cmd+N and Cmd+F", () => {
    for (const [mac, mod] of [
      [true, cmd],
      [false, ctrl],
    ] as const) {
      expect(appShortcut(k("b", mod), mac)).toBe("sidebar.fold")
      expect(appShortcut(k("B", mod), mac)).toBe("sidebar.fold")
      expect(appShortcut(k(",", mod), mac)).toBe("settings.open")
      expect(appShortcut(k("n", mod), mac)).toBe("card.new")
      expect(appShortcut(k("f", mod), mac)).toBe("search.cards")
      expect(appShortcut(k("x", mod), mac)).toBeNull()
    }
  })
  test("Mod is Cmd on a Mac and Ctrl elsewhere, alone", () => {
    expect(appShortcut(k("n", ctrl), true)).toBeNull()
    expect(appShortcut(k("n", cmd), false)).toBeNull()
    expect(appShortcut(k("n", { metaKey: true, ctrlKey: true }), true)).toBe(
      null
    )
    expect(appShortcut(k("n", { ...cmd, shiftKey: true }), true)).toBeNull()
    expect(appShortcut(k("n", { ...ctrl, altKey: true }), false)).toBeNull()
    expect(appShortcut(k("1", { ...cmd, shiftKey: true }), true)).toBeNull()
    expect(appShortcut(k("n"), true)).toBeNull()
  })
  test("leaves the palette's and dictation's keys alone", () => {
    for (const key of ["k", "p", "d"]) {
      expect(appShortcut(k(key, cmd), true)).toBeNull()
      expect(appShortcut(k(key, ctrl), false)).toBeNull()
    }
    // Ctrl+` is the terminal panel's.
    expect(appShortcut(k("`", ctrl), false)).toBeNull()
    expect(appShortcut(k("`", cmd), true)).toBeNull()
  })
})

describe("mouseShortcut", () => {
  test("side buttons are Back and Forward", () => {
    expect(mouseShortcut(3)).toBe("nav.back")
    expect(mouseShortcut(4)).toBe("nav.forward")
    expect(mouseShortcut(0)).toBeNull()
    expect(mouseShortcut(1)).toBeNull()
    expect(mouseShortcut(2)).toBeNull()
  })
})

describe("inField", () => {
  const el = (
    tagName: string,
    o: { type?: string; isContentEditable?: boolean; in?: string } = {}
  ) => ({
    tagName,
    type: o.type,
    isContentEditable: o.isContentEditable ?? false,
    closest: (sel: string) => (o.in && sel === o.in ? ({} as Element) : null),
  })
  test("text fields, selects and terminals have the keys", () => {
    expect(inField(el("TEXTAREA"))).toBe(true)
    expect(inField(el("INPUT", { type: "text" }))).toBe(true)
    expect(inField(el("INPUT", { type: "search" }))).toBe(true)
    expect(inField(el("SELECT"))).toBe(true)
    expect(inField(el("DIV", { isContentEditable: true }))).toBe(true)
    expect(inField(el("CANVAS", { in: ".rsz-view" }))).toBe(true)
  })
  test("buttons, checkboxes and the page don't", () => {
    expect(inField(null)).toBe(false)
    expect(inField(el("BODY"))).toBe(false)
    expect(inField(el("BUTTON"))).toBe(false)
    expect(inField(el("INPUT", { type: "checkbox" }))).toBe(false)
    expect(inField(el("A"))).toBe(false)
  })
})

describe("palette", () => {
  test("Back and Forward show per platform", () => {
    expect(shortcutKeys(backShortcut(true), true)).toEqual(["⌘", "["])
    expect(shortcutKeys(forwardShortcut(true), true)).toEqual(["⌘", "]"])
    expect(shortcutKeys(backShortcut(false), false)).toEqual(["Alt", "←"])
    expect(shortcutKeys(forwardShortcut(false), false)).toEqual(["Alt", "→"])
    expect(shortcutKeys("Mod+,", true)).toEqual(["⌘", ","])
    expect(shortcutKeys("Mod+1", false)).toEqual(["Ctrl", "1"])
  })
})

describe("runCommand", () => {
  test("runs a command that applies, and says so", () => {
    let ran = 0
    let on = true
    const off = register([
      { id: "t.run", title: "Run", when: () => on, run: () => ran++ },
    ])
    expect(runCommand("t.run")).toBe(true)
    expect(ran).toBe(1)
    on = false
    expect(runCommand("t.run")).toBe(false)
    expect(ran).toBe(1)
    off()
    expect(runCommand("t.run")).toBe(false)
    expect(runCommand("t.none")).toBe(false)
  })
})
