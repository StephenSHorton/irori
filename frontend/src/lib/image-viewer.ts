// Moving between the pictures of one message (card #1011): the viewer wraps,
// so Next on the last picture shows the first and Previous on the first
// shows the last.

// stepIndex is the picture `dir` steps from `index` among `count`, wrapping.
export function stepIndex(index: number, dir: 1 | -1, count: number): number {
  if (count <= 0) return 0
  return (((index + dir) % count) + count) % count
}

// navDirection is which way a key press moves the viewer: ArrowRight 1,
// ArrowLeft -1, anything else (or a chord, or typing in a field) null.
export function navDirection(e: {
  key: string
  altKey?: boolean
  ctrlKey?: boolean
  metaKey?: boolean
  shiftKey?: boolean
  target?: unknown
}): 1 | -1 | null {
  if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return null
  const t = e.target as { tagName?: string; isContentEditable?: boolean } | null
  const tag = t?.tagName?.toUpperCase()
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return null
  if (t?.isContentEditable) return null
  if (e.key === "ArrowRight") return 1
  if (e.key === "ArrowLeft") return -1
  return null
}
