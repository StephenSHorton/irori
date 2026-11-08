// Card #765: a window nobody can see needn't re-read its lists. whileShown
// wraps a reload so that while the page is hidden (minimised, or closed to
// the tray) the signals that would run it are only noted, and it runs once
// when the page shows again: one read for a whole hidden stretch, however
// many cards changed meanwhile.

type Doc = {
  readonly hidden: boolean
  addEventListener(type: "visibilitychange", fn: () => void): void
  removeEventListener(type: "visibilitychange", fn: () => void): void
}

const page = (): Doc | null =>
  typeof document === "undefined" ? null : (document as Doc)

// whileShown returns fn held while the page is hidden, and the function
// that stops it (the held call is dropped).
export function whileShown(
  fn: () => void,
  doc: Doc | null = page()
): { run: () => void; stop: () => void } {
  if (!doc) return { run: fn, stop: () => {} }
  let owed = false
  const onShow = () => {
    if (doc.hidden || !owed) return
    owed = false
    fn()
  }
  doc.addEventListener("visibilitychange", onShow)
  return {
    run() {
      if (doc.hidden) owed = true
      else fn()
    },
    stop() {
      owed = false
      doc.removeEventListener("visibilitychange", onShow)
    },
  }
}
