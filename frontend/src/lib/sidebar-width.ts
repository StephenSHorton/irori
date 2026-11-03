// The desktop sidebar's width (card #309): dragging its right edge sets it
// between minWidth and maxWidth, and the window remembers it on this device.
// Dragging an open sidebar well past the narrow end folds it to icons;
// dragging a folded one out unfolds it. The two thresholds differ so a hand
// resting near one doesn't flick it back and forth.

export const defaultWidth = 240 // 15rem, the old w-60
export const minWidth = 192 // 12rem
export const maxWidth = 512 // 32rem
export const foldedWidth = 56 // w-14
// An open sidebar folds when the pointer goes this far left of minWidth.
export const foldBelow = minWidth - 40
// A folded one opens when the pointer goes this far right of its edge.
export const unfoldAbove = foldedWidth + 48
// A drag that just folded it opens it again only back past the fold line,
// or every move between the two lines would flick it open and shut.
export const refoldMargin = 16

export const widthKey = "irori.sidebarWidth"

export function clampWidth(w: number): number {
  if (!Number.isFinite(w)) return defaultWidth
  return Math.round(Math.min(maxWidth, Math.max(minWidth, w)))
}

// readWidth is the remembered width, or the default when there is none or
// storage is shut (a private window).
export function readWidth(): number {
  try {
    const v = localStorage.getItem(widthKey)
    return v === null ? defaultWidth : clampWidth(Number(v))
  } catch {
    return defaultWidth
  }
}

export function saveWidth(w: number | null) {
  try {
    if (w === null) localStorage.removeItem(widthKey)
    else localStorage.setItem(widthKey, String(clampWidth(w)))
  } catch {
    // A private window keeps it for this run only.
  }
}

// Drag is one drag of the edge: whether the sidebar is folded, its open
// width (kept while folded, so unfolding returns to it), and catching: it
// was just unfolded to that width and waits for the pointer to reach its
// edge before following, so it doesn't jump to the pointer or fold back.
// foldedHere: this drag folded it (so the higher line opens it again).
// start: the open width when the drag began; folding goes back to it, since
// on the way to the fold line the width ran down to the minimum.
export type Drag = {
  folded: boolean
  width: number
  catching: boolean
  foldedHere?: boolean
  start?: number
}

// dragTo is where the pointer at x (from the sidebar's left edge) leaves a
// drag.
export function dragTo(x: number, d: Drag): Drag {
  if (d.folded) {
    if (x <= (d.foldedHere ? foldBelow + refoldMargin : unfoldAbove)) return d
    if (x < d.width) return { ...d, folded: false, catching: true }
    return { ...d, folded: false, width: clampWidth(x), catching: false }
  }
  if (d.catching) {
    // Back over the icons folds it again; reaching the edge picks it up.
    if (x <= foldedWidth) return { ...d, folded: true, foldedHere: true }
    if (x < d.width) return d
    return { ...d, folded: false, width: clampWidth(x), catching: false }
  }
  if (x < foldBelow)
    return { ...d, folded: true, foldedHere: true, width: d.start ?? d.width }
  return { ...d, folded: false, width: clampWidth(x), catching: false }
}
