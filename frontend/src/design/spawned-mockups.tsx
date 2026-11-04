// Card #439: design-round mockups for spawned agents (docs/design/spawned-agents.md).
// Browser preview only (?mockup=spawned&view=1..9); static JSX on made-up
// data, no lib/ calls, no state. Lives on the mockup branch, never main.
// The small pieces and the app frame are copied from the merge desk mockups
// (branch lemmings/404-merge-desk-mockups) so the rounds look the same.

import type { ReactNode } from "react"
import {
  AtSignIcon,
  CheckIcon,
  ChevronDownIcon,
  CircleAlertIcon,
  CpuIcon,
  FileTextIcon,
  FolderGitIcon,
  GitBranchIcon,
  GitMergeIcon,
  GitPullRequestIcon,
  HomeIcon,
  HourglassIcon,
  InboxIcon,
  LaptopIcon,
  LayoutGridIcon,
  LightbulbIcon,
  MessagesSquareIcon,
  MonitorIcon,
  PanelLeftIcon,
  PaperclipIcon,
  PlusIcon,
  ScrollTextIcon,
  ServerIcon,
  SettingsIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
  SmartphoneIcon,
  SquareIcon,
  TerminalIcon,
  UserRoundCheckIcon,
  XIcon,
  ZapIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Spinner } from "@/components/ui/spinner"
import { cn } from "@/lib/utils"

// ---------- people and agents ----------

type Who = { name: string; kind: "person" | "agent"; tone?: string }
const ST: Who = { name: "Stephen", kind: "person" }
// Terminal agents: Stephen's, kept on purpose.
const JU: Who = { name: "juniper", kind: "agent", tone: "bg-chart-1" }
const BA: Who = { name: "basil", kind: "agent", tone: "bg-chart-3" }
// Spawned agents: one per card (or per part of a card).
const FE: Who = { name: "fern", kind: "agent", tone: "bg-chart-2" }
const SO: Who = { name: "sorrel", kind: "agent", tone: "bg-chart-4" }
const YA: Who = { name: "yarrow", kind: "agent", tone: "bg-chart-5" }
const CL: Who = { name: "clover", kind: "agent", tone: "bg-chart-3" }
const TH: Who = { name: "thyme", kind: "agent", tone: "bg-chart-1" }
const NE: Who = { name: "nettle", kind: "agent", tone: "bg-chart-2" }
const MA: Who = { name: "mallow", kind: "agent", tone: "bg-chart-5" }
const RU: Who = { name: "rue", kind: "agent", tone: "bg-chart-4" }

// One moment (14:40) for every view, so the counts agree:
// PC, cap 6: fern (lead #61), clover (#61, waiting on step 1), yarrow (#61,
//   finished, waits for fern's merge), thyme (#58), mallow (#66), and an
//   agent starting for #64 = 6 of 6. sorrel (#61) ended 14:31.
//   #67 queued 1st for the PC. #65 needs Windows; app-repo isn't trusted on
//   the PC, so it can't start and doesn't move to the Mac.
// Mac, cap 12: nettle (#63, needs the Mac) = 1 of 12.
// Terminal: juniper (PC, idle), basil (Mac, on #57).
// Badges: 6 on cards · 1 starting · 1 queued.

// ---------- small pieces (from the desk mockups) ----------

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

function Dot({ s }: { s: "working" | "idle" | "gone" | "queued" }) {
  return (
    <span
      className={cn(
        "inline-block size-2 shrink-0 rounded-full",
        s === "working" && "bg-info",
        s === "idle" && "bg-success",
        s === "queued" && "bg-warning",
        s === "gone" && "bg-muted-foreground/40"
      )}
    />
  )
}

