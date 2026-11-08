import { describe, expect, test } from "bun:test"

import {
  applyAppearance,
  clean,
  contrast,
  customTokens,
  followSystem,
  defaults,
  glassStyles,
  hexToOklch,
  oklchToHex,
  themes,
  themesCSS,
  themeTokens,
  type Mode,
  type Tokens,
} from "./appearance"

const modes: Mode[] = ["light", "dark"]
const AA = 4.5

// Text sits on the page and on glass (the frost's tint at 62% over the
// page); both must read.
function checks(k: Tokens): Record<string, number> {
  const bg = k.background
  const glass = { ...k["glass-tint"], a: 0.62 }
  return {
    "foreground on background": contrast(k.foreground, bg),
    "foreground on glass": contrast(k.foreground, glass, bg),
    "muted text on background": contrast(k["muted-foreground"], bg),
    "muted text on glass": contrast(k["muted-foreground"], glass, bg),
    "ink-2 on background": contrast(k["ink-2"], bg),
    "primary text on primary": contrast(k["primary-foreground"], k.primary, bg),
    "accent text on accent": contrast(k["accent-foreground"], k.accent, bg),
  }
}

describe("themes pass WCAG AA", () => {
  for (const t of themes)
    for (const m of modes)
      test(`${t.id} ${m}`, () => {
        for (const [what, ratio] of Object.entries(checks(themeTokens(t, m))))
          expect({ what, ok: ratio >= AA, ratio }).toMatchObject({
            what,
            ok: true,
          })
      })
})

describe("custom colours are guarded", () => {
  const picks = [
    "#ff0000",
    "#ffff00",
    "#00ff00",
    "#0000ff",
    "#ffffff",
    "#000000",
    "#808080",
  ]
  for (const accent of picks)
    for (const tint of ["#000000", "#ffffff", "#ff00ff"])
      for (const m of modes)
        test(`${accent} on ${tint} ${m}`, () => {
          const k = customTokens({ base: "blue", accent, tint }, m)
          for (const [what, ratio] of Object.entries(checks(k)))
            expect({ what, ok: ratio >= AA, ratio }).toMatchObject({
              what,
              ok: true,
            })
        })
})

test("hex round-trips through OKLCH", () => {
  for (const h of ["#3b82f6", "#d9480f", "#000000", "#ffffff", "#7a7a7a"])
    expect(oklchToHex(hexToOklch(h))).toBe(h)
})

test("design/themes.css is up to date (bun scripts/gen-themes.ts)", async () => {
  // LF line ends, like gen-models --check and xtask bindings --check: a
  // Windows checkout with core.autocrlf=true has CRLF here (#344).
  const file = (
    await Bun.file(new URL("../design/themes.css", import.meta.url)).text()
  ).replace(/\r\n/g, "\n")
  expect(file).toBe(themesCSS())
})

test("index.html's pre-paint lists match", async () => {
  const html = await Bun.file(
    new URL("../../index.html", import.meta.url)
  ).text()
  const list = (name: string) =>
    JSON.parse(
      new RegExp(`var ${name} = (\\[[^\\]]*\\])`).exec(html)![1]
    ) as string[]
  expect(list("themes")).toEqual(themes.map((t) => t.id))
  expect(list("glasses")).toEqual(glassStyles.map((g) => g.id))
  expect(html).toContain(`data-theme="${defaults.theme}"`)
  expect(html).toContain(`data-glass="${defaults.glass}"`)
})

test("clean keeps what it knows and defaults the rest", () => {
  expect(clean(null)).toEqual({ theme: "blue", mode: "dark", glass: "frost" })
  expect(clean({ theme: "nope", mode: "sepia" as never, glass: "x" })).toEqual(
    defaults
  )
  const c = clean({
    theme: "custom",
    custom: { base: "forest", accent: "#ff8800", tint: "#112233" },
  })
  expect(c.theme).toBe("custom")
  expect(c.css).toContain('[data-theme="custom"].dark')
  // A custom theme without colours falls back to the default.
  expect(clean({ theme: "custom" }).theme).toBe("blue")
})

// Card #1067: "system" follows the computer's flip with no pane open.
test("followSystem re-applies a system choice when the computer flips", () => {
  const g = globalThis as unknown as Record<string, unknown>
  const had = { window: "window" in g, document: "document" in g }
  const saved = { window: g.window, document: g.document }
  let dark = false
  const subs = new Set<() => void>()
  const classes = new Set<string>()
  g.window = {
    matchMedia: () => ({
      get matches() {
        return dark
      },
      addEventListener: (_: string, f: () => void) => subs.add(f),
      removeEventListener: (_: string, f: () => void) => subs.delete(f),
    }),
  }
  g.document = {
    documentElement: {
      dataset: {},
      style: {},
      classList: {
        toggle: (c: string, on: boolean) =>
          on ? classes.add(c) : classes.delete(c),
      },
    },
    getElementById: () => null,
  }
  try {
    applyAppearance({ ...defaults, mode: "system" })
    expect(classes.has("dark")).toBe(false)
    const stop = followSystem()
    dark = true
    subs.forEach((f) => f())
    expect(classes.has("dark")).toBe(true)
    applyAppearance({ ...defaults, mode: "light" })
    dark = false
    dark = true
    subs.forEach((f) => f())
    expect(classes.has("dark")).toBe(false) // a fixed choice ignores the flip
    stop()
    expect(subs.size).toBe(0)
  } finally {
    // Put back what was there, or leave nothing behind: a stray key makes
    // other files (the ELK canvas tests) think a DOM exists.
    if (had.window) g.window = saved.window
    else delete g.window
    if (had.document) g.document = saved.document
    else delete g.document
  }
})
