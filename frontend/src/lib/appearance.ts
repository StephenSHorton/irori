// Appearance (card #275): the theme, light/dark/system and the glass style,
// saved per device in localStorage and applied before the first paint by
// the inline script in index.html (it reads storeKey; keep the two in step,
// appearance.test.ts checks the theme list).
//
// The themes are made here and written to design/themes.css by
// `bun scripts/gen-themes.ts` (appearance.test.ts fails when the file is
// stale). Custom colours are derived from an accent and a background tint
// with the same ramp, then nudged until text passes WCAG AA (4.5:1).

import { useCallback, useEffect, useState } from "react"

// ---- colour ------------------------------------------------------------

// An OKLCH colour, with an optional alpha over whatever is behind it.
export interface Oklch {
  l: number
  c: number
  h: number
  a?: number
}

const ok = (l: number, c: number, h: number, a?: number): Oklch => ({
  l,
  c,
  h: ((h % 360) + 360) % 360,
  a,
})

const num = (n: number) => String(Math.round(n * 1000) / 1000)

export function css(x: Oklch): string {
  const base = `${num(x.l)} ${num(x.c)} ${num(x.h)}`
  return x.a === undefined ? `oklch(${base})` : `oklch(${base} / ${num(x.a)})`
}

type Rgb = [number, number, number] // linear sRGB, 0..1