function Tag({ n }: { n: string }) {
  return <span className="rounded bg-secondary px-1 font-mono text-[11px] text-primary">#{n}</span>
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

function M({ at }: { at: string }) {
  return <span className="rounded bg-primary/15 px-1 font-medium text-primary">@{at}</span>
}

function Code({ children }: { children: ReactNode }) {
  return <code className="rounded bg-secondary px-1 font-mono text-[11.5px] text-ink">{children}</code>
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

function Kind({ spawned, n }: { spawned?: boolean; n?: string }) {
  return spawned ? (
    <span className="rounded border border-info/40 px-1 text-[10px] text-info uppercase">{n && /^\d/.test(n) ? `for #${n}` : `for this ${n}`}</span>
  ) : (
    <span className="rounded border border-border px-1 text-[10px] text-muted-foreground uppercase">terminal</span>
  )
}

function Msg({
  w,
  at,
  children,
  extra,
  hl,
  sp,
}: {
  w: Who
  at: string
  children: ReactNode
  extra?: ReactNode
  hl?: boolean
  sp?: string
}) {
  return (
    <div className={cn("flex gap-2.5 rounded-lg px-2 py-1.5", hl && "bg-secondary/70")}>
      <Av w={w} />
      <div className="grid min-w-0 flex-1 gap-0.5">
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-medium text-ink">{w.name}</span>
          {w.kind === "agent" && <Kind spawned={!!sp} n={sp} />}
          <span className="text-[11px] text-muted-foreground tabular-nums">{at}</span>
        </div>
        <div className="text-[13px] leading-relaxed text-ink-2">{children}</div>
        {extra}
      </div>
    </div>
  )
}

function Auto({ at, children, extra }: { at: string; children: ReactNode; extra?: ReactNode }) {
  return (
    <div className="flex gap-2.5 rounded-lg px-2 py-1.5">
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-border bg-secondary text-muted-foreground">
        <ZapIcon className="size-3.5" />
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

function Sys({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 px-2 py-1 text-[11.5px] text-muted-foreground">
      <span className="h-px flex-1 bg-border" />
      {children}
      <span className="h-px flex-1 bg-border" />
    </div>
  )
}

function Btn({ children, primary, danger, className }: { children: ReactNode; primary?: boolean; danger?: boolean; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex w-fit items-center gap-1.5 rounded-md border px-2.5 py-1 text-[12px] whitespace-nowrap",
        primary
          ? "border-primary bg-primary font-medium text-primary-foreground"
          : danger
            ? "border-destructive/50 text-destructive"
            : "border-border bg-card text-ink-2",
        className
      )}
    >
      {children}
    </span>
  )
}

function Toggle({ on }: { on: boolean }) {
  return (
    <span className={cn("relative inline-block h-5 w-9 shrink-0 rounded-full", on ? "bg-success" : "bg-muted")}>
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

function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="grid grid-cols-[140px_1fr] items-start gap-3 py-1.5">
      <span className="pt-1 text-[12.5px] text-muted-foreground">{label}</span>
      <div className="grid gap-1">
        {children}
        {hint && <span className="text-[11.5px] text-muted-foreground">{hint}</span>}
      </div>
    </div>
  )
}

function Caps({ children }: { children: ReactNode }) {
  return <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{children}</div>
}

function Composer({ ph, popup }: { ph: string; popup?: ReactNode }) {
  return (
    <div className="relative">
      {popup}
      <div className="flex items-center gap-2 rounded-xl border border-border bg-card px-3 py-2.5">
        <span className="flex-1 text-[13px] text-muted-foreground">{ph}</span>
        <AtSignIcon className="size-4 text-muted-foreground" />
        <PaperclipIcon className="size-4 text-muted-foreground" />
        <span className="rounded-md bg-primary px-3 py-1 text-[12px] font-medium text-primary-foreground">Send</span>
      </div>
    </div>
  )
}

// ---------- the app's frame ----------

function NavRow({ icon: I, label, current, count }: { icon: typeof HomeIcon; label: string; current?: boolean; count?: number }) {
  return (
    <div className={cn("flex h-9 items-center gap-3 rounded-md px-2.5 text-sm text-ink-2", current && "bg-secondary font-medium text-ink")}>
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

function RosterRow({ w, sub, s, tag, inset }: { w: Who; sub: string; s: "working" | "idle" | "queued" | "gone"; tag?: string; inset?: boolean }) {
  return (
    <div className={cn("flex items-center gap-2 rounded-md px-2 py-1 text-[12.5px]", inset && "pl-4", s === "gone" && "opacity-50")}>
      <Av w={w} size={5} />
      <span className="grid min-w-0 flex-1 leading-tight">
        <span className="truncate font-medium text-ink">{w.name}</span>
        <span className="truncate text-[10.5px] text-muted-foreground">{sub}</span>
      </span>
      {tag && <Tag n={tag} />}
      <Dot s={s} />
    </div>
  )
}

// The sidebar's Agents section split into Yours and On cards.
function AgentsSplit({ big }: { big?: boolean }) {
  return (
    <div className="grid gap-1">
      <div className="flex items-baseline px-2.5">
        <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Agents</span>
        {big && <span className="ml-auto text-[11px] text-ink-2">PC 6/6 · Mac 1/12</span>}
      </div>
      <div className="flex flex-wrap gap-1 px-2.5 pb-1">
        <Badge variant="info" className="tabular-nums normal-case">6 on cards</Badge>
        <Badge variant="secondary" className="tabular-nums normal-case">1 starting</Badge>
        <Badge variant="warning" className="tabular-nums normal-case">1 queued</Badge>
      </div>
      <div className="px-2.5 pt-1 text-[10.5px] font-medium tracking-wide text-muted-foreground uppercase">Yours</div>
      <RosterRow w={JU} sub="terminal · PC · idle" s="idle" />
      <RosterRow w={BA} sub="terminal · Mac · on #57" s="working" tag="57" />
      <div className="px-2.5 pt-2 text-[10.5px] font-medium tracking-wide text-muted-foreground uppercase">On cards</div>
      <div className="mx-1 rounded-md border border-border/70 py-0.5">
        <div className="flex items-center gap-1.5 px-2 pt-0.5 text-[10.5px] text-muted-foreground">
          <Tag n="61" /> Co-op save sync · 3 agents
        </div>
        <RosterRow w={FE} sub="lead · PC · working" s="working" inset />
        <RosterRow w={CL} sub="helper · PC · waiting on step 1" s="queued" inset />
        <RosterRow w={YA} sub="helper · finished, waits for merge" s="gone" inset />
      </div>
      <RosterRow w={TH} sub="PC · started 14:02" s="working" tag="58" />
      <RosterRow w={MA} sub="PC · started 14:20" s="working" tag="66" />
      <RosterRow w={NE} sub="Mac · started 13:50" s="working" tag="63" />
    </div>
  )
}

function Sidebar({ current = "home", split = true }: { current?: string; split?: boolean }) {
  return (
    <aside className="flex h-full w-[248px] shrink-0 flex-col border-r border-border bg-sidebar/60">
      <div className="flex h-14 items-center gap-2 px-4">
        <span className="text-lg font-semibold tracking-tight text-ink">Irori</span>
        <span className="text-sm text-muted-foreground">囲炉裏</span>
        <PanelLeftIcon className="ml-auto size-4 text-muted-foreground" />
      </div>
      <nav className="flex flex-col gap-0.5 px-2">
        <NavRow icon={HomeIcon} label="Home" current={current === "home"} />
        <NavRow icon={MessagesSquareIcon} label="Talk" current={current === "talk"} count={2} />
        <NavRow icon={LayoutGridIcon} label="Boards" current={current === "boards"} />
        <NavRow icon={InboxIcon} label="Held" current={current === "held"} />
        <div className="px-2.5 pt-4 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Pinned boards</div>
        <div className="flex h-8 items-center gap-2.5 px-2.5 text-sm text-ink-2">
          <span className="rounded bg-secondary px-1 font-mono text-[10px] text-muted-foreground">gm</span>
          Game
        </div>
      </nav>
      <div className="mt-auto px-1.5 pb-3">{split && <AgentsSplit big />}</div>
      <div className="flex items-center border-t border-border px-4 py-3 text-xs text-ink-2">
        Stephen&apos;s PC
        <SettingsIcon className="ml-auto size-4 text-muted-foreground" />
      </div>
    </aside>
  )
}

function Header({ title, right }: { title: ReactNode; right?: ReactNode }) {
  return (
    <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border px-6">
      <h1 className="text-xl font-semibold text-ink">{title}</h1>
      <div className="ml-auto flex items-center gap-2">{right}</div>
    </header>
  )
}

function Window({ children, current }: { children: ReactNode; current?: string }) {
  return (
    <div className="flex h-full overflow-hidden">
      <Sidebar current={current} />
      <div className="flex min-w-0 flex-1 flex-col">{children}</div>
    </div>
  )
}

function Side({ title, children, w = 380 }: { title: string; children: ReactNode; w?: number }) {
  return (
    <div style={{ width: w }} className="flex shrink-0 flex-col gap-3 border-l border-border px-4 py-4 text-[12.5px] leading-snug text-ink-2">
      <div className="text-[15px] font-semibold text-ink">{title}</div>
      {children}
    </div>
  )
}

// ---------- card bits ----------

function Working({ children, tone = "info" }: { children: ReactNode; tone?: "info" | "warning" }) {
  return (
    <div className={cn("flex items-center gap-2 text-[12.5px]", tone === "info" ? "text-info" : "text-warning")}>
      {tone === "info" ? <Spinner className="size-3.5" /> : <HourglassIcon className="size-3.5" />}
      {children}
    </div>
  )
}

function HomeCard({ n, t, repo, line, tone, who }: { n: string; t: string; repo: string; line: ReactNode; tone?: "info" | "warning"; who?: ReactNode }) {
  return (
    <div className="grid gap-1.5 rounded-xl border border-border bg-card px-4 py-3">
      <div className="flex items-baseline gap-2">
        <span className="font-mono text-[12px] text-muted-foreground">#{n}</span>
        <span className="truncate text-[14px] font-medium text-ink">{t}</span>
        <span className="ml-auto font-mono text-[11px] text-muted-foreground">{repo}</span>
      </div>
      <div className="flex items-center gap-3">
        <Working tone={tone}>{line}</Working>
        <span className="ml-auto flex items-center gap-1">{who}</span>
      </div>
    </div>
  )
}

function Faces({ ws }: { ws: Who[] }) {
  return (
    <span className="flex -space-x-1">
      {ws.map((w) => (
        <span key={w.name} className="rounded-md ring-2 ring-card">
          <Av w={w} size={5} />
        </span>
      ))}
    </span>
  )
}

// ======================================================================
// sa-1: sidebar and Home
// ======================================================================

function View1() {
  return (
    <Window current="home">
      <Header title="Home" right={<span className="text-[12.5px] text-muted-foreground">Game board · spawning on</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="grid min-w-0 flex-1 content-start gap-4 overflow-hidden px-6 py-5">
          <div className="flex items-center gap-2 rounded-xl border border-border bg-card px-4 py-3 text-[13px] text-muted-foreground">
            Ask the agents, or post a card…
            <span className="ml-auto rounded-md bg-primary px-3 py-1 text-[12px] font-medium text-primary-foreground">Ask</span>
          </div>
          <Caps>Working now · 7 agents on 5 cards</Caps>
          <div className="grid gap-2">
            <HomeCard n="61" t="Co-op save sync" repo="game-repo" line="3 agents started for this card · lead fern merging · 2/6 steps" who={<Faces ws={[FE, CL, YA]} />} />
            <HomeCard n="58" t="Lantern flicker on the dock" repo="game-repo" line="an agent started for this card is working" who={<Faces ws={[TH]} />} />
            <HomeCard n="66" t="Dock props: LODs" repo="game-repo" line="an agent started for this card is working" who={<Faces ws={[MA]} />} />
            <HomeCard n="63" t="Notarize the Mac build" repo="app-repo" line="an agent started for this card is working · on the Mac" who={<Faces ws={[NE]} />} />
            <HomeCard n="57" t="Crash reporter" repo="app-repo" line="basil is working (you gave it this card)" who={<Faces ws={[BA]} />} />
          </div>
          <Caps>Starting · queued · can&apos;t start</Caps>
          <div className="grid gap-2">
            <HomeCard n="64" t="Shop prices from one table" repo="game-repo" line="starting an agent on the PC (6 of 6 with it)" />
            <HomeCard n="67" t="Fish schooling AI" repo="game-repo" tone="warning" line="queued: 1st for the PC (at its cap)" />
            <HomeCard n="65" t="Windows installer: code signing" repo="app-repo" tone="warning" line="can't start: app-repo isn't trusted for agents on the PC (needs Windows) · in Needs you" />
          </div>
        </div>
        <Side title="Every card gets its own agent">
          <p>Every card on a spawning board gets its own fresh agent, started as soon as it is posted. When the device is full it says its place in the queue instead.</p>
          <div className="grid gap-1.5">
            <Caps>The sidebar&apos;s Agents</Caps>
            <div className="grid grid-cols-[86px_1fr] gap-2">
              <span className="font-medium text-ink">Yours</span>
              <span>terminal agents you started and keep. Never handed cards unless you give them one.</span>
            </div>
            <div className="grid grid-cols-[86px_1fr] gap-2">
              <span className="font-medium text-ink">On cards</span>
              <span>live spawned agents, tagged with their card. Several on one card sit together under it; a finished helper waiting for its merge is greyed, an ended one is gone.</span>
            </div>
            <div className="grid grid-cols-[86px_1fr] gap-2">
              <span className="font-medium text-ink">Badges</span>
              <span>spawned agents on cards, starting and queued (6, 1, 1); no &ldquo;free&rdquo; count on spawning boards.</span>
            </div>
          </div>
          <Note>
            A card goes away from Working now when it is Done, and its agents go with it. What they found stays on the card.
          </Note>
        </Side>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-2: a card with one spawned agent
// ======================================================================

function CardTop({ n, title, status, sub, line }: { n: string; title: string; status: ReactNode; sub: ReactNode; line: ReactNode }) {
  return (
    <div className="grid gap-2 border-b border-border px-5 py-4">
      <div className="flex items-baseline gap-2">
        <span className="font-mono text-[13px] text-muted-foreground">#{n}</span>
        <span className="text-lg font-semibold text-ink">{title}</span>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-[12px] text-muted-foreground">
        {status}
        {sub}
      </div>
      {line}
    </div>
  )
}

function Tabs({ items, on }: { items: string[]; on: string }) {
  return (
    <div className="flex gap-1 border-b border-border px-5 pt-1 text-[13px]">
      {items.map((t) => (
        <span key={t} className={cn("flex items-center gap-1.5 px-3 py-2 text-ink-2", t === on && "border-b-2 border-primary font-medium text-ink")}>
          {t.startsWith("Terminal") && <TerminalIcon className="size-3.5" />}
          {t}
        </span>
      ))}
    </div>
  )
}

function Term({ lines }: { lines: [string, string?][] }) {
  return (
    <div className="m-4 flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border bg-black/60 font-mono text-[12px] leading-relaxed">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        <TerminalIcon className="size-3.5" /> claude attach 3f9a1c2e · thyme · game-repo/.claude/worktrees/58-lantern-flicker
        <span className="ml-auto">detach: ctrl+]</span>
      </div>
      <div className="grid gap-0.5 px-3 py-2">
        {lines.map(([t, c], i) => (
          <span key={i} className={c ?? "text-ink-2"}>
            {t}
          </span>
        ))}
      </div>
    </div>
  )
}

function SpawnedPerson({ w, step, wt, branch, state, dev = "PC", lead }: { w: Who; step?: string; wt: string; branch: string; state: ReactNode; dev?: string; lead?: boolean }) {
  return (
    <div className="grid gap-1.5 rounded-lg border border-border bg-card p-2.5">
      <div className="flex items-center gap-2">
        <Av w={w} size={6} />
        <span className="grid leading-tight">
          <span className="flex items-center gap-1.5 text-[13px] font-medium text-ink">
            {w.name} {lead && <Badge variant="info" className="h-4 px-1 text-[9.5px]">lead</Badge>}
          </span>
          <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
            <MonitorIcon className="size-3" /> {dev} · started for this card{step && <> · {step}</>}
          </span>
        </span>
        <span className="ml-auto">{state}</span>
      </div>
      <div className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
        <FolderGitIcon className="size-3.5" /> {wt}
      </div>
      <div className="flex items-center gap-1.5 font-mono text-[11px] text-muted-foreground">
        <GitBranchIcon className="size-3.5" /> {branch}
      </div>
      <div className="flex flex-wrap gap-1.5 pt-0.5">
        <Btn><TerminalIcon className="size-3.5" />Attach</Btn>
        <Btn><ScrollTextIcon className="size-3.5" />Logs</Btn>
        <Btn><SquareIcon className="size-3" />Stop</Btn>
        <Btn danger><XIcon className="size-3.5" />End</Btn>
      </div>
    </div>
  )
}

function View2() {
  return (
    <Window current="boards">
      <Header title="Card #58" right={<span className="text-[12.5px] text-muted-foreground">board: Game · game-repo</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <CardTop
            n="58"
            title="Lantern flicker on the dock"
            status={<Badge variant="info">Working</Badge>}
            sub={<span>held by thyme · 2/4 steps · Sonnet · medium · acceptEdits + allow-list</span>}
            line={<Working>an agent started for this card is working · since 14:02 on the PC</Working>}
          />
          <Tabs items={["Thread 6", "Steps", "Terminal · thyme"]} on="Terminal · thyme" />
          <Term
            lines={[
              ["● Read src/world/lantern.rs (212 lines)"],
              ["● The flicker comes from two timers: tick_light() and the dock's day/night fade both write alpha.", "text-ink"],
              ["● Update(src/world/lantern.rs)"],
              ["  ⎿  Updated with 9 additions and 14 removals", "text-success"],
              ["● Bash(cargo test -p game lantern)"],
              ["  ⎿  test result: ok. 12 passed; 0 failed", "text-success"],
              ["● irori - board_step (MCP)(task_id: 58, step: 3, state: done)"],
              ["● irori - board_comment (MCP)(task_id: 58, text: \"Step 3: one writer for alpha …\")"],
              ["✻ Checking the night scene for the second flicker…", "text-info"],
              [""],
              ["> _", "text-ink"],
            ]}
          />
        </div>
        <Side title="Who's on it" w={400}>
          <SpawnedPerson
            w={TH}
            wt=".claude/worktrees/58-lantern-flicker"
            branch="thyme/58-lantern-flicker @ 7c1e4a0"
            state={<Badge variant="info"><Spinner className="size-3" />working</Badge>}
          />
          <div className="grid gap-1.5 rounded-lg border border-border px-3 py-2 text-[12px]">
            <div className="grid grid-cols-[60px_1fr] gap-2"><span className="font-medium text-ink">Attach</span><span>opens it in this card&apos;s terminal tab (on now). Typing here talks to it; it stays spawned.</span></div>
            <div className="grid grid-cols-[60px_1fr] gap-2"><span className="font-medium text-ink">Logs</span><span>its output so far, read only.</span></div>
            <div className="grid grid-cols-[60px_1fr] gap-2"><span className="font-medium text-ink">Stop</span><span>cuts the current turn; the agent stays.</span></div>
            <div className="grid grid-cols-[60px_1fr] gap-2"><span className="font-medium text-ink">End</span><span>it writes Where and Left off, then Irori stops and removes it. The card stays open and unheld.</span></div>
          </div>
          <Note>
            No Revive for a spawned agent. If it ends or crashes, the next agent is a fresh one that starts from the card&apos;s Where and Left off.
          </Note>
          <div className="grid gap-1">
            <Caps>Watching</Caps>
            <span className="flex items-center gap-1.5"><Av w={ST} size={5} /> Stephen</span>
          </div>
        </Side>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-3: a lead and three helpers on one card
// ======================================================================

function Lane({ w, step, wt, branch, state, children, lead }: { w: Who; step: string; wt: string; branch: string; state: ReactNode; children?: ReactNode; lead?: boolean }) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-2 rounded-xl border bg-card p-3", lead ? "border-info/60" : "border-border")}>
      <div className="flex items-center gap-2">
        <Av w={w} size={6} />
        <span className="grid leading-tight">
          <span className="flex items-center gap-1.5 text-[13px] font-medium text-ink">
            {w.name}
            {lead ? <Badge variant="info" className="h-4 px-1 text-[9.5px]">lead</Badge> : <span className="text-[10px] text-muted-foreground uppercase">helper</span>}
          </span>
          <span className="text-[11px] text-muted-foreground">PC · spawned for #61</span>
        </span>
      </div>
      <div className="rounded-md bg-secondary/60 px-2 py-1.5 text-[12px] text-ink">{step}</div>
      <div className="grid gap-1 font-mono text-[10.5px] text-muted-foreground">
        <span className="flex items-center gap-1 truncate"><FolderGitIcon className="size-3 shrink-0" />{wt}</span>
        <span className="flex items-center gap-1 truncate"><GitBranchIcon className="size-3 shrink-0" />{branch}</span>
      </div>
      <div>{state}</div>
      {children}
    </div>
  )
}

function View3() {
  return (
    <Window current="boards">
      <Header title="Card #61" right={<span className="text-[12.5px] text-muted-foreground">board: Game · game-repo · up to 4 agents</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <CardTop
            n="61"
            title="Co-op save sync"
            status={<Badge variant="info">Working</Badge>}
            sub={<span>held by fern (lead) · 3 helpers, 1 ended · 2/6 steps · card branch fern/61-coop-save-sync</span>}
            line={<Working>4 agents started for this card: fern leads; sorrel has ended, yarrow waits for its merge, clover waits on step 1</Working>}
          />
          <Tabs items={["Lanes", "Thread 14", "Steps", "Terminal"]} on="Lanes" />
          <div className="grid min-h-0 flex-1 grid-cols-4 gap-3 overflow-hidden p-4">
            <Lane
              lead
              w={FE}
              step="1 · Protocol constants, then merge the parts"
              wt="worktrees/61-coop-save-sync"
              branch="fern/61-coop-save-sync"
              state={<Badge variant="info"><GitMergeIcon />merging 2 branches</Badge>}
            >
              <div className="grid gap-1 rounded-md border border-info/40 px-2 py-1.5 text-[11px] text-ink-2">
                <span className="flex items-center gap-1.5"><CheckIcon className="size-3 text-success" />merged 61-save-format (sorrel) @ a41c09e</span>
                <span className="flex items-center gap-1.5"><Spinner className="size-3 text-info" />merging 61-net-messages (yarrow)</span>
                <span className="flex items-center gap-1.5 text-muted-foreground"><span className="size-3" />then cargo test, one PR for #61</span>
              </div>
            </Lane>
            <Lane
              w={SO}
              step="2 · Save format v2"
              wt="worktrees/61-save-format"
              branch="61-save-format @ a41c09e"
              state={<Badge variant="success"><CheckIcon />finished · hand-off written</Badge>}
            >
              <span className="text-[11px] text-muted-foreground">merged by fern, then ended 14:31 · closing comment and a note on the thread</span>
            </Lane>
            <Lane
              w={YA}
              step="3 · Net messages for the save"
              wt="worktrees/61-net-messages"
              branch="61-net-messages @ 5d2f7b8"
              state={<Badge variant="success"><CheckIcon />finished · waits for the merge</Badge>}
            >
              <span className="text-[11px] text-muted-foreground">finished 14:36 · hand-off written · stays idle so a conflict reaches it; goes after fern merges</span>
            </Lane>
            <Lane
              w={CL}
              step="4 · UI: sync indicator"
              wt="worktrees/61-sync-ui"
              branch="61-sync-ui"
              state={<Badge variant="warning"><HourglassIcon />waiting on fern&apos;s step 1</Badge>}
            >
              <span className="rounded-md bg-secondary px-2 py-1 font-mono text-[10.5px] text-ink-2">waits_on step #61/1 (Protocol constants)</span>
            </Lane>
          </div>
          <div className="mx-4 mb-4 grid gap-1 rounded-xl border border-border bg-card px-4 py-2.5 text-[12px]">
            <Caps>Steps · owners</Caps>
            {[
              ["1", "Protocol constants", "fern", "working"],
              ["2", "Save format v2", "sorrel", "done"],
              ["3", "Net messages for the save", "yarrow", "done"],
              ["4", "UI: sync indicator", "clover", "waiting"],
              ["5", "Merge and run the checks", "fern", "next"],
              ["6", "One PR on the game-repo desk", "fern", "next"],
            ].map(([n, t, o, s]) => (
              <div key={n} className="grid grid-cols-[20px_1fr_90px_80px] items-center gap-2 text-ink-2">
                <span className="text-muted-foreground">{n}</span>
                <span className={cn(s === "done" && "text-muted-foreground line-through")}>{t}</span>
                <span>{o}</span>
                <span className={cn("text-[11px]", s === "working" && "text-info", s === "waiting" && "text-warning", s === "done" && "text-success", s === "next" && "text-muted-foreground")}>{s}</span>
              </div>
            ))}
          </div>
        </div>
        <Side title="One card, four agents" w={330}>
          <p>fern was spawned for #61 as its lead, like every spawned agent, and planned it into steps with owners. Its <Code>board_steps</Code> call asked for helpers with <Code>spawn: true</Code>; Irori made a worktree for each from the card branch and started sorrel, yarrow and clover in them.</p>
          <p>clover&apos;s part needs fern&apos;s constants, so it waits on that step instead of guessing.</p>
          <p>Finished helpers commit, write Where and hand off. fern&apos;s acceptance closes their steps; a helper stays until fern has merged its branch, then goes. fern runs the checks and opens one PR.</p>
          <Note>Only #61 asks you &ldquo;Is this done?&rdquo;, once, not each helper. Helpers never push to main. At most 4 agents on one card; more queue on the card.</Note>
        </Side>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-4: the thread on that card
// ======================================================================

function ProposedNote({ by, text }: { by: string; text: string }) {
  return (
    <div className="mt-1 grid w-fit max-w-[560px] gap-1 rounded-lg border border-success/50 bg-success-foreground/40 px-2.5 py-1.5 text-[12px]">
      <span className="flex items-center gap-1.5 font-medium text-success">
        <LightbulbIcon className="size-3.5" /> Proposed note · game-repo · from {by}
      </span>
      <span className="text-ink-2">{text}</span>
      <span className="flex gap-1.5 pt-0.5"><Btn primary>Keep note</Btn><Btn>Edit</Btn><Btn>Drop</Btn></span>
    </div>
  )
}

function View4() {
  const picker = (
    <div className="grid gap-0.5 rounded-lg border border-border bg-card p-1 text-[12.5px]">
      {[
        [<Av key="a" w={FE} size={5} />, "the agent on #61", "fern · lead · working"],
        [<Av key="b" w={CL} size={5} />, "clover", "on #61 · waiting"],
        [<Av key="c" w={JU} size={5} />, "juniper", "terminal · PC · idle"],
        [<Av key="d" w={BA} size={5} />, "basil", "terminal · Mac · on #57"],
      ].map(([a, n, s], i) => (
        <div key={i} className={cn("flex items-center gap-2 rounded-md px-2 py-1.5", i === 0 && "bg-secondary")}>
          {a}
          <span className="font-medium text-ink">{n}</span>
          <span className="ml-auto text-[11px] text-muted-foreground">{s}</span>
        </div>
      ))}
    </div>
  )
  return (
    <Window current="boards">
      <Header title="Card #61 · Thread" right={<span className="text-[12.5px] text-muted-foreground">Co-op save sync · 4 agents: 1 ended, 1 waits for its merge</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-3">
            <Msg w={SO} sp="61" at="14:12">
              <M at="fern" /> does the save header keep the version byte at offset 0, or move it after the magic?
            </Msg>
            <Msg w={FE} sp="61" at="14:13">
              After the magic. <M at="juniper" /> you did the old save loader: does anything read byte 0 directly?
            </Msg>
            <Msg w={JU} at="14:15">
              Yes, <Code>load_slot()</Code> in <Code>src/save/slots.rs</Code> peeks byte 0 to pick the loader. Move it and update that peek, or v1 saves stop loading.
            </Msg>
            <Msg w={SO} sp="61" at="14:29" extra={<ProposedNote by="sorrel" text="Save files: load_slot() picks the loader from the version byte after the magic (since #61); keep the v1 path for old saves." />}>
              <b className="text-ink">Closing.</b> Save format v2 on <Code>61-save-format</Code> @ <Code>a41c09e</Code>; v1 saves still load (test added). Where and Left off written. Ending now.
            </Msg>
            <Sys>sorrel ended · hand-off written · 14:31</Sys>
            <Msg w={YA} sp="61" at="14:35" extra={<ProposedNote by="yarrow" text="Net messages over 1 KB go through the chunked channel; SaveSync uses it." />}>
              <b className="text-ink">Closing.</b> Net messages on <Code>61-net-messages</Code> @ <Code>5d2f7b8</Code>. <M at="fern" /> ready to merge.
            </Msg>
            <Sys>yarrow finished · hand-off written · waits for fern&apos;s merge · 14:36</Sys>
            <Msg w={BA} at="14:40">
              <M at="sorrel" /> does v2 keep the slot thumbnails?
            </Msg>
            <Auto at="14:40" extra={<span className="mt-0.5 text-[11.5px] text-muted-foreground">sorrel ended at 14:31; messages to it land here and wake the card&apos;s holder, never start an agent</span>}>
              A message for sorrel, who has finished, landed on this card. <M at="fern" /> is holding #61.
            </Auto>
            <Msg w={FE} sp="61" at="14:41">
              <M at="basil" /> yes: thumbnails are unchanged (sorrel&apos;s Left off says so).
            </Msg>
          </div>
          <div className="p-4"><Composer ph="Comment on #61 · @the agent on #61" /></div>
        </div>
        <Side title="Everyone talks on the card" w={340}>
          <p>Spawned and terminal agents are all ordinary members: mentions wake whoever is named, and the card thread is where the parts meet.</p>
          <div className="grid gap-1.5">
            <Caps>On this thread</Caps>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">helper asks lead</span><span>sorrel mentions fern</span></div>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">lead asks yours</span><span>fern mentions juniper, a terminal agent, which answers</span></div>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">before it ends</span><span>each helper posts a closing comment and proposes a note</span></div>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">after it ends</span><span>a message to a finished agent lands on its card</span></div>
          </div>
          <Caps>Typing @ on this card</Caps>
          {picker}
          <Note>Findings go on the card, its thread or a note, never only in a DM, so they outlive the agent that found them.</Note>
        </Side>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-5: New card dialog
// ======================================================================

function Radio({ on, children, sub }: { on?: boolean; children: ReactNode; sub?: ReactNode }) {
  return (
    <div className={cn("grid gap-1.5 rounded-lg border px-3 py-2", on ? "border-primary/70 bg-primary/5" : "border-border")}>
      <div className="flex items-center gap-2 text-[13px] text-ink">
        <span className={cn("flex size-4 items-center justify-center rounded-full border", on ? "border-primary" : "border-muted-foreground/50")}>
          {on && <span className="size-2 rounded-full bg-primary" />}
        </span>
        {children}
      </div>
      {sub && <div className="pl-6">{sub}</div>}
    </div>
  )
}

function View5() {
  return (
    <Window current="boards">
      <Header title="Game" right={<Btn><PlusIcon className="size-3.5" />New card</Btn>} />
      <div className="relative flex min-h-0 flex-1">
        <div className="grid flex-1 grid-cols-3 content-start gap-3 p-6 opacity-40">
          {["#61 Co-op save sync", "#58 Lantern flicker", "#57 Crash reporter", "#64 Shop prices", "#62 Fishing rod v2", "#59 Dock props"].map((t) => (
            <div key={t} className="h-24 rounded-xl border border-border bg-card px-4 py-3 text-[13px] text-ink">{t}</div>
          ))}
        </div>
        <div className="absolute inset-0 bg-background/50" />
        <div className="absolute top-6 left-1/2 grid w-[640px] -translate-x-1/2 gap-3 rounded-2xl border border-border bg-background p-5 shadow-2xl">
          <div className="flex items-center">
            <span className="text-[16px] font-semibold text-ink">New card</span>
            <XIcon className="ml-auto size-4 text-muted-foreground" />
          </div>
          <div className="rounded-lg border border-border bg-card px-3 py-2 text-[14px] text-ink">Weather system: rain, fog and wind</div>
          <div className="h-14 rounded-lg border border-border bg-card px-3 py-2 text-[12.5px] text-ink-2">Rain and fog layers, wind on the sails, a weather table the islands can override. Done when a storm crosses the sea in co-op.</div>
          <div className="flex gap-3 text-[12.5px]">
            <Select>Board: Game</Select>
            <Select><FolderGitIcon className="size-3.5" />game-repo</Select>
          </div>
          <Caps>Agent</Caps>
          <div className="grid gap-2">
            <Radio
              on
              sub={
                <div className="grid gap-2">
                  <div className="flex items-center gap-2 text-[12.5px] text-ink-2">
                    <span className="flex items-center overflow-hidden rounded-md border border-border">
                      <span className="px-2 py-0.5 text-muted-foreground">−</span>
                      <span className="border-x border-border px-3 py-0.5 font-medium text-ink">Up to 3 agents</span>
                      <span className="px-2 py-0.5 text-muted-foreground">+</span>
                    </span>
                    <span className="text-[11.5px] text-muted-foreground">board default 4 · 1 = the lead works alone</span>
                  </div>
                  <div className="grid gap-1.5 rounded-md border border-border px-2.5 py-2 text-[12.5px] text-ink-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="w-16 font-medium text-ink">Lead</span>
                      <Select>Opus</Select> <Select>medium</Select>
                      <span className="text-[11.5px] text-muted-foreground">plans, asks for helpers, merges</span>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="w-16 font-medium text-ink">Helpers</span>
                      <Select>Sonnet</Select> <Select>medium</Select>
                      <span className="text-[11.5px] text-muted-foreground">up to 2, each in its own worktree from the card branch</span>
                    </div>
                    <span className="text-[11.5px] text-muted-foreground">With 1 agent it is the board default (Sonnet · medium). Set here, in the app only.</span>
                  </div>
                </div>
              }
            >
              <ZapIcon className="size-4 text-info" /> A fresh agent <span className="text-[11.5px] text-muted-foreground">started on the PC when a slot frees (6 of 6 now), ends when the card is Done</span>
            </Radio>
            <Radio
              sub={
                <div className="flex flex-wrap gap-1.5">
                  {[
                    [JU, "juniper · PC · idle"],
                    [BA, "basil · Mac · on #57"],
                  ].map(([w, s]) => (
                    <span key={(w as Who).name} className="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[12px] text-ink-2">
                      <Av w={w as Who} size={5} /> {s as string}
                    </span>
                  ))}
                </div>
              }
            >
              <UserRoundCheckIcon className="size-4 text-ink-2" /> Give to <Select>a terminal agent</Select>
              <span className="text-[11.5px] text-muted-foreground">keeps its context; no spawn</span>
            </Radio>
            <div className="px-1 text-[11.5px] text-muted-foreground">&ldquo;Any free agent&rdquo; shows only on boards with spawning off.</div>
          </div>
          <div className="flex justify-end gap-2 pt-1"><Btn>Cancel</Btn><Btn primary>Post card</Btn></div>
        </div>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-6: Settings › Agents
// ======================================================================

function View6() {
  return (
    <Window current="settings">
      <Header title="Settings" />
      <div className="flex min-h-0 flex-1">
        <div className="grid w-[190px] shrink-0 content-start gap-0.5 border-r border-border p-3 text-[13px] text-ink-2">
          {["General", "Appearance", "Agents", "Notifications", "Devices", "People", "Maintenance"].map((t) => (
            <span key={t} className={cn("rounded-md px-2.5 py-1.5", t === "Agents" && "bg-secondary font-medium text-ink")}>{t}</span>
          ))}
        </div>
        <div className="grid min-w-0 flex-1 grid-cols-[1fr_400px] content-start gap-5 overflow-hidden px-7 py-5">
          <div className="grid content-start gap-2">
            <div className="flex items-baseline gap-3">
              <span className="text-[17px] font-semibold text-ink">Agents</span>
              <span className="text-[12.5px] text-muted-foreground">a fresh agent per card · set in the app only, never by a tool</span>
            </div>
            <div className="rounded-xl border border-border bg-card px-5 py-2">
              <div className="flex items-center gap-3 border-b border-border py-2">
                <Select>Board: Game</Select>
                <span className="ml-auto flex items-center gap-2 text-[12.5px] text-ink-2">Spawn agents for cards <Toggle on /></span>
              </div>
              <Field label="Model and effort" hint="Leads of cards set to more than one agent: Opus · medium. Per card in the New card dialog.">
                <span className="flex gap-2"><Select>Sonnet</Select><Select>medium</Select></span>
              </Field>
              <Field label="Permission mode" hint="Always passed, from this board's own settings file; never inherited. Never bypass on shared boards.">
                <Select><ShieldCheckIcon className="size-3.5" />acceptEdits + allow and deny lists</Select>
                <div className="flex flex-wrap gap-1">
                  {["cargo test *", "cargo build *", "bun run typecheck", "git add *", "git commit *", "git push origin 61-*", "gh pr create", "irori tools"].map((a) => (
                    <span key={a} className="rounded-md bg-secondary px-1.5 py-0.5 font-mono text-[11px] text-ink-2">{a}</span>
                  ))}
                  <span className="flex items-center gap-1 rounded-md border border-dashed border-border px-1.5 py-0.5 text-[11px] text-primary"><PlusIcon className="size-3" />Add</span>
                </div>
                <div className="flex flex-wrap gap-1">
                  <span className="text-[11px] text-muted-foreground">deny:</span>
                  {["git push * main", "git push --force *", "rm -rf *"].map((a) => (
                    <span key={a} className="rounded-md bg-destructive/15 px-1.5 py-0.5 font-mono text-[11px] text-destructive">{a}</span>
                  ))}
                </div>
                <span className="text-[11.5px] text-muted-foreground">started from the repo&apos;s CLAUDE.md checks line and its merge policy</span>
              </Field>
              <Field label="Merge policy">
                <span className="flex flex-wrap gap-2 text-[12px]">
                  <Select><span className="font-mono">game-repo</span>: one PR on the desk</Select>
                  <Select><span className="font-mono">app-repo</span>: lead pushes to main</Select>
                </span>
              </Field>
              <Field label="Worktree per card" hint="Always a worktree per agent when a card has more than one.">
                <Select>Code cards: own worktree · others: repo root</Select>
              </Field>
              <Field label="Agents per card" hint="Every spawned agent leads and may ask for helpers up to this.">
                <Select>up to 4</Select>
              </Field>
              <Field label="Idle limit" hint="A spawned agent blocked on you, or a quiet thread's agent, writes Left off and goes after this.">
                <Select>30 minutes</Select>
              </Field>
              <Field label="Daily limit" hint="When reached, new cards ask before spawning.">
                <span className="flex items-center gap-2"><Toggle on={false} /> <span className="text-[12.5px] text-muted-foreground">off</span></span>
              </Field>
            </div>
          </div>
          <div className="grid content-start gap-3">
            <div className="grid gap-2 rounded-xl border border-border bg-card px-4 py-3">
              <Caps>Today</Caps>
              <div className="flex items-baseline gap-4">
                <span className="text-[24px] font-semibold text-ink tabular-nums">14</span>
                <span className="text-[12.5px] text-muted-foreground">agents spawned</span>
                <span className="text-[24px] font-semibold text-ink tabular-nums">$3.20</span>
                <span className="text-[12.5px] text-muted-foreground">their cost</span>
              </div>
              <span className="text-[11.5px] text-muted-foreground">11 on the PC, 3 on the Mac · now: 6 on cards, 1 starting, 1 queued</span>
            </div>
            <div className="grid gap-2 rounded-xl border border-border bg-card px-4 py-3">
              <Caps>Per device · you pick</Caps>
              {[
                [MonitorIcon, "Stephen's PC", "at most 6 at once", "6 of 6: 5 on cards, 1 starting"],
                [LaptopIcon, "Stephen's Mac", "at most 12 at once", "1 of 12"],
              ].map(([I, d, c, s]) => {
                const Icon = I as typeof MonitorIcon
                return (
                  <div key={d as string} className="flex items-center gap-2 text-[12.5px]">
                    <Icon className="size-4 text-ink-2" />
                    <span className="grid leading-tight"><span className="text-ink">{d as string}</span><span className="text-[11px] text-muted-foreground">{s as string}</span></span>
                    <span className="ml-auto"><Select>{c as string}</Select></span>
                  </div>
                )
              })}
              <span className="text-[11.5px] text-muted-foreground">Starting points, not limits from memory (about 0.5 GB an agent). If Claude reports a plan usage limit, new spawns wait as &ldquo;queued: usage limit&rdquo; until it resets. Phone and home server never spawn.</span>
            </div>
            <div className="grid gap-1.5 rounded-xl border border-border bg-card px-4 py-3">
              <Caps>Trusted repos for agents</Caps>
              {[
                ["PC", "game-repo", true],
                ["PC", "app-repo", false],
                ["Mac", "app-repo", true],
                ["Mac", "game-repo", null],
              ].map(([d, r, ok]) => (
                <div key={`${d}${r}`} className="flex items-center gap-2 text-[12.5px]">
                  {ok ? <ShieldCheckIcon className="size-4 text-success" /> : ok === null ? <FolderGitIcon className="size-4 text-muted-foreground" /> : <ShieldAlertIcon className="size-4 text-warning" />}
                  <span className="w-10 text-muted-foreground">{d as string}</span>
                  <span className="font-mono text-ink">{r as string}</span>
                  <span className="ml-auto">{ok ? <span className="text-[11.5px] text-muted-foreground">trusted</span> : ok === null ? <span className="text-[11.5px] text-muted-foreground">not on this device</span> : <Btn>Trust…</Btn>}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-7: Needs you
// ======================================================================

function NeedRow({ icon, title, sub, actions, on }: { icon: ReactNode; title: ReactNode; sub: ReactNode; actions: ReactNode; on?: boolean }) {
  return (
    <div className={cn("grid gap-1.5 rounded-xl border bg-card px-4 py-3", on ? "border-primary/60 ring-2 ring-primary/20" : "border-border")}>
      <div className="flex items-center gap-2">
        {icon}
        <span className="text-[13.5px] font-medium text-ink">{title}</span>
      </div>
      <div className="text-[12px] text-ink-2">{sub}</div>
      <div className="flex gap-1.5">{actions}</div>
    </div>
  )
}

function View7() {
  return (
    <Window current="home">
      <Header title="Home" />
      <div className="relative flex min-h-0 flex-1">
        <div className="grid w-[560px] shrink-0 content-start gap-3 px-6 py-5">
          <Caps>Needs you · 3</Caps>
          <NeedRow
            on
            icon={<ShieldAlertIcon className="size-4 text-warning" />}
            title="Can't start: repo not trusted"
            sub={<><Chip n="65" t="Windows installer: code signing" /> needs Windows, so it runs on Stephen&apos;s PC, where app-repo isn&apos;t trusted for agents. The Mac has app-repo trusted but can&apos;t build this.</>}
            actions={<><Btn primary>Trust and start…</Btn><Btn>Give to a terminal agent</Btn></>}
          />
          <NeedRow
            icon={<HourglassIcon className="size-4 text-info" />}
            title="Waiting for your permission"
            sub={<><Chip n="58" t="Lantern flicker on the dock" /> thyme wants to run <Code>cargo run --example dock_night</Code>, not on the allow-list.</>}
            actions={<><Btn primary><TerminalIcon className="size-3.5" />Attach</Btn><Btn>Logs</Btn><Btn>Add to allow-list</Btn></>}
          />
          <NeedRow
            icon={<CircleAlertIcon className="size-4 text-destructive" />}
            title="Agent crashed twice"
            sub={<><Chip n="62" t="Fishing rod v2" /> two spawned agents ended without a hand-off (14:05, 14:19). Irori stopped spawning for this card.</>}
            actions={<><Btn primary>Try again</Btn><Btn>Logs</Btn><Btn>Give to a terminal agent</Btn></>}
          />
        </div>
        <div className="flex flex-1 items-start justify-center bg-background/40 pt-10">
          <div className="grid w-[520px] gap-3 rounded-2xl border border-border bg-background p-5 shadow-2xl">
            <div className="flex items-center gap-2">
              <ShieldAlertIcon className="size-5 text-warning" />
              <span className="text-[16px] font-semibold text-ink">Trust app-repo for agents?</span>
              <XIcon className="ml-auto size-4 text-muted-foreground" />
            </div>
            <div className="grid gap-1.5 rounded-lg border border-border bg-card px-3 py-2 text-[12.5px]">
              <div className="grid grid-cols-[70px_1fr] gap-2"><span className="text-muted-foreground">Repo</span><span className="font-mono text-ink">~/projects/app-repo</span></div>
              <div className="grid grid-cols-[70px_1fr] gap-2"><span className="text-muted-foreground">Device</span><span className="text-ink">Stephen&apos;s PC</span></div>
              <div className="grid grid-cols-[70px_1fr] gap-2"><span className="text-muted-foreground">For</span><span className="text-ink">#65 Windows installer: code signing, and later cards in this repo on this PC</span></div>
            </div>
            <p className="text-[13px] leading-snug text-ink-2">
              Claude Code will run in this folder without asking you first. Spawned agents here use the Game board&apos;s mode: <b className="text-ink">acceptEdits + allow-list</b>.
            </p>
            <p className="text-[12px] text-muted-foreground">Asked once per repo per device. Only you can confirm it, here; no agent, tool, card or other person can. #65 starts when the PC has a slot (1 queued ahead of it).</p>
            <div className="flex justify-end gap-2"><Btn>Not now</Btn><Btn primary><ShieldCheckIcon className="size-3.5" />Trust and start</Btn></div>
          </div>
        </div>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-8: the roster across devices
// ======================================================================

function DevCol({ icon: I, name, cap, children }: { icon: typeof MonitorIcon; name: string; cap: ReactNode; children: ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-2 rounded-xl border border-border bg-card p-4">
      <div className="flex items-center gap-2">
        <I className="size-4 text-ink" />
        <span className="text-[14px] font-medium text-ink">{name}</span>
        <span className="ml-auto text-[12px] text-muted-foreground">{cap}</span>
      </div>
      {children}
    </div>
  )
}

function DevAgent({ w, n, t, repo, why, s = "working" }: { w: Who; n: string; t: string; repo: string; why: string; s?: "working" | "idle" }) {
  return (
    <div className="grid gap-0.5 rounded-lg border border-border/70 px-2.5 py-2">
      <div className="flex items-center gap-2 text-[12.5px]">
        <Av w={w} size={5} />
        <span className="font-medium text-ink">{w.name}</span>
        {n !== "—" && <Tag n={n} />}
        <span className="truncate text-ink-2">{t}</span>
        <span className="ml-auto"><Dot s={s} /></span>
      </div>
      <div className="pl-7 text-[11px] text-muted-foreground"><span className="font-mono">{repo}</span> · {why}</div>
    </div>
  )
}

function View8() {
  return (
    <Window current="home">
      <Header title="Agents" right={<span className="text-[12.5px] text-muted-foreground">all devices · 6 spawned on cards · 1 starting · 1 queued · 2 terminal</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="grid min-w-0 flex-1 grid-cols-2 content-start gap-4 p-5">
          <DevCol icon={MonitorIcon} name="Stephen's PC" cap="always on · 6 of 6: 5 on cards, 1 starting">
            <Caps>On cards</Caps>
            <DevAgent w={FE} n="61" t="Co-op save sync" repo="game-repo" why="lead · merging yarrow's branch" />
            <DevAgent w={CL} n="61" t="Co-op save sync" repo="game-repo" why="helper, asked for by fern · waiting on step 1" />
            <DevAgent w={YA} n="61" t="Co-op save sync" repo="game-repo" why="helper · finished, waits for fern's merge" s="idle" />
            <DevAgent w={TH} n="58" t="Lantern flicker on the dock" repo="game-repo" why="card made on the PC" />
            <DevAgent w={MA} n="66" t="Dock props: LODs" repo="game-repo" why="card made on the phone" />
            <div className="flex items-center gap-2 rounded-lg border border-dashed border-info/60 px-2.5 py-1.5 text-[12.5px]">
              <Spinner className="size-3.5 text-info" />
              <Tag n="64" />
              <span className="text-ink-2">Shop prices from one table</span>
              <span className="ml-auto text-[11.5px] text-info">starting an agent</span>
            </div>
            <Caps>Waiting for the PC</Caps>
            <div className="flex items-center gap-2 rounded-lg border border-dashed border-warning/60 px-2.5 py-1.5 text-[12.5px]">
              <HourglassIcon className="size-4 text-warning" />
              <Tag n="67" />
              <span className="text-ink-2">Fish schooling AI</span>
              <span className="ml-auto text-[11.5px] text-warning">queued: 1st for the PC</span>
            </div>
            <div className="grid gap-0.5 rounded-lg border border-dashed border-warning/60 px-2.5 py-1.5 text-[12.5px]">
              <div className="flex items-center gap-2">
                <ShieldAlertIcon className="size-4 text-warning" />
                <Tag n="65" />
                <span className="text-ink-2">Windows installer: code signing</span>
                <span className="ml-auto text-[11.5px] text-warning">can&apos;t start: not trusted</span>
              </div>
              <span className="pl-6 text-[11px] text-muted-foreground">app-repo isn&apos;t trusted on the PC; doesn&apos;t move to the Mac because it needs Windows</span>
            </div>
            <Caps>Yours</Caps>
            <DevAgent w={JU} n="—" t="terminal agent" repo="~/projects" why="started by you · not offered cards" s="idle" />
          </DevCol>
          <DevCol icon={LaptopIcon} name="Stephen's Mac" cap="1 of 12 spawned">
            <Caps>On cards</Caps>
            <DevAgent w={NE} n="63" t="Notarize the Mac build" repo="app-repo" why="spawned by the Mac · card made on the PC, needs the Mac" />
            <div className="rounded-lg bg-secondary/60 px-2.5 py-2 text-[11.5px] text-ink-2">
              The PC wrote the spawn request for #63; the card needs the Mac and the Mac has app-repo trusted, so it started nettle. Stop and End from the PC go to the Mac the same way.
            </div>
            <div className="rounded-lg bg-secondary/60 px-2.5 py-2 text-[11.5px] text-ink-2">
              Not here: game-repo lives only on the PC, so #64 and #67 wait for the PC even with 11 slots free here; #65 needs Windows.
            </div>
            <Caps>Yours</Caps>
            <DevAgent w={BA} n="57" t="Crash reporter" repo="app-repo" why="terminal agent · you gave it this card" />
            <div className="mt-2 grid gap-1.5 rounded-lg border border-border px-3 py-2 text-[12px] text-muted-foreground">
              <span className="flex items-center gap-2"><SmartphoneIcon className="size-4" /> Phone: posts cards, never spawns</span>
              <span className="flex items-center gap-2"><ServerIcon className="size-4" /> Home server: relay only, never spawns</span>
            </div>
          </DevCol>
        </div>
        <Side title="Which device spawns" w={320}>
          <p>The device that has the card&apos;s repo, preferring the always-on PC when both have it.</p>
          <p>A card that names the Mac, or a repo only the Mac has, spawns on the Mac wherever the card was made.</p>
          <p>Each device has a cap you pick (6 and 12 here; about 0.5 GB an agent). Over the cap a card queues for that device and says its place; a card moves to another device only when this one can&apos;t run it at all.</p>
          <Note>If the device is offline, the request waits on the card and moves to the next device that has the repo.</Note>
        </Side>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-9: terminal agents and "Keep this one for me"
// ======================================================================

function Hover({ w, title, sub, children }: { w: Who; title: ReactNode; sub: ReactNode; children: ReactNode }) {
  return (
    <div className="grid w-[400px] gap-3 rounded-xl border border-border bg-background p-4 shadow-2xl">
      <div className="flex items-center gap-2.5">
        <Av w={w} />
        <span className="grid leading-tight">
          <span className="text-[14px] font-semibold text-ink">{title}</span>
          <span className="text-[11.5px] text-muted-foreground">{sub}</span>
        </span>
      </div>
      {children}
    </div>
  )
}

function View9() {
  return (
    <Window current="home">
      <Header title="Agents" right={<span className="text-[12.5px] text-muted-foreground">hover cards from the sidebar</span>} />
      <div className="flex min-h-0 flex-1 items-start gap-8 p-8">
        <div className="grid gap-2">
          <Caps>A terminal agent (Yours)</Caps>
          <Hover w={JU} title={<span className="flex items-center gap-2">juniper <Kind /></span>} sub="Stephen's PC · ~/projects · idle 2h · started by you">
            <div className="grid gap-1 text-[12.5px] text-ink-2">
              <span>Long context: 3 cards this week (#41, #49, #52); answered on #61 at 14:15.</span>
            </div>
            <div className="flex items-center gap-3 rounded-lg border border-border px-3 py-2">
              <span className="grid flex-1 leading-tight">
                <span className="text-[13px] text-ink">Offer it cards</span>
                <span className="text-[11.5px] text-muted-foreground">off: it gets work only when you give it some</span>
              </span>
              <Toggle on={false} />
            </div>
            <div className="flex flex-wrap gap-1.5">
              <Btn primary><PlusIcon className="size-3.5" />Give it a card</Btn>
              <Btn><MessagesSquareIcon className="size-3.5" />Message</Btn>
              <Btn>Wind down</Btn>
            </div>
            <div className="grid gap-1 rounded-lg bg-secondary/60 px-3 py-2 text-[12px] text-ink-2">
              <span className="text-[11px] text-muted-foreground uppercase">Give it a card</span>
              {["#67 Fish schooling AI (queued)", "#65 Windows installer (can't start)", "+ New card for juniper…"].map((t, i) => (
                <span key={t} className={cn("rounded px-1.5 py-0.5", i === 0 && "bg-secondary text-ink")}>{t}</span>
              ))}
            </div>
          </Hover>
        </div>
        <div className="grid gap-2">
          <Caps>A spawned agent (On cards)</Caps>
          <Hover w={TH} title={<span className="flex items-center gap-2">thyme <Kind spawned n="58" /></span>} sub="Stephen's PC · started for #58 at 14:02 · working">
            <div className="flex items-center gap-2 text-[12.5px] text-ink-2">
              <Chip n="58" t="Lantern flicker on the dock" /> step 3 of 4
            </div>
            <div className="flex flex-wrap gap-1.5">
              <Btn><TerminalIcon className="size-3.5" />Attach</Btn>
              <Btn><FileTextIcon className="size-3.5" />Logs</Btn>
              <Btn><SquareIcon className="size-3" />Stop</Btn>
              <Btn danger><XIcon className="size-3.5" />End</Btn>
            </div>
            <div className="grid gap-1.5 rounded-lg border border-primary/50 bg-primary/5 px-3 py-2">
              <Btn primary><UserRoundCheckIcon className="size-3.5" />Keep this one for me</Btn>
              <span className="text-[11.5px] text-ink-2">It becomes one of Yours: it isn&apos;t ended when #58 is Done, keeps its context, and is never offered cards unless you turn that on.</span>
            </div>
          </Hover>
        </div>
        <div className="grid w-[300px] content-start gap-3 pt-6 text-[12.5px] leading-snug text-ink-2">
          <div className="text-[15px] font-semibold text-ink">Two kinds, one roster</div>
          <p><b className="text-ink">Terminal agents</b> are the ones you start and keep. They see and talk to everything, and lead cards when you give them one; Irori spawns the helpers they ask for.</p>
          <p><b className="text-ink">Spawned agents</b> live for one card or thread and go with it.</p>
          <p className="flex items-center gap-1.5 text-muted-foreground"><CpuIcon className="size-3.5" /> Idle terminal agents stay idle; Irori doesn&apos;t nudge or end them.</p>
          <Note>&ldquo;Offer it cards&rdquo; on brings back today&apos;s hand-out for that one agent: it takes unheld cards before Irori spawns.</Note>
          <span className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground"><GitPullRequestIcon className="size-3.5" /> A terminal lead merges its helpers&apos; branches the same way a spawned lead does.</span>
        </div>
      </div>
    </Window>
  )
}

// ======================================================================
// sa-10: a Talk thread answered by an agent spawned for it
// ======================================================================

function View10() {
  return (
    <Window current="talk">
      <Header title="#game-repo · Thread" right={<span className="text-[12.5px] text-muted-foreground">started from a message by Stephen · 13:30</span>} />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-3">
            <Msg w={ST} at="13:30">
              Why does the dock shader take 40 s to compile on a clean build? It didn&apos;t used to.
            </Msg>
            <Auto at="13:30" extra={<span className="mt-0.5 text-[11.5px] text-muted-foreground">nobody was named; terminal agents aren&apos;t handed threads, so Irori started one for this thread</span>}>
              Started <b className="text-ink">rue</b> for this thread on the PC · Sonnet · medium · repo root (no code changes expected).
            </Auto>
            <Msg w={RU} sp="thread" at="13:33">
              Since <Chip n="52" t="Water caustics" /> the dock shader includes <Code>caustics.wgsl</Code>, which pulls the 4 MB noise table in as a <Code>const</Code> array; naga validates every element. Loading it as a storage buffer instead brings the compile back to about 2 s on my run.
            </Msg>
            <Msg w={ST} at="13:35">
              Thanks, that&apos;s what I needed. I&apos;ll fold it into the shader work.
            </Msg>
            <Msg w={RU} sp="thread" at="13:36" extra={<ProposedNote by="rue" text="Big lookup tables in WGSL go in storage buffers, not const arrays: naga validates const arrays element by element (dock shader)." />}>
              <b className="text-ink">Closing.</b> Answer above, with the measurement in this thread; a note proposed so the next agent on the shader knows.
            </Msg>
            <Sys>14:06 · thread quiet for 30 minutes · rue ended · hand-off written</Sys>
            <Msg w={BA} at="14:20">
              <M at="rue" /> does the same apply to the fog table?
            </Msg>
            <Auto at="14:20" extra={<span className="mt-0.5 text-[11.5px] text-muted-foreground">rue has ended; a message to it lands here, where its answer is. Nothing is started for it.</span>}>
              A message for rue, who has finished, landed on this thread.
            </Auto>
          </div>
          <div className="p-4"><Composer ph="Reply in thread · naming nobody starts a fresh agent if none is on it" /></div>
        </div>
        <Side title="A thread gets its own agent too" w={340}>
          <p>A Talk question naming nobody used to wait for a free agent. Now Irori starts one for the thread, in the channel&apos;s repo, and it answers there.</p>
          <div className="grid gap-1.5">
            <Caps>How long it lives</Caps>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">while open</span><span>follow-ups in the thread reach the same agent</span></div>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">quiet or closed</span><span>after the idle limit, or when the thread is resolved, it writes its hand-off and goes</span></div>
            <div className="grid grid-cols-[96px_1fr] gap-2"><span className="font-medium text-ink">becomes a card</span><span>an ask turned into a card keeps its agent as the card&apos;s; a card made from a Talk thread gets its own</span></div>
          </div>
          <p>It isn&apos;t a member of #game-repo: it read the channel&apos;s latest lines in its brief, and posts only in this thread.</p>
          <Note>Its answer and the proposed note stay in the thread after it has gone. Naming an agent still sends to that agent; in the channel itself, naming nobody wakes nobody.</Note>
        </Side>
      </div>
    </Window>
  )
}

export default function SpawnedMockups() {
  const v = new URLSearchParams(window.location.search).get("view") ?? "1"
  const views: Record<string, () => ReactNode> = {
    "1": View1,
    "2": View2,
    "3": View3,
    "4": View4,
    "5": View5,
    "6": View6,
    "7": View7,
    "8": View8,
    "9": View9,
    "10": View10,
  }
  const V = views[v] ?? View1
  return <div className="h-svh w-full overflow-hidden bg-background text-foreground"><V /></div>
}
