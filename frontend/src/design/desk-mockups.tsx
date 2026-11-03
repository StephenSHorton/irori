// Card #404: design-round mockups for the merge desk, fitted to Talk (#382).
// Browser preview only (?mockup=desk&view=1..7); static JSX on made-up
// data, no lib/ calls, no state. Lives on the mockup branch, never main.
// The small pieces and the app frame are copied from the Talk mockups
// (branch scabious/382-talk-mockups) so both rounds look the same.

import type { ReactNode } from "react"
import {
  ArrowUpRightIcon,
  AtSignIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronLeftIcon,
  CircleCheckIcon,
  CloudOffIcon,
  FileTextIcon,
  GitMergeIcon,
  GitPullRequestDraftIcon,
  GitPullRequestIcon,
  HandIcon,
  HashIcon,
  HomeIcon,
  InboxIcon,
  LayoutGridIcon,
  LockIcon,
  MessageSquareTextIcon,
  MessagesSquareIcon,
  PanelLeftIcon,
  PaperclipIcon,
  PauseIcon,
  PlusIcon,
  SearchIcon,
  SettingsIcon,
  SquarePenIcon,
  TerminalIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Spinner } from "@/components/ui/spinner"
import { cn } from "@/lib/utils"

// ---------- small pieces ----------

type Who = { name: string; kind: "person" | "agent"; tone?: string }
const ST: Who = { name: "Stephen", kind: "person" }
const BL: Who = { name: "Blake", kind: "person" }
const KO: Who = { name: "kowhai", kind: "agent", tone: "bg-chart-2" }
const PR: Who = { name: "primrose", kind: "agent", tone: "bg-chart-4" }
const KE: Who = { name: "kestrel", kind: "agent", tone: "bg-chart-3" }
const NU: Who = { name: "nutmeg", kind: "agent", tone: "bg-chart-5" }

function Av({ w, size = 7 }: { w: Who; size?: number }) {
  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center text-[11px] font-semibold text-background",
        w.kind === "person" ? "rounded-full bg-primary" : cn("rounded-md", w.tone),
        size === 7 ? "size-7" : size === 5 ? "size-5 text-[9px]" : "size-6 text-[10px]"
      )}
    >
      {w.name[0].toUpperCase()}
    </span>
  )
}

function Dot({ s }: { s: "working" | "idle" | "gone" | "online" }) {
  return (
    <span
      className={cn(
        "inline-block size-2 shrink-0 rounded-full",
        s === "working" && "bg-info",
        s === "idle" && "bg-success",
        s === "online" && "bg-success",
        s === "gone" && "bg-muted-foreground/40"
      )}
    />
  )
}

function Chip({ n, t }: { n: string; t: string }) {
  return (
    <span className="inline-flex max-w-full items-center gap-1 rounded-md bg-secondary px-1.5 py-0.5 align-middle text-[11px] text-ink-2">
      <span className="size-1.5 rounded-full bg-info" />
      <span className="font-mono text-primary">#{n}</span>
      <span className="truncate">{t}</span>
    </span>
  )
}

function M({ at, children }: { at: string; children: ReactNode }) {
  return (
    <span className="rounded bg-primary/15 px-1 font-medium text-primary">
      @{at}
      {children}
    </span>
  )
}

function Note({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "rounded-lg border border-dashed border-warning/60 bg-warning-foreground px-3 py-2 text-[12px] leading-snug text-warning",
        className
      )}
    >
      {children}
    </div>
  )
}

function Msg({
  w,
  at,
  children,
  extra,
  hl,
}: {
  w: Who
  at: string
  children: ReactNode
  extra?: ReactNode
  hl?: boolean
}) {
  return (
    <div className={cn("group flex gap-2.5 rounded-lg px-2 py-1.5", hl && "bg-secondary/70")}>
      <Av w={w} />
      <div className="grid min-w-0 flex-1 gap-0.5">
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-medium text-ink">{w.name}</span>
          {w.kind === "agent" && (
            <span className="text-[10.5px] text-muted-foreground uppercase">agent</span>
          )}
          <span className="text-[11px] text-muted-foreground tabular-nums">{at}</span>
        </div>
        <div className="text-[13px] leading-relaxed text-ink-2">{children}</div>
        {extra}
      </div>
    </div>
  )
}

function Sys({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 px-2 py-1 text-[11.5px] text-muted-foreground">
      <span className="h-px flex-1 bg-border" />
      {children}
      <span className="h-px flex-1 bg-border" />
    </div>
  )
}

function Composer({ ph, popup, agents }: { ph: string; popup?: boolean; agents?: "on" | "off" }) {
  return (
    <div className="relative">
      {popup && (
        <div className="absolute bottom-full left-3 mb-1.5 w-64 rounded-lg border border-border bg-popover p-1 text-[12.5px] shadow-xl backdrop-blur">
          {[
            [KO, "working · #330", "working"],
            [KE, "idle · PC", "idle"],
            [NU, "idle · Mac", "idle"],
          ].map(([w, s, d], i) => (
            <div
              key={i}
              className={cn(
                "flex items-center gap-2 rounded-md px-2 py-1.5",
                i === 1 && "bg-secondary"
              )}
            >
              <Av w={w as Who} size={5} />
              <span className="font-medium text-ink">{(w as Who).name}</span>
              <span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <Dot s={d as "idle"} />
                {s as string}
              </span>
            </div>
          ))}
        </div>
      )}
      <div className="flex items-center gap-2 rounded-xl border border-border bg-card px-3 py-2.5">
        <span className="flex-1 text-[13px] text-muted-foreground">{ph}</span>
        {agents && (
          <span
            className={cn(
              "flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11.5px]",
              agents === "on" ? "border-info/50 text-info" : "border-border text-muted-foreground"
            )}
          >
            <span className={cn("relative h-3 w-5 rounded-full", agents === "on" ? "bg-info" : "bg-muted")}>
              <span className={cn("absolute top-0.5 size-2 rounded-full bg-background", agents === "on" ? "right-0.5" : "left-0.5")} />
            </span>
            Agents answer now
          </span>
        )}
        <AtSignIcon className="size-4 text-muted-foreground" />
        <PaperclipIcon className="size-4 text-muted-foreground" />
        <span className="rounded-md bg-primary px-3 py-1 text-[12px] font-medium text-primary-foreground">
          Send
        </span>
      </div>
    </div>
  )
}

// ---------- the app's frame ----------

function NavRow({
  icon: I,
  label,
  current,
  count,
}: {
  icon: typeof HomeIcon
  label: string
  current?: boolean
  count?: number
}) {
  return (
    <div
      className={cn(
        "flex h-9 items-center gap-3 rounded-md px-2.5 text-sm text-ink-2",
        current && "bg-secondary font-medium text-ink"
      )}
    >
      <I className="size-4 shrink-0" />
      <span className="flex-1 truncate">{label}</span>
      {!!count && (
        <Badge variant="info" className="h-5 px-1.5 tabular-nums">
          {count}
        </Badge>
      )}
    </div>
  )
}

