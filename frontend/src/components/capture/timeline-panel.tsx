import { useEffect, useMemo, useState } from "react"
import {
  BookmarkPlusIcon,
  FileDiffIcon,
  GitCommitHorizontalIcon,
  GitPullRequestIcon,
  HistoryIcon,
  MessageSquareIcon,
  ScrollTextIcon,
} from "lucide-react"

import { DiffView, Empty, ErrorLine, LinkChip } from "@/components/capture/bits"
import { DecisionForm } from "@/components/capture/decisions-panel"
import { ChoiceSelect } from "@/components/common/choice-select"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import {
  entryId,
  EntryKind,
  linkCounts,
  LinkClasses,
  repoRel,
  shortSha,
  useTimeline,
  type CaptureRepo,
  type CommitView,
  type EditRow,
  type Entry,
  type PREntry,
  type ToolCall,
  type TurnView,
} from "@/lib/capture"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  shortPath,
  shortRepo,
  type AgentCard,
} from "@/lib/irori"
import { nameLookup, type Delivery, type GoneSession } from "@/lib/talk"

// ToolsShown is how many tool chips a turn shows before "+N more".
const ToolsShown = 12

// EditsShown is how many edited files a turn lists before "+N more"; exact
// edits come first, since heuristic ones are Bash guesses.
const EditsShown = 8

type Item =
  | { kind: "entry"; at: number; entry: Entry; roles?: string[] }
  | { kind: "message"; at: number; d: Delivery }

const ms = (v: unknown) => parseTime(v)?.getTime() ?? 0

// defaultAgent is the most recently active agent, live or gone.
export function defaultAgent(agents: AgentCard[], gone: GoneSession[]): string {
  let best = ""
  let bestAt = -1
  for (const a of agents) {
    const t = ms(a.lastActivity) || ms(a.updatedAt)
    if (t > bestAt) [best, bestAt] = [a.key, t]
  }
  for (const g of gone) {
    const t = ms(g.goneAt)
    if (t > bestAt) [best, bestAt] = [g.key, t]
  }
  return best
}

