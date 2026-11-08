// spotlight highlights one thing in the window for a while (card #784,
// Ask Irori's "take me there", after the person's DentCharter and RGBeast
// assistants). It waits for the element to appear (a page or a Settings
// pane may still be loading), scrolls it to the middle and marks it
// data-spotlight: index.css gives it a steady glowing outline that
// breathes, and components/assistant/spotlight-line.tsx draws a glowing
// line to it from the chat's go button while the chat is open. After
// SpotlightMs the mark comes off on a timer and the line goes with it.
// Every animation runs a set number of times, never forever (endless
// animations kept removed nodes alive, card #765). With Reduce Motion on
// the outline and line are still.

// SpotlightMs is how long the highlight holds (index.css's animations fit
// it).
export const SpotlightMs = 8000

// WaitMs is how long spotlight waits for the element to appear.
export const WaitMs = 8000

// SettleMs is how long the mark keeps the element in view while what is
// above it still loads (a Settings pane's skeleton growing into its real
// rows would push it out of sight).
export const SettleMs = 1500

// From is where the line starts: the go button the person tapped, or a
// lookup run each frame (the reply's button may not be drawn yet when the
// chat goes there by itself).
export type From = Element | (() => Element | null)

export type Spot = { target: Element; from: From | null }

type Mark = Spot & {
  timer: ReturnType<typeof setTimeout>
  stop: () => void
}

let current: Mark | null = null
let waiting: (() => void) | null = null
const subs = new Set<() => void>()

// currentSpot is what is highlighted now, for the line (null: nothing).
export function currentSpot(): Spot | null {
  return current
}

export function subscribeSpot(f: () => void): () => void {
  subs.add(f)
  return () => subs.delete(f)
}

function changed() {
  for (const f of subs) f()
}

function reduced(): boolean {
  return (
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
  )
}

function center(el: Element) {
  el.scrollIntoView?.({
    block: "center",
    behavior: reduced() ? "auto" : "smooth",
  })
}

// clearSpotlight takes the highlight off now.
export function clearSpotlight() {
  if (!current) return
  clearTimeout(current.timer)
  current.stop()
  current.target.removeAttribute("data-spotlight")
  current = null
  changed()
}

function mark(el: Element, from: From | null) {
  if (current) {
    clearTimeout(current.timer)
    current.stop()
    current.target.removeAttribute("data-spotlight")
    current = null
  }
  center(el)
  // A fresh attribute restarts the animation even on the same element.
  el.setAttribute("data-spotlight", "")
  const stop = keepInView(el)
  const timer = setTimeout(() => {
    if (current?.target === el) clearSpotlight()
    else el.removeAttribute("data-spotlight")
  }, SpotlightMs)
  current = { target: el, from, timer, stop }
  changed()
}

// keepInView scrolls el back to the middle when the page around it
// changes and moves it out of the window, for SettleMs; the person's own
// scrolling (wheel, touch, keys) ends it at once.
function keepInView(el: Element): () => void {
  let frame = 0
  const check = () => {
    frame = 0
    if (!el.isConnected) return
    const r = el.getBoundingClientRect()
    if (r.top < 0 || r.bottom > window.innerHeight) center(el)
  }
  const obs = new MutationObserver(() => {
    frame ||= requestAnimationFrame(check)
  })
  obs.observe(document.body, { childList: true, subtree: true })
  const events = ["wheel", "touchstart", "keydown"] as const
  let done = false
  const stop = () => {
    if (done) return
    done = true
    obs.disconnect()
    if (frame) cancelAnimationFrame(frame)
    clearTimeout(t)
    for (const e of events) window.removeEventListener(e, stop, true)
  }
  const t = setTimeout(stop, SettleMs)
  for (const e of events)
    window.addEventListener(e, stop, { capture: true, passive: true })
  return stop
}

// spotlight resolves true once it highlighted the first element matching
// selector, false when none appeared within wait ms (or a newer spotlight
// took over). Only one thing is highlighted at a time. from is where the
// chat's line starts.
export function spotlight(
  selector: string,
  opts: { from?: From | null; wait?: number } = {}
): Promise<boolean> {
  const from = opts.from ?? null
  waiting?.()
  waiting = null
  const now = document.querySelector(selector)
  if (now) {
    mark(now, from)
    return Promise.resolve(true)
  }
  return new Promise((resolve) => {
    const done = (el: Element | null) => {
      obs.disconnect()
      clearTimeout(timer)
      if (waiting === cancel) waiting = null
      if (el) mark(el, from)
      resolve(!!el)
    }
    const cancel = () => done(null)
    const obs = new MutationObserver(() => {
      const el = document.querySelector(selector)
      if (el) done(el)
    })
    obs.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["data-setting", "data-spot", "data-card-page"],
    })
    const timer = setTimeout(cancel, opts.wait ?? WaitMs)
    waiting = cancel
  })
}

// ---- the line --------------------------------------------------------------

export type Point = { x: number; y: number }
export type Rect = { left: number; top: number; right: number; bottom: number }

// linePath is the line from the go button (its left edge's middle) to the
// nearest point of the target's box, curving out level from the button
// and into the target, as an SVG path; null when the two overlap. A
// target running under the chat (a card's header, a Settings pane)
// is aimed at where it shows, left of the chat's panel.
export function linePath(
  from: Rect,
  target: Rect,
  panel?: Rect | null
): { d: string; a: Point; b: Point } | null {
  const a = { x: from.left, y: (from.top + from.bottom) / 2 }
  const to =
    panel && target.left < panel.left - 12 && target.right > panel.left
      ? // Field by field: a DOMRect's are getters, which a spread drops.
        {
          left: target.left,
          top: target.top,
          right: panel.left - 6,
          bottom: target.bottom,
        }
      : target
  const b = {
    x: Math.min(Math.max(a.x, to.left), to.right),
    y: Math.min(Math.max(a.y, to.top), to.bottom),
  }
  if (Math.hypot(b.x - a.x, b.y - a.y) < 12) return null
  const r = (n: number) => Math.round(n * 10) / 10
  return {
    d: `M ${r(a.x)} ${r(a.y)} Q ${r(b.x)} ${r(a.y)} ${r(b.x)} ${r(b.y)}`,
    a,
    b,
  }
}