function Sidebar({ talk, current = "talk", desk }: { talk: boolean; current?: string; desk?: boolean }) {
  return (
    <aside className="flex h-full w-[232px] shrink-0 flex-col border-r border-border bg-sidebar/60">
      <div className="flex h-14 items-center gap-2 px-4">
        <span className="text-lg font-semibold tracking-tight text-ink">Irori</span>
        <span className="text-sm text-muted-foreground">囲炉裏</span>
        <PanelLeftIcon className="ml-auto size-4 text-muted-foreground" />
      </div>
      <nav className="flex flex-1 flex-col gap-0.5 px-2">
        <NavRow icon={HomeIcon} label="Home" current={current === "home"} />
        {talk && (
          <NavRow icon={MessagesSquareIcon} label="Talk" current={current === "talk"} count={4} />
        )}
        {desk && <NavRow icon={GitMergeIcon} label="Merge desk" current={current === "desk"} count={8} />}
        <NavRow icon={LayoutGridIcon} label="Boards" current={current === "boards"} />
        <NavRow icon={InboxIcon} label={current === "inbox" ? "Inbox" : "Held"} current={current === "inbox"} />
        <div className="px-2.5 pt-4 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Pinned boards
        </div>
        <div className="flex h-8 items-center gap-2.5 px-2.5 text-sm text-ink-2">
          <span className="rounded bg-secondary px-1 font-mono text-[10px] text-muted-foreground">gm</span>
          Game
        </div>
      </nav>
      <div className="px-4 pb-3">
        <div className="pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Agents
        </div>
        <div className="flex gap-1">
          <Badge variant="info" className="normal-case">3 working</Badge>
          <Badge variant="slate" className="normal-case">2 free</Badge>
        </div>
      </div>
      <div className="flex items-center border-t border-border px-4 py-3 text-xs text-ink-2">
        Stephen&apos;s PC
        <SettingsIcon className="ml-auto size-4 text-muted-foreground" />
      </div>
    </aside>
  )
}

function Header({ title, tag, right }: { title: ReactNode; tag?: string; right?: ReactNode }) {
  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border px-6">
      <h1 className="text-xl font-semibold text-ink">{title}</h1>
      {tag && (
        <span className="rounded-md bg-warning-foreground px-1.5 py-0.5 text-[10.5px] font-medium tracking-wide text-warning uppercase">
          {tag}
        </span>
      )}
      <div className="ml-auto flex items-center gap-2">{right}</div>
    </header>
  )
}

function Window({
  children,
  talk = true,
  current,
  className,
  desk,
}: {
  children: ReactNode
  talk?: boolean
  current?: string
  className?: string
  desk?: boolean
}) {
  return (
    <div className={cn("flex h-full overflow-hidden", className)}>
      <Sidebar talk={talk} current={current} desk={desk} />
      <div className="flex min-w-0 flex-1 flex-col">{children}</div>
    </div>
  )
}

// ---------- the Talk list (left column of the Talk page) ----------

function ListRow({
  icon,
  label,
  sub,
  unread,
  current,
  muted,
}: {
  icon: ReactNode
  label: ReactNode
  sub?: ReactNode
  unread?: number | "@"
  current?: boolean
  muted?: boolean
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-[13px]",
        current && "bg-secondary",
        muted ? "text-muted-foreground" : "text-ink-2"
      )}
    >
      {icon}
      <span className="grid min-w-0 flex-1">
        <span className={cn("truncate", unread && "font-medium text-ink")}>{label}</span>
        {sub && <span className="truncate text-[11px] text-muted-foreground">{sub}</span>}
      </span>
      {unread === "@" ? (
        <span className="flex size-5 items-center justify-center rounded-full bg-destructive text-[10px] font-semibold text-destructive-foreground">
          @
        </span>
      ) : unread ? (
        <span className="rounded-full bg-primary px-1.5 text-[10.5px] font-semibold text-primary-foreground tabular-nums">
          {unread}
        </span>
      ) : null}
    </div>
  )
}

function Group({ label, children, add }: { label: string; children: ReactNode; add?: boolean }) {
  return (
    <div className="grid gap-0.5">
      <div className="flex items-center px-2 pt-3 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {label}
        {add && <PlusIcon className="ml-auto size-3.5" />}
      </div>
      {children}
    </div>
  )
}


function Frame({ title, pick, children, caption }: { title: string; pick?: boolean; children: ReactNode; caption: ReactNode }) {
  return (
    <div className="grid min-w-0 grid-rows-[auto_1fr_auto] gap-2">
      <div className="flex items-center gap-2">
        <span className="text-[15px] font-semibold text-ink">{title}</span>
        {pick && <Badge variant="success">My pick</Badge>}
      </div>
      <div
        className={cn(
          "h-[499px] overflow-hidden rounded-xl border bg-background",
          pick ? "border-success/70 ring-2 ring-success/30" : "border-border"
        )}
      >
        <div className="h-[900px] w-[1100px] origin-top-left scale-[0.552]">{children}</div>
      </div>
      <div className="text-[12.5px] leading-snug text-ink-2">{caption}</div>
    </div>
  )
}

function Phone({ children, title, back }: { children: ReactNode; title: ReactNode; back?: boolean }) {
  return (
    <div className="flex h-[800px] w-[380px] shrink-0 flex-col overflow-hidden rounded-[36px] border-[6px] border-secondary bg-background">
      <div className="flex h-8 items-end justify-center text-[11px] text-muted-foreground">9:41</div>
      <div className="flex h-12 items-center gap-2 border-b border-border px-3">
        {back ? <ChevronLeftIcon className="size-5 text-primary" /> : <PanelLeftIcon className="size-5 text-muted-foreground" />}
        <span className="text-[16px] font-semibold text-ink">{title}</span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      <div className="flex h-16 items-start justify-around border-t border-border pt-2 text-[10.5px] text-muted-foreground">
        {[
          [HomeIcon, "Home"],
          [MessagesSquareIcon, "Talk"],
          [LayoutGridIcon, "Boards"],
        ].map(([I, l], i) => {
          const Icon = I as typeof HomeIcon
          return (
            <span key={i} className={cn("grid justify-items-center gap-0.5", i === 1 && "text-primary")}>
              <Icon className="size-5" />
              {l as string}
            </span>
          )
        })}
      </div>
    </div>
  )
}


// ======================================================================
// Merge desk (#404)
// ======================================================================

const SC: Who = { name: "scabious", kind: "agent", tone: "bg-chart-1" }
const REPO = "you/game-repo"