// TimelinePanel is one agent's history: turns, summaries, commits and PRs
// from capture, with that agent's Irori messages merged in from the inbox.
export function TimelinePanel({
  agentKey,
  onAgent,
  agents,
  gone,
  deliveries,
  repos,
  now,
  onCommit,
}: {
  agentKey: string
  onAgent: (key: string) => void
  agents: AgentCard[]
  gone: GoneSession[]
  deliveries: Delivery[]
  repos: CaptureRepo[]
  now: Date
  onCommit: (c: CommitView) => void
}) {
  const [subagents, setSubagents] = useState(false)
  const fallback = defaultAgent(agents, gone)
  const key = agentKey || fallback
  // Keep the default once known: busy agents trade "most recent" every few
  // seconds, and the page shouldn't switch agents under the reader.
  useEffect(() => {
    if (!agentKey && fallback) onAgent(fallback)
  }, [agentKey, fallback, onAgent])
  const tl = useTimeline(key, subagents)

  const name = useMemo(
    () =>
      nameLookup(
        agents.map((a): [string, string] => [a.key, a.name]),
        gone.map((g): [string, string] => [g.key, g.name]),
        deliveries.map((d): [string, string] => [
          d.message.from.key,
          d.message.from.name ?? "",
        ])
      ),
    [agents, gone, deliveries]
  )

  const items = useMemo(() => {
    // One row per PR: the ledger has an entry per role (author, touched,
    // reviewer); the row keeps the newest and lists every role.
    const prRoles = new Map<string, string[]>()
    const out: Item[] = []
    for (const entry of tl.entries) {
      if (entry.pr) {
        const roles = prRoles.get(entry.pr.key)
        if (roles) {
          if (!roles.includes(entry.pr.role)) roles.push(entry.pr.role)
          continue
        }
        const fresh = [entry.pr.role as string]
        prRoles.set(entry.pr.key, fresh)
        out.push({ kind: "entry", at: ms(entry.at), entry, roles: fresh })
        continue
      }
      out.push({ kind: "entry", at: ms(entry.at), entry })
    }
    // Messages older than the loaded page wait for "Show older", so they
    // don't pile up at the bottom ahead of turns not loaded yet.
    const floor = tl.more ? (out.at(-1)?.at ?? 0) : -Infinity
    const seen = new Set<string>()
    for (const d of deliveries) {
      if (d.to !== key && d.message.from.key !== key) continue
      if (seen.has(d.message.id)) continue
      seen.add(d.message.id)
      const at = ms(d.message.sentAt) || ms(d.createdAt)
      if (at < floor) continue
      out.push({ kind: "message", at, d })
    }
    // Stable: the open turn (first from Go) stays on top among equals.
    return out
      .map((it, i) => ({ it, i }))
      .sort((a, b) => {
        const open = (x: Item) =>
          x.kind === "entry" && x.entry.turn?.open ? 1 : 0
        return open(b.it) - open(a.it) || b.it.at - a.it.at || a.i - b.i
      })
      .map((x) => x.it)
  }, [tl.entries, tl.more, deliveries, key])

  const live = new Set(agents.map((a) => a.key))
  const goneOnly = gone.filter((g) => !live.has(g.key))
  const known = live.has(key) || goneOnly.some((g) => g.key === key)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 px-1">
        <ChoiceSelect
          id="timeline-agent"
          label="Agent"
          showLabel
          value={key}
          onValueChange={onAgent}
          choices={[
            ...(!key ? [{ value: "", label: "No agents yet" }] : []),
            ...(key && !known ? [{ value: key, label: name(key) }] : []),
            ...(agents.length > 0
              ? [
                  {
                    group: "Live",
                    choices: agents.map((a) => ({
                      value: a.key,
                      label: `${plainLine(a.name) || a.sessionId.slice(0, 8)} · ${shortPath(a.cwd)}`,
                    })),
                  },
                ]
              : []),
            ...(goneOnly.length > 0
              ? [
                  {
                    group: "Gone",
                    choices: goneOnly.map((g) => ({
                      value: g.key,
                      label: `${plainLine(g.name) || g.sessionId.slice(0, 8)} · ${shortPath(g.cwd)}`,
                    })),
                  },
                ]
              : []),
          ]}
          className="flex-1 sm:max-w-md"
        />
        <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={subagents}
            onChange={(e) => setSubagents(e.target.checked)}
            className="accent-primary"
          />
          Subagents
        </label>
      </div>

      <ErrorLine what="the timeline" error={tl.error} />

      {!key ? (
        <Empty
          icon={<HistoryIcon className="size-6" aria-hidden />}
          title="No agents yet"
          text="Once a Claude Code session runs on this device, what it does shows up here."
        />
      ) : tl.loading && items.length === 0 ? (
        <p className="px-1 text-sm text-muted-foreground">
          Reading the timeline…
        </p>
      ) : items.length === 0 ? (
        <Empty
          icon={<HistoryIcon className="size-6" aria-hidden />}
          title="Nothing captured for this agent"
          text="Turns show up a few seconds after they happen. Sessions older than the backfill window start empty."
        />
      ) : (
        <Card size="sm" className="gap-0 py-0!">
          <ol className="divide-y divide-border">
            {items.map((it) =>
              it.kind === "message" ? (
                <MessageItem
                  key={`m:${it.d.message.id}`}
                  d={it.d}
                  name={name}
                  now={now}
                />
              ) : (
                <EntryItem
                  key={entryId(it.entry)}
                  e={it.entry}
                  roles={it.roles}
                  name={name}
                  now={now}
                  repos={repos}
                  onCommit={onCommit}
                />
              )
            )}
          </ol>
        </Card>
      )}

      {tl.more && (
        <Button
          variant="outline"
          size="sm"
          className="self-center"
          disabled={tl.loadingMore}
          onClick={() => void tl.loadMore()}
        >
          {tl.loadingMore ? "Reading…" : "Show older"}
        </Button>
      )}
    </div>
  )
}

