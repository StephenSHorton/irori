// The notification cards' look (card #527): the person's appearance
// (lib/appearance.ts, design/irori-ds.css) flattened into the plain colours
// the native card windows (crates/toast) draw with, and sent to the host.
//
// Both palettes go every time, whatever the mode: the Rust side picks light
// or dark itself and follows the OS while the choice is "system", so an OS
// flip restyles the cards without the window being open.
//
// Every colour is an opaque "#rrggbb", flattened over the card's background
// as the browser blends (mixHex), because a native window draws flat
// fills, not CSS layers. The glass itself is `tint` at `alpha` over the blurred desktop;
// `background` is the same tint over the theme's page colour, the solid
// card where the OS has no glass.

import { commands, type CardColors, type CardLook } from "../gen/bindings"
import {
  contrast,
  customTokens,
  hexToOklch,
  mixHex,
  themes,
  themeTokens,
  type Appearance,
  type Mode,
  type Oklch,
  type Tokens,
} from "./appearance"
import { inHost } from "./irori"

// The glass styles' knobs from design/irori-ds.css, per mode: the tint's
// opacity (--glass-alpha) and the rim's (--glass-border, white).
const glass: Record<string, Record<Mode, { alpha: number; rim: number }>> = {
  frost: { light: { alpha: 0.62, rim: 0.7 }, dark: { alpha: 0.62, rim: 0.12 } },
  clear: { light: { alpha: 0.3, rim: 0.7 }, dark: { alpha: 0.3, rim: 0.12 } },
  milk: { light: { alpha: 0.84, rim: 0.7 }, dark: { alpha: 0.84, rim: 0.12 } },
  rim: { light: { alpha: 0.56, rim: 0.95 }, dark: { alpha: 0.56, rim: 0.24 } },
  ember: { light: { alpha: 0.64, rim: 0.7 }, dark: { alpha: 0.6, rim: 0.12 } },
  liquid: { light: { alpha: 0.22, rim: 0.9 }, dark: { alpha: 0.3, rim: 0.3 } },
}

// --radius: 0.875rem, and 1.5rem under Liquid (in dip, at 16px a rem).
const radius = (g: string) => (g === "liquid" ? 24 : 14)

// The status hues from design/irori-ds.css (they don't follow the theme).
const status: Record<Mode, { success: Oklch; warn: Oklch; error: Oklch }> = {
  light: {
    success: { l: 0.5, c: 0.13, h: 150 },
    warn: { l: 0.52, c: 0.12, h: 70 },
    error: { l: 0.55, c: 0.2, h: 27 },
  },
  dark: {
    success: { l: 0.78, c: 0.14, h: 150 },
    warn: { l: 0.84, c: 0.13, h: 80 },
    error: { l: 0.68, c: 0.19, h: 27 },
  },
}

const white = (a: number): Oklch => ({ l: 1, c: 0, h: 0, a })

// Below this the rim can't be told from the card (light mode's white rim
// on near-white glass, about 1.05:1).
const minEdge = 1.15

function tokens(a: Appearance, m: Mode): Tokens {
  if (a.theme === "custom" && a.custom) return customTokens(a.custom, m)
  return themeTokens(themes.find((t) => t.id === a.theme) ?? themes[0], m)
}

function knobs(g: string, m: Mode) {
  return (glass[g] ?? glass.frost)[m]
}

// cardColors is one mode's palette.
export function cardColors(a: Appearance, m: Mode): CardColors {
  const t = tokens(a, m)
  const k = knobs(a.glass, m)
  const tint = a.glass === "ember" ? t["glass-ember-tint"] : t["glass-tint"]
  const background = mixHex(t.background, { ...tint, a: k.alpha })
  const bg = hexToOklch(background)
  // The glass rim, as the app draws it. On the desktop the card has no
  // backdrop glow to stand out from, so where the white rim all but
  // vanishes into a pale card, the theme's hairline (--border) is used
  // instead to keep an edge.
  // The test is on the flattened rim, the colour the card will draw
  // (contrast's own compositing is in linear light, brighter than mixHex).
  const rim = mixHex(bg, white(k.rim))
  const border = contrast(hexToOklch(rim), bg) >= minEdge ? rim : mixHex(bg, t.border)
  const hover = m === "light" ? { ...t.foreground, a: 0.08 } : white(0.1)
  const s = status[m]
  return {
    background,
    tint: mixHex(tint),
    foreground: mixHex(t.foreground),
    muted: mixHex(bg, t["muted-foreground"]),
    border,
    hover: mixHex(bg, hover),
    button: mixHex(bg, t.secondary),
    primary: mixHex(t.primary),
    onPrimary: mixHex(t.primary, t["primary-foreground"]),
    // Info takes the theme's accent, so Sakura's is pink, Forest's green.
    info: mixHex(t.primary),
    success: mixHex(s.success),
    warn: mixHex(s.warn),
    error: mixHex(s.error),
  }
}

// cardLook is the whole look, both modes, for settings_set_card_look.
export function cardLook(a: Appearance): CardLook {
  // One alpha for both palettes: the mode that will show. Under "system"
  // that could be either, so it takes dark's; the two differ only for
  // Ember (.64/.60) and Liquid (.22/.30).
  const m: Mode = a.mode === "light" ? "light" : "dark"
  return {
    theme: a.theme,
    mode: a.mode,
    glass: a.glass,
    alpha: knobs(a.glass, m).alpha,
    radius: radius(a.glass),
    light: cardColors(a, "light"),
    dark: cardColors(a, "dark"),
  }
}

let lastSent = ""

// sendCardLook passes the look to the host when it changed. Only the
// window has a host; the browser preview's ?theme= flags never reach it.
// It never throws: a card in the old look is no reason to break the page.
export async function sendCardLook(a: Appearance): Promise<void> {
  if (!inHost()) return
  try {
    const look = cardLook(a)
    const json = JSON.stringify(look)
    if (json === lastSent) return
    lastSent = json
    await commands.settingsSetCardLook(look)
  } catch (err) {
    lastSent = ""
    console.error("irori: couldn't send the notification cards' look", err)
  }
}