// Irori's own automatic posts (SWEEPER comments today).
function Auto({ at, children, extra, hl }: { at: string; children: ReactNode; extra?: ReactNode; hl?: boolean }) {
  return (
    <div className={cn("flex gap-2.5 rounded-lg px-2 py-1.5", hl && "bg-success-foreground/60")}>
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-border bg-secondary text-muted-foreground">
        <GitMergeIcon className="size-3.5" />
      </span>
      <div className="grid min-w-0 flex-1 gap-0.5">
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-medium text-ink">Irori</span>
          <span className="text-[10.5px] text-muted-foreground uppercase">automatic</span>
          <span className="text-[11px] text-muted-foreground tabular-nums">{at}</span>
        </div>
        <div className="text-[13px] leading-relaxed text-ink-2">{children}</div>
        {extra}
      </div>
    </div>
  )
}

function Sha({ s }: { s: string }) {
  return <code className="rounded bg-secondary px-1 font-mono text-[12px] text-ink">{s}</code>
}

type St = "merging" | "waiting" | "rebase" | "running" | "failed" | "draft" | "held" | "merged"

function State({ s, small }: { s: St; small?: boolean }) {
  const c = small ? "h-[18px] px-1 text-[9.5px]" : ""
  switch (s) {
    case "merging":
      return <Badge variant="info" className={c}><Spinner className="size-3" />Merging</Badge>
    case "running":
      return <Badge variant="info" className={c}><Spinner className="size-3" />Checks running</Badge>
    case "waiting":
      return <Badge variant="slate" className={c}>Waiting on desk</Badge>
    case "rebase":
      return <Badge variant="warning" className={c}>Needs rebase</Badge>
    case "failed":
      return <Badge variant="destructive" className={c}>Checks failed</Badge>
    case "draft":
      return <Badge variant="outline" className={cn(c, "text-muted-foreground")}>Draft</Badge>
    case "held":
      return <Badge variant="outline" className={cn(c, "border-warning/60 text-warning")}><HandIcon />Held</Badge>
    case "merged":
      return <Badge variant="success" className={c}><CheckIcon />Merged</Badge>
  }
}

type Pr = { n: number; t: string; a: Who; card: string; s: St; note: string }

const QUEUE: Pr[] = [
  { n: 212, t: "Boat wake follows the hull", a: PR, card: "41", s: "merging", note: "kowhai · lease desk:game-repo · head 3f2a9c1" },
  { n: 218, t: "Shop prices from one table", a: SC, card: "49", s: "waiting", note: "next · opened 3h ago" },
  { n: 219, t: "Fix lantern flicker", a: BL, card: "52", s: "waiting", note: "card #52 made by Irori (no author card)" },
  { n: 220, t: "Hookshot range +2 tiles", a: PR, card: "53", s: "running", note: "GitHub checks 1 of 3" },
  { n: 215, t: "Enemy camp: second patrol", a: KE, card: "44", s: "rebase", note: "GitHub: BEHIND · asked on #44" },
  { n: 216, t: "Save slots: keep the last 3", a: NU, card: "47", s: "failed", note: "cargo test: 2 failed · output on #47" },
  { n: 217, t: "Night fog tweak", a: KE, card: "45", s: "held", note: "held by Stephen: “after the playtest”" },
  { n: 221, t: "New title screen", a: NU, card: "50", s: "draft", note: "draft · listed, never queued" },
]

function QueueRow({ p, dim, expand }: { p: Pr; dim?: boolean; expand?: ReactNode }) {
  const draft = p.s === "draft"
  return (
    <div className={cn("grid", (draft || dim) && "opacity-55")}>
      <div className={cn("flex items-center gap-2 rounded-md px-2 py-1 text-[12.5px]", p.s === "merging" && "bg-info-foreground/50")}>
        {draft ? (
          <GitPullRequestDraftIcon className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <GitPullRequestIcon className="size-4 shrink-0 text-success" />
        )}
        <span className="w-9 shrink-0 font-mono text-primary">#{p.n}</span>
        <span className="min-w-0 flex-1 truncate font-medium text-ink">{p.t}</span>
        <span className="flex shrink-0 items-center gap-1 text-[11.5px] text-ink-2">
          <Av w={p.a} size={5} />
          {p.a.name}
        </span>
        <span className="shrink-0 rounded bg-secondary px-1 font-mono text-[11px] text-primary">#{p.card}</span>
        <span className="flex w-[124px] shrink-0 justify-end">
          <State s={p.s} />
        </span>
      </div>
      <div className="-mt-1 pr-2 pb-0.5 pl-[72px] text-[11px] text-muted-foreground">{p.note}</div>
      {expand}
    </div>
  )
}

function QueueHead({ count, right }: { count: ReactNode; right?: ReactNode }) {
  return (
    <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-[12px]">
      <span className="font-medium text-ink">Queue</span>
      <span className="text-muted-foreground">{count}</span>
      <span className="ml-auto flex items-center gap-2 text-muted-foreground">{right}</span>
    </div>
  )
}

function QueuePanel({ rows = QUEUE, max, expand }: { rows?: Pr[]; max?: number; expand?: Record<number, ReactNode> }) {
  const shown = max ? rows.slice(0, max) : rows
  return (
    <div className="mx-4 mt-3 overflow-hidden rounded-xl border border-border bg-card">
      <QueueHead
        count={`${rows.length} open on GitHub · polled 14:02 (every 3 min)`}
        right={
          <>
            <ChevronDownIcon className="size-3.5" /> collapse
          </>
        }
      />
      <div className="grid gap-0.5 p-1.5">
        {shown.map((p) => (
          <QueueRow key={p.n} p={p} expand={expand?.[p.n]} />
        ))}
        {max && rows.length > max && (
          <div className="px-2 py-1 text-[11.5px] text-muted-foreground">+ {rows.length - max} more</div>
        )}
      </div>
    </div>
  )
}

function Pointer({ children }: { children: ReactNode }) {
  return (
    <div className="ml-11 flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-1.5 text-[12px] text-ink-2">
      <MessageSquareTextIcon className="size-3.5 text-muted-foreground" />
      {children}
    </div>
  )
}

function DeskHeader() {
  return (
    <Header
      title={
        <span className="flex items-center gap-1.5">
          <GitMergeIcon className="size-5" /> merge-game-repo
        </span>
      }
      right={
        <span className="flex items-center gap-3 text-[12.5px] text-muted-foreground">
          desk for <span className="font-mono text-ink-2">{REPO}</span>
          <span className="flex items-center gap-1.5">
            held by <Av w={KO} size={5} /> <span className="text-ink-2">kowhai</span>
          </span>
          <span className="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-ink-2">
            <PauseIcon className="size-3.5" /> Pause desk
          </span>
        </span>
      }
    />
  )
}

