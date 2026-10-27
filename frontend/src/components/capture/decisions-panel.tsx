import { useMemo, useState } from "react"
import { GavelIcon, PlusIcon, XIcon } from "lucide-react"

import {
  AnchorBadge,
  areaClass,
  Empty,
  ErrorLine,
  fieldClass,
} from "@/components/capture/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import {
  capture,
  lineRange,
  useDecisions,
  type CaptureRepo,
  type DecisionView,
} from "@/lib/capture"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  sessionShort,
  shortRepo,
} from "@/lib/irori"
import { errorText } from "@/lib/talk"

// Mirrors decision.MaxStatement / MaxWhy.
const MaxStatement = 500
const MaxWhy = 2000

export type DecisionSeed = {
  repoKey?: string
  path?: string
  lines?: string
  supersedes?: string[]
  turnId?: string
  messageId?: string
}

// DecisionsPanel lists decisions with their re-checked anchors and the
// supersede chain, and lets the person record one.
export function DecisionsPanel({
  repos,
  now,
}: {
  repos: CaptureRepo[]
  now: Date
}) {
  const [repoKey, setRepoKey] = useState("")
  const [query, setQuery] = useState("")
  const [showOld, setShowOld] = useState(false)
  const [seed, setSeed] = useState<DecisionSeed | null>(null)
  const { decisions, loading, error } = useDecisions(repoKey, query)

  const byId = useMemo(
    () => new Map(decisions.map((d) => [d.id, d])),
    [decisions]
  )
  const current = decisions.filter((d) => !(d.supersededBy?.length ?? 0))
  const shown = showOld ? decisions : current
  const old = decisions.length - current.length

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 px-1">
        <select
          aria-label="Repo"
          value={repoKey}
          onChange={(e) => setRepoKey(e.target.value)}
          className={`${fieldClass} w-48`}
        >
          <option value="">All repos</option>
          {repos.map((r) => (
            <option key={r.repoKey} value={r.repoKey}>
              {shortRepo(r.repoKey)}
            </option>
          ))}
        </select>
        <input
          type="search"
          aria-label="Search decisions"
          placeholder="Search statements and reasons"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className={`${fieldClass} min-w-40 flex-1 px-2.5`}
        />
        {old > 0 && (
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={showOld}
              onChange={(e) => setShowOld(e.target.checked)}
              className="accent-primary"
            />
            Superseded ({old})
          </label>
        )}
        <Button size="sm" onClick={() => setSeed(seed ? null : { repoKey })}>
          <PlusIcon />
          New decision
        </Button>
      </div>

      {seed && (
        <Card size="sm" className="px-4">
          <DecisionForm
            key={JSON.stringify(seed)}
            repos={repos}
            initial={seed}
            byId={byId}
            submitLabel="Record decision"
            onCancel={() => setSeed(null)}
            onDone={() => setSeed(null)}
          />
        </Card>
      )}

      <ErrorLine what="decisions" error={error} />

      {loading && decisions.length === 0 ? (
        <p className="px-1 text-sm text-muted-foreground">Reading decisions…</p>
      ) : shown.length === 0 ? (
        <Empty
          icon={<GavelIcon className="size-6" aria-hidden />}
          title={query || repoKey ? "No decisions match" : "No decisions yet"}
          text="Agents record them with decision_record; you can add one here or mark a turn as a decision in the Timeline."
        />
      ) : (
        <Card size="sm" className="gap-0 py-0!">
          <ul className="divide-y divide-border">
            {shown.map((d) => (
              <DecisionRow
                key={d.id}
                d={d}
                byId={byId}
                now={now}
                onSupersede={() =>
                  setSeed({
                    repoKey: d.anchor?.repoKey ?? "",
                    path: d.anchor?.path ?? "",
                    lines: lineRange(d.anchor?.start, d.anchor?.end),
                    supersedes: [d.id],
                  })
                }
              />
            ))}
          </ul>
        </Card>
      )}
    </div>
  )
}

function byName(d: DecisionView): string {
  if (d.by.kind === "user") return "you"
  return plainLine(d.by.name ?? "") || `agent ${sessionShort(d.by.key)}`
}

function DecisionRow({
  d,
  byId,
  now,
  onSupersede,
}: {
  d: DecisionView
  byId: Map<string, DecisionView>
  now: Date
  onSupersede: () => void
}) {
  const at = parseTime(d.at)
  const a = d.anchor
  const lines = lineRange(a?.start, a?.end)
  const moved =
    d.status?.state === "moved" ? lineRange(d.status.start, d.status.end) : ""
  const old = (d.supersededBy?.length ?? 0) > 0
  return (
    <li className={`flex flex-col gap-1 px-4 py-3 ${old ? "opacity-70" : ""}`}>
      <div className="flex min-w-0 items-start gap-2">
        <p className="min-w-0 flex-1 text-[13px] font-medium whitespace-pre-wrap text-ink">
          {plainBlock(d.statement)}
        </p>
        <span className="flex shrink-0 items-center gap-2">
          {old && (
            <Badge variant="slate" className="h-4.5 px-1.5 text-[11px]">
              superseded
            </Badge>
          )}
          {!old && (
            <Button variant="ghost" size="xs" onClick={onSupersede}>
              Supersede
            </Button>
          )}
          <time
            className="text-xs text-muted-foreground tabular-nums"
            dateTime={at?.toISOString()}
            title={at?.toLocaleString()}
          >
            {relativeTime(at, now)}
          </time>
        </span>
      </div>
      {d.why && (
        <p className="line-clamp-3 text-[13px] whitespace-pre-wrap text-ink-2">
          {plainBlock(d.why)}
        </p>
      )}
      <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
        <span>by {byName(d)}</span>
        {d.trust !== "user" && d.by.kind !== "user" && (
          <span title="Recorded by an agent, not by you">· agent</span>
        )}
        {a ? (
          <>
            <span>·</span>
            <span
              className="min-w-0 truncate font-mono text-ink-2"
              title={`${a.repoKey} ${a.path}${lines ? `:${lines}` : ""}`}
            >
              {shortRepo(a.repoKey)} {plainLine(a.path)}
              {lines && `:${lines}`}
            </span>
            <AnchorBadge d={d} />
            {moved && moved !== lines && <span>now {moved}</span>}
          </>
        ) : (
          <span>· not anchored</span>
        )}
      </div>
      <Chain label="Supersedes" ids={d.supersedes} byId={byId} />
      <Chain label="Superseded by" ids={d.supersededBy} byId={byId} />
    </li>
  )
}

