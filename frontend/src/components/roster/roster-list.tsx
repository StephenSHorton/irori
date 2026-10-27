import { useState } from "react"
import { FolderGitIcon, FolderIcon, FlameIcon, HistoryIcon } from "lucide-react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Badge } from "@/components/ui/badge"
import { Card } from "@/components/ui/card"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  shortPath,
  shortRepo,
  type AgentCard,
} from "@/lib/irori"
import { talk, type AgentCounts, type GoneSession } from "@/lib/talk"

type Group = {
  id: string // repoKey, else cwd
  repoKey: string
  cwds: string[]
  agents: AgentCard[]
  newest: number
}

function activityOf(a: AgentCard): number {
  return (parseTime(a.lastActivity) ?? parseTime(a.updatedAt))?.getTime() ?? 0
}

// groupAgents buckets agents by repoKey, falling back to the working
// directory outside git; groups and the agents in them are ordered newest
// activity first.
export function groupAgents(agents: AgentCard[]): Group[] {
  const byId = new Map<string, Group>()
  for (const a of agents) {
    const id = a.repoKey ? `repo:${a.repoKey}` : `cwd:${a.cwd}`
    const g = byId.get(id) ?? {
      id,
      repoKey: a.repoKey,
      cwds: [],
      agents: [],
      newest: 0,
    }
    g.agents.push(a)
    if (!g.cwds.includes(a.cwd)) g.cwds.push(a.cwd)
    g.newest = Math.max(g.newest, activityOf(a))
    byId.set(id, g)
  }
  const groups = [...byId.values()]
  for (const g of groups) g.agents.sort((x, y) => activityOf(y) - activityOf(x))
  return groups.sort((x, y) => y.newest - x.newest)
}

// RosterList shows every live agent grouped by repo, then the sessions that
// left recently with a Revive button.
export function RosterList({
  agents,
  counts,
  gone,
  goneError,
  now,
}: {
  agents: AgentCard[]
  counts: Map<string, AgentCounts>
  gone: GoneSession[]
  goneError: string | null
  now: Date
}) {
  return (
    <div className="flex flex-col gap-6">
      {agents.length === 0 ? (
        <EmptyState />
      ) : (
        groupAgents(agents).map((g) => (
          <section
            key={g.id}
            aria-label={g.repoKey || g.cwds[0] || "no directory"}
          >
            <h2 className="mb-2 flex items-center gap-1.5 px-1 text-xs font-medium text-muted-foreground">
              {g.repoKey ? (
                <FolderGitIcon className="size-3.5" aria-hidden />
              ) : (
                <FolderIcon className="size-3.5" aria-hidden />
              )}
              <GroupLabel g={g} />
              <span className="tabular-nums">· {g.agents.length}</span>
            </h2>
            <Card size="sm" className="gap-0 py-0!">
              <ul className="divide-y divide-border">
                {g.agents.map((a) => (
                  <AgentRow
                    key={a.key}
                    agent={a}
                    counts={counts.get(a.key)}
                    showCwd={g.cwds.length > 1}
                    now={now}
                  />
                ))}
              </ul>
            </Card>
          </section>
        ))
      )}

      {(gone.length > 0 || goneError) && (
        <GoneList gone={gone} error={goneError} now={now} />
      )}
    </div>
  )
}

function GroupLabel({ g }: { g: Group }) {
  const label = g.repoKey ? shortRepo(g.repoKey) : shortPath(g.cwds[0] ?? "")
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span className="truncate font-mono text-ink-2" tabIndex={0} />}
      >
        {label}
      </TooltipTrigger>
      <TooltipContent className="max-w-md font-mono break-all">
        {g.repoKey && <div>{g.repoKey}</div>}
        {g.cwds.map((c) => (
          <div key={c}>{c || "(no directory)"}</div>
        ))}
      </TooltipContent>
    </Tooltip>
  )
}

