import { useEffect, useMemo, useState } from "react"
import {
  ChevronDownIcon,
  ChevronRightIcon,
  FileDiffIcon,
  GitCommitHorizontalIcon,
  GavelIcon,
  SearchIcon,
} from "lucide-react"

import {
  AnchorBadge,
  DiffView,
  Empty,
  ErrorLine,
  fieldClass,
  LinkChip,
} from "@/components/capture/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import {
  capture,
  linkCounts,
  LinkClasses,
  repoRel,
  shortSha,
  useCommits,
  type CaptureRepo,
  type CommitView,
  type EditRow,
  type WhyResult,
} from "@/lib/capture"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  shortRepo,
} from "@/lib/irori"
import { errorText } from "@/lib/talk"

export type HistoryFocus = { repoKey: string; sha: string }

// HistoryPanel is a repo's commits with how each links to captured edits,
// and a "why" box for a path and line.
export function HistoryPanel({
  repos,
  focus,
  name,
  onTimeline,
  now,
}: {
  repos: CaptureRepo[]
  focus: HistoryFocus | null
  name: (key: string) => string
  onTimeline: (key: string) => void
  now: Date
}) {
  const [picked, setPicked] = useState<string | null>(focus?.repoKey ?? null)
  // The default (the first repo, live ones first) is kept once chosen, so
  // the list doesn't jump when another repo goes live.
  if (picked === null && repos.length > 0) setPicked(repos[0].repoKey)
  const repoKey = picked ?? ""
  const { commits, loading, error } = useCommits(repoKey)
  const [open, setOpen] = useState<Set<string>>(() => new Set())
  const [pinned, setPinned] = useState<CommitView | null>(null)
  const [jump, setJump] = useState<string | null>(null)

  // A commit chip from the Timeline opens here: fetch it in case it is
  // past the list's limit, expand it and scroll to it.
  useEffect(() => {
    if (!focus?.sha) return
    let live = true
    capture.commit(focus.sha).then(
      (c) => {
        if (!live || !c) return
        setPinned(c)
        setOpen((o) => new Set(o).add(c.sha))
        setJump(c.sha)
      },
      () => {}
    )
    return () => {
      live = false
    }
  }, [focus?.sha])

  useEffect(() => {
    if (!jump) return
    document
      .getElementById(`commit-${jump}`)
      ?.scrollIntoView({ block: "center" })
  }, [jump, commits])

  const list = useMemo(() => {
    if (!pinned || (repoKey && pinned.repoKey !== repoKey)) return commits
    if (commits.some((c) => c.sha === pinned.sha)) return commits
    return [pinned, ...commits]
  }, [commits, pinned, repoKey])

  const toggle = (sha: string) =>
    setOpen((o) => {
      const n = new Set(o)
      if (n.has(sha)) n.delete(sha)
      else n.add(sha)
      return n
    })

  const known = !repoKey || repos.some((r) => r.repoKey === repoKey)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 px-1">
        <label htmlFor="history-repo" className="text-sm font-medium text-ink">
          Repo
        </label>
        <select
          id="history-repo"
          value={repoKey}
          onChange={(e) => setPicked(e.target.value)}
          className={`${fieldClass} flex-1 sm:max-w-md`}
        >
          <option value="">All repos</option>
          {!known && <option value={repoKey}>{shortRepo(repoKey)}</option>}
          {repos.map((r) => (
            <option key={r.repoKey} value={r.repoKey}>
              {shortRepo(r.repoKey)}
              {r.live ? " · live" : ""}
            </option>
          ))}
        </select>
      </div>

      <WhyBox
        repoKey={repoKey}
        name={name}
        onTimeline={onTimeline}
        now={now}
        onCommit={(c) => {
          setPicked(c.repoKey)
          setPinned(c)
          setOpen((o) => new Set(o).add(c.sha))
          setJump(c.sha)
        }}
      />

      <section aria-labelledby="commits-h" className="mt-2 flex flex-col gap-2">
        <div className="flex items-center gap-2 px-1">
          <h2 id="commits-h" className="text-sm font-medium text-ink">
            Commits
          </h2>
          <span className="text-xs text-muted-foreground tabular-nums">
            {list.length}
          </span>
        </div>

        <ErrorLine what="commits" error={error} />

        {loading && list.length === 0 ? (
          <p className="px-1 text-sm text-muted-foreground">Reading commits…</p>
        ) : list.length === 0 ? (
          <Empty
            icon={<GitCommitHorizontalIcon className="size-6" aria-hidden />}
            title="No commits seen yet"
            text="Irori watches each repo an agent works in and links new commits to the edits behind them."
          />
        ) : (
          <Card size="sm" className="gap-0 py-0!">
            <ul className="divide-y divide-border">
              {list.map((c) => (
                <CommitRow
                  key={c.sha}
                  c={c}
                  open={open.has(c.sha)}
                  focused={jump === c.sha}
                  onToggle={() => toggle(c.sha)}
                  name={name}
                  onTimeline={onTimeline}
                  now={now}
                />
              ))}
            </ul>
          </Card>
        )}
      </section>
    </div>
  )
}

