// Card #382: design-round mockups for Talk (threads, channels, DMs).
// Browser preview only (?mockup=talk&view=1..5); static JSX on made-up
// data, no lib/ calls, no state. Lives on the mockup branch, never main.

import type { ReactNode } from "react"
import {
  ArrowUpRightIcon,
  AtSignIcon,
  CheckIcon,
  ChevronLeftIcon,
  EyeIcon,
  HashIcon,
  HomeIcon,
  InboxIcon,
  LayoutGridIcon,
  MessageSquareTextIcon,
  MessagesSquareIcon,
  PanelLeftIcon,
  PaperclipIcon,
  PlusIcon,
  SearchIcon,
  SettingsIcon,
  SquarePenIcon,
  XIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
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

function Sidebar({ talk, current = "talk" }: { talk: boolean; current?: string }) {
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
        <NavRow icon={LayoutGridIcon} label="Boards" current={current === "boards"} />
        <NavRow icon={InboxIcon} label={current === "inbox" ? "Inbox" : "Held"} current={current === "inbox"} />
        <div className="px-2.5 pt-4 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          Pinned boards
        </div>
        <div className="flex h-8 items-center gap-2.5 px-2.5 text-sm text-ink-2">
          <span className="rounded bg-secondary px-1 font-mono text-[10px] text-muted-foreground">ml</span>
          MLG
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
        Stephen&apos;s MacBook
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
}: {
  children: ReactNode
  talk?: boolean
  current?: string
  className?: string
}) {
  return (
    <div className={cn("flex h-full overflow-hidden", className)}>
      <Sidebar talk={talk} current={current} />
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

function TalkList({
  current = "suzuri",
  threadsFromChannels = false,
}: {
  current?: string
  threadsFromChannels?: boolean
}) {
  const hash = <HashIcon className="size-4 shrink-0 text-muted-foreground" />
  const thr = <MessageSquareTextIcon className="size-4 shrink-0 text-muted-foreground" />
  return (
    <div className="flex w-[260px] shrink-0 flex-col gap-1 overflow-hidden border-r border-border px-2 py-3">
      <div className="mx-1 flex items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-1.5 text-[12.5px] text-muted-foreground">
        <SearchIcon className="size-3.5" /> Search talk
        <SquarePenIcon className="ml-auto size-4 text-ink-2" />
      </div>
      <Group label="Threads · 3 open" add>
        <ListRow icon={thr} label="Why is the Mac build slow?" sub="waiting for a free agent" />
        <ListRow
          icon={thr}
          label="Restarting Irori re-toasts…"
          sub={threadsFromChannels ? "from #irori · kowhai answered" : "kowhai answered"}
          unread={1}
          current={current === "thread"}
        />
        <ListRow icon={thr} label="Lobby list: Steam or relay?" sub="you answered" />
      </Group>
      <Group label="Channels" add>
        <ListRow icon={hash} label="suzuri" unread={current === "suzuri" ? undefined : 3} current={current === "suzuri"} />
        <ListRow icon={hash} label="releases" unread="@" current={current === "releases"} />
        <ListRow icon={hash} label="irori" />
        <ListRow icon={hash} label="mlg" sub="MLG · shared with Blake" />
      </Group>
      <Group label="Direct messages" add>
        <ListRow icon={<Av w={KO} size={5} />} label="kowhai" sub="working on #330" current={current === "dm"} unread={current === "dm" ? undefined : 1} />
        <ListRow icon={<Av w={BL} size={5} />} label="Blake" sub="MLG" />
        <ListRow
          icon={<EyeIcon className="size-4 shrink-0 text-muted-foreground" />}
          label="kowhai ↔ kestrel"
          sub="agents talking · read-only"
          muted
        />
      </Group>
      <div className="mt-auto px-2 pt-2 text-[11px] text-muted-foreground">
        Resolved threads: 41 · searchable
      </div>
    </div>
  )
}

// ---------- 1: where it lives ----------

function Frame({ title, pick, children, caption }: { title: string; pick?: boolean; children: ReactNode; caption: ReactNode }) {
  return (
    <div className="grid min-w-0 grid-rows-[auto_1fr_auto] gap-2">
      <div className="flex items-center gap-2">
        <span className="text-[15px] font-semibold text-ink">{title}</span>
        {pick && <Badge variant="success">My pick</Badge>}
      </div>
      <div
        className={cn(
          "h-[640px] overflow-hidden rounded-xl border bg-background",
          pick ? "border-success/70 ring-2 ring-success/30" : "border-border"
        )}
      >
        <div className="h-[900px] w-[1100px] origin-top-left scale-[0.581]">{children}</div>
      </div>
      <div className="text-[12.5px] leading-snug text-ink-2">{caption}</div>
    </div>
  )
}

function ChannelBody({ compact }: { compact?: boolean }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-1 px-4 py-3">
      <Msg w={ST} at="10:02">
        Is anything still blocking the suzuri move? <M at="kowhai">{""}</M>
      </Msg>
      <Msg w={KO} at="10:03">
        Only the PC transfer check; kestrel is on it. <Chip n="330" t="Own helpers" />
      </Msg>
      {!compact && (
        <Msg w={KE} at="10:21">
          Live send/receive passed on the PC with no Go installed.
        </Msg>
      )}
      <div className="mt-auto">
        <Composer ph="Message #suzuri" />
      </div>
    </div>
  )
}

function View1() {
  return (
    <div className="grid grid-cols-3 gap-6 p-6">
      <Frame
        title="A · Talk in the sidebar"
        pick
        caption={
          <>
            One new sidebar item. Threads, channels and DMs in one list, the conversation beside
            it. <b>It replaces</b> Home&apos;s Threads panel and the MESSAGE cards in Needs you
            (a message to you is a DM). Home keeps only what needs you.
          </>
        }
      >
        <Window>
          <Header title="Talk" />
          <div className="flex min-h-0 flex-1">
            <TalkList />
            <div className="flex min-w-0 flex-1 flex-col">
              <div className="flex h-11 items-center gap-2 border-b border-border px-4 text-sm font-medium text-ink">
                <HashIcon className="size-4" /> suzuri
              </div>
              <ChannelBody />
            </div>
          </div>
        </Window>
      </Frame>
      <Frame
        title="B · Inside Inbox"
        caption={
          <>
            Held becomes Inbox, with tabs: Held · Threads · Channels · DMs. No new sidebar item,
            but policy holds and conversations share one screen, and a channel is two clicks
            deep.
          </>
        }
      >
        <Window current="inbox">
          <Header title="Inbox" />
          <div className="flex gap-1 border-b border-border px-6 pt-2 text-[13px]">
            {["Held 3", "Threads 3", "Channels", "DMs 1"].map((t, i) => (
              <span
                key={t}
                className={cn(
                  "rounded-t-md px-3 py-2 text-ink-2",
                  i === 2 && "border-b-2 border-primary font-medium text-ink"
                )}
              >
                {t}
              </span>
            ))}
          </div>
          <div className="flex min-h-0 flex-1">
            <div className="w-[220px] border-r border-border p-2">
              <ListRow icon={<HashIcon className="size-4" />} label="suzuri" current />
              <ListRow icon={<HashIcon className="size-4" />} label="releases" unread="@" />
              <ListRow icon={<HashIcon className="size-4" />} label="irori" />
            </div>
            <ChannelBody />
          </div>
        </Window>
      </Frame>
      <Frame
        title="C · Docked beside the Board"
        caption={
          <>
            Today&apos;s Threads panel grows tabs (Threads · Channels · DMs) and docks on every
            page. Talk is always in view, but long history is cramped in a 360px column and it
            squeezes the grid.
          </>
        }
      >
        <Window talk={false} current="home">
          <Header title="Home" />
          <div className="flex min-h-0 flex-1 gap-3 p-4">
            <div className="grid flex-1 content-start gap-3">
              {["Fix the boat clipping through the dock", "Review PR #212", "Ship the release"].map((t) => (
                <div key={t} className="rounded-xl border border-border bg-card p-4 text-[14px] font-medium text-ink">
                  {t}
                  <div className="mt-2 h-2 w-2/3 rounded bg-secondary" />
                </div>
              ))}
            </div>
            <div className="flex w-[360px] flex-col rounded-xl border border-border bg-card">
              <div className="flex gap-3 border-b border-border px-3 py-2 text-[12px]">
                <span className="text-muted-foreground">Threads 3</span>
                <span className="font-medium text-ink">Channels</span>
                <span className="text-muted-foreground">DMs 1</span>
                <XIcon className="ml-auto size-4 text-muted-foreground" />
              </div>
              <div className="flex items-center gap-1 px-3 pt-2 text-[12px] text-ink">
                <HashIcon className="size-3.5" /> suzuri ▾
              </div>
              <ChannelBody compact />
            </div>
          </div>
        </Window>
      </Frame>
    </div>
  )
}

// ---------- 2: a channel ----------

function Members() {
  const row = (w: Who, s: "working" | "idle" | "gone" | "online", what: string) => (
    <div className="flex items-center gap-2 px-1 py-1 text-[12.5px]">
      <Av w={w} size={6} />
      <span className={cn("font-medium", s === "gone" ? "text-muted-foreground line-through" : "text-ink")}>
        {w.name}
      </span>
      <span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <Dot s={s} /> {what}
      </span>
    </div>
  )
  return (
    <div className="flex w-[260px] shrink-0 flex-col gap-1 border-l border-border px-3 py-3">
      <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        Members · 6
      </div>
      {row(ST, "online", "here")}
      {row(KO, "working", "#330")}
      {row(PR, "working", "#334")}
      {row(KE, "idle", "PC")}
      {row(NU, "idle", "Mac")}
      {row({ name: "scabious", kind: "agent", tone: "bg-chart-2" }, "gone", "gone 2h")}
      <div className="mt-2 grid gap-1 rounded-lg border border-border p-2.5 text-[11.5px] text-muted-foreground">
        <span className="font-medium text-ink-2">On the board: Personal</span>
        Agents join when you @ them or add them. Leaving a channel is one click; it stays.
      </div>
    </div>
  )
}

function ChannelHistory({ withThreadLink, withPointer }: { withThreadLink?: boolean; withPointer?: boolean }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-4 pt-3">
      <Sys>Yesterday</Sys>
      <Msg w={PR} at="16:40">
        Windows installer now takes over Go&apos;s install: <Chip n="334" t="Installers take over Go" />
      </Msg>
      <Sys>Today</Sys>
      <Msg w={ST} at="10:02">
        Is anything still blocking the move? <M at="kowhai">{""}</M>
      </Msg>
      <Msg
        w={KO}
        at="10:03"
        extra={
          withThreadLink ? (
            <span className="mt-1 flex items-center gap-2 text-[12px] text-primary">
              <span className="flex -space-x-1">
                <Av w={KE} size={5} />
                <Av w={ST} size={5} />
              </span>
              3 replies · open thread
              <Badge variant="success" className="h-4 px-1.5 text-[10px]">resolved</Badge>
            </span>
          ) : null
        }
      >
        Only the live PC transfer. It needs one Firewall click from you, or we wait for #334&apos;s install.
      </Msg>
      {withPointer && (
        <div className="ml-11 flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-1.5 text-[12px] text-ink-2">
          <MessageSquareTextIcon className="size-3.5 text-muted-foreground" />
          Stephen started a thread: <span className="font-medium text-ink">“Firewall prompt on the PC”</span>
          <span className="text-muted-foreground">· in Threads</span>
        </div>
      )}
      <Msg
        w={ST}
        at="10:08"
        hl
        extra={
          <span className="mt-1 flex items-center gap-2 text-[11.5px] text-muted-foreground">
            <ArrowUpRightIcon className="size-3.5" />
            Made a card from this:
            <Chip n="371" t="Tag 0.10.0 once #96 merges" />
          </span>
        }
      >
        Let&apos;s tag 0.10.0 the moment #96 merges, not before.
      </Msg>
      <Msg w={NU} at="10:09">
        Took <Chip n="371" t="Tag 0.10.0 once #96 merges" />; I&apos;ll wait on the merge.
      </Msg>
      <Msg w={ST} at="10:14">
        <M at="kestrel">{""}</M> can you run cargo test on the PC at c2885cb?
      </Msg>
    </div>
  )
}

