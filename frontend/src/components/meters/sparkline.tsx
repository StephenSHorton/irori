import { cn } from "cn"

// Sparkline is a small area-and-line graph of the newest readings (card
// #599), drawn in currentColor: set the colour with `color` on it or a
// parent. Readings fill from the right, as Task Manager's do, across
// `slots` places; a missing reading draws as 0.
export function Sparkline({
  values,
  max = 100,
  slots,
  width,
  height,
  className,
}: {
  values: readonly (number | null)[]
  // The top of the scale: a number (100 for percents), or "auto" for a
  // rate, scaled to the largest reading shown.
  max?: number | "auto"
  // How many readings the width holds; the values' length by default.
  slots?: number
  width: number
  height: number
  className?: string
}) {
  const n = Math.max(slots ?? values.length, 2)
  const shown = values.slice(-n)
  const top =
    max === "auto"
      ? Math.max(...shown.map((v) => v ?? 0), 1e-9) * 1.1
      : max
  const W = 100
  const H = 100
  const step = W / (n - 1)
  const start = n - shown.length
  const pts = shown.map((v, i) => {
    const y = H - (Math.min(Math.max(v ?? 0, 0), top) / top) * H
    return `${((start + i) * step).toFixed(2)},${y.toFixed(2)}`
  })
  const line = pts.length ? `M${pts.join("L")}` : ""
  const area =
    pts.length > 1
      ? `${line}L${W},${H}L${(start * step).toFixed(2)},${H}Z`
      : ""
  return (
    <svg
      aria-hidden
      width={width}
      height={height}
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      className={cn("shrink-0 overflow-visible", className)}
    >
      {area && <path d={area} fill="currentColor" fillOpacity={0.16} stroke="none" />}
      {pts.length > 1 && (
        <path
          d={line}
          fill="none"
          stroke="currentColor"
          strokeWidth={1.25}
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  )
}