function When({ at, now }: { at: unknown; now: Date }) {
  const t = parseTime(at)
  return (
    <time
      className="shrink-0 text-xs text-muted-foreground tabular-nums"
      dateTime={t?.toISOString()}
      title={t?.toLocaleString()}
    >
      {relativeTime(t, now)}
    </time>
  )
}

function EntryItem({
  e,
  roles,
  name,
  now,
  repos,
  onCommit,
}: {
  e: Entry
  roles?: string[]
  name: (key: string) => string
  now: Date
  repos: CaptureRepo[]
  onCommit: (c: CommitView) => void
}) {
  if (e.kind === EntryKind.EntryTurn && e.turn) {
    return <TurnItem t={e.turn} now={now} repos={repos} />
  }
  if (e.kind === EntryKind.EntryCommit && e.commit) {
    return <CommitItem c={e.commit} name={name} now={now} onCommit={onCommit} />
  }
  if (e.kind === EntryKind.EntryPR && e.pr) {
    return <PRItem p={e.pr} roles={roles ?? [e.pr.role]} now={now} />
  }
  if (e.kind === EntryKind.EntrySummary && e.summary) {
    const s = e.summary
    return (
      <li className="flex gap-2.5 px-4 py-2.5">
        <ScrollTextIcon
          className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-ink-2">
              {summaryLabel(s.kind)}
            </span>
            {s.agentId && <SubagentBadge id={s.agentId} />}
            <span className="ml-auto">
              <When at={s.at} now={now} />
            </span>
          </div>
          <p className="line-clamp-3 text-[13px] whitespace-pre-wrap text-ink-2">
            {plainBlock(s.text)}
          </p>
        </div>
      </li>
    )
  }
  return null
}

function summaryLabel(kind: string): string {
  switch (kind) {
    case "away_summary":
      return "While you were away"
    case "compact_boundary":
      return "Context compacted"
    case "compact_summary":
      return "Compaction summary"
    case "ai_title":
    case "custom_title":
      return "Title"
  }
  return plainLine(kind)
}

function SubagentBadge({ id }: { id: string }) {
  return (
    <Badge
      variant="slate"
      className="h-4.5 px-1.5 font-mono text-[11px]"
      title={`Subagent ${id}`}
    >
      sub {plainLine(id).slice(0, 7)}
    </Badge>
  )
}

