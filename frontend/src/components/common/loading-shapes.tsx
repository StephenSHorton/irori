import { cn } from "cn"

// Shapes for Loading (components/ui/loading.tsx, card #598): plain markup
// in the size and rhythm of what loads, which Loading measures into a
// skeleton. They hold no hooks and read no context, so drawing one has no
// side effects (a real row rendered on sample data would watch fake ids).
// The words only set each line's length; nobody sees them.

// AnswersShape is an item's answers in a Notifications row (card #644):
// its question, then its buttons.
export function AnswersShape() {
  return (
    <div className="grid gap-1.5">
      <p className="text-sm">Which region should the staging box run in?</p>
      <span className="flex gap-1.5">
        <span skullet-block="" className="block h-8 w-28 rounded-md" />
        <span skullet-block="" className="block h-8 w-20 rounded-md" />
        <span skullet-block="" className="block h-8 w-20 rounded-md" />
      </span>
    </div>
  )
}

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
          <span
            skullet-block=""
            className="mt-0.5 block size-4 shrink-0 rounded-full"
          />
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

// PostRowsShape is a run of Talk posts (a channel or a DM): a face, the
// name and time on one line, the message under it, as PostRow draws them.
export function PostRowsShape({
  rows = 4,
  className,
}: {
  rows?: number
  className?: string
}) {
  const posts = [
    ["sample-heron", "Looked at the list read; it waits on the store's lock."],
    ["You", "Go ahead with the smaller change first."],
    [
      "sample-otter",
      "Pushed the branch and opened the pull request, checks are running now. The screenshots are on the card.",
    ],
    ["sample-heron", "Done. Ticked the step."],
  ]
  return (
    <div className={cn("grid grid-cols-[minmax(0,1fr)] gap-0.5", className)}>
      {Array.from({ length: rows }, (_, i) => {
        const [who, text] = posts[i % posts.length]
        return (
          <div key={i} className="flex gap-2.5 rounded-lg px-2 py-1.5">
            <span
              skullet-block=""
              className="mt-0.5 block size-[26px] shrink-0"
            />
            <div className="grid min-w-0 flex-1 gap-0.5">
              <div className="flex min-w-0 items-baseline gap-2">
                <span className="text-[13px] font-medium text-ink">{who}</span>
                <span className="text-[11px] text-muted-foreground tabular-nums">
                  10:4{i}
                </span>
              </div>
              <p className="text-[13px] leading-relaxed text-ink-2">{text}</p>
            </div>
          </div>
        )
      })}
    </div>
  )
}

// The Home and board shapes (card #598): a card tile, a grid of them, a
// Needs you row, Home's progress rows, the ask box and a filter row, in
// the classes the real ones use (card-grid.tsx, needs-you.tsx,
// progress-rows.tsx, ask-box.tsx).

const tileTitles = [
  "Tune the boost pads on the harbour track",
  "Lobby sync for the boat race",
  "Pick the race map for the demo",
  "Old lap-time leaderboard idea",
  "Fix the boat clipping through the dock",
  "Release notes for the next version",
]

// CardTileShape is one card tile: its chips, title, progress bar and the
// line under it.
export function CardTileShape({ i = 0 }: { i?: number }) {
  return (
    <div className="flex min-w-0 flex-col gap-2.5 p-3.5 text-sm glass-panel">
      <span className="flex min-w-0 flex-wrap items-center gap-1.5">
        <span className="font-mono text-xs text-muted-foreground">
          mlg#{i + 1}
        </span>
        <span skullet-block="" className="block h-5 w-20 rounded-md" />
        <span skullet-block="" className="block h-5 w-10 rounded-md" />
      </span>
      <span className="text-sm leading-snug font-medium text-ink">
        {tileTitles[i % tileTitles.length]}
      </span>
      <span skullet-block="" className="block h-1 w-full bg-muted" />
      <span className="text-[11px] text-muted-foreground">
        50% · sample-heron working
      </span>
    </div>
  )
}

// TileGridShape is a grid of card tiles in the masonry's 260px columns.
export function TileGridShape({
  tiles = 4,
  className,
}: {
  tiles?: number
  className?: string
}) {
  return (
    <div
      className={cn(
        "grid [grid-template-columns:repeat(auto-fill,minmax(260px,1fr))] items-start gap-3",
        className
      )}
    >
      {Array.from({ length: tiles }, (_, i) => (
        <CardTileShape key={i} i={i} />
      ))}
    </div>
  )
}