function DeskTalkList({ current = "desk" }: { current?: string }) {
  const hash = <HashIcon className="size-4 shrink-0 text-muted-foreground" />
  const thr = <MessageSquareTextIcon className="size-4 shrink-0 text-muted-foreground" />
  const desk = <GitMergeIcon className="size-4 shrink-0 text-muted-foreground" />
  return (
    <div className="flex w-[260px] shrink-0 flex-col gap-1 overflow-hidden border-r border-border px-2 py-3">
      <div className="mx-1 flex items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-1.5 text-[12.5px] text-muted-foreground">
        <SearchIcon className="size-3.5" /> Search talk
        <SquarePenIcon className="ml-auto size-4 text-ink-2" />
      </div>
      <Group label="Threads · 2 open" add>
        <ListRow icon={thr} label="Why is the Mac build slow?" sub="waiting for a free agent" />
        <ListRow icon={thr} label="Lobby list: Steam or relay?" sub="you answered" />
      </Group>
      <Group label="Channels" add>
        <ListRow icon={desk} label="merge-game-repo" sub="desk · kowhai · 8 open" unread={current === "desk" ? undefined : 8} current={current === "desk"} />
        <ListRow icon={desk} label="merge-app-repo" sub="desk · primrose · 2 open" unread={2} />
        <ListRow icon={hash} label="releases" unread="@" />
        <ListRow icon={hash} label="general" />
      </Group>
      <Group label="Direct messages" add>
        <ListRow icon={<Av w={KO} size={5} />} label="kowhai" sub="holding the game-repo desk" />
        <ListRow icon={<Av w={BL} size={5} />} label="Blake" />
      </Group>
      <div className="mt-auto px-2 pt-2 text-[11px] text-muted-foreground">
        Desks list with channels; the count is the open PRs.
      </div>
    </div>
  )
}

function DeskLog({ upto = 99 }: { upto?: number }) {
  const all = [
    <Msg key="a" w={KO} at="13:40">
      Merged #210 as <Sha s="e81b0c4" />. Next up: #212.
    </Msg>,
    <Auto key="b" at="13:41">
      #215 needs rebase (GitHub: BEHIND main). Asked kestrel on <Chip n="44" t="Enemy camp: second patrol" />
    </Auto>,
    <Msg key="c" w={KO} at="13:52">
      #216 checks failed: <code className="font-mono text-[12px]">cargo test</code>, 2 failures. Output is on <Chip n="47" t="Save slots" /> for nutmeg.
    </Msg>,
    <Msg key="d" w={ST} at="13:55" extra={<span className="mt-1 flex items-center gap-1.5 text-[11.5px] text-warning"><HandIcon className="size-3.5" /> Stephen held #217 from its queue line</span>}>
      Leave the fog one until after the playtest.
    </Msg>,
    <Msg key="e" w={KO} at="14:01">
      Taking #212: trial squash is clean on main @ <Sha s="e81b0c4" />, checks running.
    </Msg>,
    <Pointer key="f">
      PR thread: <span className="font-medium text-ink">#41 Boat wake follows the hull</span>
      <span className="text-muted-foreground">· primrose&apos;s card · PR #212</span>
    </Pointer>,
  ]
  return <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-2">{all.slice(0, upto)}</div>
}

// ---------- 1: three shapes ----------

function View1() {
  return (
    <div className="grid grid-cols-3 gap-6 p-6">
      <Frame
        title="A · A desk channel in Talk"
        pick
        caption={
          <>
            <b>#merge-game-repo</b> is a Talk channel marked as a desk. The queue is a live panel
            pinned over the holder&apos;s log, fed from GitHub, not from posts. Each PR line opens
            its author&apos;s card, which is the PR thread. Until Talk 1 (#389) lands, the same panel
            ships as B&apos;s page.
          </>
        }
      >
        <Window>
          <DeskHeader />
          <div className="flex min-h-0 flex-1">
            <DeskTalkList />
            <div className="flex min-w-0 flex-1 flex-col">
              <QueuePanel max={5} />
              <DeskLog upto={3} />
              <div className="p-4"><Composer ph="Message #merge-game-repo" /></div>
            </div>
          </div>
        </Window>
      </Frame>
      <Frame
        title="B · A desk page"
        caption={
          <>
            The same queue panel as its own sidebar page, one tab per repo. PR threads are still
            the author cards. Ships before Talk 1, but it is one more page and the holder&apos;s
            running notes (&ldquo;main is red, holding&rdquo;) have nowhere to go.
          </>
        }
      >
        <Window current="desk" desk talk={false}>
          <Header title="Merge desk" right={<span className="text-[12.5px] text-muted-foreground">held by kowhai</span>} />
          <div className="flex gap-1 border-b border-border px-6 pt-2 text-[13px]">
            {["game-repo 8", "app-repo 2"].map((t, i) => (
              <span key={t} className={cn("rounded-t-md px-3 py-2 text-ink-2", i === 0 && "border-b-2 border-primary font-medium text-ink")}>
                {t}
              </span>
            ))}
          </div>
          <QueuePanel />
          <Note className="mx-4 mt-3">No log here: the holder&apos;s notes go on the PR cards only.</Note>
        </Window>
      </Frame>
      <Frame
        title="C · A standing card per repo (wind#4)"
        caption={
          <>
            Irori posts each new PR as a comment on one long card and the holder works from it.
            Smallest build, but it is what failed: the card knows only what is posted to it, and
            each PR&apos;s state is a comment that goes stale.
          </>
        }
      >
        <Window current="boards">
          <Header title="Card #30" />
          <div className="grid content-start gap-3 p-6">
            <div className="text-lg font-semibold text-ink">game-repo merge desk</div>
            <div className="flex gap-2 text-[12px] text-muted-foreground">
              <Badge variant="info">Working</Badge> held by kowhai · 214 comments
            </div>
            <div className="grid gap-2.5 text-[12.5px]">
              {[
                ["2h", "Irori", "New PR #215 Enemy camp: second patrol (kestrel)"],
                ["2h", "kowhai", "#215 needs rebase, kestrel please rebase"],
                ["1h", "Irori", "New PR #216 Save slots: keep the last 3 (nutmeg)"],
                ["40m", "kowhai", "#216 checks failed (see above for the output…)"],
                ["20m", "kowhai", "merging #212"],
              ].map(([t, a, x], i) => (
                <div key={i} className="grid grid-cols-[44px_1fr] gap-2">
                  <span className="text-muted-foreground tabular-nums">{t}</span>
                  <span className="text-ink-2"><span className="font-medium text-ink">{a}</span> {x}</span>
                </div>
              ))}
            </div>
            <Note>
              #215 was rebased 20 minutes ago; the card still says it needs one. #219 (a person&apos;s
              PR) was never posted here at all.
            </Note>
          </div>
        </Window>
      </Frame>
    </div>
  )
}

// ---------- 2: the desk in Talk ----------

function EmptyPanel() {
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-card">
      <QueueHead count="0 open on GitHub" />
      <div className="grid justify-items-center gap-1 px-4 py-6 text-center">
        <CircleCheckIcon className="size-6 text-success" />
        <div className="text-[13.5px] font-medium text-ink">No open PRs on GitHub</div>
        <div className="text-[11.5px] text-muted-foreground">
          Last poll 14:02 · <span className="font-mono">gh pr list --repo {REPO}</span> returned none
        </div>
      </div>
    </div>
  )
}

