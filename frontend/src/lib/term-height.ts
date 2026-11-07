// The card-page terminal's height (card #730): dragging the handle on its
// bottom edge (or its arrow keys) sets it between minHeight and the
// window's height less some room for the page, and this device remembers
// it, so every terminal opens at the height last chosen.

export const defaultHeight = 360
export const minHeight = 200
// What the window keeps above and below the tallest terminal: the card's
// top bar and the tabs row, so the handle stays on screen.
export const pageRoom = 96

export const heightKey = "irori.term.height"

// maxHeight is the tallest the terminal goes in a window innerHeight tall,
// never under minHeight.
export function maxHeight(innerHeight: number): number {
  if (!Number.isFinite(innerHeight)) return defaultHeight
  return Math.max(minHeight, Math.round(innerHeight - pageRoom))
}

// clampHeight keeps h between minHeight and maxHeight(innerHeight); a bad
// value is the default.
export function clampHeight(h: number, innerHeight: number): number {
  if (!Number.isFinite(h))
    return Math.min(defaultHeight, maxHeight(innerHeight))
  return Math.round(Math.min(maxHeight(innerHeight), Math.max(minHeight, h)))
}

// readHeight is the remembered height, or the default when there is none
// or storage is shut (a private window). It is not clamped to the window:
// the box's CSS max-height does that, so a window that is only opened
// smaller (with no drag there) still gets the full height back when grown.
export function readHeight(): number {
  try {
    const v = localStorage.getItem(heightKey)
    if (v === null) return defaultHeight
    const n = Number(v)
    return Number.isFinite(n) && n >= minHeight ? Math.round(n) : defaultHeight
  } catch {
    return defaultHeight
  }
}

export function saveHeight(h: number | null) {
  try {
    if (h === null) localStorage.removeItem(heightKey)
    else localStorage.setItem(heightKey, String(Math.round(h)))
  } catch {
    // A private window keeps it for this run only.
  }
}
