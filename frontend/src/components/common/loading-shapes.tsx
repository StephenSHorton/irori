import { cn } from "cn"

// Shapes for Loading (components/ui/loading.tsx, card #598): plain markup
// in the size and rhythm of what loads, which Loading measures into a
// skeleton. They hold no hooks and read no context, so drawing one has no
// side effects (a real row rendered on sample data would watch fake ids).
// The words only set each line's length; nobody sees them.

// MessageRowsShape is a run of thread messages: the time column, then a
// name and one or two lines, as the card's Thread and Talk draw them.
export function MessageRowsShape({
  rows = 3,
  className,
}: {
  rows?: number
  className?: string
}) {
  const lines = [
    ["sample-heron", "Looked at the list read; it waits on the store's lock."],
    ["Stephen", "Go ahead with the smaller change first."],
    [
      "sample-otter",
      "Pushed the branch and opened the pull request, checks are running now. The screenshots are on the card.",
    ],
    ["sample-heron", "Done. Ticked the step."],
  ]
  return (
    <div className={cn("grid gap-3 text-[12.5px]", className)}>
      {Array.from({ length: rows }, (_, i) => {
        const [who, text] = lines[i % lines.length]
        return (
          <div key={i} className="grid grid-cols-[56px_minmax(0,1fr)] gap-2">
            <span className="text-muted-foreground tabular-nums">10:4{i}</span>
            <div className="grid min-w-0 gap-1">
              <span className="font-medium text-ink">{who}</span>
              <p className="text-ink-2">{text}</p>
            </div>
          </div>
        )
      })}
    </div>
  )
}

// ListRowsShape is a column of list rows: a dot or face, a title, a short
// line under it.
export function ListRowsShape({
  rows = 4,
  sub = true,
  className,
  rowClassName,
}: {
  rows?: number
  sub?: boolean
  className?: string
  rowClassName?: string
}) {
  const titles = [
    "Loading states across the app",
    "Talk: channels and threads",
    "Settings panes",
    "Release notes for the next version",
    "Phone width check",
  ]
  return (
    <ul className={cn("grid gap-1", className)}>
      {Array.from({ length: rows }, (_, i) => (
        <li
          key={i}
          className={cn("flex items-start gap-2 px-2 py-1.5", rowClassName)}
        >
          <span className="mt-0.5 block size-4 shrink-0 rounded-full" />
          <span className="grid min-w-0 gap-0.5">
            <span className="text-[13px] text-ink">
              {titles[i % titles.length]}
            </span>
            {sub && (
              <span className="text-[11.5px] text-muted-foreground">
                sample-heron · 12 min
              </span>
            )}
          </span>
        </li>
      ))}
    </ul>
  )
}