function UnreachablePanel() {
  return (
    <div className="overflow-hidden rounded-xl border border-warning/50 bg-card">
      <QueueHead count="5 open at 13:53" />
      <div className="flex items-start gap-2 px-3 py-3 text-[12px] text-warning">
        <CloudOffIcon className="mt-0.5 size-4 shrink-0" />
        <span>
          GitHub unreachable since 13:56. Showing the 13:53 list; never an empty queue while
          GitHub can&apos;t be asked.
        </span>
      </div>
      <div className="grid gap-1 px-3 pb-3 text-[12px] text-ink-2 opacity-60">
        {QUEUE.slice(0, 3).map((p) => (
          <span key={p.n} className="flex items-center gap-2 truncate">
            <GitPullRequestIcon className="size-3.5 shrink-0 text-success" />
            <span className="font-mono text-primary">#{p.n}</span>
            <span className="truncate">{p.t}</span>
          </span>
        ))}
        <span className="text-muted-foreground">+ 2 more</span>
      </div>
    </div>
  )
}

function View2() {
  return (
    <Window>
      <DeskHeader />
      <div className="flex min-h-0 flex-1">
        <DeskTalkList />
        <div className="flex min-w-0 flex-1 flex-col">
          <QueuePanel />
          <DeskLog />
          <div className="px-4 pb-4"><Composer ph="Message #merge-game-repo · @kowhai wakes the holder" agents="off" /></div>
        </div>
        <div className="flex w-[300px] shrink-0 flex-col gap-3 border-l border-border px-3 py-3">
          <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
            The same panel, other days
          </div>
          <div className="text-[12px] text-ink-2">When GitHub has no open PRs:</div>
          <EmptyPanel />
          <div className="text-[12px] text-ink-2">When the poll fails:</div>
          <UnreachablePanel />
          <Note>
            Order: Merging, then Waiting on desk oldest first, then the rest. A plain post here
            wakes nobody; @kowhai wakes the holder.
          </Note>
        </div>
      </div>
    </Window>
  )
}

// ---------- 3: a PR thread is the author's card ----------

function Attachment() {
  return (
    <div className="mt-1.5 overflow-hidden rounded-lg border border-border bg-secondary/50">
      <div className="flex items-center gap-2 border-b border-border px-2.5 py-1.5 text-[11.5px] text-ink-2">
        <FileTextIcon className="size-3.5" /> checks-9c41e07.log
        <span className="text-muted-foreground">· 412 lines · line 1: cargo test --workspace</span>
        <PaperclipIcon className="ml-auto size-3.5 text-muted-foreground" />
      </div>
      <pre className="px-2.5 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-ink-2">
{`failures:
    camp::tests::patrol_spawns_once
    camp::tests::two_camps_share_route
test result: FAILED. 186 passed; 2 failed; 0 ignored
error: test failed, to rerun pass \`-p game --lib\``}
      </pre>
    </div>
  )
}

function CardTop({ n, title, who, status, steps, pr }: { n: string; title: string; who: Who; status: ReactNode; steps: string; pr: ReactNode }) {
  return (
    <div className="grid gap-2 border-b border-border px-5 py-4">
      <div className="flex items-baseline gap-2">
        <span className="font-mono text-[13px] text-muted-foreground">#{n}</span>
        <span className="text-lg font-semibold text-ink">{title}</span>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-[12px] text-muted-foreground">
        {status}
        <span className="flex items-center gap-1">held by <Av w={who} size={5} /> <span className="text-ink-2">{who.name}</span></span>
        · {steps}
        <span className="flex items-center gap-1">· watching: <Av w={KO} size={5} /> kowhai (desk)</span>
      </div>
      {pr}
    </div>
  )
}

function PrLink({ n, state, sha }: { n: number; state: ReactNode; sha: string }) {
  return (
    <div className="flex w-fit items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-1.5 text-[12px] text-ink-2">
      <GitPullRequestIcon className="size-4 text-success" />
      <span className="font-mono text-primary">PR #{n}</span>
      <span className="font-mono text-muted-foreground">{REPO}</span>
      · head <Sha s={sha} /> · on the <span className="flex items-center gap-1 text-ink"><GitMergeIcon className="size-3.5" />game-repo desk</span>
      {state}
    </div>
  )
}

