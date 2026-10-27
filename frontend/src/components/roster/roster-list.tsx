import { FolderIcon, FlameIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Card } from "@/components/ui/card"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  parseTime,
  plainLine,
  relativeTime,
  shortPath,
  type AgentCard,
} from "@/lib/irori"

type Group = { cwd: string; agents: AgentCard[]; newest: number }

function activityOf(a: AgentCard): number {
  return (parseTime(a.lastActivity) ?? parseTime(a.updatedAt))?.getTime() ?? 0
}

// groupByCwd buckets agents by working directory; groups and the agents in
// them are ordered newest activity first.
export function groupByCwd(agents: AgentCard[]): Group[] {
  const byCwd = new Map<string, Group>()
  for (const a of agents) {
    const g = byCwd.get(a.cwd) ?? { cwd: a.cwd, agents: [], newest: 0 }
    g.agents.push(a)
    g.newest = Math.max(g.newest, activityOf(a))
    byCwd.set(a.cwd, g)
  }
  const groups = [...byCwd.values()]
  for (const g of groups) g.agents.sort((x, y) => activityOf(y) - activityOf(x))
  return groups.sort((x, y) => y.newest - x.newest)
}

// RosterList shows every live agent, grouped by working directory.
export function RosterList({
  agents,
  now,
}: {
  agents: AgentCard[]
  now: Date
}) {
  if (agents.length === 0) return <EmptyState />
  return (
    <div className="flex flex-col gap-6">
      {groupByCwd(agents).map((g) => (
        <section key={g.cwd} aria-label={g.cwd || "no directory"}>
          <h2 className="mb-2 flex items-center gap-1.5 px-1 text-xs font-medium text-muted-foreground">
            <FolderIcon className="size-3.5" aria-hidden />
            <CwdLabel cwd={g.cwd} />
            <span className="tabular-nums">· {g.agents.length}</span>
          </h2>
          <Card size="sm" className="gap-0 py-0!">
            <ul className="divide-y divide-border">
              {g.agents.map((a) => (
                <AgentRow key={a.key} agent={a} now={now} />
              ))}
            </ul>
          </Card>
        </section>
      ))}
    </div>
  )
}

function CwdLabel({ cwd }: { cwd: string }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span className="truncate font-mono text-ink-2" tabIndex={0} />}
      >
        {shortPath(cwd)}
      </TooltipTrigger>
      <TooltipContent className="max-w-md font-mono break-all">
        {cwd || "(no directory)"}
      </TooltipContent>
    </Tooltip>
  )
}

function AgentRow({ agent, now }: { agent: AgentCard; now: Date }) {
  const name = plainLine(agent.name) || agent.sessionId.slice(0, 8)
  const title = plainLine(agent.title)
  const text = plainLine(agent.lastText)
  const last = parseTime(agent.lastActivity) ?? parseTime(agent.updatedAt)

  return (
    <li className="flex flex-col gap-1.5 px-4 py-3">
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate font-medium text-ink">{name}</span>
        {title && (
          <span className="truncate text-muted-foreground" title={title}>
            {title}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-2">
          <time
            className="text-xs text-muted-foreground tabular-nums"
            dateTime={last?.toISOString()}
            title={last?.toLocaleString()}
          >
            {relativeTime(last, now)}
          </time>
          <StatusBadge status={agent.status} />
        </span>
      </div>
      <p
        className={
          text
            ? "truncate text-[13px] text-ink-2"
            : "truncate text-[13px] text-muted-foreground italic"
        }
        title={text || undefined}
      >
        {text || "No reply yet"}
      </p>
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <span>{kindLabel(agent.sessionKind)}</span>
        {agent.entrypoint && <span>· {agent.entrypoint}</span>}
        <span className="ml-auto font-mono">pid {agent.pid}</span>
      </div>
    </li>
  )
}

function kindLabel(kind: string): string {
  if (kind === "bg") return "background"
  return kind || "session"
}

// StatusBadge spends the accent on busy, keeps idle quiet and shows any
// other status Claude Code reports as a signal with its own word.
export function StatusBadge({ status }: { status: string }) {
  if (status === "busy") {
    return (
      <Badge variant="info">
        <span
          className="size-1.5 animate-pulse rounded-full bg-current"
          aria-hidden
        />
        busy
      </Badge>
    )
  }
  if (status === "idle") return <Badge variant="slate">idle</Badge>
  return <Badge variant="warning">{plainLine(status) || "unknown"}</Badge>
}

// EmptyState explains where the roster comes from when nothing is running.
function EmptyState() {
  return (
    <Card className="items-center px-6 py-12! text-center">
      <FlameIcon className="size-8 text-muted-foreground" aria-hidden />
      <div className="flex max-w-md flex-col gap-2">
        <h2 className="text-base font-medium text-ink">
          No agents by the fire
        </h2>
        <p className="text-muted-foreground">
          Irori sees the Claude Code sessions running on this device by reading{" "}
          <code className="rounded bg-muted px-1 font-mono text-xs text-ink-2">
            ~/.claude/sessions
          </code>
          . Start <code className="font-mono text-xs">claude</code> in any
          project and it shows up here.
        </p>
      </div>
    </Card>
  )
}
