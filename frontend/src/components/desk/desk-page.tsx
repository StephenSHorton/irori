// DeskPage is the interim merge desk page (card #417, docs/design/
// merge-desk.md §1, §11 slice 3, mockup desk-2): every desk down the
// left, and the open desk's queue: every open PR of ours on GitHub (the
// person's, their space members' and their agents'; card #625) with its
// state and author card, for review. The desk never merges (card #418):
// each agent asks the person "Can I merge PR #N?" on its own card. A desk
// with none of ours open is left off the list.

import { useState, type ReactNode } from "react"
import {
  CheckIcon,
  CircleCheckIcon,
  CloudOffIcon,
  GitMergeIcon,
  GitPullRequestDraftIcon,
  GitPullRequestIcon,
  SettingsIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import {
  deskOn,
  deskQueue,
  deskShown,
  inputsOf,
  claimCountsLine,
  ownerName,
  queueCount,
  reviewParts,
  stateLabel,
  type Desk,
  type DeskState,
  type QueueLine,
} from "@/lib/desk"
import { cards } from "@/lib/cards"
import { Tick } from "@/components/common/ago"
import { parseTime, relativeTime } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

export function StateChip({ s }: { s: DeskState }) {
  const label = stateLabel[s]
  switch (s) {
    case "running":
      return (
        <Badge variant="info">
          <Spinner className="size-3" />
          {label}
        </Badge>
      )
    case "ready":
      return <Badge variant="slate">{label}</Badge>
    case "rebase":
      return <Badge variant="warning">{label}</Badge>
    case "failed":
      return <Badge variant="destructive">{label}</Badge>
    case "draft":
      return (
        <Badge variant="outline" className="text-muted-foreground">
          {label}
        </Badge>
      )
    case "merged":
      return (
        <Badge variant="success">
          <CheckIcon />
          {label}
        </Badge>
      )
  }
}

function timeOf(at: string | undefined, now: Date): string {
  const t = parseTime(at ?? "")
  return t ? relativeTime(t, now) : "never"
}

function clock(at: string | undefined): string {
  const t = parseTime(at ?? "")
  return t
    ? t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : "—"
}

// QueueLineRow is one PR: its number (to GitHub), title, author, author
// card, and state.
function QueueLineRow({
  l,
  nameOf,
  onOpenCard,
  onError,
}: {
  l: QueueLine
  nameOf: (key: string) => string
  onOpenCard: (id: string) => void
  onError: (e: string) => void
}) {
  const dim = l.state === "draft" || l.state === "merged"
  return (
    <div
      className="grid min-w-0 grid-cols-[minmax(0,1fr)] rounded-md"
      data-pr={l.pr}
    >
      <div
        className={cn(
          "flex items-center gap-2 px-2 py-1 text-[12.5px]",
          dim && "opacity-60"
        )}
      >
        {l.state === "draft" ? (
          <GitPullRequestDraftIcon className="size-4 shrink-0 text-muted-foreground" />
        ) : l.state === "merged" ? (
          <GitMergeIcon className="size-4 shrink-0 text-success" />
        ) : (
          <GitPullRequestIcon className="size-4 shrink-0 text-success" />
        )}
        {l.url ? (
          <button
            type="button"
            className="w-10 shrink-0 text-left font-mono text-primary hover:underline"
            title="Open on GitHub"
            onClick={() =>
              void cards
                .openURL(l.url)
                .catch((e: unknown) => onError(errorText(e)))
            }
          >
            #{l.pr}
          </button>
        ) : (
          <span className="w-10 shrink-0 font-mono text-primary">#{l.pr}</span>
        )}
        <span className="min-w-0 flex-1 truncate font-medium text-ink">
          {l.title}
        </span>
        <span className="hidden shrink-0 text-[11.5px] text-ink-2 sm:inline">
          {l.author.name ||
            (l.author.agent && nameOf(l.author.agent)) ||
            (l.login && `@${l.login}`)}
        </span>
        {l.author.card && l.author.cardRef && (
          <button
            type="button"
            className="shrink-0 rounded bg-secondary px-1 font-mono text-[11px] text-primary hover:underline"
            title="Open the author's card (the PR's thread)"
            onClick={() => onOpenCard(l.author.card!)}
          >
            {l.author.cardRef}
          </button>
        )}
        <span className="flex shrink-0 justify-end sm:w-[132px]">
          <StateChip s={l.state} />
        </span>
      </div>
      <div className="-mt-1 flex min-h-5 flex-wrap items-center gap-x-2 gap-y-0.5 pr-2 pb-0.5 pl-8 text-[11px] text-muted-foreground sm:pl-[72px]">
        <span className="min-w-0 truncate">{l.note}</span>
      </div>
      <ReviewRow l={l} />
    </div>
  )
}

// ReviewRow is a PR's review under its line (merge desk §12.9): GitHub's
// state, who holds the claim or was asked, the related agents, and why
// nothing is picked. It draws nothing for a PR with no review to show.
function ReviewRow({ l }: { l: QueueLine }) {
  const r = reviewParts(l.author.review)
  if (!r.github && !r.pick && !r.claimer && !r.related.length && !r.why)
    return null
  return (
    <div
      className="-mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 pr-2 pb-1 pl-8 text-[11px] sm:pl-[72px]"
      data-review={l.pr}
    >
      {r.github && (
        <Badge
          variant={r.github === "Approved" ? "success" : "outline"}
          className="text-[10.5px]"
        >
          {r.github}
        </Badge>
      )}
      {r.claimer && (
        <span
          className="max-w-full min-w-0 break-words text-ink-2"
          data-claimer
        >
          Claimed by <span className="font-medium text-ink">{r.claimer}</span>
          {r.until && (
            <span className="text-muted-foreground">
              {" "}
              until {clock(r.until)}
            </span>
          )}
        </span>
      )}
      {r.pick && <span className="text-ink-2">{r.pick}</span>}
      {r.related.length > 0 && (
        <span
          className="max-w-full min-w-0 break-words text-muted-foreground"
          data-related
        >
          Related: {r.related.join("; ")}
        </span>
      )}
      {r.why && (
        <span
          className="max-w-full min-w-0 break-words text-muted-foreground"
          data-why
        >
          {r.why}
        </span>
      )}
    </div>
  )
}

function QueueHead({ count, right }: { count: ReactNode; right?: ReactNode }) {
  return (
    <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-[12px]">
      <span className="font-medium text-ink">Queue</span>
      <span className="text-muted-foreground">{count}</span>
      <span className="ml-auto flex items-center gap-2 text-muted-foreground">
        {right}
      </span>
    </div>
  )
}

// DeskPanel is one desk's queue (§1, §3): never blank. Before the first
// poll it says so; with no open PRs of ours it says so and when it last
// asked; when GitHub can't be asked it keeps the last list.
export function DeskPanel({
  desk,
  nameOf,
  onOpenCard,
}: {
  desk: Desk
  nameOf: (key: string) => string
  onOpenCard: (id: string) => void
}) {
  const [error, setError] = useState("")
  const lines = deskQueue(inputsOf(desk))
  const snap = desk.prs
  const repo = ownerName(desk.settings.repoKey)
  const lastPoll = desk.poll?.at ?? snap?.at
  const down = desk.poll?.unreachableSince
  return (
    <div
      className={cn(
        "overflow-hidden rounded-xl border bg-card",
        down ? "border-warning/50" : "border-border"
      )}
      data-desk={desk.settings.repoKey}
    >
      <QueueHead
        count={
          snap ? (
            down ? (
              `${snap.prs.length} open at ${clock(snap.at)}`
            ) : (
              <Tick
                of={(now) =>
                  `${queueCount(snap)} · polled ${timeOf(lastPoll, now)} (every 3 min)`
                }
              />
            )
          ) : (
            queueCount(snap)
          )
        }
      />
      {down && (
        <div
          className="flex items-start gap-2 px-3 pt-2.5 text-[12px] text-warning"
          role="status"
        >
          <CloudOffIcon className="mt-0.5 size-4 shrink-0" />
          <span>
            GitHub unreachable since {clock(down)}.{" "}
            {snap ? `Showing the ${clock(snap.at)} list; ` : ""}never an empty
            queue while GitHub can&apos;t be asked.
          </span>
        </div>
      )}
      {snap?.truncated && (
        <div className="px-3 pt-2.5 text-[12px] text-warning">
          Showing {snap.prs.length} of ours; GitHub has more open PRs than it
          listed.
        </div>
      )}
      {error && (
        <div className="px-3 pt-2.5 text-[12px] text-destructive" role="alert">
          {error}
        </div>
      )}
      {!snap ? (
        <div className="px-4 py-6 text-center text-[12.5px] text-muted-foreground">
          {desk.settings.pollDevice
            ? "Waiting for the first poll of GitHub's open PRs."
            : "No device polls GitHub for this desk: pick one in Settings › Merge desks."}
        </div>
      ) : lines.length === 0 ? (
        <div className="grid justify-items-center gap-1 px-4 py-6 text-center">
          <CircleCheckIcon className="size-6 text-success" />
          <div className="text-[13.5px] font-medium text-ink">
            No open PRs of yours on {repo}
          </div>
          <div className="text-[11.5px] text-muted-foreground">
            Last poll {clock(lastPoll)} · the desk shows here again once you or
            your people open one
          </div>
        </div>
      ) : (
        <div className="grid grid-cols-[minmax(0,1fr)] gap-0.5 p-1.5">
          {lines.map((l) => (
            <QueueLineRow
              key={`${l.state === "merged" ? "m" : "o"}${l.pr}`}
              l={l}
              nameOf={nameOf}
              onOpenCard={onOpenCard}
              onError={setError}
            />
          ))}
        </div>
      )}
    </div>
  )
}

// DeskHeader: whose desk it is, and its settings.
function DeskHeader({
  desk,
  onSettings,
}: {
  desk: Desk
  onSettings: () => void
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <span className="flex items-center gap-1.5 text-[17px] font-semibold text-ink">
        <GitMergeIcon className="size-5" />
        <span className="font-mono">{ownerName(desk.settings.repoKey)}</span>
      </span>
      {claimCountsLine(desk.claimCounts) && (
        <span
          className="w-full text-[11.5px] text-muted-foreground sm:w-auto"
          data-claim-counts
        >
          Claims: {claimCountsLine(desk.claimCounts)}
        </span>
      )}
      <span className="ml-auto flex items-center gap-1.5">
        <Button
          size="sm"
          variant="ghost"
          title="Merge desk settings"
          onClick={onSettings}
        >
          <SettingsIcon />
        </Button>
      </span>
    </div>
  )
}

export function DeskPage({
  desks,
  open,
  onOpen,
  onOpenCard,
  onSettings,
  nameOf,
}: {
  desks: Desk[]
  // The desk open, by repo key ("" the first).
  open: string
  onOpen: (repoKey: string) => void
  onOpenCard: (id: string) => void
  onSettings: () => void
  nameOf: (key: string) => string
}) {
  // Only desks with an open PR of ours are listed (card #625). A link to
  // one that has none left opens it anyway, with its calm empty queue.
  const on = desks.filter(deskShown)
  const desk =
    desks.find((d) => deskOn(d) && !!open && d.settings.repoKey === open) ??
    on[0]
  if (!desk) {
    return (
      <div className="grid justify-items-start gap-2 p-5 text-[13px] text-ink-2 glass-panel">
        <span className="text-[15px] font-medium text-ink">
          No open PRs right now
        </span>
        A merge desk lists a repo&apos;s open PRs from GitHub for review. It
        never merges: each agent asks you &ldquo;Can I merge PR #N?&rdquo; on
        its own card. A desk shows here while you or your people have a PR open
        in a repo you work in.
        <Button size="sm" onClick={onSettings}>
          Merge desk settings
        </Button>
      </div>
    )
  }
  return (
    <div className="flex min-h-0 flex-1 gap-4">
      {on.length > 1 && (
        <nav
          className="hidden min-h-0 w-[220px] shrink-0 content-start gap-0.5 overflow-y-auto pb-6 md:grid"
          aria-label="Merge desks"
        >
          {on.map((d) => {
            const n = d.prs?.prs.length
            const here = d === desk
            return (
              <button
                key={d.settings.repoKey}
                type="button"
                className={cn(
                  "grid rounded-md px-2.5 py-1.5 text-left",
                  here
                    ? "bg-secondary text-ink"
                    : "text-ink-2 hover:bg-secondary/60"
                )}
                onClick={() => onOpen(d.settings.repoKey)}
              >
                <span className="truncate font-mono text-[13px]">
                  {ownerName(d.settings.repoKey)}
                </span>
                <span className="text-[11.5px] text-muted-foreground">
                  {n !== undefined && `${n} open`}
                </span>
              </button>
            )
          })}
        </nav>
      )}
      <div className="grid min-w-0 flex-1 grid-cols-[minmax(0,1fr)] content-start gap-3 overflow-y-auto pb-20 mac:overscroll-none">
        <DeskHeader desk={desk} onSettings={onSettings} />
        <DeskPanel
          desk={desk}

          nameOf={nameOf}
          onOpenCard={onOpenCard}
        />
        <p className="text-[11.5px] text-muted-foreground">
          Order: Ready for review oldest first, then the rest. Each PR&apos;s
          thread is its author&apos;s card. The desk never merges: each agent
          asks you before it merges its own PR.
        </p>
      </div>
    </div>
  )
}