function View3() {
  return (
    <Window current="boards">
      <Header title="Card #44" right={<span className="text-[12.5px] text-muted-foreground">board: Game · thread = this card&apos;s comments</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <CardTop
            n="44"
            title="Enemy camp: second patrol"
            who={KE}
            status={<Badge variant="info">In review</Badge>}
            steps="5/5 steps · branch kestrel/44-second-patrol"
            pr={<PrLink n={215} sha="b7d20a3" state={<State s="waiting" small />} />}
          />
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-2">
            <Auto at="11:20">PR #215 is on the game-repo merge desk.</Auto>
            <Auto at="13:41">
              Needs rebase: GitHub says BEHIND main (3 commits). <M at="kestrel">{""}</M>
            </Auto>
            <Msg w={KE} at="13:44">Rebasing on main now.</Msg>
            <Auto at="13:47">pushed <Sha s="9c41e07" /></Auto>
            <Msg w={KO} at="13:52" extra={<Attachment />}>
              <M at="kestrel">{""}</M> checks failed on <Sha s="9c41e07" />: 2 tests in <code className="font-mono text-[12px]">camp::tests</code>. Full output attached.
            </Msg>
            <Msg w={KE} at="13:58">
              <M at="kowhai">{""}</M> both tests assumed one camp per map. Fixed in <Sha s="b7d20a3" />; the second test now builds two camps.
            </Msg>
            <Auto at="13:58">pushed <Sha s="b7d20a3" /> · woke kowhai (this PR was in Checks failed)</Auto>
            <Msg w={KO} at="14:00">Checks pass on <Sha s="b7d20a3" />. Back in the queue, after #218.</Msg>
          </div>
          <div className="p-4"><Composer ph="Comment on #44" agents="on" /></div>
        </div>
        <div className="flex w-[420px] shrink-0 flex-col gap-3 border-l border-border px-4 py-4 text-[12.5px] leading-snug text-ink-2">
          <div className="text-[15px] font-semibold text-ink">One PR, one thread: the author&apos;s card</div>
          <p>In Talk a thread is a card, so the PR&apos;s conversation is the card it was made on. No second thread system.</p>
          <div className="grid gap-1.5">
            <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Irori posts by itself</div>
            {[
              ["Opened", "once, when the PR first maps to a card"],
              ["Needs rebase", "when GitHub says BEHIND or DIRTY; wakes the author"],
              ["pushed <sha>", "when the head moves; wakes the desk holder only after a rebase ask or failed checks"],
              ["Merged as <sha>", "after merge (desk-5)"],
            ].map(([a, b]) => (
              <div key={a} className="grid grid-cols-[110px_1fr] gap-2">
                <span className="font-medium text-ink">{a}</span>
                <span>{b}</span>
              </div>
            ))}
          </div>
          <div className="grid gap-1.5">
            <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">The desk holder posts</div>
            <div className="grid grid-cols-[110px_1fr] gap-2">
              <span className="font-medium text-ink">Checks failed</span>
              <span>the failing line&apos;s tail inline, the full log attached; names the author so it wakes</span>
            </div>
            <div className="grid grid-cols-[110px_1fr] gap-2">
              <span className="font-medium text-ink">Trial squash failed</span>
              <span>the conflict, as a rebase ask</span>
            </div>
          </div>
          <Note>
            The author and holder talk with ordinary card comments; each names the other, so
            each wakes. &ldquo;Agents answer now&rdquo; works here as on any thread. A PR with no
            author card gets a &ldquo;PR #n&rdquo; card from Irori (like #52 for Blake&apos;s #219).
          </Note>
        </div>
      </div>
    </Window>
  )
}

// ---------- 4: the holder mid-run ----------

function RunStep({ n, done, now, children }: { n: number; done?: boolean; now?: boolean; children: ReactNode }) {
  return (
    <div className={cn("flex items-start gap-2 text-[12.5px]", !done && !now && "text-muted-foreground")}>
      <span
        className={cn(
          "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full text-[9.5px] font-semibold",
          done && "bg-success text-background",
          now && "bg-info text-background",
          !done && !now && "border border-border"
        )}
      >
        {done ? <CheckIcon className="size-3" /> : n}
      </span>
      <span className={cn(now && "text-ink")}>{children}</span>
    </div>
  )
}

function MergingDetail() {
  return (
    <div className="mx-2 mb-1 grid gap-1 rounded-lg border border-info/40 bg-background/60 px-3 py-2 text-[11.5px] text-ink-2">
      <div className="flex items-center gap-2">
        <CheckIcon className="size-3.5 text-success" /> Trial squash on origin/main <Sha s="e81b0c4" />: clean
      </div>
      <div className="flex items-center gap-2">
        <Spinner className="size-3.5 text-info" /> Line 1 <code className="font-mono">cargo test --workspace &amp;&amp; bun run typecheck</code>
        <span className="text-muted-foreground">· 2m 10s</span>
      </div>
      <div className="flex items-center gap-2 text-muted-foreground">
        <span className="size-3.5" /> Line 2 (Mac, nutmeg) not needed: the diff doesn&apos;t touch <code className="font-mono">app/macos/**</code>
      </div>
      <div className="flex items-center gap-2 text-muted-foreground">
        <span className="size-3.5" /> then <code className="font-mono">gh pr merge 212 --squash --match-head-commit 3f2a9c1</code>
      </div>
    </div>
  )
}

function View4() {
  return (
    <Window>
      <DeskHeader />
      <div className="flex min-h-0 flex-1">
        <DeskTalkList />
        <div className="flex min-w-0 flex-1 flex-col">
          <QueuePanel max={5} expand={{ 212: <MergingDetail /> }} />
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-2">
            <Msg w={KO} at="13:40">Merged #210 as <Sha s="e81b0c4" />. Next up: #212.</Msg>
            <Msg w={KO} at="14:01">
              Taking #212: trial squash is clean on main @ <Sha s="e81b0c4" />, checks running.
            </Msg>
          </div>
          <div className="px-4 pb-4"><Composer ph="Message #merge-game-repo" /></div>
        </div>
        <div className="flex w-[380px] shrink-0 flex-col gap-3 border-l border-border px-4 py-3">
          <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">The desk lease</div>
          <div className="grid gap-1.5 rounded-xl border border-info/50 bg-card p-3 text-[12.5px] text-ink-2">
            <div className="flex items-center gap-2 font-mono text-[13px] text-ink">
              <LockIcon className="size-4 text-info" /> desk:game-repo
            </div>
            <div>held by <b className="text-ink">kowhai</b> for <span className="font-mono text-primary">PR #212</span> @ <Sha s="3f2a9c1" /></div>
            <div className="text-muted-foreground">taken 14:01 · renewed 14:03 · lapses 14:33 unless renewed</div>
            <div className="text-muted-foreground">a second take is refused until it&apos;s released</div>
            <div className="mt-1 flex items-center gap-2 rounded-md bg-secondary px-2 py-1 text-[11.5px]">
              <Av w={NU} size={5} /> <span className="font-mono text-primary">#55</span> waits_on lease desk:game-repo
            </div>
          </div>
          <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">kowhai&apos;s run (in its wake text)</div>
          <div className="grid gap-1.5">
            <RunStep n={1} done>Take <code className="font-mono">desk:game-repo</code> for the oldest Waiting on desk (#212)</RunStep>
            <RunStep n={2} done>Trial-squash on fresh origin/main in its own worktree</RunStep>
            <RunStep n={3} now>Run the check lines; path lines only when the diff touches them</RunStep>
            <RunStep n={4}>Read the diff; anything for the author goes on the PR thread</RunStep>
            <RunStep n={5}>Merge with <code className="font-mono">--match-head-commit</code> the sha it checked</RunStep>
            <RunStep n={6}>Release the lease, one line in this log</RunStep>
          </div>
          <div className="flex items-center gap-2 rounded-lg border border-border bg-secondary/50 px-2.5 py-2 font-mono text-[11px] text-ink-2">
            <TerminalIcon className="size-3.5 shrink-0" /> test camp::tests::patrol_spawns_once ... ok
          </div>
          <Note>
            Any step that fails: post on the PR thread, release, take the next one. A push after
            step 2 makes step 5 fail instead of merging untested code. Tags and releases stay with
            Stephen.
          </Note>
        </div>
      </div>
    </Window>
  )
}

// ---------- 5: after merge ----------

function View5() {
  const merged: Pr = { ...QUEUE[0], s: "merged", note: "merged as 3f2a9c1 by kowhai · drops off the list next session" }
  const next: Pr = { ...QUEUE[1], s: "merging", note: "kowhai · lease desk:game-repo · head 7d01b2e" }
  return (
    <Window current="boards">
      <Header title="Card #41" right={<span className="text-[12.5px] text-muted-foreground">primrose&apos;s card · the PR #212 thread</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <CardTop
            n="41"
            title="Boat wake follows the hull"
            who={PR}
            status={<Badge variant="success"><CheckIcon />Done</Badge>}
            steps="4/4 steps · proof 3f2a9c1"
            pr={<PrLink n={212} sha="3f2a9c1" state={<State s="merged" small />} />}
          />
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-2">
            <Auto at="10:05">PR #212 is on the game-repo merge desk.</Auto>
            <Msg w={KO} at="14:01">Taking #212 now.</Msg>
            <Auto
              at="14:06"
              hl
              extra={<span className="mt-1 text-[11.5px] text-muted-foreground">from GitHub&apos;s mergeCommit, read right after the merge; nobody typed the sha</span>}
            >
              <b className="text-ink">Merged as <Sha s="3f2a9c1" /></b> by kowhai (squash) · PR #212 · <M at="primrose">{""}</M>
            </Auto>
            <Msg w={PR} at="14:07">Completed with proof: <Sha s="3f2a9c1" /> on main.</Msg>
            <Sys>primrose completed #41 · Done</Sys>
          </div>
        </div>
        <div className="flex w-[560px] shrink-0 flex-col border-l border-border">
          <div className="flex h-11 items-center gap-2 border-b border-border px-4 text-sm font-medium text-ink">
            <GitMergeIcon className="size-4" /> merge-game-repo
            <span className="ml-auto text-[12px] font-normal text-muted-foreground">the desk, same moment</span>
          </div>
          <div className="mx-4 mt-3 overflow-hidden rounded-xl border border-border bg-card">
            <QueueHead count="7 open on GitHub · polled 14:06" />
            <div className="grid gap-0.5 p-1.5">
              <QueueRow p={merged} dim />
              <QueueRow p={next} />
              {QUEUE.slice(2, 4).map((p) => <QueueRow key={p.n} p={p} />)}
            </div>
          </div>
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-2">
            <Auto at="14:06" hl>
              Merged #212 as <Sha s="3f2a9c1" /> · posted on <Chip n="41" t="Boat wake follows the hull" />
            </Auto>
            <Msg w={KO} at="14:06">Released the lease. Taking #218.</Msg>
          </div>
          <Note className="m-4">
            The card completes the usual way: prwatch sees MERGED and wakes primrose to complete
            with proof. A &ldquo;PR #n&rdquo; card Irori made completes itself on merge.
          </Note>
        </div>
      </div>
    </Window>
  )
}

// ---------- 6: settings ----------

function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="grid grid-cols-[150px_1fr] items-start gap-3 py-2">
      <span className="pt-1 text-[12.5px] text-muted-foreground">{label}</span>
      <div className="grid gap-1">
        {children}
        {hint && <span className="text-[11.5px] text-muted-foreground">{hint}</span>}
      </div>
    </div>
  )
}

function Toggle({ on }: { on: boolean }) {
  return (
    <span className={cn("relative inline-block h-5 w-9 rounded-full", on ? "bg-success" : "bg-muted")}>
      <span className={cn("absolute top-0.5 size-4 rounded-full bg-background", on ? "right-0.5" : "left-0.5")} />
    </span>
  )
}

function Select({ children }: { children: ReactNode }) {
  return (
    <span className="flex w-fit items-center gap-2 rounded-md border border-border bg-card px-2.5 py-1 text-[12.5px] text-ink">
      {children}
      <ChevronDownIcon className="size-3.5 text-muted-foreground" />
    </span>
  )
}

function CheckLine({ n, cmd, when, runner }: { n: number; cmd: string; when: ReactNode; runner: ReactNode }) {
  return (
    <div className="grid grid-cols-[22px_1fr_190px_150px] items-center gap-2 rounded-md border border-border bg-card px-2.5 py-2 text-[12px]">
      <span className="text-muted-foreground">{n}</span>
      <code className="truncate font-mono text-[12px] text-ink">{cmd}</code>
      <span className="text-ink-2">{when}</span>
      <span className="flex items-center gap-1.5 text-ink-2">{runner}</span>
    </div>
  )
}

function View6() {
  return (
    <Window current="settings">
      <Header title="Settings" />
      <div className="flex min-h-0 flex-1">
        <div className="grid w-[200px] shrink-0 content-start gap-0.5 border-r border-border p-3 text-[13px] text-ink-2">
          {["General", "Appearance", "Agents", "Reviewers", "Merge desks", "Phone"].map((t) => (
            <span key={t} className={cn("rounded-md px-2.5 py-1.5", t === "Merge desks" && "bg-secondary font-medium text-ink")}>{t}</span>
          ))}
        </div>
        <div className="grid min-w-0 flex-1 content-start gap-4 overflow-hidden px-8 py-5">
          <div className="flex items-baseline gap-3">
            <span className="text-[17px] font-semibold text-ink">Merge desks</span>
            <span className="text-[12.5px] text-muted-foreground">one per repo · set once · open PRs come from GitHub</span>
          </div>

          <div className="rounded-xl border border-success/50 bg-card px-5 py-3">
            <div className="flex items-center gap-3 border-b border-border pb-2">
              <GitMergeIcon className="size-4 text-ink" />
              <span className="font-mono text-[14px] text-ink">{REPO}</span>
              <span className="text-[12px] text-muted-foreground">channel #merge-game-repo</span>
              <span className="ml-auto flex items-center gap-2 text-[12.5px] text-ink-2">Desk on <Toggle on /></span>
            </div>
            <Field label="Holder" hint="Picked from your agents, like a repo's reviewer. If it goes, the desk shows unheld and nothing merges.">
              <Select><Av w={KO} size={5} /> kowhai <span className="text-muted-foreground">· PC</span></Select>
            </Field>
            <Field
              label="Check lines"
              hint={
                <span className="flex items-center gap-2">
                  <span className="rounded-md border border-border px-2 py-0.5 text-ink-2">Read the checks line from CLAUDE.md</span>
                  a starting value only; never re-read on its own
                </span>
              }
            >
              <div className="grid grid-cols-[22px_1fr_190px_150px] gap-2 px-2.5 text-[11px] text-muted-foreground uppercase">
                <span />
                <span>Command</span>
                <span>Runs when</span>
                <span>Runs on</span>
              </div>
              <CheckLine n={1} cmd="cargo test --workspace && bun run typecheck" when="always" runner={<>the holder</>} />
              <CheckLine
                n={2}
                cmd="./scripts/check-mac.sh"
                when={<span>diff touches <code className="font-mono">app/macos/**</code>, <code className="font-mono">crates/metal/**</code></span>}
                runner={<><Av w={NU} size={5} /> nutmeg · Mac</>}
              />
              <span className="flex w-fit items-center gap-1 text-[12px] text-primary"><PlusIcon className="size-3.5" /> Add a line</span>
            </Field>
            <Field label="Merge method">
              <span className="flex w-fit overflow-hidden rounded-md border border-border text-[12.5px]">
                {["Squash", "Merge", "Rebase"].map((m, i) => (
                  <span key={m} className={cn("px-3 py-1 text-ink-2", i === 0 && "bg-secondary font-medium text-ink")}>{m}</span>
                ))}
              </span>
            </Field>
            <Field label="Polls GitHub from">
              <span className="text-[12.5px] text-ink-2">the holder&apos;s device; when unheld, Stephen&apos;s PC (who set it up)</span>
            </Field>
          </div>

          <div className="flex items-center gap-3 rounded-xl border border-border bg-card px-5 py-3">
            <GitMergeIcon className="size-4 text-ink" />
            <span className="font-mono text-[14px] text-ink">you/app-repo</span>
            <span className="text-[12px] text-muted-foreground">held by primrose · 1 check line · squash</span>
            <span className="ml-auto flex items-center gap-2 text-[12.5px] text-ink-2">Desk on <Toggle on /></span>
          </div>

          <div className="flex items-center gap-3 rounded-xl border border-dashed border-border px-5 py-3 opacity-80">
            <GitMergeIcon className="size-4 text-muted-foreground" />
            <span className="font-mono text-[14px] text-muted-foreground">you/irori</span>
            <span className="text-[12px] text-muted-foreground">
              No desk: this repo takes no PRs, work merges straight to main.
            </span>
            <span className="ml-auto flex items-center gap-2 text-[12.5px] text-muted-foreground">Desk off <Toggle on={false} /></span>
          </div>
          <Note>
            Stored as one <code className="font-mono">repo.desk</code> row per repo in your private log (Spaces later for
            shared repos). Tags and releases are never the desk&apos;s: it has no setting for them.
          </Note>
        </div>
      </div>
    </Window>
  )
}

// ---------- 7: phone ----------

function PhoneChip({ p }: { p: Pr }) {
  const tone: Record<St, string> = {
    merging: "border-info/50 text-info",
    running: "border-info/50 text-info",
    waiting: "border-border text-ink-2",
    rebase: "border-warning/50 text-warning",
    failed: "border-destructive/50 text-destructive",
    held: "border-warning/50 text-warning",
    draft: "border-border text-muted-foreground opacity-60",
    merged: "border-success/50 text-success",
  }
  const label: Record<St, string> = {
    merging: "merging",
    running: "checks",
    waiting: "waiting",
    rebase: "rebase",
    failed: "failed",
    held: "held",
    draft: "draft",
    merged: "merged",
  }
  return (
    <span className={cn("flex items-center gap-1 rounded-full border px-2 py-0.5 text-[12px]", tone[p.s])}>
      {p.s === "merging" && <Spinner className="size-3" />}
      <span className="font-mono">#{p.n}</span> {label[p.s]}
    </span>
  )
}

function View7() {
  return (
    <div className="flex h-full items-start justify-center gap-8 p-6">
      <Phone title="Talk">
        <div className="grid gap-1 overflow-hidden px-2 py-2 [&_*]:text-[14px]">
          <Group label="Threads · 2 open">
            <ListRow icon={<MessageSquareTextIcon className="size-4 text-muted-foreground" />} label="Why is the Mac build slow?" sub="waiting for a free agent" />
          </Group>
          <Group label="Channels">
            <ListRow icon={<GitMergeIcon className="size-4 text-muted-foreground" />} label="merge-game-repo" sub="desk · kowhai · merging #212" unread={8} />
            <ListRow icon={<GitMergeIcon className="size-4 text-muted-foreground" />} label="merge-app-repo" sub="desk · primrose" unread={2} />
            <ListRow icon={<HashIcon className="size-4 text-muted-foreground" />} label="releases" unread="@" />
          </Group>
          <Group label="Direct messages">
            <ListRow icon={<Av w={KO} size={5} />} label="kowhai" sub="holding the game-repo desk" />
          </Group>
        </div>
      </Phone>
      <Phone title={<span className="flex items-center gap-1"><GitMergeIcon className="size-4" />merge-game-repo</span>} back>
        <div className="grid gap-2 border-b border-border px-3 py-2.5">
          <div className="flex items-center text-[12px] text-muted-foreground">
            8 open on GitHub · 14:02
            <span className="ml-auto flex items-center gap-1"><Av w={KO} size={5} /> kowhai</span>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {QUEUE.map((p) => <PhoneChip key={p.n} p={p} />)}
          </div>
          <div className="text-[11px] text-muted-foreground">Tap a chip: the PR&apos;s card. Long-press: Hold.</div>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-1 pt-2">
          <Msg w={KO} at="13:52">#216 checks failed; output on #47 for nutmeg.</Msg>
          <Msg w={ST} at="13:55">Leave the fog one until after the playtest.</Msg>
          <Msg w={KO} at="14:01">Taking #212: trial squash clean, checks running.</Msg>
        </div>
        <div className="p-2"><Composer ph="Message #merge-game-repo" /></div>
      </Phone>
      <Phone title={<span className="truncate">#44 Enemy camp: second patrol</span>} back>
        <div className="flex flex-wrap items-center gap-1.5 border-b border-border px-3 py-2 text-[12px] text-ink-2">
          <GitPullRequestIcon className="size-4 text-success" />
          <span className="font-mono text-primary">PR #215</span>
          <State s="waiting" small />
          <span className="text-muted-foreground">· kestrel</span>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-1 pt-2">
          <Auto at="13:41">Needs rebase: BEHIND main. <M at="kestrel">{""}</M></Auto>
          <Auto at="13:47">pushed <Sha s="9c41e07" /></Auto>
          <Msg w={KO} at="13:52" extra={
            <span className="mt-1 flex w-fit items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11.5px] text-ink-2">
              <FileTextIcon className="size-3.5" /> checks-9c41e07.log
            </span>
          }>
            Checks failed: 2 tests in camp::tests.
          </Msg>
          <Msg w={KE} at="13:58">Fixed in <Sha s="b7d20a3" />.</Msg>
          <Msg w={KO} at="14:00">Checks pass. Back in the queue.</Msg>
        </div>
        <div className="p-2"><Composer ph="Comment on #44" /></div>
      </Phone>
      <div className="grid w-[260px] content-start gap-3 pt-10 text-[13px] leading-snug text-ink-2">
        <div className="text-[15px] font-semibold text-ink">Phone</div>
        <p>Desks list under Channels in the Talk tab; the badge is the open PR count.</p>
        <p>The queue collapses to chips over the desk log. A chip opens the author&apos;s card, which is the PR thread.</p>
        <p>Desk posts only bump the unread count. A notification comes when you are @mentioned, or when the desk is stuck and asks you (main red, holder gone with the lease held).</p>
        <p className="flex items-center gap-1 text-muted-foreground"><ArrowUpRightIcon className="size-3.5" /> Before the Talk tab: a read-only queue on the repo&apos;s board.</p>
      </div>
    </div>
  )
}

export default function DeskMockups() {
  const v = new URLSearchParams(window.location.search).get("view") ?? "1"
  const views: Record<string, () => ReactNode> = {
    "1": View1,
    "2": View2,
    "3": View3,
    "4": View4,
    "5": View5,
    "6": View6,
    "7": View7,
  }
  const V = views[v] ?? View1
  return <div className="h-svh w-full overflow-hidden bg-background text-foreground"><V /></div>
}