function toLinear(x: Oklch): Rgb {
  const hr = (x.h * Math.PI) / 180
  const A = x.c * Math.cos(hr)
  const B = x.c * Math.sin(hr)
  const l = (x.l + 0.3963377774 * A + 0.2158037573 * B) ** 3
  const m = (x.l - 0.1055613458 * A - 0.0638541728 * B) ** 3
  const s = (x.l - 0.0894841775 * A - 1.291485548 * B) ** 3
  const clip = (v: number) => Math.min(1, Math.max(0, v))
  return [
    clip(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    clip(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    clip(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ]
}

// over composites x on an opaque bg (in linear light, close enough for a
// contrast check of thin tints).
function over(x: Oklch, bg: Rgb): Rgb {
  const fg = toLinear(x)
  const a = x.a ?? 1
  return [0, 1, 2].map((i) => fg[i] * a + bg[i] * (1 - a)) as Rgb
}

const luminance = ([r, g, b]: Rgb) => 0.2126 * r + 0.7152 * g + 0.0722 * b

// contrast is the WCAG ratio of fg on bg, each composited on what's under it.
export function contrast(fg: Oklch, bg: Oklch, under?: Oklch): number {
  const base = under ? toLinear(under) : ([1, 1, 1] as Rgb)
  const b = over(bg, base)
  const f = over(fg, b)
  const [hi, lo] = [luminance(f), luminance(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

// hexToOklch reads "#rrggbb".
export function hexToOklch(hex: string): Oklch {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim())
  if (!m) return ok(0.6, 0.15, 250)
  const v = parseInt(m[1], 16)
  const lin = [(v >> 16) & 255, (v >> 8) & 255, v & 255].map((c) => {
    const s = c / 255
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
  })
  const [r, g, b] = lin
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
  const mm = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
  const L = 0.2104542553 * l + 0.793617785 * mm - 0.0040720468 * s
  const A = 1.9779984951 * l - 2.428592205 * mm + 0.4505937099 * s
  const B = 0.0259040371 * l + 0.7827717662 * mm - 0.808675766 * s
  const c = Math.hypot(A, B)
  const h = c < 1e-4 ? 0 : (Math.atan2(B, A) * 180) / Math.PI
  return ok(L, c, h)
}

// oklchToHex is the nearest sRGB "#rrggbb" (for the colour pickers).
export function oklchToHex(x: Oklch): string {
  return linearToHex(toLinear(x))
}

const encode = (v: number) =>
  v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055

function linearToHex(rgb: Rgb): string {
  return encodedToHex(rgb.map(encode) as Rgb)
}

function encodedToHex(rgb: Rgb): string {
  return (
    "#" +
    rgb
      .map((s) =>
        Math.round(Math.min(1, Math.max(0, s)) * 255)
          .toString(16)
          .padStart(2, "0")
      )
      .join("")
  )
}

// mixHex flattens layers, bottom first, into one opaque "#rrggbb": each is
// composited by its alpha over those below it; the first counts as opaque.
// The notification cards (card #527) need plain colours: a native window
// can't stack CSS layers. It blends in encoded sRGB, as the browser does
// when it paints a translucent layer, so the result matches the window
// (blending in linear light would make white 12% over a dark card read
// about twice as bright as the app shows it).
export function mixHex(...layers: [Oklch, ...Oklch[]]): string {
  const enc = (x: Oklch) => toLinear(x).map(encode) as Rgb
  let rgb = enc(layers[0])
  for (const x of layers.slice(1)) {
    const fg = enc(x)
    const a = x.a ?? 1
    rgb = rgb.map((b, i) => fg[i] * a + b * (1 - a)) as Rgb
  }
  return encodedToHex(rgb)
}

// ---- themes -------------------------------------------------------------

export type Mode = "light" | "dark"

// The colour tokens a theme sets; everything else (radius, glass knobs,
// status hues) lives in design/irori-ds.css.
export interface Tokens {
  background: Oklch
  foreground: Oklch
  primary: Oklch
  "primary-foreground": Oklch
  secondary: Oklch
  "muted-surface": Oklch
  muted: Oklch
  "muted-foreground": Oklch
  accent: Oklch
  "accent-foreground": Oklch
  border: Oklch
  input: Oklch
  ring: Oklch
  info: Oklch
  "info-foreground": Oklch
  "chart-1": Oklch
  "glass-tint": Oklch
  "glass-ember-tint": Oklch
  "ink-2": Oklch
  "blue-deep": Oklch
  person: Oklch
  "person-surface": Oklch
  // The light behind the glass: three soft spots, as hues.
  backdrop: [Oklch, Oklch, Oklch]
}

// A seed: the neutrals' hue and chroma, the accent's per mode, and the
// backdrop's three hues. Lightness comes from the fixed ramp below, so
// text stays readable whatever the hues are.
export interface Seed {
  nh: number
  nc: number
  light: { h: number; c: number; l?: number }
  dark: { h: number; c: number; l?: number }
  spots: [number, number, number]
  // Chroma of the backdrop spots (Mono turns it down).
  spotC?: number
}

// Info is blue (250) beside the status hues; next to a blue accent it
// moves to cyan so the two never read as one.
const apart = (a: number, b: number) => Math.abs(((a - b + 540) % 360) - 180)
const infoHue = (accentHue: number) => (apart(accentHue, 250) < 35 ? 200 : 250)
// The person's face is violet (300), clear of the status hues; next to a
// violet accent it moves to rose.
const personHue = (accentHue: number) =>
  apart(accentHue, 300) < 40 ? 345 : 300

export function tokensFrom(seed: Seed, mode: Mode): Tokens {
  const { nh, nc } = seed
  const sc = seed.spotC ?? 1
  if (mode === "light") {
    const a = seed.light
    const p = ok(a.l ?? 0.56, a.c, a.h)
    const fg = ok(0.22, nc, nh)
    const ih = infoHue(a.h)
    return {
      background: ok(0.955, nc * 0.67, nh + 15),
      foreground: fg,
      primary: p,
      "primary-foreground": ok(0.99, 0.005, nh + 15),
      secondary: { ...fg, a: 0.06 },
      "muted-surface": { ...fg, a: 0.06 },
      muted: { ...fg, a: 0.06 },
      "muted-foreground": ok(0.46, nc * 1.33, nh + 5),
      accent: { ...p, a: 0.1 },
      "accent-foreground": ok(Math.min(p.l, 0.42), a.c * 0.82, a.h),
      border: { ...fg, a: 0.12 },
      input: { ...fg, a: 0.16 },
      ring: p,
      info: ok(0.5, 0.13, ih),
      "info-foreground": ok(0.5, 0.13, ih, 0.12),
      "chart-1": p,
      "glass-tint": ok(0.995, 0.004, nh + 15),
      "glass-ember-tint": ok(0.97, Math.min(0.035, a.c * 0.2), a.h + 22),
      "ink-2": ok(0.36, nc * 1.17, nh + 2),
      "blue-deep": ok(Math.max(0.2, p.l - 0.06), a.c, a.h - 2),
      person: ok(0.5, 0.15, personHue(a.h)),
      "person-surface": ok(0.5, 0.15, personHue(a.h), 0.12),
      backdrop: [
        ok(0.8, 0.12 * sc, seed.spots[0], 0.55),
        ok(0.78, 0.09 * sc, seed.spots[1], 0.35),
        ok(0.8, 0.07 * sc, seed.spots[2], 0.35),
      ],
    }
  }
  const a = seed.dark
  const p = ok(a.l ?? 0.75, a.c, a.h)
  const ih = infoHue(a.h)
  return {
    background: ok(0.17, nc, nh - 5),
    foreground: ok(0.96, Math.min(0.006, nc), nh + 20),
    primary: p,
    "primary-foreground": ok(0.2, Math.min(0.03, a.c * 0.2), a.h - 12),
    secondary: ok(1, 0, 0, 0.07),
    "muted-surface": ok(1, 0, 0, 0.07),
    muted: ok(1, 0, 0, 0.07),
    "muted-foreground": ok(0.76, nc * 1.17, nh + 10),
    accent: { ...p, a: 0.14 },
    "accent-foreground": ok(Math.max(p.l, 0.84), a.c * 0.8, a.h + 8),
    border: ok(1, 0, 0, 0.11),
    input: ok(1, 0, 0, 0.14),
    ring: p,
    info: ok(0.76, 0.12, ih),
    "info-foreground": ok(0.76, 0.12, ih, 0.15),
    "chart-1": p,
    "glass-tint": ok(0.23, nc * 1.17, nh - 2),
    "glass-ember-tint": ok(0.27, Math.min(0.06, a.c * 0.4), a.h - 10),
    "ink-2": ok(0.86, Math.min(0.01, nc), nh + 15),
    "blue-deep": ok(p.l - 0.07, a.c, a.h - 4),
    person: ok(0.76, 0.12, personHue(a.h)),
    "person-surface": ok(0.76, 0.12, personHue(a.h), 0.16),
    backdrop: [
      ok(0.5, 0.14 * sc, seed.spots[0], 0.4),
      ok(0.42, 0.12 * sc, seed.spots[1], 0.28),
      ok(0.44, 0.08 * sc, seed.spots[2], 0.3),
    ],
  }
}

export interface Theme {
  id: string
  name: string
  note: string
  seed: Seed
  // Exact tokens that win over the seed's (Hearth keeps #266's numbers).
  exact?: Partial<Record<Mode, Partial<Tokens>>>
}

// Hearth is the look #266 shipped, number for number.
const hearthExact: Record<Mode, Partial<Tokens>> = {
  light: {
    background: ok(0.955, 0.008, 70),
    foreground: ok(0.22, 0.012, 50),
    primary: ok(0.56, 0.17, 38),
    "primary-foreground": ok(0.99, 0.005, 70),
    secondary: ok(0.22, 0.012, 50, 0.06),
    "muted-surface": ok(0.22, 0.012, 50, 0.06),
    muted: ok(0.22, 0.012, 50, 0.06),
    "muted-foreground": ok(0.46, 0.016, 55),
    accent: ok(0.56, 0.17, 38, 0.1),
    "accent-foreground": ok(0.42, 0.14, 38),
    border: ok(0.22, 0.012, 50, 0.12),
    input: ok(0.22, 0.012, 50, 0.16),
    ring: ok(0.56, 0.17, 38),
    "chart-1": ok(0.56, 0.17, 38),
    "glass-tint": ok(0.995, 0.004, 70),
    "glass-ember-tint": ok(0.97, 0.035, 60),
    "ink-2": ok(0.36, 0.014, 52),
    "blue-deep": ok(0.5, 0.17, 36),
    backdrop: [
      ok(0.8, 0.12, 55, 0.55),
      ok(0.78, 0.09, 340, 0.35),
      ok(0.8, 0.07, 195, 0.35),
    ],
  },
  dark: {
    background: ok(0.17, 0.012, 45),
    foreground: ok(0.96, 0.006, 70),
    primary: ok(0.75, 0.15, 52),
    "primary-foreground": ok(0.2, 0.03, 40),
    "muted-foreground": ok(0.76, 0.014, 60),
    accent: ok(0.75, 0.15, 52, 0.14),
    "accent-foreground": ok(0.84, 0.12, 60),
    ring: ok(0.75, 0.15, 52),
    "chart-1": ok(0.75, 0.15, 52),
    "glass-tint": ok(0.23, 0.014, 48),
    "glass-ember-tint": ok(0.27, 0.06, 42),
    "ink-2": ok(0.86, 0.01, 65),
    "blue-deep": ok(0.68, 0.16, 48),
    backdrop: [
      ok(0.5, 0.14, 45, 0.4),
      ok(0.42, 0.12, 330, 0.28),
      ok(0.44, 0.08, 195, 0.3),
    ],
  },
}

export const themes: Theme[] = [
  {
    id: "blue",
    name: "Blue",
    note: "Cool slate, a clear blue accent",
    seed: {
      nh: 255,
      nc: 0.014,
      light: { h: 255, c: 0.17, l: 0.53 },
      dark: { h: 250, c: 0.14, l: 0.74 },
      spots: [245, 290, 200],
    },
  },
  {
    id: "hearth",
    name: "Hearth",
    note: "Warm ash, an ember accent",
    seed: {
      nh: 55,
      nc: 0.012,
      light: { h: 38, c: 0.17 },
      dark: { h: 52, c: 0.15 },
      spots: [55, 340, 195],
    },
    exact: hearthExact,
  },
  {
    id: "midnight",
    name: "Midnight",
    note: "Deep indigo, a violet glow",
    seed: {
      nh: 280,
      nc: 0.02,
      light: { h: 285, c: 0.17, l: 0.5 },
      dark: { h: 290, c: 0.13, l: 0.74 },
      spots: [280, 320, 230],
    },
  },
  {
    id: "forest",
    name: "Forest",
    note: "Moss and bark, a green accent",
    seed: {
      nh: 145,
      nc: 0.014,
      light: { h: 150, c: 0.13, l: 0.5 },
      dark: { h: 150, c: 0.13, l: 0.76 },
      spots: [140, 90, 190],
    },
  },
  {
    id: "sakura",
    name: "Sakura",
    note: "Petal pink on soft stone",
    seed: {
      nh: 350,
      nc: 0.012,
      light: { h: 355, c: 0.16, l: 0.54 },
      dark: { h: 355, c: 0.12, l: 0.78 },
      spots: [355, 20, 300],
    },
  },
  {
    id: "mono",
    name: "Mono",
    note: "Greys only; the status colours keep theirs",
    seed: {
      nh: 0,
      nc: 0,
      light: { h: 0, c: 0, l: 0.3 },
      dark: { h: 0, c: 0, l: 0.88 },
      spots: [0, 0, 0],
      spotC: 0,
    },
  },
]

export const defaultTheme = "blue"

export function themeTokens(t: Theme, mode: Mode): Tokens {
  return { ...tokensFrom(t.seed, mode), ...(t.exact?.[mode] ?? {}) }
}

// ---- custom colours -----------------------------------------------------

export interface Custom {
  base: string // the theme it started from
  accent: string // "#rrggbb"
  tint: string // "#rrggbb": the background's hue
}

// customSeed keeps the hues and chroma of the picks; the ramp sets the
// lightness, so a near-white or near-black pick still reads.
export function customSeed(c: Custom): Seed {
  const base = themes.find((t) => t.id === c.base) ?? themes[0]
  const a = hexToOklch(c.accent)
  const n = hexToOklch(c.tint)
  const ac = Math.min(a.c, 0.2)
  return {
    nh: n.h,
    nc: Math.min(n.c * 0.3, 0.03),
    light: { h: a.h, c: ac, l: base.seed.light.l },
    dark: { h: a.h, c: Math.min(ac, 0.16), l: base.seed.dark.l },
    spots: [a.h, n.h + 40, n.h - 60],
  }
}

const AA = 4.5

// guard nudges the accent until primary text, accent text and the ring
// pass AA in that mode.
export function guard(t: Tokens, mode: Mode): Tokens {
  const out = { ...t }
  const bg = out.background
  const step = mode === "light" ? -0.02 : 0.02
  const fgs = [ok(0.99, 0.005, out.primary.h), ok(0.2, 0.03, out.primary.h)]
  for (let i = 0; i < 30; i++) {
    const best = fgs.reduce((x, y) =>
      contrast(x, out.primary, bg) >= contrast(y, out.primary, bg) ? x : y
    )
    out["primary-foreground"] = best
    if (contrast(best, out.primary, bg) >= AA) break
    // Move the accent away from its text.
    const away = best.l > 0.5 ? -0.02 : 0.02
    out.primary = { ...out.primary, l: out.primary.l + away }
  }
  out.ring = out.primary
  out["chart-1"] = out.primary
  out.accent = { ...out.primary, a: out.accent.a }
  let af = out["accent-foreground"]
  for (let i = 0; i < 40 && contrast(af, out.accent, bg) < AA; i++) {
    af = { ...af, l: Math.min(1, Math.max(0, af.l + step)) }
  }
  out["accent-foreground"] = af
  return out
}

export function customTokens(c: Custom, mode: Mode): Tokens {
  return guard(tokensFrom(customSeed(c), mode), mode)
}

// ---- CSS ----------------------------------------------------------------

function block(sel: string, t: Tokens): string {
  const lines = Object.entries(t)
    .filter(([k]) => k !== "backdrop")
    .map(([k, v]) => `  --${k}: ${css(v as Oklch)};`)
  const [s1, s2, s3] = t.backdrop
  lines.push(
    "  --backdrop:",
    `    radial-gradient(56rem 44rem at 8% -12%, ${css(s1)}, transparent 62%),`,
    `    radial-gradient(44rem 40rem at 100% 30%, ${css(s2)}, transparent 60%),`,
    `    radial-gradient(50rem 42rem at 40% 118%, ${css(s3)}, transparent 62%);`
  )
  return `${sel} {\n${lines.join("\n")}\n}\n`
}

const sel = (id: string, mode: Mode) =>
  `html[data-ds="irori"][data-theme="${id}"]${mode === "dark" ? ".dark" : ""}`

// themeCSS is one theme's light and dark blocks.
export function themeCSS(id: string, tokens: (m: Mode) => Tokens): string {
  return (
    block(sel(id, "light"), tokens("light")) +
    block(sel(id, "dark"), tokens("dark"))
  )
}

// themesCSS is design/themes.css.
export function themesCSS(): string {
  return (
    "/* Generated by `bun scripts/gen-themes.ts` from src/lib/appearance.ts\n" +
    "   (card #275). Don't edit: change the themes there and run it again. */\n\n" +
    themes.map((t) => themeCSS(t.id, (m) => themeTokens(t, m))).join("\n")
  )
}

export function customCSS(c: Custom): string {
  return themeCSS("custom", (m) => customTokens(c, m))
}

// ---- the saved choice -----------------------------------------------------

export const storeKey = "irori.appearance"

export type ModeChoice = Mode | "system"

export const glassStyles = [
  { id: "frost", name: "Frost", note: "Even frost, soft hairline rim" },
  { id: "clear", name: "Clear", note: "Barely frosted; the backdrop shows" },
  { id: "milk", name: "Milk", note: "Heavy frost, almost paper" },
  { id: "rim", name: "Rim", note: "Lit edges and a top sheen" },
  { id: "ember", name: "Ember", note: "Glass tinted by the accent" },
  { id: "liquid", name: "Liquid", note: "Clear, saturated, thick bright rims" },
] as const

export interface Appearance {
  theme: string // a theme id, or "custom"
  mode: ModeChoice
  glass: string
  custom?: Custom
  // customCSS(custom), kept so index.html can apply it before first paint.
  css?: string
}

export const defaults: Appearance = {
  theme: defaultTheme,
  mode: "dark",
  glass: "frost",
}

// clean drops what this build doesn't know.
export function clean(a: Partial<Appearance> | null | undefined): Appearance {
  const out: Appearance = { ...defaults }
  if (!a || typeof a !== "object") return out
  if (a.mode === "light" || a.mode === "dark" || a.mode === "system")
    out.mode = a.mode
  if (glassStyles.some((g) => g.id === a.glass)) out.glass = a.glass!
  if (themes.some((t) => t.id === a.theme)) out.theme = a.theme!
  const c = a.custom
  if (
    c &&
    typeof c.accent === "string" &&
    typeof c.tint === "string" &&
    /^#[0-9a-f]{6}$/i.test(c.accent) &&
    /^#[0-9a-f]{6}$/i.test(c.tint)
  ) {
    out.custom = {
      base: themes.some((t) => t.id === c.base) ? c.base : defaultTheme,
      accent: c.accent,
      tint: c.tint,
    }
    if (a.theme === "custom") {
      out.theme = "custom"
      out.css = customCSS(out.custom)
    }
  }
  return out
}

export function loadAppearance(): Appearance {
  try {
    return clean(JSON.parse(localStorage.getItem(storeKey) ?? "null"))
  } catch {
    return { ...defaults }
  }
}

function saveAppearance(a: Appearance) {
  try {
    localStorage.setItem(storeKey, JSON.stringify(a))
  } catch {
    // Private mode or full storage: the choice holds for this run.
  }
}

const systemDark = () =>
  typeof window !== "undefined" &&
  window.matchMedia?.("(prefers-color-scheme: dark)").matches

export function resolvedMode(m: ModeChoice): Mode {
  return m === "system" ? (systemDark() ? "dark" : "light") : m
}

// What the page shows now (applyAppearance's last), which the browser
// preview's flags may have changed from the saved choice.
let shown: Appearance | null = null

// Told after every applyAppearance (card #527: main.tsx passes the look on
// to the notification cards, so this file needn't know the host).
const appliedListeners = new Set<(a: Appearance) => void>()

// onAppearanceApplied calls fn after each applyAppearance; it returns the
// unsubscribe.
export function onAppearanceApplied(fn: (a: Appearance) => void): () => void {
  appliedListeners.add(fn)
  return () => appliedListeners.delete(fn)
}

// applyAppearance sets the page as index.html's script does.
export function applyAppearance(a: Appearance) {
  shown = a
  const html = document.documentElement
  html.dataset.ds = "irori"
  html.dataset.theme = a.theme
  html.dataset.glass = a.glass
  const dark = resolvedMode(a.mode) === "dark"
  html.classList.toggle("dark", dark)
  html.style.colorScheme = dark ? "dark" : "light"
  let style = document.getElementById("irori-custom-theme")
  if (a.theme === "custom" && a.css) {
    if (!style) {
      style = document.createElement("style")
      style.id = "irori-custom-theme"
      document.head.appendChild(style)
    }
    style.textContent = a.css
  } else {
    style?.remove()
  }
  for (const fn of appliedListeners) fn(a)
}

// useAppearance is the saved choice and its setter; it follows the
// system's light/dark while the choice is "system".
export function useAppearance() {
  const [a, setA] = useState<Appearance>(() => shown ?? loadAppearance())

  useEffect(() => {
    if (a.mode !== "system") return
    const mq = window.matchMedia("(prefers-color-scheme: dark)")
    const on = () => applyAppearance(a)
    mq.addEventListener("change", on)
    return () => mq.removeEventListener("change", on)
  }, [a])

  const set = useCallback((patch: Partial<Appearance>) => {
    setA((cur) => {
      const next = clean({ ...cur, ...patch })
      saveAppearance(next)
      applyAppearance(next)
      return next
    })
  }, [])

  return { appearance: a, set }
}

// The swatch colours of a theme in a mode: background, accent, text.
export function swatch(id: string, mode: Mode, custom?: Custom) {
  const t =
    id === "custom" && custom
      ? customTokens(custom, mode)
      : themeTokens(themes.find((x) => x.id === id) ?? themes[0], mode)
  return {
    bg: css(t.background),
    accent: css(t.primary),
    text: css(t.foreground),
  }
}
