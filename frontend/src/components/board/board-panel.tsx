import { useMemo, useState } from "react"
import {
  ChevronRightIcon,
  ClipboardListIcon,
  LockIcon,
  PlusIcon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { Card as Panel } from "@/components/ui/card"
import { cn } from "@/lib/utils"
import {
  parseTime,
  plainLine,
  relativeTime,
  shortRepo,
  untilTime,
  type AgentCard,
} from "@/lib/irori"
import { nameLookup, type GoneSession } from "@/lib/talk"
import {
  TaskState,
  waitsOnPerson,
  type BoardState,
  type Card,
  type LeaseCard,
} from "@/lib/board"
import {
  blockedLabel,
  displayResource,
  fieldClass,
  LinkChip,
  Pill,
  repoLabel,
} from "./bits"
import { CardDetail, LeaseDetail } from "./card-detail"
import { RaiseForm } from "./raise-form"

type Filter = "all" | "you" | "blocked" | "shared"

const filters: Array<{ id: Filter; label: string }> = [
  { id: "all", label: "All" },
  { id: "you", label: "Needs you" },
  { id: "blocked", label: "Blocked" },
  { id: "shared", label: "Shared" },
]

type Lane = "need" | "blocked" | "working" | "open" | "done"

const leaseSel = (resource: string) => `lease:${resource}`

// BoardPanel is the Board tab: lanes of cards on the left, the selected
// card's facts, comments and actions on the right (the "Irori Board"
// prototype). A scratch pad for live work, not a tracker: Done is collapsed
// and nothing times a card out.
export function BoardPanel({
  board: state,
  agents,
  gone,
  now,
}: {
  board: BoardState
  agents: AgentCard[]
  gone: GoneSession[]
  now: Date
}) {
  const { view } = state
  const [filter, setFilter] = useState<Filter>("all")
  const [repo, setRepo] = useState("")
  const [picked, setPicked] = useState("")
  const [raising, setRaising] = useState(false)
  const [showDone, setShowDone] = useState(false)

  const name = useMemo(
    () =>
      nameLookup(
        agents.map((a): [string, string] => [a.key, a.name]),
        gone.map((g): [string, string] => [g.key, g.name])
      ),
    [agents, gone]
  )
  const all = useMemo(
    () => [
      ...view.needsYou,
      ...view.blocked,
      ...view.working,
      ...view.open,
      ...view.done,
    ],
    [view]
  )
  const title = useMemo(() => {
    const m = new Map(all.map((c) => [c.id, plainLine(c.title)]))
    return (id: string) => m.get(id) ?? ""
  }, [all])
  const repos = useMemo(() => {
    const ks = new Set<string>()
    for (const c of all) if (c.repoKey) ks.add(c.repoKey)
    for (const a of agents) if (a.repoKey) ks.add(a.repoKey)
    return [...ks].sort()
  }, [all, agents])

  const keep = (c: Card) => {
    if (repo && c.repoKey !== repo) return false
    if (filter === "blocked") return c.state === TaskState.Blocked
    if (filter === "shared") return Boolean(c.shared)
    return true
  }
  const lanes: Array<{ id: Lane; title: string; cards: Card[] }> = [
    { id: "need", title: "Needs you", cards: view.needsYou.filter(keep) },
    {
      id: "blocked",
      title: "Blocked",
      cards: filter === "you" ? [] : view.blocked.filter(keep),
    },
    {
      id: "working",
      title: "In progress",
      cards:
        filter === "you" || filter === "blocked"
          ? []
          : view.working.filter(keep),
    },
    {
      id: "open",
      title: "Open to pick up",
      cards:
        filter === "you" || filter === "blocked" ? [] : view.open.filter(keep),
    },
  ]
  const leases =
    filter === "all" || filter === "blocked"
      ? view.leases.filter(
          (l) =>
            !repo ||
            l.resource.includes(`:${repo}:`) ||
            !l.resource.startsWith("path:")
        )
      : []
  const done = filter === "you" ? [] : view.done.filter(keep)

  // The selection: what was picked if it is still on the Board, else the
  // first card that needs the person, else the first card.
  const visible = [...lanes.flatMap((l) => l.cards), ...(showDone ? done : [])]
  const pickedCard = all.find((c) => c.id === picked)
  const pickedLease = view.leases.find((l) => leaseSel(l.resource) === picked)
  const selected = pickedCard ?? (pickedLease ? null : (visible[0] ?? null))
  const selKey = pickedLease
    ? leaseSel(pickedLease.resource)
    : (selected?.id ?? "")
  const empty = all.length === 0 && view.leases.length === 0 && !state.loading

  return (
    <div className="flex flex-col gap-4">
      {state.error && (
        <p className="rounded-lg bg-destructive/10 px-4 py-2 text-sm text-destructive">
          Could not load the Board: {state.error}
        </p>
      )}

      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
        <Panel size="sm" className="gap-0 py-0!">
          <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2.5">
            <div
              role="group"
              aria-label="Filter cards"
              className="inline-flex rounded-lg bg-muted p-[3px]"
            >
              {filters.map((f) => (
                <button
                  key={f.id}
                  type="button"
                  aria-pressed={filter === f.id}
                  onClick={() => setFilter(f.id)}
                  className="flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[13px] font-medium text-muted-foreground transition-colors outline-none hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 aria-pressed:bg-background aria-pressed:text-ink aria-pressed:shadow-sm"
                >
                  {f.label}
                  {f.id === "you" && view.needsYou.length > 0 && (
                    <span className="rounded-full bg-warning-foreground px-1.5 text-[11px] text-warning tabular-nums">
                      {view.needsYou.length}
                    </span>
                  )}
                </button>
              ))}
            </div>
            {repos.length > 1 && (
              <select
                aria-label="Repo"
                value={repo}
                onChange={(e) => setRepo(e.target.value)}
                className={cn(fieldClass, "h-7 max-w-40 text-[13px]")}
              >
                <option value="">All repos</option>
                {repos.map((r) => (
                  <option key={r} value={r}>
                    {shortRepo(r)}
                  </option>
                ))}
              </select>
            )}
            <Button
              size="sm"
              className="ml-auto"
              aria-expanded={raising}
              onClick={() => setRaising(!raising)}
            >
              <PlusIcon aria-hidden />
              New card
            </Button>
          </div>

          {raising && (
            <RaiseForm
              agents={agents}
              repos={repos}
              defaultRepo={repo}
              onDone={(id) => {
                setRaising(false)
                if (id) setPicked(id)
              }}
            />
          )}

          {state.loading ? (
            <p className="px-4 py-4 text-sm text-muted-foreground">
              Reading the Board…
            </p>
          ) : empty ? (
            <EmptyBoard />
          ) : (
            <div className="flex flex-col pb-1">
              {lanes.map((lane) =>
                lane.cards.length === 0 &&
                !(lane.id === "blocked" && leases.length > 0) ? null : (
                  <section key={lane.id} aria-label={lane.title}>
                    <LaneHead
                      title={lane.title}
                      n={
                        lane.cards.length +
                        (lane.id === "blocked" ? leases.length : 0)
                      }
                      hot={lane.id === "need"}
                    />
                    <ul>
                      {lane.cards.map((c) => (
                        <CardRow
                          key={c.id}
                          c={c}
                          lane={lane.id}
                          selected={selKey === c.id}
                          onSelect={() => setPicked(c.id)}
                          name={name}
                          title={title}
                          now={now}
                        />
                      ))}
                      {lane.id === "blocked" &&
                        leases.map((l) => (
                          <LeaseRow
                            key={l.resource}
                            l={l}
                            selected={selKey === leaseSel(l.resource)}
                            onSelect={() => setPicked(leaseSel(l.resource))}
                            name={name}
                            now={now}
                          />
                        ))}
                    </ul>
                  </section>
                )
              )}
              {lanes.every((l) => l.cards.length === 0) &&
                leases.length === 0 && (
                  <p className="px-4 py-3 text-[13px] text-muted-foreground">
                    Nothing here with these filters.
                  </p>
                )}
              {view.doneTotal > 0 && filter !== "you" && (
                <section aria-label="Done">
                  <button
                    type="button"
                    aria-expanded={showDone}
                    onClick={() => setShowDone(!showDone)}
                    className="group flex w-full items-center gap-1.5 px-4 pt-3 pb-1.5 text-[11.5px] font-semibold tracking-wide text-muted-foreground uppercase outline-none hover:text-ink focus-visible:text-ink"
                  >
                    <ChevronRightIcon
                      className="size-3.5 transition-transform group-aria-expanded:rotate-90"
                      aria-hidden
                    />
                    Done
                    <span className="tabular-nums">{view.doneTotal}</span>
                  </button>
                  {showDone && (
                    <ul>
                      {done.map((c) => (
                        <CardRow
                          key={c.id}
                          c={c}
                          lane="done"
                          selected={selKey === c.id}
                          onSelect={() => setPicked(c.id)}
                          name={name}
                          title={title}
                          now={now}
                        />
                      ))}
                      {view.doneTotal > view.done.length && (
                        <li className="px-4 py-2 text-xs text-muted-foreground">
                          The newest {view.done.length} of {view.doneTotal}.
                          Agents find older ones with search.
                        </li>
                      )}
                    </ul>
                  )}
                </section>
              )}
            </div>
          )}
        </Panel>

        {pickedLease ? (
          <LeaseDetail
            l={pickedLease}
            name={name}
            title={title}
            now={now}
            onBlocked={(id) => setPicked(id)}
          />
        ) : selected ? (
          <CardDetail
            key={selected.id}
            card={selected}
            agents={agents}
            name={name}
            title={title}
            now={now}
          />
        ) : (
          !state.loading &&
          !empty && (
            <Panel size="sm" className="px-4 text-[13px] text-muted-foreground">
              Pick a card to see its comments.
            </Panel>
          )
        )}
      </div>
    </div>
  )
}

function LaneHead({
  title,
  n,
  hot,
}: {
  title: string
  n: number
  hot?: boolean
}) {
  return (
    <h3
      className={cn(
        "flex gap-1.5 px-4 pt-3 pb-1.5 text-[11.5px] font-semibold tracking-wide uppercase",
        hot ? "text-warning" : "text-muted-foreground"
      )}
    >
      {title}
      <span className="tabular-nums">{n}</span>
    </h3>
  )
}

function stripe(c: Card, lane: Lane): string {
  if (lane === "done") return "bg-success"
  if (c.state === TaskState.Blocked)
    return waitsOnPerson(c) ? "bg-destructive" : "bg-warning"
  if (lane === "need") return "bg-warning"
  return "bg-blue"
}

function CardRow({
  c,
  lane,
  selected,
  onSelect,
  name,
  title,
  now,
}: {
  c: Card
  lane: Lane
  selected: boolean
  onSelect: () => void
  name: (key: string) => string
  title: (id: string) => string
  now: Date
}) {
  const updated = parseTime(c.updatedAt)
  const assigned = new Set(c.assigned ?? [])
  const working = (c.holders ?? []).filter((h) => !assigned.has(h))
  const n = c.comments ?? 0
  const repo = repoLabel(c)
  const links = c.links ?? []
  return (
    <li className="border-t border-border first:border-t-0">
      <button
        type="button"
        aria-current={selected ? "true" : undefined}
        onClick={onSelect}
        className={cn(
          "grid w-full grid-cols-[4px_minmax(0,1fr)] gap-2.5 px-4 py-2 text-left outline-none focus-visible:bg-muted/60",
          selected ? "bg-info-foreground/70" : "hover:bg-muted/50"
        )}
      >
        <span className={cn("rounded", stripe(c, lane))} aria-hidden />
        <span className="flex min-w-0 flex-col gap-1">
          <span className="font-medium [overflow-wrap:anywhere] text-ink">
            {plainLine(c.title) || c.id}
          </span>
          {c.state === TaskState.Blocked && c.reason && (
            <span className="line-clamp-2 text-xs text-ink-2">
              {plainLine(c.reason)}
            </span>
          )}
          <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
            {c.state === TaskState.Blocked &&
              (c.resolved ? (
                <Pill tone="done">Blocker cleared</Pill>
              ) : (
                <Pill tone={waitsOnPerson(c) ? "you" : "block"}>
                  Waiting on {blockedLabel(c.blockedBy, name, title)}
                </Pill>
              ))}
            {c.state === TaskState.InputRequired && (
              <Pill tone="you">Needs your input</Pill>
            )}
            {lane !== "done" &&
              working.map((h) => (
                <Pill key={h} tone="task" title={h}>
                  {plainLine(name(h))}
                </Pill>
              ))}
            {lane !== "done" &&
              [...assigned].map((h) => (
                <Pill
                  key={h}
                  tone="plain"
                  title={`${h}: assigned, not picked up yet`}
                >
                  {plainLine(name(h))} · assigned
                </Pill>
              ))}
            {c.shared && <Pill tone="shared">Shared</Pill>}
            {lane === "done" && (
              <Pill tone={c.state === TaskState.Completed ? "done" : "plain"}>
                {c.state === TaskState.Completed ? "Done" : plainLine(c.state)}
              </Pill>
            )}
            {repo && <span title={c.repoKey}>{repo}</span>}
            {repo && <span aria-hidden>·</span>}
            <span>
              {n} {n === 1 ? "comment" : "comments"}
            </span>
            <span aria-hidden>·</span>
            <time
              dateTime={updated?.toISOString()}
              title={updated?.toLocaleString()}
            >
              {relativeTime(updated, now)}
            </time>
          </span>
          {links.length > 0 && (
            <span className="flex flex-wrap gap-1">
              {links.slice(0, 3).map((l) => (
                <LinkChip key={l.ref} link={l} />
              ))}
              {links.length > 3 && (
                <span className="text-xs text-muted-foreground">
                  +{links.length - 3}
                </span>
              )}
            </span>
          )}
        </span>
      </button>
    </li>
  )
}

function LeaseRow({
  l,
  selected,
  onSelect,
  name,
  now,
}: {
  l: LeaseCard
  selected: boolean
  onSelect: () => void
  name: (key: string) => string
  now: Date
}) {
  const until = parseTime(l.until)
  const blocks = l.blocks?.length ?? 0
  const refused = l.refused?.length ?? 0
  return (
    <li className="border-t border-border first:border-t-0">
      <button
        type="button"
        aria-current={selected ? "true" : undefined}
        onClick={onSelect}
        className={cn(
          "grid w-full grid-cols-[4px_minmax(0,1fr)] gap-2.5 px-4 py-2 text-left outline-none focus-visible:bg-muted/60",
          selected ? "bg-info-foreground/70" : "hover:bg-muted/50"
        )}
      >
        <span className="rounded bg-warning" aria-hidden />
        <span className="flex min-w-0 flex-col gap-1">
          <span className="flex min-w-0 items-center gap-1.5">
            <LockIcon className="size-3.5 shrink-0 text-warning" aria-hidden />
            <span
              className="truncate font-mono text-[12.5px] text-ink"
              title={plainLine(l.resource)}
            >
              {displayResource(plainLine(l.resource))}
            </span>
          </span>
          <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
            <Pill tone="block">Lease</Pill>
            {blocks > 0 && (
              <Pill tone="block">
                blocks {blocks} {blocks === 1 ? "task" : "tasks"}
              </Pill>
            )}
            {refused > 0 && (
              <Pill tone="plain">
                refused {refused} {refused === 1 ? "agent" : "agents"}
              </Pill>
            )}
            <span>held by {plainLine(name(l.holder))}</span>
            <span aria-hidden>·</span>
            <span>expires {untilTime(until, now)}</span>
          </span>
        </span>
      </button>
    </li>
  )
}

function EmptyBoard() {
  return (
    <div className="flex flex-col items-center gap-2 px-6 py-10 text-center">
      <ClipboardListIcon className="size-7 text-muted-foreground" aria-hidden />
      <h2 className="text-base font-medium text-ink">Nothing on the Board</h2>
      <p className="max-w-sm text-[13px] text-muted-foreground">
        Agents post what they're working on, what blocks them and hand-offs
        here. Use New card to post one yourself. Plans and specs belong in the
        project's own tracker; a card can link to them.
      </p>
    </div>
  )
}