// AskBoxShape is the ask box over a board: one line and its Send.
export function AskBoxShape() {
  return (
    <div className="flex items-center gap-2 p-1.5 glass-panel">
      <span className="flex-1 px-3 py-2 text-sm text-muted-foreground">
        Ask about anything…
      </span>
      <span skullet-block="" className="block h-8 w-14 rounded-md" />
    </div>
  )
}

// FilterRowShape is a board's filter row: its tabs, then search and the
// List/Canvas switch at the end.
export function FilterRowShape({ tabs = 3 }: { tabs?: number }) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <span className="flex items-center gap-1">
        {Array.from({ length: tabs }, (_, i) => (
          <span
            key={i}
            className={cn("block h-8 rounded-md", i === 1 ? "w-36" : "w-14")}
          />
        ))}
      </span>
      <span className="ml-auto flex items-center gap-2">
        <span skullet-block="" className="block h-8 w-56 rounded-md" />
        <span skullet-block="" className="block h-8 w-28 rounded-md" />
      </span>
    </div>
  )
}

// NeedRowShape is one Needs you row: kind, where, when, the title, a line
// and its buttons.
export function NeedRowShape({ i = 0 }: { i?: number }) {
  return (
    <article className="grid min-w-0 gap-2 px-4 py-3 glass-panel">
      <div className="flex min-w-0 items-center gap-x-2 text-[12.5px] text-muted-foreground">
        <span skullet-block="" className="block h-5 w-20 rounded-md" />
        <span>#1{i}</span>
        <span className="ml-auto">2m ago</span>
      </div>
      <h3 className="text-[15px] leading-snug font-semibold">
        {tileTitles[(i + 4) % tileTitles.length]}
      </h3>
      <p className="text-sm text-muted-foreground">
        Widened the collider so boats stop at the edge. Checked with the cargo
        test, then twenty dockings with no clipping.
      </p>
      <span className="flex gap-1.5">
        <span skullet-block="" className="block h-8 w-16 rounded-md" />
        <span skullet-block="" className="block h-8 w-24 rounded-md" />
      </span>
    </article>
  )
}

// NeedsYouShape is Home's Needs you: its heading and a few rows.
export function NeedsYouShape({ rows = 2 }: { rows?: number }) {
  return (
    <section className="grid min-w-0 gap-2.5">
      <h2 className="px-1 text-base font-semibold">Needs you · 3</h2>
      <div className="grid min-w-0 gap-2.5">
        {Array.from({ length: rows }, (_, i) => (
          <NeedRowShape key={i} i={i} />
        ))}
      </div>
    </section>
  )
}

// ProgressRowsShape is Home's Agent progress list: ref, title, the line
// that says why, the state and the bar.
export function ProgressRowsShape({ rows = 4 }: { rows?: number }) {
  return (
    <div className="grid min-w-0 gap-1.5">
      <h3 className="px-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        Working 4
      </h3>
      <ul className="grid min-w-0 overflow-hidden glass-panel sm:grid-cols-[max-content_minmax(0,1fr)_minmax(0,16rem)_5.5rem_4.5rem] sm:gap-x-3">
        {Array.from({ length: rows }, (_, i) => (
          <li
            key={i}
            className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 border-b border-border/60 px-4 py-2.5 last:border-b-0 sm:col-span-full sm:grid-cols-subgrid"
          >
            <span className="font-mono text-xs text-muted-foreground sm:min-w-[4.5rem]">
              #{20 + i}
            </span>
            <span className="min-w-0 truncate text-sm font-medium">
              {tileTitles[i % tileTitles.length]}
            </span>
            <span className="col-span-3 col-start-2 row-start-2 truncate text-[12.5px] text-muted-foreground sm:col-span-1 sm:col-start-3 sm:row-start-1">
              Running the checks
            </span>
            <span
              skullet-block=""
              className="col-start-3 row-start-1 block h-5 w-16 justify-self-end rounded-md sm:col-start-4"
            />
            <span
              skullet-block=""
              className="hidden h-1 bg-muted sm:col-start-5 sm:row-start-1 sm:block"
            />
          </li>
        ))}
      </ul>
    </div>
  )
}

// HomeShape is Home while its cards load: the ask box, Needs you, then
// Agent progress with its filter row.
export function HomeShape() {
  return (
    <div className="grid min-w-0 content-start gap-3.5">
      <AskBoxShape />
      <NeedsYouShape />
      <h2 className="px-1 pt-2 text-base font-semibold">Agent progress</h2>
      <FilterRowShape tabs={4} />
      <ProgressRowsShape />
    </div>
  )
}