function CommitRow({
  c,
  open,
  focused,
  onToggle,
  name,
  onTimeline,
  now,
}: {
  c: CommitView
  open: boolean
  focused: boolean
  onToggle: () => void
  name: (key: string) => string
  onTimeline: (key: string) => void
  now: Date
}) {
  const at = parseTime(c.at)
  const counts = linkCounts(c.link)
  const links = new Map((c.link?.files ?? []).map((f) => [f.path, f]))
  const sessions = c.link?.sessions ?? []
  const committer = c.link?.committer?.key
  const Chevron = open ? ChevronDownIcon : ChevronRightIcon
  return (
    <li
      id={`commit-${c.sha}`}
      className={focused ? "bg-info-foreground/40" : ""}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full min-w-0 flex-col gap-1 px-4 py-2.5 text-left hover:bg-muted/50"
      >
        <span className="flex min-w-0 items-center gap-2">
          <Chevron
            className="size-3.5 shrink-0 text-muted-foreground"
            aria-hidden
          />
          <span className="shrink-0 font-mono text-xs text-info">
            {shortSha(c.sha)}
          </span>
          <span
            className="min-w-0 truncate text-[13px] text-ink"
            title={plainLine(c.subject)}
          >
            {plainLine(c.subject)}
          </span>
          <time
            className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums"
            dateTime={at?.toISOString()}
            title={at?.toLocaleString()}
          >
            {relativeTime(at, now)}
          </time>
        </span>
        <span className="flex min-w-0 flex-wrap items-center gap-1.5 pl-5.5 text-xs text-muted-foreground">
          <span className="truncate">{plainLine(c.author)}</span>
          {c.op && c.op !== "commit" && (
            <span className="truncate">· {plainLine(c.op)}</span>
          )}
          {c.branch && (
            <span className="font-mono">· {plainLine(c.branch)}</span>
          )}
          <span className="ml-auto flex flex-wrap gap-1">
            {c.link ? (
              LinkClasses.filter((l) => counts[l]).map((l) => (
                <LinkChip key={l} link={l} count={counts[l]} />
              ))
            ) : (
              <Badge
                variant="outline"
                className="h-4.5 px-1.5 text-[11px] text-muted-foreground"
              >
                not linked yet
              </Badge>
            )}
          </span>
        </span>
      </button>

      {open && (
        <div className="flex flex-col gap-2 px-4 pb-3 pl-10">
          <ul className="flex flex-col gap-0.5">
            {c.files.map((f) => {
              const l = links.get(f.path)
              return (
                <li
                  key={f.path}
                  className="flex min-w-0 items-center gap-2 text-xs"
                >
                  <span
                    className="w-3 shrink-0 font-mono text-muted-foreground"
                    title="git status"
                  >
                    {plainLine(f.status)}
                  </span>
                  <span
                    className="min-w-0 truncate font-mono text-ink"
                    title={plainLine(f.path)}
                  >
                    {plainLine(f.path)}
                  </span>
                  <span className="ml-auto flex shrink-0 items-center gap-1.5">
                    {(l?.edits?.length ?? 0) > 0 && (
                      <span className="text-muted-foreground tabular-nums">
                        {l!.edits!.length} edit
                        {l!.edits!.length === 1 ? "" : "s"}
                      </span>
                    )}
                    {l && <LinkChip link={l.link} />}
                  </span>
                </li>
              )
            })}
            {(c.filesOmitted ?? 0) > 0 && (
              <li className="text-xs text-muted-foreground">
                +{c.filesOmitted} more files
              </li>
            )}
          </ul>
          <div className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
            {sessions.length === 0 && !committer ? (
              <span>No captured session is behind this commit.</span>
            ) : (
              <>
                <span>Behind it:</span>
                {sessions.map((k) => (
                  <SessionLink
                    key={k}
                    k={k}
                    name={name}
                    onTimeline={onTimeline}
                  />
                ))}
                {(c.link?.turns?.length ?? 0) > 0 && (
                  <span className="tabular-nums">
                    · {c.link!.turns!.length} turn
                    {c.link!.turns!.length === 1 ? "" : "s"}
                  </span>
                )}
                {committer && (
                  <>
                    <span>· committed by</span>
                    <SessionLink
                      k={committer}
                      name={name}
                      onTimeline={onTimeline}
                    />
                  </>
                )}
              </>
            )}
            <span className="ml-auto font-mono" title={c.sha}>
              {shortRepo(c.repoKey)}
            </span>
          </div>
        </div>
      )}
    </li>
  )
}