function MsgMenu() {
  return (
    <div className="absolute top-[318px] right-[300px] z-10 grid w-48 rounded-lg border border-border bg-popover p-1 text-[12.5px] shadow-xl">
      {["Reply in thread", "Make a card", "Copy link", "Mark unread"].map((t, i) => (
        <span key={t} className={cn("rounded-md px-2 py-1.5 text-ink-2", i === 1 && "bg-secondary text-ink")}>
          {t}
        </span>
      ))}
    </div>
  )
}

function View2() {
  return (
    <Window>
      <Header
        title={
          <span className="flex items-center gap-1.5">
            <HashIcon className="size-5" /> suzuri
          </span>
        }
        right={
          <span className="text-[12.5px] text-muted-foreground">
            The suzuri app: releases, the Rust move, bugs
          </span>
        }
      />
      <div className="relative flex min-h-0 flex-1">
        <TalkList />
        <div className="flex min-w-0 flex-1 flex-col">
          <ChannelHistory />
          <div className="px-4 pb-4">
            <Composer ph="@kes" popup />
          </div>
        </div>
        <Members />
        <MsgMenu />
      </div>
    </Window>
  )
}

// 2a: Slack-style, a thread opens from a channel message (side pane).
function View2a() {
  return (
    <Window>
      <Header title={<span className="flex items-center gap-1.5"><HashIcon className="size-5" /> suzuri</span>} />
      <div className="flex min-h-0 flex-1">
        <TalkList />
        <div className="flex min-w-0 flex-1 flex-col">
          <ChannelHistory withThreadLink />
          <div className="px-4 pb-4"><Composer ph="Message #suzuri" /></div>
        </div>
        <div className="flex w-[380px] shrink-0 flex-col border-l border-border">
          <div className="flex h-11 items-center gap-2 border-b border-border px-4 text-[13px]">
            <span className="font-medium text-ink">Thread</span>
            <span className="text-muted-foreground">in #suzuri</span>
            <Badge variant="success" className="ml-2">resolved</Badge>
            <XIcon className="ml-auto size-4 text-muted-foreground" />
          </div>
          <div className="grid content-start gap-1 p-3">
            <Msg w={KO} at="10:03">Only the live PC transfer. It needs one Firewall click…</Msg>
            <Msg w={ST} at="10:05">Run it now, I&apos;ll click Allow.</Msg>
            <Msg w={KE} at="10:31">Passed, and no prompt came up.</Msg>
            <Msg w={ST} at="10:32">Great. <span className="text-success">✓ Resolved</span></Msg>
          </div>
          <div className="mt-auto p-3"><Composer ph="Reply…" /></div>
        </div>
      </div>
    </Window>
  )
}

