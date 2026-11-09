import { useEffect, useState, useSyncExternalStore } from "react"

import {
  currentSpot,
  linePath,
  subscribeSpot,
  type From,
  type Point,
} from "@/lib/spotlight"

// SpotlightLine is the glowing line from the chat's go button to what it
// highlighted (card #784, as the person's DentCharter and RGBeast
// assistants draw it): a dashed curve with a dot at each end, over
// everything, never in the way of a click; not from a button something
// covers (Settings opened over the chat). It follows both
// ends each frame while the highlight holds, and is gone with it. With the
// chat closed (a phone) there is no button to start from, so no line: the
// outline alone shows the place.
export function SpotlightLine() {
  const spot = useSyncExternalStore(subscribeSpot, currentSpot, currentSpot)
  const [line, setLine] = useState<{
    d: string
    a: Point
    b: Point
  } | null>(null)

  useEffect(() => {
    if (!spot) return
    let frame = 0
    const tick = () => {
      frame = requestAnimationFrame(tick)
      const from = resolve(spot.from)
      const f = from?.isConnected ? from.getBoundingClientRect() : null
      const t = spot.target.isConnected
        ? spot.target.getBoundingClientRect()
        : null
      const panel = from?.closest("[data-assistant-panel]")
      // A go button covered (Settings over the chat) starts no line.
      const shown =
        !!f &&
        !!from &&
        from.contains(
          document.elementFromPoint(f.left + f.width / 2, f.top + f.height / 2)
        )
      const next =
        shown && f.width && t?.width
          ? linePath(f, t, panel?.getBoundingClientRect())
          : null
      setLine((was) => (was?.d === next?.d ? was : next))
    }
    frame = requestAnimationFrame(tick)
    return () => {
      cancelAnimationFrame(frame)
      setLine(null)
    }
  }, [spot])

  if (!spot || !line) return null
  return (
    <svg
      aria-hidden
      data-spotlight-line=""
      className="pointer-events-none fixed inset-0 z-[60] h-full w-full overflow-visible"
    >
      <defs>
        <filter
          id="irori-spot-glow"
          x="-50%"
          y="-50%"
          width="200%"
          height="200%"
        >
          <feGaussianBlur stdDeviation="2" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>
      <path
        d={line.d}
        fill="none"
        stroke="var(--primary)"
        strokeWidth={2}
        strokeLinecap="round"
        filter="url(#irori-spot-glow)"
        className="irori-spot-line"
      />
      {[line.a, line.b].map((p, i) => (
        <circle
          key={i}
          cx={p.x}
          cy={p.y}
          r={4.5}
          fill="var(--primary)"
          stroke="var(--background)"
          strokeWidth={1.5}
          className="irori-spot-dot"
        />
      ))}
    </svg>
  )
}

function resolve(from: From | null): Element | null {
  if (!from) return null
  return typeof from === "function" ? from() : from
}
