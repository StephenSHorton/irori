import { useMemo, useState } from "react"
import { ClipboardListIcon, LockIcon } from "lucide-react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Badge } from "@/components/ui/badge"
import { Card } from "@/components/ui/card"
import {
  parseTime,
  plainLine,
  relativeTime,
  shortRepo,
  untilTime,
  type AgentCard,
} from "@/lib/irori"
import {
  nameLookup,
  talk,
  taskOpen,
  TaskState,
  type GoneSession,
  type Lease,
  type Task,
  type WorkState,
} from "@/lib/talk"

// WorkPanel is tasks and leases, with cancel and release for the person.
export function WorkPanel({
  work,
  agents,
  gone,
  now,
}: {
  work: WorkState
  agents: AgentCard[]
  gone: GoneSession[]
  now: Date
}) {
  const [showDone, setShowDone] = useState(false)
  const name = useMemo(
    () =>
      nameLookup(
        agents.map((a): [string, string] => [a.key, a.name]),
        gone.map((g): [string, string] => [g.key, g.name])
      ),
    [agents, gone]
  )
  const tasks = work.tasks.filter((t) => showDone || taskOpen(t.state))
  const done =
    work.tasks.length - work.tasks.filter((t) => taskOpen(t.state)).length

  return (
    <div className="flex flex-col gap-6">
      {work.error && (
        <p className="rounded-lg bg-destructive/10 px-4 py-2 text-sm text-destructive">
          Could not load tasks and leases: {work.error}
        </p>
      )}

      <section aria-labelledby="tasks-h" className="flex flex-col gap-2">
        <div className="flex items-center gap-2 px-1">
          <h2 id="tasks-h" className="text-sm font-medium text-ink">
            Tasks
          </h2>
          <span className="text-xs text-muted-foreground tabular-nums">
            {tasks.length}
          </span>
          {done > 0 && (
            <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
              <input
                type="checkbox"
                checked={showDone}
                onChange={(e) => setShowDone(e.target.checked)}
                className="accent-primary"
              />
              Show finished ({done})
            </label>
          )}
        </div>
        {work.loading ? (
          <p className="px-1 text-sm text-muted-foreground">Reading tasks…</p>
        ) : tasks.length === 0 ? (
          <Empty
            icon={<ClipboardListIcon className="size-6" aria-hidden />}
            title="No open tasks"
            text="Agents create and claim tasks with the task tools; they show up here."
          />
        ) : (
          <Card size="sm" className="gap-0 overflow-x-auto py-0!">
            <table className="w-full text-left text-sm">
              <thead className="text-xs text-muted-foreground">
                <tr className="border-b">
                  <th className="px-4 py-2 font-medium">Title</th>
                  <th className="px-2 py-2 font-medium">State</th>
                  <th className="px-2 py-2 font-medium">Holder</th>
                  <th className="px-2 py-2 font-medium">Repo</th>
                  <th className="px-2 py-2 font-medium">Updated</th>
                  <th className="px-4 py-2" aria-label="Actions" />
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {tasks.map((t) => (
                  <TaskRow key={t.id} t={t} name={name} now={now} />
                ))}
              </tbody>
            </table>
          </Card>
        )}
      </section>

      <section aria-labelledby="leases-h" className="flex flex-col gap-2">
        <div className="flex items-center gap-2 px-1">
          <h2 id="leases-h" className="text-sm font-medium text-ink">
            Leases
          </h2>
          <span className="text-xs text-muted-foreground tabular-nums">
            {work.leases.length}
          </span>
        </div>
        {work.loading ? null : work.leases.length === 0 ? (
          <Empty
            icon={<LockIcon className="size-6" aria-hidden />}
            title="Nothing leased"
            text="Leases are advisory claims on files or named resources such as a port; they expire on their own."
          />
        ) : (
          <Card size="sm" className="gap-0 overflow-x-auto py-0!">
            <table className="w-full text-left text-sm">
              <thead className="text-xs text-muted-foreground">
                <tr className="border-b">
                  <th className="px-4 py-2 font-medium">Resource</th>
                  <th className="px-2 py-2 font-medium">Holder</th>
                  <th className="px-2 py-2 font-medium">Expires</th>
                  <th className="px-4 py-2" aria-label="Actions" />
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {work.leases.map((l) => (
                  <LeaseRow key={l.resource} l={l} name={name} now={now} />
                ))}
              </tbody>
            </table>
          </Card>
        )}
      </section>
    </div>
  )
}

