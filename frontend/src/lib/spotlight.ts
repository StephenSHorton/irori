// spotlight briefly highlights one thing in the window (card #784, Ask
// Irori's "take me there"): it waits for the element to appear (a page or
// a Settings pane may still be loading), scrolls it to the middle and
// marks it data-spotlight, which index.css pulses twice and then lets go.
// The mark is always taken off on a timer: the animation runs once, never
// forever (endless animations kept removed nodes alive, card #765). With
// Reduce Motion on, index.css shows a still outline for the same time.

// SpotlightMs is how long the mark stays (index.css's animation fits it).
export const SpotlightMs = 2800

// WaitMs is how long spotlight waits for the element to appear.
export const WaitMs = 8000

let current: { el: Element; timer: ReturnType<typeof setTimeout> } | null = null
let waiting: (() => void) | null = null

function clear() {
  if (!current) return
  clearTimeout(current.timer)
  current.el.removeAttribute("data-spotlight")
  current = null
}

function mark(el: Element) {
  clear()
  const reduce =
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
  el.scrollIntoView?.({ block: "center", behavior: reduce ? "auto" : "smooth" })
  // A fresh attribute restarts the animation even on the same element.
  el.setAttribute("data-spotlight", "")
  const timer = setTimeout(() => {
    if (current?.el === el) current = null
    el.removeAttribute("data-spotlight")
  }, SpotlightMs)
  current = { el, timer }
}

// spotlight resolves true once it highlighted the first element matching
// selector, false when none appeared within wait ms (or a newer spotlight
// took over). Only one thing is highlighted at a time.
export function spotlight(selector: string, wait = WaitMs): Promise<boolean> {
  waiting?.()
  waiting = null
  const now = document.querySelector(selector)
  if (now) {
    mark(now)
    return Promise.resolve(true)
  }
  return new Promise((resolve) => {
    const done = (el: Element | null) => {
      obs.disconnect()
      clearTimeout(timer)
      if (waiting === cancel) waiting = null
      if (el) mark(el)
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
    const timer = setTimeout(cancel, wait)
    waiting = cancel
  })
}