// 2b: threads only as their own list; a channel message can start one,
// which leaves a pointer in the channel.
function View2b() {
  return (
    <Window>
      <Header title={<span className="flex items-center gap-1.5"><MessageSquareTextIcon className="size-5" /> Firewall prompt on the PC</span>}
        right={<><Badge variant="info">answered</Badge><span className="rounded-md border border-border px-3 py-1 text-[12.5px] text-ink-2">Resolve</span></>} />
      <div className="flex min-h-0 flex-1">
        <TalkList current="thread" threadsFromChannels />
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-2 border-b border-border px-4 py-2 text-[12px] text-muted-foreground">
            Started from a message in <span className="flex items-center text-primary"><HashIcon className="size-3.5" />suzuri</span>
            · the channel shows a one-line pointer
          </div>
          <div className="grid content-start gap-1 p-4">
            <Msg w={KO} at="10:03">Only the live PC transfer. It needs one Firewall click from you, or we wait for #334&apos;s install.</Msg>
            <Msg w={ST} at="10:05">Run it now, I&apos;ll click Allow. <M at="kestrel">{""}</M></Msg>
            <Msg w={KE} at="10:31">Passed against the installed bundle, and no Firewall prompt came up.</Msg>
          </div>
          <div className="mt-auto px-4 pb-4"><Composer ph="Reply in this thread…" /></div>
        </div>
        <div className="w-[420px] shrink-0 border-l border-border opacity-80">
          <div className="flex h-11 items-center gap-1.5 border-b border-border px-4 text-[13px] text-muted-foreground">
            <HashIcon className="size-4" /> suzuri (how the channel reads)
          </div>
          <ChannelHistory withPointer />
        </div>
      </div>
    </Window>
  )
}