function TaskRow({
  t,
  name,
  now,
}: {
  t: Task
  name: (key: string) => string
  now: Date
}) {
  const updated = parseTime(t.updatedAt)
  const title = plainLine(t.title) || t.id
  return (
    <tr className="align-middle">
      <td className="max-w-80 px-4 py-2">
        <span
          className="block truncate font-medium text-ink"
          title={plainLine(t.objective) || title}
        >
          {title}
        </span>
        {t.note && (
          <span className="block truncate text-xs text-muted-foreground">
            {plainLine(t.note)}
          </span>
        )}
      </td>
      <td className="px-2 py-2">
        <TaskStateBadge state={t.state} />
      </td>
      <td className="max-w-40 truncate px-2 py-2 text-ink-2" title={t.holder}>
        {t.holder ? (
          plainLine(name(t.holder))
        ) : (
          <span className="text-muted-foreground">unclaimed</span>
        )}
      </td>
      <td
        className="max-w-48 truncate px-2 py-2 font-mono text-xs text-ink-2"
        title={t.repoKey}
      >
        {t.repoKey ? (
          shortRepo(t.repoKey)
        ) : (
          <span className="font-sans text-muted-foreground">—</span>
        )}
      </td>
      <td className="px-2 py-2 text-xs whitespace-nowrap text-muted-foreground tabular-nums">
        <time
          dateTime={updated?.toISOString()}
          title={updated?.toLocaleString()}
        >
          {relativeTime(updated, now)}
        </time>
      </td>
      <td className="px-4 py-2 text-right">
        {taskOpen(t.state) && (
          <ConfirmButton
            label="Cancel"
            question="Cancel this task?"
            confirmLabel="Cancel task"
            variant="destructive"
            onConfirm={() => talk.cancelTask(t.id)}
          />
        )}
      </td>
    </tr>
  )
}

function LeaseRow({
  l,
  name,
  now,
}: {
  l: Lease
  name: (key: string) => string
  now: Date
}) {
  const until = parseTime(l.until)
  const resource = plainLine(l.resource)
  return (
    <tr className="align-middle">
      <td className="max-w-96 px-4 py-2">
        <span
          className="block truncate font-mono text-xs text-ink"
          title={resource}
        >
          {displayResource(resource)}
        </span>
        {l.stolenFrom && (
          <span className="block truncate text-xs text-muted-foreground">
            taken over from {plainLine(name(l.stolenFrom))}
          </span>
        )}
      </td>
      <td className="max-w-40 truncate px-2 py-2 text-ink-2" title={l.holder}>
        {plainLine(name(l.holder))}
      </td>
      <td className="px-2 py-2 text-xs whitespace-nowrap text-muted-foreground tabular-nums">
        <time dateTime={until?.toISOString()} title={until?.toLocaleString()}>
          {untilTime(until, now)}
        </time>
      </td>
      <td className="px-4 py-2 text-right">
        <ConfirmButton
          label="Release"
          question="Release this lease?"
          confirmLabel="Release"
          variant="destructive"
          onConfirm={() => talk.releaseLease(l.resource)}
        />
      </td>
    </tr>
  )
}

// displayResource shortens a path lease's repo key:
// path:github.com/o/r:src/** -> o/r: src/**.
function displayResource(r: string): string {
  if (!r.startsWith("path:")) return r
  const rest = r.slice(5)
  const i = rest.lastIndexOf(":")
  if (i <= 0) return r
  return `${shortRepo(rest.slice(0, i))}: ${rest.slice(i + 1)}`
}

function TaskStateBadge({ state }: { state: TaskState }) {
  switch (state) {
    case TaskState.Working:
      return <Badge variant="info">working</Badge>
    case TaskState.Submitted:
      return <Badge variant="slate">submitted</Badge>
    case TaskState.InputRequired:
      return <Badge variant="warning">needs input</Badge>
    case TaskState.Completed:
      return <Badge variant="success">completed</Badge>
  }
  return (
    <Badge variant="outline" className="text-muted-foreground">
      {plainLine(state) || "unknown"}
    </Badge>
  )
}

function Empty({
  icon,
  title,
  text,
}: {
  icon: React.ReactNode
  title: string
  text: string
}) {
  return (
    <Card
      size="sm"
      className="flex-row items-center gap-3 px-4 text-muted-foreground"
    >
      {icon}
      <div className="flex flex-col">
        <span className="font-medium text-ink">{title}</span>
        <span className="text-[13px]">{text}</span>
      </div>
    </Card>
  )
}