function SessionLink({
  k,
  name,
  onTimeline,
}: {
  k: string
  name: (key: string) => string
  onTimeline: (key: string) => void
}) {
  return (
    <button
      type="button"
      onClick={() => onTimeline(k)}
      className="rounded-md bg-muted px-1.5 text-ink-2 hover:underline"
      title="Open this session's timeline"
    >
      {plainLine(name(k))}
    </button>
  )
}

// WhyBox asks what is behind a path (and line) in the picked repo.
function WhyBox({
  repoKey,
  name,
  onTimeline,
  onCommit,
  now,
}: {
  repoKey: string
  name: (key: string) => string
  onTimeline: (key: string) => void
  onCommit: (c: CommitView) => void
  now: Date
}) {
  const [path, setPath] = useState("")
  const [line, setLine] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<WhyResult | null>(null)
  const [diff, setDiff] = useState<EditRow | null>(null)

  const n = Number(line.trim() || "0")
  const lineOk = Number.isInteger(n) && n >= 0
  const ready = path.trim() !== "" && lineOk && !busy

  const ask = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    setDiff(null)
    try {
      setResult(await capture.why(path.trim(), n, repoKey))
    } catch (err) {
      setResult(null)
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card size="sm" className="gap-2 px-4">
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void ask()
        }}
      >
        <label htmlFor="why-path" className="text-sm font-medium text-ink">
          Why
        </label>
        <input
          id="why-path"
          placeholder={
            repoKey
              ? `Path in ${shortRepo(repoKey)}, e.g. src/main.go`
              : "Absolute path"
          }
          value={path}
          onChange={(e) => setPath(e.target.value)}
          className={`${fieldClass} min-w-40 flex-1 px-2.5 font-mono text-xs`}
        />
        <input
          aria-label="Line"
          placeholder="Line"
          inputMode="numeric"
          value={line}
          onChange={(e) => setLine(e.target.value)}
          className={`${fieldClass} w-20 px-2.5 font-mono text-xs ${lineOk ? "" : "border-destructive"}`}
        />
        <Button type="submit" size="sm" disabled={!ready}>
          <SearchIcon />
          {busy ? "Looking…" : "Ask"}
        </Button>
      </form>
      {error && <p className="text-xs break-words text-destructive">{error}</p>}
      {result && (
        <WhyAnswer
          r={result}
          name={name}
          onTimeline={onTimeline}
          onCommit={onCommit}
          diff={diff}
          setDiff={setDiff}
          now={now}
        />
      )}
    </Card>
  )
}