function Chain({
  label,
  ids,
  byId,
}: {
  label: string
  ids?: string[]
  byId: Map<string, DecisionView>
}) {
  if (!ids?.length) return null
  return (
    <div className="flex min-w-0 flex-col gap-0.5 border-l-2 border-border pl-2.5 text-xs text-muted-foreground">
      {ids.map((id) => {
        const t = byId.get(id)
        return (
          <span
            key={id}
            className="truncate"
            title={t ? plainLine(t.statement) : id}
          >
            {label}:{" "}
            {t ? (
              plainLine(t.statement)
            ) : (
              <span className="font-mono">{id}</span>
            )}
          </span>
        )
      })}
    </div>
  )
}

// DecisionForm records a decision as the person. A path anchors it in the
// chosen repo's worktree (relative) or anywhere (absolute); lines are
// optional ("40" or "40-52"; empty anchors the whole file).
export function DecisionForm({
  repos,
  initial,
  byId,
  submitLabel,
  onCancel,
  onDone,
}: {
  repos: CaptureRepo[]
  initial: DecisionSeed
  byId?: Map<string, DecisionView>
  submitLabel: string
  onCancel: () => void
  onDone: () => void
}) {
  const [statement, setStatement] = useState("")
  const [why, setWhy] = useState("")
  const [repoKey, setRepoKey] = useState(initial.repoKey ?? "")
  const [path, setPath] = useState(initial.path ?? "")
  const [lines, setLines] = useState(initial.lines ?? "")
  const [supersedes, setSupersedes] = useState(initial.supersedes ?? [])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const s = statement.trim()
  const tooLong = s.length > MaxStatement || why.trim().length > MaxWhy
  const ready = s !== "" && !tooLong && !busy
  const knownRepo = !repoKey || repos.some((r) => r.repoKey === repoKey)

  const submit = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    try {
      await capture.recordDecision({
        statement: s,
        why: why.trim(),
        path: path.trim(),
        lines: path.trim() ? lines.trim() : "",
        repoKey,
        supersedes,
        turnId: initial.turnId,
        messageId: initial.messageId,
      })
      onDone()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form
      className="flex flex-col gap-2"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <textarea
        aria-label="Decision"
        value={statement}
        rows={2}
        autoFocus
        placeholder="What was decided, in one or two sentences"
        onChange={(e) => setStatement(e.target.value)}
        className={areaClass}
      />
      <textarea
        aria-label="Why"
        value={why}
        rows={2}
        placeholder="Why (optional)"
        onChange={(e) => setWhy(e.target.value)}
        className={areaClass}
      />
      <div className="flex flex-wrap items-center gap-2">
        <select
          aria-label="Repo"
          value={repoKey}
          onChange={(e) => setRepoKey(e.target.value)}
          className={`${fieldClass} w-44`}
        >
          <option value="">No repo</option>
          {!knownRepo && <option value={repoKey}>{shortRepo(repoKey)}</option>}
          {repos.map((r) => (
            <option key={r.repoKey} value={r.repoKey}>
              {shortRepo(r.repoKey)}
            </option>
          ))}
        </select>
        <input
          aria-label="Path"
          placeholder="Path to anchor (optional)"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          className={`${fieldClass} min-w-40 flex-1 px-2.5 font-mono text-xs`}
        />
        <input
          aria-label="Lines"
          placeholder="Lines, e.g. 40-52"
          value={lines}
          disabled={!path.trim()}
          onChange={(e) => setLines(e.target.value)}
          className={`${fieldClass} w-32 px-2.5 font-mono text-xs disabled:opacity-50`}
        />
      </div>
      {supersedes.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          Replaces
          {supersedes.map((id) => (
            <Badge key={id} variant="slate" className="max-w-80">
              <span className="truncate">
                {byId?.get(id) ? plainLine(byId.get(id)!.statement) : id}
              </span>
              <button
                type="button"
                aria-label="Don't replace this one"
                onClick={() => setSupersedes((x) => x.filter((y) => y !== id))}
              >
                <XIcon className="size-3" />
              </button>
            </Badge>
          ))}
        </div>
      )}
      <div className="flex items-center gap-3">
        {tooLong && (
          <span className="text-xs text-destructive">
            Keep it under {MaxStatement} characters (why: {MaxWhy}).
          </span>
        )}
        {error && (
          <span className="min-w-0 text-xs break-words text-destructive">
            {error}
          </span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={!ready}>
            {busy ? "Recording…" : submitLabel}
          </Button>
        </span>
      </div>
    </form>
  )
}