// ---------- 3: DM vs today's card thread ----------

function View3() {
  return (
    <div className="grid h-full grid-cols-2 gap-6 p-6">
      <div className="grid grid-rows-[auto_1fr] gap-2">
        <div className="text-[15px] font-semibold text-ink">Today: a card&apos;s thread (#329)</div>
        <div className="overflow-hidden rounded-xl border border-border bg-card p-5">
          <div className="text-lg font-semibold text-ink">Move rust-suzuri into StephenSHorton/suzuri</div>
          <div className="mt-1 flex gap-2 text-[12px] text-muted-foreground">
            <Badge variant="info">In review</Badge> held by scabious · 7/7 steps
          </div>
          <div className="mt-4 grid gap-3 text-[12.5px]">
            {[
              ["scabious", "Cancelled #331–#333; #330 already covers them. #329 waits on #330 and #334."],
              ["primrose", "#334 has landed on PR #96's branch: 587f7a5…"],
              ["zorilla", "Heads-up from #368: I'm landing on rust-suzuri main, so your s7 re-merge…"],
              ["scabious", "PR #96 is ready for your review…"],
            ].map(([a, t], i) => (
              <div key={i} className="grid grid-cols-[56px_1fr] gap-2">
                <span className="text-muted-foreground tabular-nums">{["1h", "40m", "20m", "5m"][i]}</span>
                <span className="text-ink-2">
                  <span className="font-medium text-ink">{a}</span> {t}
                </span>
              </div>
            ))}
          </div>
          <Note className="mt-5">
            About the card&apos;s work only. It ends when the card is Done. Agents post updates
            into it; you mostly read.
          </Note>
        </div>
      </div>
      <div className="grid grid-rows-[auto_1fr] gap-2">
        <div className="text-[15px] font-semibold text-ink">New: a DM with an agent (kowhai)</div>
        <div className="flex flex-col overflow-hidden rounded-xl border border-border bg-card">
          <div className="flex items-center gap-2.5 border-b border-border px-4 py-3">
            <Av w={KO} />
            <div className="grid">
              <span className="text-[14px] font-medium text-ink">kowhai</span>
              <span className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
                <Dot s="working" /> working on <Chip n="330" t="Own helpers" /> · Mac · ~/projects/suzuri-move
              </span>
            </div>
          </div>
          <div className="grid content-start gap-1 p-3">
            <Sys>Monday</Sys>
            <Msg w={ST} at="09:12">How would you feel about dropping the workspace panel altogether?</Msg>
            <Msg w={KO} at="09:13">
              Happy to. It&apos;s the only thing that needs Go&apos;s binary. I&apos;d make it a card so others see it.
            </Msg>
            <Msg w={ST} at="09:14" hl extra={
              <span className="mt-1 flex items-center gap-2 text-[11.5px] text-muted-foreground">
                <ArrowUpRightIcon className="size-3.5" /> Made a card: <Chip n="330" t="Drop the workspace panel" />
              </span>}>
              Do it.
            </Msg>
            <Sys>Today</Sys>
            <Msg w={KO} at="13:02">
              Panel&apos;s gone and the PC check passed. It&apos;s on PR #96. Anything else before I go idle?
            </Msg>
          </div>
          <div className="mt-auto p-3"><Composer ph="Message kowhai" /></div>
          <Note className="m-3 mt-0">
            Lasts across cards and restarts. Every message wakes kowhai (it&apos;s a DM). If kowhai
            is gone, the DM stays and says so; the next message offers “ask another agent”.
          </Note>
        </div>
      </div>
    </div>
  )
}

// ---------- 4: how agents behave ----------

function View4() {
  return (
    <div className="grid h-full grid-cols-[1fr_520px] gap-6 p-6">
      <div className="flex flex-col overflow-hidden rounded-xl border border-border bg-card">
        <div className="flex h-11 items-center gap-2 border-b border-border px-4 text-sm font-medium text-ink">
          <HashIcon className="size-4" /> releases
          <span className="ml-2 text-[12px] font-normal text-muted-foreground">members: Stephen, Blake, nutmeg, kestrel</span>
        </div>
        <div className="grid content-start gap-1 p-3">
          <Msg w={ST} at="14:00">0.10.2 looked good on the phone.</Msg>
          <Note className="ml-11">① No @: no agent wakes. Members see it next time they read #releases.</Note>
          <Msg w={ST} at="14:01"><M at="nutmeg">{""}</M> cut 0.10.2 when the Mac check is in.</Msg>
          <Note className="ml-11">② @nutmeg wakes nutmeg now (same rule as cards, decision 39). Rate limit and Held still apply.</Note>
          <Msg w={NU} at="14:02">On it: I&apos;ll make a card so it shows on the Board. <Chip n="372" t="Release 0.10.2" /></Msg>
          <Msg w={ST} at="14:05"><M at="Blake">{""}</M> want to try it on your phone?</Msg>
          <Note className="ml-11">③ Names only a person, so “Agents answer now” turned itself off: no agent is woken or handed it, and Blake gets a desktop card. Members still read it at their next turn and may reply if they have something to add or correct. (#378: today one agent per person answers.)</Note>
          <Msg w={BL} at="14:20">Installed, works.</Msg>
          <Sys>nutmeg came back from idle at 15:10</Sys>
          <Msg w={NU} at="15:11">Released 0.10.2; Blake&apos;s phone reported it. <Chip n="372" t="Release 0.10.2" /></Msg>
          <Msg w={KE} at="15:12">Small correction: the Android build is 0.10.2-1, the tag was re-cut once.</Msg>
          <Note className="ml-11">④ kestrel wasn&apos;t woken; it read #releases at its next turn and spoke up because it had a correction.</Note>
        </div>
        <div className="mt-auto p-3"><Composer ph="@Blake does it still crash on rotate?" agents="off" /></div>
      </div>
      <div className="grid content-start gap-3">
        <div className="text-[15px] font-semibold text-ink">What nutmeg gets when ② wakes it</div>
        <pre className="overflow-hidden rounded-xl border border-border bg-secondary/60 p-4 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-ink-2">
{`[irori-wake] Stephen mentioned you in #releases:
  "@nutmeg cut 0.10.2 when the Mac check is in."

Since you last read #releases (yesterday 18:02):
  Stephen 14:00  0.10.2 looked good on the phone.
  (3 earlier messages: talk_read #releases for more)

Answer in the channel with talk_post #releases.
Work goes on a card: board_create from=<this message>.`}
        </pre>
        <div className="text-[15px] font-semibold text-ink">The rules, short</div>
        <ul className="grid gap-1.5 text-[12.5px] leading-snug text-ink-2">
          <li>• <b>Channels</b>: an agent wakes only when @-mentioned. Otherwise it reads the channel when it wants (talk_read), and gets “since you last read” on its next wake.</li>
          <li>• <b>DMs</b>: every message wakes the other side.</li>
          <li>• <b>“Agents answer now”</b> (composer and thread toggle): on when you name nobody or an agent, off when you name only people; flip it either way. Off: nobody is woken or handed it.</li><li>• <b>Never siloed</b>: agents in a channel or thread always see it at their next turn, and may reply when they have something relevant or a correction.</li>
          <li>• <b>Agent↔agent DMs</b> show in Talk read-only, so nothing between agents is hidden.</li>
          <li>• Work never stays in talk: an agent makes a card, and the message links it.</li>
        </ul>
      </div>
    </div>
  )
}

// ---------- 5: phone ----------

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

function View5() {
  return (
    <div className="flex h-full items-start justify-center gap-10 p-6">
      <Phone title="Talk">
        <div className="overflow-hidden px-2 [&_*]:text-[14px]">
          <TalkListPhone />
        </div>
      </Phone>
      <Phone title={<span className="flex items-center gap-1"><HashIcon className="size-4" />suzuri</span>} back>
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-1 pt-2">
          <Msg w={ST} at="10:02">Is anything still blocking the move? <M at="kowhai">{""}</M></Msg>
          <Msg w={KO} at="10:03">Only the live PC transfer. <Chip n="330" t="Own helpers" /></Msg>
          <Msg w={ST} at="10:08" extra={<span className="mt-1 flex items-center gap-1 text-[11px] text-muted-foreground"><ArrowUpRightIcon className="size-3" /> <Chip n="371" t="Tag 0.10.0" /></span>}>
            Let&apos;s tag 0.10.0 the moment #96 merges.
          </Msg>
          <Msg w={NU} at="10:09">Took #371.</Msg>
        </div>
        <div className="p-2"><Composer ph="Message #suzuri" /></div>
      </Phone>
      <div className="grid w-[340px] content-start gap-3 pt-10 text-[13px] leading-snug text-ink-2">
        <div className="text-[15px] font-semibold text-ink">Phone</div>
        <p>Talk becomes a bottom tab beside Home and Boards. The list is the same three groups; a tap opens the conversation full-screen, with Back.</p>
        <p>Long-press a message: Reply in thread / Make a card. Members and status are behind the channel name.</p>
        <p>A mention or DM is a phone notification; plain channel messages only bump the unread count.</p>
      </div>
    </div>
  )
}

function TalkListPhone() {
  return (
    <div className="grid gap-1 py-2">
      <Group label="Threads · 3 open">
        <ListRow icon={<MessageSquareTextIcon className="size-4 text-muted-foreground" />} label="Restarting Irori re-toasts…" sub="kowhai answered" unread={1} />
        <ListRow icon={<MessageSquareTextIcon className="size-4 text-muted-foreground" />} label="Why is the Mac build slow?" sub="waiting for a free agent" />
      </Group>
      <Group label="Channels">
        <ListRow icon={<HashIcon className="size-4 text-muted-foreground" />} label="suzuri" unread={3} />
        <ListRow icon={<HashIcon className="size-4 text-muted-foreground" />} label="releases" unread="@" />
        <ListRow icon={<HashIcon className="size-4 text-muted-foreground" />} label="mlg" sub="shared with Blake" />
      </Group>
      <Group label="Direct messages">
        <ListRow icon={<Av w={KO} size={5} />} label="kowhai" sub="working on #330" unread={1} />
        <ListRow icon={<Av w={BL} size={5} />} label="Blake" />
      </Group>
      <div className="flex items-center gap-1 px-2 pt-2 text-[12px] text-success"><CheckIcon className="size-3.5" /> 41 resolved threads, searchable</div>
    </div>
  )
}

// ---------- 6: the people layer above boards (#383) ----------

function Swatch({ c, on }: { c: string; on?: boolean }) {
  return <span className={cn("size-5 rounded-full", c, on && "ring-2 ring-ink ring-offset-2 ring-offset-background")} />
}

function SpaceCard({
  name,
  who,
  boards,
  channels,
  note,
  pick,
}: {
  name: string
  who: ReactNode
  boards: string[]
  channels: string[]
  note: string
  pick?: boolean
}) {
  return (
    <div className={cn("grid gap-3 rounded-xl border bg-card p-4", pick ? "border-primary/60" : "border-border")}>
      <div className="flex items-center gap-2">
        <span className="text-[15px] font-semibold text-ink">{name}</span>
        <span className="ml-auto flex items-center gap-1">{who}</span>
      </div>
      <div className="grid grid-cols-2 gap-3 text-[12.5px]">
        <div className="grid content-start gap-1">
          <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Boards</span>
          {boards.map((b) => (
            <span key={b} className="flex items-center gap-1.5 text-ink-2"><LayoutGridIcon className="size-3.5 text-muted-foreground" />{b}</span>
          ))}
        </div>
        <div className="grid content-start gap-1">
          <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Channels</span>
          {channels.map((b) => (
            <span key={b} className="flex items-center gap-1 text-ink-2"><HashIcon className="size-3.5 text-muted-foreground" />{b}</span>
          ))}
        </div>
      </div>
      <div className="border-t border-border pt-2 text-[11.5px] text-muted-foreground">{note}</div>
    </div>
  )
}

function View6() {
  const BLc: Who = { name: "Blake", kind: "person" }
  const AD: Who = { name: "Addam", kind: "person" }
  return (
    <div className="grid h-full grid-cols-[300px_1fr_360px] gap-6 p-6">
      <div className="grid content-start gap-4">
        <div className="text-[15px] font-semibold text-ink">You</div>
        <div className="grid gap-3 rounded-xl border border-border bg-card p-4">
          <div className="flex items-center gap-3">
            <span className="flex size-12 items-center justify-center rounded-full bg-chart-2 text-lg font-semibold text-background">S</span>
            <div className="grid">
              <span className="text-[15px] font-medium text-ink">Stephen</span>
              <span className="text-[12px] text-muted-foreground">3 devices · 6 agents</span>
            </div>
          </div>
          <div className="grid gap-1.5">
            <span className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Your colour (#385)</span>
            <div className="flex gap-2">
              <Swatch c="bg-chart-2" on /><Swatch c="bg-primary" /><Swatch c="bg-chart-3" /><Swatch c="bg-chart-4" /><Swatch c="bg-destructive" />
            </div>
            <span className="text-[11.5px] text-muted-foreground">Others see your name, colour and avatar everywhere you share: boards, channels, DMs. Your agents wear it as a stripe.</span>
          </div>
        </div>
        <div className="text-[15px] font-semibold text-ink">Friends</div>
        <div className="grid gap-1 rounded-xl border border-border bg-card p-3 text-[13px]">
          {[[BLc, "bg-chart-4", "MLG"], [AD, "bg-chart-3", "Irori"]].map(([w, c, s], i) => (
            <div key={i} className="flex items-center gap-2 px-1 py-1">
              <span className={cn("flex size-6 items-center justify-center rounded-full text-[10px] font-semibold text-background", c as string)}>{(w as Who).name[0]}</span>
              <span className="font-medium text-ink">{(w as Who).name}</span>
              <span className="ml-auto text-[11.5px] text-muted-foreground">in {s as string}</span>
            </div>
          ))}
          <span className="px-1 pt-1 text-[12px] text-primary">+ Invite a friend</span>
        </div>
      </div>
      <div className="grid content-start gap-4">
        <div className="flex items-baseline gap-2">
          <span className="text-[15px] font-semibold text-ink">Spaces: a group of people that holds many boards and channels</span>
        </div>
        <SpaceCard
          name="Personal"
          who={<><Av w={ST} size={6} /><span className="text-[11.5px] text-muted-foreground">just you + your agents</span></>}
          boards={["Irori", "suzuri", "pipeline", "lemmings"]}
          channels={["irori", "suzuri", "releases"]}
          note="Today's private board, grown up: everything not shared lives here."
        />
        <SpaceCard
          pick
          name="MLG"
          who={<><Av w={ST} size={6} /><span className="flex size-6 items-center justify-center rounded-full bg-chart-4 text-[10px] font-semibold text-background">B</span></>}
          boards={["Farming", "Website refresh", "Art"]}
          channels={["general", "mlg-art", "builds"]}
          note="Shared with Blake. Who's in, and what reaches your agents from them (Open / Members wake / Ask me / Closed, decision 24), is set once here, not per board."
        />
        <SpaceCard
          name="Irori (with Addam)"
          who={<><Av w={ST} size={6} /><span className="flex size-6 items-center justify-center rounded-full bg-chart-3 text-[10px] font-semibold text-background">A</span></>}
          boards={["irori#"]}
          channels={["general"]}
          note="A friend can be in some spaces and not others. A DM with a friend sits above all spaces."
        />
      </div>
      <div className="grid content-start gap-3">
        <div className="text-[15px] font-semibold text-ink">How the sidebar reads</div>
        <div className="h-[460px] overflow-hidden rounded-xl border border-border bg-background">
          <aside className="flex h-full w-full flex-col bg-sidebar/60">
            <div className="m-2 flex items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-2 text-[13px]">
              <span className="flex size-5 items-center justify-center rounded bg-chart-4 text-[10px] font-semibold text-background">M</span>
              <span className="font-medium text-ink">MLG</span>
              <span className="text-muted-foreground">· with Blake</span>
              <span className="ml-auto text-muted-foreground">⌄</span>
            </div>
            <nav className="grid gap-0.5 px-2">
              <NavRow icon={HomeIcon} label="Home" />
              <NavRow icon={MessagesSquareIcon} label="Talk" current count={2} />
              <NavRow icon={LayoutGridIcon} label="Boards" />
            </nav>
            <div className="px-3 pt-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Channels</div>
            <div className="grid px-2">
              <ListRow icon={<HashIcon className="size-4 text-muted-foreground" />} label="general" unread={2} />
              <ListRow icon={<HashIcon className="size-4 text-muted-foreground" />} label="builds" />
            </div>
            <div className="px-3 pt-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Direct messages</div>
            <div className="grid px-2">
              <ListRow icon={<span className="flex size-5 items-center justify-center rounded-full bg-chart-4 text-[9px] font-semibold text-background">B</span>} label="Blake" />
            </div>
          </aside>
        </div>
        <Note>
          The switcher picks a space; Home, Talk and Boards then show that space. “All spaces” is
          one more entry, for Needs you across everything.
        </Note>
      </div>
    </div>
  )
}

export default function TalkMockups() {
  const v = new URLSearchParams(window.location.search).get("view") ?? "1"
  const view =
    v === "1" ? <View1 /> :
    v === "2" ? <View2 /> :
    v === "2a" ? <View2a /> :
    v === "2b" ? <View2b /> :
    v === "3" ? <View3 /> :
    v === "4" ? <View4 /> :
    v === "6" ? <View6 /> : <View5 />
  return <div className="h-svh w-full overflow-hidden bg-background text-foreground">{view}</div>
}