function WhyAnswer({
  r,
  name,
  onTimeline,
  onCommit,
  diff,
  setDiff,
  now,
}: {
  r: WhyResult
  name: (key: string) => string
  onTimeline: (key: string) => void
  onCommit: (c: CommitView) => void
  diff: EditRow | null
  setDiff: (e: EditRow | null) => void
  now: Date
}) {
  const turns = r.turns ?? []
  const decisions = r.decisions ?? []
  const nothing =
    !r.blameSha &&
    r.edits.length === 0 &&
    turns.length === 0 &&
    decisions.length === 0
  return (
    <div className="flex flex-col gap-2 border-t border-border pt-2 text-[13px]">
      <div className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
        <span className="truncate font-mono text-ink-2">
          {shortRepo(r.repoKey)} {plainLine(r.path)}
          {r.line ? `:${r.line}` : ""}
        </span>
        {r.line ? (
          r.blameSha ? (
            <>
              <span>· last changed in</span>
              {r.commit ? (
                <button
                  type="button"
                  className="rounded-md bg-info-foreground px-1.5 font-mono text-info hover:underline"
                  onClick={() => onCommit(r.commit!)}
                >
                  {shortSha(r.blameSha)}
                </button>
              ) : (
                <span className="font-mono">{shortSha(r.blameSha)}</span>
              )}
              {r.commit && (
                <span className="truncate">{plainLine(r.commit.subject)}</span>
              )}
              {!r.commit && <span>(a commit Irori didn't see)</span>}
            </>
          ) : (
            <span>· not committed yet</span>
          )
        ) : null}
      </div>

      {nothing && (
        <p className="text-xs text-muted-foreground">
          Nothing captured explains this path yet.
        </p>
      )}

      {turns.length > 0 && (
        <ul className="flex flex-col gap-1">
          {turns.map((t) => (
            <li key={t.id} className="flex min-w-0 items-center gap-2 text-xs">
              <button
                type="button"
                onClick={() => onTimeline(t.key)}
                className="shrink-0 rounded-md bg-muted px-1.5 text-ink-2 hover:underline"
                title="Open this session's timeline"
              >
                {plainLine(name(t.key))}
              </button>
              <span
                className="min-w-0 truncate text-ink-2"
                title={plainBlock(t.prompt)}
              >
                {plainLine(t.prompt) || "No prompt text"}
              </span>
              <span className="ml-auto shrink-0 text-muted-foreground tabular-nums">
                {relativeTime(parseTime(t.startedAt), now)}
              </span>
            </li>
          ))}
        </ul>
      )}

      {r.edits.length > 0 && (
        <ul className="flex flex-col gap-0.5">
          {r.edits.map((e) => (
            <li key={e.id}>
              <button
                type="button"
                disabled={!e.patch}
                onClick={() => setDiff(diff?.id === e.id ? null : e)}
                className="flex w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-0.5 text-left text-xs hover:bg-muted disabled:cursor-default disabled:hover:bg-transparent"
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
                <span className="truncate text-muted-foreground">
                  by {plainLine(name(e.key))}
                </span>
                <span className="ml-auto shrink-0 text-muted-foreground tabular-nums">
                  {relativeTime(parseTime(e.at), now)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {diff?.patch && (
        <DiffView
          blobId={diff.patch}
          title={plainLine(repoRel(diff.path))}
          onClose={() => setDiff(null)}
        />
      )}

      {decisions.length > 0 && (
        <ul className="flex flex-col gap-1">
          {decisions.map((d) => (
            <li key={d.id} className="flex min-w-0 items-center gap-2 text-xs">
              <GavelIcon
                className="size-3.5 shrink-0 text-muted-foreground"
                aria-hidden
              />
              <span
                className="min-w-0 truncate text-ink"
                title={plainBlock(d.statement)}
              >
                {plainLine(d.statement)}
              </span>
              <span className="ml-auto shrink-0">
                <AnchorBadge d={d} />
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