function AgentRow({
  agent,
  counts,
  showCwd,
  now,
}: {
  agent: AgentCard
  counts?: AgentCounts
  showCwd: boolean
  now: Date
}) {
  const name = plainLine(agent.name) || agent.sessionId.slice(0, 8)
  const rawTitle = plainLine(agent.title)
  // Claude Code often names a session after its title; show it once.
  const title = rawTitle.toLowerCase() === name.toLowerCase() ? "" : rawTitle
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
          {counts && counts.unread > 0 && (
            <Badge variant="info" title="Messages waiting for this agent">
              {counts.unread} unread
            </Badge>
          )}
          {counts && counts.held > 0 && (
            <Badge variant="warning" title="Messages held for you in the Inbox">
              {counts.held} held
            </Badge>
          )}
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
      <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
        <span>{kindLabel(agent.sessionKind)}</span>
        {agent.entrypoint && <span>· {agent.entrypoint}</span>}
        {agent.permissionMode === "bypassPermissions" && (
          <Badge
            variant="slate"
            className="h-4.5 px-1.5 text-[11px]"
            title="Runs with bypassed permissions: messages from agents that don't are held"
          >
            bypass permissions
          </Badge>
        )}
        {showCwd && (
          <span className="truncate font-mono" title={agent.cwd}>
            · {shortPath(agent.cwd)}
          </span>
        )}
        <span className="ml-auto shrink-0 font-mono">pid {agent.pid}</span>
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

// GoneList is the sessions that left the roster recently. Revive runs
// claude --bg --resume in the session's folder after a confirm.
function GoneList({
  gone,
  error,
  now,
}: {
  gone: GoneSession[]
  error: string | null
  now: Date
}) {
  return (
    <section aria-labelledby="gone-h">
      <h2
        id="gone-h"
        className="mb-2 flex items-center gap-1.5 px-1 text-xs font-medium text-muted-foreground"
      >
        <HistoryIcon className="size-3.5" aria-hidden />
        Recently gone
        <span className="tabular-nums">· {gone.length}</span>
      </h2>
      {error && (
        <p className="mb-2 px-1 text-xs text-destructive">
          Could not read recent sessions: {error}
        </p>
      )}
      <Card size="sm" className="gap-0 py-0!">
        <ul className="divide-y divide-border">
          {gone.map((g) => (
            <GoneRow key={g.key} g={g} now={now} />
          ))}
        </ul>
      </Card>
    </section>
  )
}

function GoneRow({ g, now }: { g: GoneSession; now: Date }) {
  const [output, setOutput] = useState<string | null>(null)
  const name = plainLine(g.name) || g.sessionId.slice(0, 8)
  const at = parseTime(g.goneAt)
  const where = shortPath(g.cwd)

  return (
    <li className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate font-medium text-ink-2">{name}</span>
        <span
          className="truncate font-mono text-xs text-muted-foreground"
          title={g.cwd}
        >
          {where}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          <time
            className="text-xs text-muted-foreground tabular-nums"
            dateTime={at?.toISOString()}
            title={at?.toLocaleString()}
          >
            left {relativeTime(at, now)}
          </time>
          {output !== null ? (
            // Revived: no second button, or a quick second click could start
            // the same session again before the row drops off the list.
            <span className="text-xs text-muted-foreground">revived</span>
          ) : g.transcript ? (
            <ConfirmButton
              label="Revive"
              question={
                <>
                  Run <code className="font-mono">claude --bg --resume</code> in{" "}
                  <span className="font-mono">{where}</span>?
                  {!g.trusted && (
                    <span className="block text-warning">
                      Claude Code may refuse: this folder isn't marked trusted.
                    </span>
                  )}
                </>
              }
              confirmLabel="Revive"
              onConfirm={async () => setOutput(await talk.revive(g.sessionId))}
            />
          ) : (
            <span className="text-xs text-muted-foreground">no transcript</span>
          )}
        </span>
      </div>
      {output !== null && (
        <div className="rounded-lg bg-muted/60 px-3 py-2 font-mono text-xs whitespace-pre-wrap text-ink-2">
          {plainBlock(output) || "Started."}
        </div>
      )}
    </li>
  )
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