function TurnItem({
  t,
  now,
  repos,
}: {
  t: TurnView
  now: Date
  repos: CaptureRepo[]
}) {
  const [diff, setDiff] = useState<EditRow | null>(null)
  const [marking, setMarking] = useState(false)
  const [allTools, setAllTools] = useState(false)
  const [allEdits, setAllEdits] = useState(false)
  const [marked, setMarked] = useState(false)
  const tools = t.tools ?? []
  const shown = allTools ? tools : tools.slice(0, ToolsShown)
  const hidden = tools.length - shown.length
  const edits = [...(t.edits ?? [])].sort(
    (a, b) => Number(a.fidelity !== "exact") - Number(b.fidelity !== "exact")
  )
  const shownEdits = allEdits ? edits : edits.slice(0, EditsShown)
  const prompt = plainBlock(t.prompt)
  const reply = plainBlock(t.reply ?? "")

  return (
    <li className="flex flex-col gap-1.5 px-4 py-3">
      <div className="flex min-w-0 items-center gap-2">
        {t.open ? (
          <Badge variant="info">
            <span
              className="size-1.5 animate-pulse rounded-full bg-current"
              aria-hidden
            />
            in progress
          </Badge>
        ) : (
          <span className="text-xs font-medium text-ink-2">Turn</span>
        )}
        {t.origin && t.origin !== "human" && (
          <Badge
            variant="slate"
            className="h-4.5 px-1.5 text-[11px]"
            title="Where the prompt came from"
          >
            {plainLine(t.originName || t.origin)}
          </Badge>
        )}
        {t.agentId && <SubagentBadge id={t.agentId} />}
        {t.interrupted && (
          <Badge variant="warning" className="h-4.5 px-1.5 text-[11px]">
            interrupted
          </Badge>
        )}
        {t.branch && (
          <span
            className="truncate font-mono text-xs text-muted-foreground"
            title={`Branch ${t.branch}`}
          >
            {plainLine(t.branch)}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {marked && <span className="text-xs text-success">Recorded</span>}
          <Button
            variant="ghost"
            size="xs"
            onClick={() => setMarking((m) => !m)}
            title="Record a decision that came out of this turn"
          >
            <BookmarkPlusIcon />
            Mark as decision
          </Button>
          <When at={t.startedAt} now={now} />
        </span>
      </div>

      <p
        className={
          prompt
            ? `${t.origin === "human" ? "line-clamp-4" : "line-clamp-2"} text-[13px] whitespace-pre-wrap text-ink`
            : "text-[13px] text-muted-foreground italic"
        }
        title={prompt || undefined}
      >
        {prompt || "No prompt text"}
      </p>
      {reply && (
        <p
          className="line-clamp-3 border-l-2 border-border pl-2.5 text-[13px] whitespace-pre-wrap text-ink-2"
          title={reply}
        >
          {reply}
        </p>
      )}

      {tools.length > 0 && (
        <div className="flex min-w-0 flex-wrap gap-1">
          {shown.map((c) => (
            <ToolChip key={c.id} c={c} />
          ))}
          {(hidden > 0 || (t.toolsMore ?? 0) > 0) && !allTools && (
            <button
              type="button"
              className="h-5 rounded-md px-1.5 text-[11px] text-muted-foreground hover:bg-muted"
              onClick={() => setAllTools(true)}
            >
              +{hidden + (t.toolsMore ?? 0)} more
            </button>
          )}
          {allTools && (t.toolsMore ?? 0) > 0 && (
            <span className="h-5 px-1.5 text-[11px] text-muted-foreground">
              +{t.toolsMore} not listed
            </span>
          )}
        </div>
      )}

      {edits.length > 0 && (
        <ul className="flex min-w-0 flex-col gap-0.5">
          {shownEdits.map((e) => (
            <li key={e.id} className="min-w-0">
              <button
                type="button"
                disabled={!e.patch}
                onClick={() => setDiff((d) => (d?.id === e.id ? null : e))}
                className="flex w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-0.5 text-left text-xs hover:bg-muted disabled:cursor-default disabled:hover:bg-transparent"
                title={
                  e.patch
                    ? "Show the diff"
                    : "No patch was stored for this edit"
                }
              >
                <FileDiffIcon
                  className="size-3.5 shrink-0 text-muted-foreground"
                  aria-hidden
                />
                <span className="min-w-0 truncate font-mono text-ink">
                  {plainLine(repoRel(e.path))}
                </span>
                <span className="shrink-0 font-mono text-success tabular-nums">
                  +{e.adds}
                </span>
                <span className="shrink-0 font-mono text-destructive tabular-nums">
                  −{e.dels}
                </span>
                {e.create && (
                  <span className="shrink-0 text-muted-foreground">new</span>
                )}
                {e.fidelity !== "exact" && (
                  <Badge variant="warning" className="h-4 px-1 text-[10px]">
                    {plainLine(e.fidelity)}
                  </Badge>
                )}
                <span className="ml-auto shrink-0 text-muted-foreground">
                  {plainLine(e.tool)}
                </span>
              </button>
            </li>
          ))}
          {edits.length > shownEdits.length && (
            <li>
              <button
                type="button"
                className="h-5 rounded-md px-1.5 text-[11px] text-muted-foreground hover:bg-muted"
                onClick={() => setAllEdits(true)}
              >
                +{edits.length - shownEdits.length} more files
              </button>
            </li>
          )}
        </ul>
      )}

      {diff?.patch && (
        <DiffView
          blobId={diff.patch}
          title={plainLine(repoRel(diff.path))}
          onClose={() => setDiff(null)}
        />
      )}

      {marking && (
        <div className="rounded-lg border border-border bg-white/60 p-3">
          <DecisionForm
            repos={repos}
            initial={{
              repoKey: t.repoKey ?? "",
              path:
                edits.length === 1 && !/^([A-Za-z]:)?[\\/]/.test(edits[0].path)
                  ? edits[0].path
                  : "",
              turnId: t.turnId,
            }}
            submitLabel="Record decision"
            onCancel={() => setMarking(false)}
            onDone={() => {
              setMarking(false)
              setMarked(true)
            }}
          />
        </div>
      )}
    </li>
  )
}

function ToolChip({ c }: { c: ToolCall }) {
  const target = plainLine(c.target ?? "")
  return (
    <span
      className={`inline-flex h-5 max-w-full min-w-0 items-center gap-1 rounded-md px-1.5 text-[11px] ${
        c.isError ? "bg-destructive/10 text-destructive" : "bg-muted text-ink-2"
      }`}
      title={`${c.name}${target ? `: ${target}` : ""}${c.isError ? " (failed)" : ""}`}
    >
      <span className="shrink-0 font-medium">{plainLine(c.name)}</span>
      {target && (
        <span className="max-w-64 truncate font-mono text-muted-foreground">
          {target}
        </span>
      )}
    </span>
  )
}

function CommitItem({
  c,
  name,
  now,
  onCommit,
}: {
  c: CommitView
  name: (key: string) => string
  now: Date
  onCommit: (c: CommitView) => void
}) {
  const counts = linkCounts(c.link)
  const committer = c.link?.committer?.key
  return (
    <li className="flex gap-2.5 px-4 py-2.5">
      <GitCommitHorizontalIcon
        className="mt-0.5 size-4 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex min-w-0 items-center gap-2">
          <button
            type="button"
            onClick={() => onCommit(c)}
            className="shrink-0 rounded-md bg-info-foreground px-1.5 font-mono text-xs text-info hover:underline"
            title="Open in History"
          >
            {shortSha(c.sha)}
          </button>
          <span
            className="min-w-0 truncate text-[13px] text-ink"
            title={plainLine(c.subject)}
          >
            {plainLine(c.subject)}
          </span>
          <span className="ml-auto">
            <When at={c.at} now={now} />
          </span>
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span className="font-mono">{shortRepo(c.repoKey)}</span>
          {c.branch && (
            <span className="font-mono">· {plainLine(c.branch)}</span>
          )}
          {committer && (
            <span>· committed by {plainLine(name(committer))}</span>
          )}
          <span className="ml-auto flex gap-1">
            {LinkClasses.filter((l) => counts[l]).map((l) => (
              <LinkChip key={l} link={l} count={counts[l]} />
            ))}
          </span>
        </div>
      </div>
    </li>
  )
}

function PRItem({ p, roles, now }: { p: PREntry; roles: string[]; now: Date }) {
  return (
    <li className="flex items-center gap-2.5 px-4 py-2.5">
      <GitPullRequestIcon
        className="size-4 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <span
        className="min-w-0 truncate font-mono text-[13px] text-ink"
        title={p.url || undefined}
      >
        {plainLine(p.repo)}#{p.number}
      </span>
      {roles.map((r) => (
        <Badge
          key={r}
          variant={r === "author" ? "info" : "slate"}
          className="h-4.5 px-1.5 text-[11px]"
        >
          {plainLine(r)}
        </Badge>
      ))}
      <span className="ml-auto">
        <When at={p.at} now={now} />
      </span>
    </li>
  )
}

function MessageItem({
  d,
  name,
  now,
}: {
  d: Delivery
  name: (key: string) => string
  now: Date
}) {
  const m = d.message
  const to = d.to ? name(d.to) : d.repoKey ? shortRepo(d.repoKey) : m.to.value
  return (
    <li className="flex gap-2.5 px-4 py-2.5">
      <MessageSquareIcon
        className="mt-0.5 size-4 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-2 text-xs">
          <span className="truncate font-medium text-ink-2">
            {plainLine(name(m.from.key))} → {plainLine(to)}
          </span>
          <Badge variant="slate" className="h-4.5 px-1.5 text-[11px]">
            {plainLine(d.state)}
          </Badge>
          <span className="ml-auto">
            <When at={m.sentAt} now={now} />
          </span>
        </div>
        <p className="line-clamp-3 text-[13px] whitespace-pre-wrap text-ink-2">
          {plainBlock(m.text)}
        </p>
      </div>
    </li>
  )
}
