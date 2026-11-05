// DeskPage is the interim merge desk page (card #417, docs/design/
// merge-desk.md §1, §11 slice 3, mockup desk-2): every desk down the
// left, and the open desk's queue: every open PR on GitHub with its
// state, author card, and Stephen's holds and OKs. The queue panel is
// DeskPanel so slice 8 can move it into the desk's Talk channel as is.

import { useState, type ReactNode } from "react"
import {
  CheckIcon,
  CircleCheckIcon,
  CloudOffIcon,
  GitMergeIcon,
  GitPullRequestDraftIcon,
  GitPullRequestIcon,
  HandIcon,
  PauseIcon,
  PlayIcon,
  SettingsIcon,
  ShieldAlertIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import {
  deskActions,
  deskOn,
  deskQueue,
  inputsOf,
  ownerName,
  paused,
  queueCount,
  stateLabel,
  type Desk,
  type DeskState,
  type QueueLine,
} from "@/lib/desk"
import { cards } from "@/lib/cards"
import { parseTime, relativeTime } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

export function StateChip({ s }: { s: DeskState }) {
  const label = stateLabel[s]
  switch (s) {
    case "merging":
    case "running":
      return (
        <Badge variant="info">
          <Spinner className="size-3" />
          {label}
        </Badge>
      )
    case "waiting":
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
    case "held":
      return (
        <Badge variant="outline" className="border-warning/60 text-warning">
          <HandIcon />
          {label}
        </Badge>
      )
    case "outside":
      return (
        <Badge
          variant="outline"
          className="border-destructive/60 text-destructive"
        >
          <ShieldAlertIcon />
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
// card, and state, with the person's Hold and, on an Outside PR, OK.
function QueueLineRow({
  l,
  repoKey,
  nameOf,
  onOpenCard,
  onError,
}: {
  l: QueueLine
  repoKey: string
  nameOf: (key: string) => string
  onOpenCard: (id: string) => void
  onError: (e: string) => void
}) {
  const [holding, setHolding] = useState(false)
  const [note, setNote] = useState("")
  const dim = l.state === "draft" || l.state === "merged"
  const run = (p: Promise<void>) =>
    p.catch((e: unknown) => onError(errorText(e)))
  const canHold = l.state !== "merged" && l.state !== "merging"
  return (
    <div
      className={cn(
        "group grid rounded-md",
        l.state === "merging" && "bg-info-foreground/50"
      )}
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
      <div className="-mt-1 flex min-h-5 flex-wrap items-center gap-2 pr-2 pb-0.5 pl-[72px] text-[11px] text-muted-foreground">
        <span className="min-w-0 truncate">{l.note}</span>
        <span className="ml-auto flex items-center gap-1">
          {l.canOk && (
            <Button
              size="xs"
              variant="outline"
              title="Let the desk check and merge this PR at this head; a new push needs a new OK"
              onClick={() => run(deskActions.ok(repoKey, l.pr, l.headSha))}
            >
              OK this PR
            </Button>
          )}
          {l.state === "held" ? (
            <Button
              size="xs"
              variant="ghost"
              onClick={() => run(deskActions.hold(repoKey, l.pr, "", true))}
            >
              Unhold
            </Button>
          ) : (
            canHold &&
            !holding && (
              <Button
                size="xs"
                variant="ghost"
                className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100"
                title="Not this one yet: the holder skips it"
                onClick={() => setHolding(true)}
              >
                <HandIcon /> Hold
              </Button>
            )
          )}
        </span>
      </div>
      {holding && (
        <form
          className="flex items-center gap-2 pr-2 pb-1.5 pl-[72px]"
          onSubmit={(e) => {
            e.preventDefault()
            setHolding(false)
            void run(deskActions.hold(repoKey, l.pr, note.trim(), false))
            setNote("")
          }}
        >
          <Input
            autoFocus
            className="h-7 text-[12px]"
            placeholder="Why (optional), e.g. after the playtest"
            value={note}
            onChange={(e) => setNote(e.target.value)}
          />
          <Button size="xs" type="submit">
            Hold
          </Button>
          <Button
            size="xs"
            variant="ghost"
            type="button"
            onClick={() => setHolding(false)}
          >
            Cancel
          </Button>
        </form>
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
// poll it says so; with no open PRs it says GitHub has none and when it
// last asked; when GitHub can't be asked it keeps the last list.
export function DeskPanel({
  desk,
  now,
  nameOf,
  onOpenCard,
}: {
  desk: Desk
  now: Date
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
          snap
            ? down
              ? `${snap.prs.length} open at ${clock(snap.at)}`
              : `${queueCount(snap)} · polled ${timeOf(lastPoll, now)} (every 3 min)`
            : queueCount(snap)
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
          Showing the first {snap.prs.length}; GitHub has more.
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
            No open PRs on GitHub
          </div>
          <div className="text-[11.5px] text-muted-foreground">
            Last poll {clock(lastPoll)} ·{" "}
            <span className="font-mono">gh pr list --repo {repo}</span> returned
            none
          </div>
        </div>
      ) : (
        <div className="grid gap-0.5 p-1.5">
          {lines.map((l) => (
            <QueueLineRow
              key={`${l.state === "merged" ? "m" : "o"}${l.pr}`}
              l={l}
              repoKey={desk.settings.repoKey}
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

// DeskHeader: whose desk it is, its holder, and Pause desk.
function DeskHeader({
  desk,
  nameOf,
  onSettings,
}: {
  desk: Desk
  nameOf: (key: string) => string
  onSettings: () => void
}) {
  const [error, setError] = useState("")
  const p = paused(desk)
  const holder = desk.settings.holder
  return (
    <div className="grid gap-1">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="flex items-center gap-1.5 text-[17px] font-semibold text-ink">
          <GitMergeIcon className="size-5" />
          <span className="font-mono">{ownerName(desk.settings.repoKey)}</span>
        </span>
        <span className="text-[12.5px] text-muted-foreground">
          {holder ? (
            <>
              held by <span className="text-ink-2">{nameOf(holder)}</span>
            </>
          ) : (
            <span className="text-warning">
              unheld: nothing merges until you pick a holder
            </span>
          )}
        </span>
        <span className="ml-auto flex items-center gap-1.5">
          <Button
            size="sm"
            variant={p ? "default" : "outline"}
            title={
              p
                ? "Let the holder take the desk lease again"
                : "The holder takes no lease; the queue keeps following GitHub"
            }
            onClick={() =>
              deskActions
                .pause(desk.settings.repoKey, !p)
                .catch((e: unknown) => setError(errorText(e)))
            }
          >
            {p ? <PlayIcon /> : <PauseIcon />}
            {p ? "Resume desk" : "Pause desk"}
          </Button>
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
      {p && (
        <p className="text-[12px] text-warning">
          Paused{desk.pause?.at ? ` ${clock(desk.pause.at)}` : ""}: the holder
          takes no lease; the queue still follows GitHub.
        </p>
      )}
      {error && (
        <p className="text-[12px] text-destructive" role="alert">
          {error}
        </p>
      )}
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
  now,
}: {
  desks: Desk[]
  // The desk open, by repo key ("" the first).
  open: string
  onOpen: (repoKey: string) => void
  onOpenCard: (id: string) => void
  onSettings: () => void
  nameOf: (key: string) => string
  now: Date
}) {
  const on = desks.filter(deskOn)
  const desk = on.find((d) => d.settings.repoKey === open) ?? on[0]
  if (!desk) {
    return (
      <div className="grid justify-items-start gap-2 p-5 text-[13px] text-ink-2 glass-panel">
        <span className="text-[15px] font-medium text-ink">
          No merge desks yet
        </span>
        A merge desk lists a repo&apos;s open PRs from GitHub and lets one agent
        merge them in turn.
        <Button size="sm" onClick={onSettings}>
          Set one up in Settings
        </Button>
      </div>
    )
  }
  return (
    <div className="flex min-h-0 flex-1 gap-4">
      {on.length > 1 && (
        <nav
          className="hidden w-[220px] shrink-0 content-start gap-0.5 md:grid"
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
                  {d.settings.holder ? nameOf(d.settings.holder) : "unheld"}
                  {n !== undefined && ` · ${n} open`}
                  {paused(d) && " · paused"}
                </span>
              </button>
            )
          })}
        </nav>
      )}
      <div className="grid min-w-0 flex-1 content-start gap-3 overflow-y-auto pb-6">
        <DeskHeader desk={desk} nameOf={nameOf} onSettings={onSettings} />
        <DeskPanel
          desk={desk}
          now={now}
          nameOf={nameOf}
          onOpenCard={onOpenCard}
        />
        <p className="text-[11.5px] text-muted-foreground">
          Order: Merging, then Waiting on desk oldest first, then the rest. Each
          PR&apos;s thread is its author&apos;s card. Holds, pause and OKs are
          yours alone; agents can&apos;t write them.
        </p>
      </div>
    </div>
  )
}
