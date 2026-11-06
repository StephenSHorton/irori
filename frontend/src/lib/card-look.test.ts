import { describe, expect, test } from "bun:test"

import {
  contrast,
  glassStyles,
  hexToOklch,
  themes,
  type Appearance,
  type Mode,
} from "./appearance"
import { cardLook, sendCardLook } from "./card-look"

const modes: Mode[] = ["light", "dark"]
const keys = [
  "background",
  "tint",
  "foreground",
  "muted",
  "border",
  "hover",
  "button",
  "primary",
  "onPrimary",
  "info",
  "success",
  "warn",
  "error",
] as const
const custom = { base: "blue", accent: "#e2683c", tint: "#5a6b7c" }
const ids = [...themes.map((t) => t.id), "custom"]
const look = (theme: string, mode: Appearance["mode"], glass = "frost") =>
  cardLook({
    theme,
    mode,
    glass,
    custom: theme === "custom" ? custom : undefined,
  })

const ratio = (fg: string, bg: string) =>
  contrast(hexToOklch(fg), hexToOklch(bg))
// Hue distance in degrees.
const apart = (a: number, b: number) => Math.abs(((a - b + 540) % 360) - 180)

describe("every theme, mode and glass gives a readable card", () => {
  for (const id of ids)
    for (const m of modes)
      for (const g of glassStyles)
        test(`${id} ${m} ${g.id}`, () => {
          const c = look(id, m, g.id)[m]!
          for (const k of keys) expect(c[k]).toMatch(/^#[0-9a-f]{6}$/)
          expect(ratio(c.foreground!, c.background!)).toBeGreaterThanOrEqual(
            4.5
          )
          expect(ratio(c.muted!, c.background!)).toBeGreaterThanOrEqual(3)
          expect(ratio(c.onPrimary!, c.primary!)).toBeGreaterThanOrEqual(4.5)
          // The card keeps an edge on the desktop.
          expect(ratio(c.border!, c.background!)).toBeGreaterThanOrEqual(1.1)
        })
})

describe("the info stripe takes the theme's accent", () => {
  for (const m of modes) {
    test(`sakura ${m} is pink`, () => {
      expect(
        apart(hexToOklch(look("sakura", m)[m]!.info!).h, 355)
      ).toBeLessThan(25)
    })
    test(`forest ${m} is green`, () => {
      expect(
        apart(hexToOklch(look("forest", m)[m]!.info!).h, 150)
      ).toBeLessThan(25)
    })
    test(`custom ${m} follows the picked accent`, () => {
      const want = hexToOklch(custom.accent).h
      expect(
        apart(hexToOklch(look("custom", m)[m]!.info!).h, want)
      ).toBeLessThan(25)
    })
  }
})

test("radius: 14, and 24 under liquid", () => {
  expect(look("blue", "dark").radius).toBe(14)
  expect(look("blue", "dark", "liquid").radius).toBe(24)
})

test("alpha follows the glass style and the mode that shows", () => {
  expect(look("blue", "dark").alpha).toBe(0.62)
  expect(look("blue", "light", "liquid").alpha).toBe(0.22)
  expect(look("blue", "dark", "liquid").alpha).toBe(0.3)
  expect(look("blue", "system", "ember").alpha).toBe(0.6)
})

test("ember tints the glass with the accent", () => {
  for (const m of modes) {
    const frost = look("hearth", m)[m]!
    const ember = look("hearth", m, "ember")[m]!
    expect(ember.tint).not.toBe(frost.tint)
    expect(hexToOklch(ember.tint!).c).toBeGreaterThan(hexToOklch(frost.tint!).c)
  }
})

test("light mode's rim falls back to the hairline", () => {
  const c = look("blue", "light").light!
  // The white rim would be about 1.05:1 on the pale card; the hairline is
  // darker than the card, about 1.12:1.
  expect(hexToOklch(c.border!).l).toBeLessThan(hexToOklch(c.background!).l)
  expect(ratio(c.border!, c.background!)).toBeGreaterThan(1.1)
})

test("system mode keeps both palettes", () => {
  const l = look("blue", "system")
  expect(l.mode).toBe("system")
  expect(hexToOklch(l.light!.background!).l).toBeGreaterThan(0.8)
  expect(hexToOklch(l.dark!.background!).l).toBeLessThan(0.35)
  // Both are sent whatever the mode, so the host can follow the OS.
  expect(look("blue", "light").dark).toEqual(l.dark)
})

test("custom is its own palette", () => {
  expect(look("custom", "dark").dark!.primary).not.toBe(
    look("blue", "dark").dark!.primary
  )
  expect(look("custom", "dark").theme).toBe("custom")
})

describe("sendCardLook", () => {
  type Invoke = (cmd: string, args: unknown) => Promise<unknown>
  const g = globalThis as unknown as {
    window?: { __TAURI_INTERNALS__?: { invoke: Invoke } }
  }

  test("sends only in the host, skips a look already sent, and never throws", async () => {
    const had = g.window
    const sent: unknown[] = []
    let fail = false
    try {
      // The browser preview: no host, nothing goes.
      g.window = {}
      await sendCardLook({ theme: "forest", mode: "dark", glass: "frost" })
      expect(sent).toHaveLength(0)

      g.window = {
        __TAURI_INTERNALS__: {
          invoke: async (cmd, args) => {
            if (fail) throw new Error("host gone")
            sent.push([cmd, args])
            return null
          },
        },
      }
      await sendCardLook({ theme: "forest", mode: "dark", glass: "frost" })
      await sendCardLook({ theme: "forest", mode: "dark", glass: "frost" })
      expect(sent).toHaveLength(1)
      expect(sent[0]).toEqual([
        "settings_set_card_look",
        { look: cardLook({ theme: "forest", mode: "dark", glass: "frost" }) },
      ])

      // A failed send is logged, not thrown, and the same look goes again
      // next time.
      fail = true
      const err = console.error
      console.error = () => {}
      try {
        await sendCardLook({ theme: "sakura", mode: "light", glass: "rim" })
      } finally {
        console.error = err
      }
      fail = false
      await sendCardLook({ theme: "sakura", mode: "light", glass: "rim" })
      expect(sent).toHaveLength(2)
    } finally {
      g.window = had
    }
  })

  test("one send at a time; a burst ends on its last look", async () => {
    const had = g.window
    const sent: string[] = []
    const gates: Array<() => void> = []
    try {
      g.window = {
        __TAURI_INTERNALS__: {
          invoke: (_cmd, args) =>
            new Promise((done) => {
              sent.push((args as { look: { theme: string } }).look.theme)
              gates.push(() => done(null))
            }),
        },
      }
      const first = sendCardLook({ theme: "blue", mode: "dark", glass: "milk" })
      // A drag while the first is out: only the newest is kept.
      void sendCardLook({ theme: "hearth", mode: "dark", glass: "milk" })
      void sendCardLook({ theme: "midnight", mode: "dark", glass: "milk" })
      void sendCardLook({ theme: "mono", mode: "dark", glass: "milk" })
      expect(sent).toEqual(["blue"])
      gates.shift()!()
      await new Promise((r) => setTimeout(r, 0))
      expect(sent).toEqual(["blue", "mono"])
      gates.shift()!()
      await first
      expect(sent).toEqual(["blue", "mono"])
    } finally {
      g.window = had
    }
  })
})
