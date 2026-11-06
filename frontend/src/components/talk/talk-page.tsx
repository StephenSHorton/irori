// TalkPage is Talk (card #390, design docs/design/talk-channels.md §1 b1,
// mockups talk-1 and talk-2b): one list beside the conversation. The list
// holds Threads (the person's asks to the room, card #116, once a panel
// beside Home's Board); Channels and Direct messages join it once Talk 1
// (#389) stores them. Resolved threads leave the list but stay one click
// away, folded at its foot.

import { cardRef } from "@/components/cards/bits"
import { useContext, useEffect, useState, type ReactNode } from "react"
import {
  ChevronDownIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  EyeIcon,
  HashIcon,
  MessageSquareTextIcon,
  PlusIcon,
  SearchIcon,
  SquarePenIcon,
  UserIcon,
} from "lucide-react"

import { AskBox } from "@/components/cards/ask-box"
import {
  ListRowsShape,
  MessageRowsShape,
} from "@/components/common/loading-shapes"
import { Loading } from "@/components/ui/loading"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Face } from "@/components/cards/faces"
import { type ThreadSummary } from "@/lib/cards"
import {
  dmGone,
  dmPerson,
  dmWith,
  listed,
  readAll,
  talkApi,
  waitingText,
  type Conversation as Conv,
  type Person,
} from "@/lib/conversations"
import { AgentRef } from "@/lib/models"
import { AllSpaces, convInSpace } from "@/lib/spaces"
import { LiveRoster } from "@/lib/thread"
import { parseTime, plainLine, relativeTime } from "@/lib/irori"
import { useNarrow } from "@/lib/media"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"
import {
  agentRows,
  agentSectionRows,
  archived,
  broadcastTo,
  dmSections,
  sectionRows,
  type Broadcast,
  sectionHeader,
  useFolds,
  type SectionKey,
} from "@/lib/talk-sections"
import {
  ThreadCards,
  ThreadView,
  openAfterResolve,
  resolvedShown,
  resolvedToggle,
  stateBadges,
  stateLine,
} from "./threads"
import { agentStatus } from "@/lib/agent-status"
import { ConversationView, memberStatus, statusOf } from "./conversation"
import { memberName, nameOr } from "@/lib/names"

// talkMatches is whether a thread fits the list's search: its title, what
// was asked, its #ref, or the newest reply.
export function talkMatches(t: ThreadSummary, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [t.title, t.text, t.short && cardRef(t), t.lastText ?? ""].some(
    (s) => !!s && s.toLowerCase().includes(q)
  )
}

// needsYou is whether a thread waits on the person: an agent answered and
// it isn't resolved, or no computer can start its agent (card #541). Talk's
// count in the sidebar counts these.
export function needsYou(t: Pick<ThreadSummary, "state" | "noAgent">): boolean {
  return t.state === "answered" || !!t.noAgent
}

// convMatches is whether a channel or DM fits the list's search: its name,
// topic, or a member's name.
export function convMatches(c: Conv, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [
    c.name,
    c.topic ?? "",
    c.lastText ?? "",
    ...c.members.map((m) => m.name ?? ""),
  ].some((s) => s.toLowerCase().includes(q))
}

export function TalkPage({
  threads: everyThread,
  conversations,
  agents,
  open,
  onOpen,
  now,
  why = "",
  board = "",
  space = AllSpaces,
  threadsLoading = false,
  conversationsLoading = false,
}: {
  // Every log's, each with its space (card #558).
  threads: ThreadSummary[]
  // True until each list's first read lands (card #598): its section is a
  // skeleton till then, not "No open threads", and never on a reload.
  threadsLoading?: boolean
  conversationsLoading?: boolean
  // Channels and DMs (Talk 1, #389).
  conversations: Conv[]
  // The live agents a new DM can go to.
  agents: { key: string; name: string; status: string }[]
  // The conversation open ("" none: the new-thread box).
  open: string
  onOpen: (id: string) => void
  now: Date
  // Why nobody is free to take a waiting thread (card #172).
  why?: string
  // The shared space the sidebar picked (card #393): new threads start
  // there, beside the ones listed, and so do new channels.
  board?: string
  // The picked space itself (card #461): the channels and DMs listed.
  space?: string
}) {
  const narrow = useNarrow()
  const [query, setQuery] = useState("")
  const [showResolvedPick, setShowResolved] = useState(false)
  // On a narrow window the pen opens the new-thread box over the list.
  const [composing, setComposing] = useState(false)
  const threads = everyThread.filter((t) => convInSpace(t, space))
  const shown = threads.filter((t) => talkMatches(t, query))
  const live = shown.filter((t) => t.state !== "resolved")
  const resolved = shown.filter((t) => t.state === "resolved")
  const showResolved =
    resolvedShown(showResolvedPick, open, threads) || !!query.trim()
  // The open one is looked up in all of them, so a link to another space's
  // channel or thread still opens; the list shows the picked space's (card
  // #461, #558).
  const current = everyThread.find((t) => t.id === open)
  const conv = current
    ? undefined
    : conversations.find((c) => c.convId === open)
  const { channels, dms } = listed(
    conversations.filter((c) => convInSpace(c, space) && convMatches(c, query))
  )
  const [adding, setAdding] = useState<"" | "channel" | "dm">("")
  const liveRoster = useContext(LiveRoster)
  const goneDm = (c: Conv) => dmGone(c, liveRoster)
  // The DM opened from the main list stays there while it is open, though
  // reading it would archive it (card #473); one opened from the Archive
  // stays in the Archive.
  // null: not decided yet, as the list hasn't loaded the open one (a reload
  // or a link straight to a DM); it is decided once the list has it.
  const keptFor = (id: string): string | null => {
    const c = conversations.find((x) => x.convId === id)
    if (!c) return null
    return c.kind === "dm" && !c.readOnly && !archived(c, goneDm) ? id : ""
  }
  const [kept, setKept] = useState(() => ({ open, keep: keptFor(open) }))
  const keep =
    kept.open === open && kept.keep !== null ? kept.keep : keptFor(open)
  if (kept.open !== open || kept.keep !== keep) setKept({ open, keep })
  const sections = dmSections(dms, goneDm, keep ?? "")
  const [isFolded, toggleFold] = useFolds()
  // Search looks through everything, folded or not.
  const searching = !!query.trim()
  const folded = (k: SectionKey) => !searching && isFolded(k)
  const [allShown, setAllShown] = useState<
    Partial<Record<SectionKey, boolean>>
  >({})
  const convRows = (k: SectionKey, items: Conv[]) =>
    sectionRows(items, folded(k), open, searching || !!allShown[k])
  const [marking, setMarking] = useState(false)
  const convSection = (
    k: SectionKey,
    label: string,
    items: Conv[],
    empty: string,
    extra?: ReactNode,
    add?: { onAdd: () => void; label: string }
  ) => {
    const { rows, hidden } = convRows(k, items)
    const { unread, markAll } = sectionHeader(k, folded(k), items)
    return (
      <Group
        label={label}
        count={items.length}
        unread={unread}
        folded={folded(k)}
        onToggle={searching ? undefined : () => toggleFold(k)}
        onAdd={add?.onAdd}
        addLabel={add?.label}
        onMarkRead={
          markAll && !marking
            ? () => {
                setMarking(true)
                void readAll(items).finally(() => setMarking(false))
              }
            : undefined
        }
      >
        {extra}
        {!folded(k) && (
          <Rows loading={conversationsLoading} label={label}>
            {items.length === 0 && !extra && (
              <p className="px-2 py-1 text-[12px] text-muted-foreground">
                {empty}
              </p>
            )}
            {rows.map((c) => (
              <ConvItem
                key={c.convId}
                c={c}
                now={now}
                current={c.convId === open}
                onClick={() => onOpen(c.convId)}
              />
            ))}
            {hidden > 0 && (
              <button
                type="button"
                className="px-2 py-0.5 text-left text-[11px] text-muted-foreground hover:text-ink"
                onClick={() => setAllShown({ ...allShown, [k]: true })}
              >
                Show {hidden} more
              </button>
            )}
          </Rows>
        )}
      </Group>
    )
  }
  // Agents talking (card #486): a message sent to many agents is one row,
  // newest first, at most agentsCap of them; search finds the rest.
  // A broadcast holding the open DM starts unfolded; its chevron still folds it.
  const [unfolded, setUnfolded] = useState<Record<string, boolean>>({})
  const isUnfolded = (b: Broadcast) =>
    unfolded[b.id] ?? b.convs.some((c) => c.convId === open)
  const agentsSection = () => {
    const all = agentRows(sections.agents)
    const { rows, hidden, capped } = agentSectionRows(
      all,
      folded("agents"),
      open,
      !!allShown.agents,
      searching
    )
    return (
      <Group
        label="Agents talking"
        count={all.length}
        folded={folded("agents")}
        onToggle={searching ? undefined : () => toggleFold("agents")}
      >
        {rows.map((r) =>
          r.kind === "dm" ? (
            <ConvItem
              key={r.c.convId}
              c={r.c}
              now={now}
              current={r.c.convId === open}
              onClick={() => onOpen(r.c.convId)}
            />
          ) : (
            <BroadcastItem
              key={r.b.id}
              b={r.b}
              now={now}
              open={open}
              unfolded={isUnfolded(r.b)}
              onToggle={() =>
                setUnfolded({ ...unfolded, [r.b.id]: !isUnfolded(r.b) })
              }
              onOpen={onOpen}
            />
          )
        )}
        {hidden > 0 && !folded("agents") && (
          <button
            type="button"
            className="px-2 py-0.5 text-left text-[11px] text-muted-foreground hover:text-ink"
            onClick={() => setAllShown({ ...allShown, agents: true })}
          >
            Show {hidden} more
          </button>
        )}
        {capped > 0 && !folded("agents") && hidden === 0 && (
          <p className="px-2 py-0.5 text-[11px] text-muted-foreground">
            {capped} older not listed. Search finds them.
          </p>
        )}
      </Group>
    )
  }
  // Folded, Threads lists no rows either, as every section (card #484).
  const liveThreads = folded("threads") ? [] : live

  const list = (
    <nav
      aria-label="Talk"
      className={cn(
        "flex min-h-0 shrink-0 flex-col gap-1 overflow-y-auto py-3 mac:overscroll-none",
        narrow ? "w-full px-1" : "w-[260px] border-r border-border px-2"
      )}
    >
      <div className="mx-1 flex items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-1.5 text-[12.5px]">
        <SearchIcon
          className="size-3.5 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search talk"
          aria-label="Search talk"
          className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-muted-foreground"
        />
        <button
          type="button"
          onClick={() => (narrow ? setComposing(true) : onOpen(""))}
          aria-label="New thread"
          title="Start a thread: ask the room about anything"
          className="shrink-0 rounded p-0.5 text-ink-2 hover:bg-secondary hover:text-ink"
        >
          <SquarePenIcon className="size-4" aria-hidden />
        </button>
      </div>
      <Group
        label="Threads"
        count={live.length}
        unread={live.filter(needsYou).length}
        folded={folded("threads")}
        onToggle={searching ? undefined : () => toggleFold("threads")}
      >
        {!folded("threads") && (
          <Rows loading={threadsLoading} label="Threads">
            {live.length === 0 && (
              <p className="px-2 py-1 text-[12px] text-muted-foreground">
                {query.trim()
                  ? "No open thread matches."
                  : "No open threads. Start one with the pen above."}
              </p>
            )}
            {liveThreads.map((t) => (
              <ThreadItem
                key={t.id}
                t={t}
                current={t.id === open}
                why={why}
                onClick={() => onOpen(t.id)}
              />
            ))}
          </Rows>
        )}
      </Group>
      {convSection(
        "channels",
        "Channels",
        channels,
        searching ? "No channel matches." : "No channels yet. Make one with +.",
        adding === "channel" ? (
          <NewChannel
            space={board}
            onDone={(id) => {
              setAdding("")
              if (id) onOpen(id)
            }}
          />
        ) : undefined,
        {
          onAdd: () => setAdding(adding === "channel" ? "" : "channel"),
          label: "New channel",
        }
      )}
      {convSection(
        "dms",
        "Direct messages",
        sections.yours,
        searching
          ? "No direct message matches."
          : "No direct messages. Start one with +.",
        adding === "dm" ? (
          <NewDm
            agents={agents}
            onDone={(id) => {
              setAdding("")
              if (id) onOpen(id)
            }}
          />
        ) : undefined,
        {
          onAdd: () => setAdding(adding === "dm" ? "" : "dm"),
          label: "New direct message",
        }
      )}
      {sections.agents.length > 0 && agentsSection()}
      {sections.gone.length > 0 &&
        convSection("gone", "Archive", sections.gone, "")}
      {resolved.length > 0 && (
        <div className="mt-auto grid grid-cols-[minmax(0,1fr)] gap-0.5 pt-3">
          <button
            type="button"
            className="px-2 text-left text-[11px] text-muted-foreground hover:text-ink"
            aria-expanded={showResolved}
            onClick={() => {
              const t = resolvedToggle(showResolvedPick, open, threads)
              setShowResolved(t.pick)
              if (t.closeOpen) onOpen("")
            }}
          >
            Resolved threads: {resolved.length}
            {showResolved ? " · hide" : " · show"}
          </button>
          {showResolved &&
            resolved.map((t) => (
              <ThreadItem
                key={t.id}
                t={t}
                current={t.id === open}
                why={why}
                onClick={() => onOpen(t.id)}
              />
            ))}
        </div>
      )}
    </nav>
  )

  // A link straight to a thread or channel (a reload, /#talk/<id>) waits
  // for the lists' first read to find it (card #598): a skeleton of the
  // pane till then, not the new-thread box. Thread or channel isn't known
  // yet, so the shape is a plain header over messages.
  const finding =
    !!open && !current && !conv && (threadsLoading || conversationsLoading)

  const conversation = current ? (
    <Conversation
      key={current.id}
      t={current}
      now={now}
      narrow={narrow}
      onBack={() => onOpen("")}
      onResolved={() => onOpen(openAfterResolve(open, current.id))}
    />
  ) : conv ? (
    <ConversationView
      key={conv.convId}
      conv={conv}
      now={now}
      narrow={narrow}
      onBack={() => onOpen("")}
    />
  ) : finding ? (
    <Loading
      loading
      label="Loading the conversation"
      className={cn("min-w-0 flex-1", narrow ? "px-1 py-2" : "p-4")}
    >
      <PaneShape />
    </Loading>
  ) : (
    <div className="grid min-w-0 flex-1 content-start gap-3 p-4">
      <h2 className="flex items-center gap-2 text-[13px] font-medium text-ink">
        {narrow && threads.length > 0 && (
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label="Back to Talk"
            onClick={() => setComposing(false)}
          >
            <ChevronLeftIcon />
          </Button>
        )}
        Start a thread
      </h2>
      <p className="text-[12.5px] text-muted-foreground">
        Ask the room about anything. An agent on the cards you name answers, or
        an idle one does. It stays in Threads until you resolve it.
      </p>
      <AskBox
        board={board}
        onThread={(id) => {
          setComposing(false)
          onOpen(id)
        }}
      />
    </div>
  )

  // A phone or a narrow window shows one at a time: the list, or the
  // conversation with Back.
  if (narrow)
    return (
      <div className="flex min-h-0 w-full min-w-0 flex-1 flex-col overflow-x-hidden">
        {current || conv || finding || composing ? conversation : list}
      </div>
    )
  return (
    <div className="mb-4 flex min-h-0 flex-1 overflow-hidden glass-panel">
      {list}
      {conversation}
    </div>
  )
}

// Rows are a section's rows inside Loading (card #598): a skeleton of
// list rows until the list's first read lands, then the rows in place.
// Loading puts its own block between, so the rows keep their grid here.
function Rows({
  loading,
  label,
  children,
}: {
  loading: boolean
  label: string
  children: ReactNode
}) {
  return (
    <Loading loading={loading} label={`Loading ${label}`}>
      {loading ? (
        <ListRowsShape rows={3} className="gap-0.5" />
      ) : (
        <div className="grid grid-cols-[minmax(0,1fr)] gap-0.5">{children}</div>
      )}
    </Loading>
  )
}

// PaneShape is the right pane's skeleton while a linked conversation is
// still being found: a title line, a line under it, then messages.
function PaneShape() {
  return (
    <div className="grid content-start gap-3">
      <h2 className="flex items-center gap-2 text-[15px] font-semibold text-ink">
        <span className="block size-4 shrink-0 rounded" />
        Talk: channels and threads
      </h2>
      <p className="text-[12.5px] text-ink-2">
        Looked at the list read; it waits on the store's lock.
      </p>
      <MessageRowsShape rows={4} />
    </div>
  )
}

// Group is one section of the list. With onToggle its name folds it; a
// folded section shows its count and how many have something new.
function Group({
  label,
  children,
  onAdd,
  addLabel,
  count,
  unread = 0,
  folded = false,
  onToggle,
  onMarkRead,
}: {
  label: string
  children: ReactNode
  onAdd?: () => void
  addLabel?: string
  count?: number
  unread?: number
  folded?: boolean
  onToggle?: () => void
  // Mark all read (the Archive, card #473): shown while something is new.
  onMarkRead?: () => void
}) {
  const Chevron = folded ? ChevronRightIcon : ChevronDownIcon
  const name = (
    <>
      {onToggle && <Chevron className="size-3 shrink-0" aria-hidden />}
      <span className="truncate">{label}</span>
      {count !== undefined && count > 0 && (
        <span className="shrink-0 font-normal whitespace-nowrap normal-case tabular-nums">
          · {count}
        </span>
      )}
      {folded && unread > 0 && (
        <span
          className="shrink-0 rounded-full bg-primary px-1.5 text-[10px] font-semibold whitespace-nowrap text-primary-foreground normal-case tabular-nums"
          title={`${unread} with something new`}
        >
          {unread} new
        </span>
      )}
    </>
  )
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-0.5">
      <h2 className="flex items-center px-2 pt-3 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {onToggle ? (
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={!folded}
            className="flex min-w-0 items-center gap-1 rounded uppercase hover:text-ink"
          >
            {name}
          </button>
        ) : (
          <span className="flex min-w-0 items-center gap-1">{name}</span>
        )}
        {onAdd && (
          <button
            type="button"
            onClick={onAdd}
            aria-label={addLabel}
            title={addLabel}
            className="ml-auto rounded p-0.5 hover:bg-secondary hover:text-ink"
          >
            <PlusIcon className="size-3.5" aria-hidden />
          </button>
        )}
        {onMarkRead && (
          <button
            type="button"
            onClick={onMarkRead}
            title="Mark every conversation here read"
            className="ml-auto shrink-0 rounded px-1 text-[10px] font-normal tracking-normal normal-case hover:bg-secondary hover:text-ink"
          >
            Mark all read
          </button>
        )}
      </h2>
      {children}
    </div>
  )
}

// ConvItem is a channel or a DM in the list, with its unread count (a red
// @ when it mentions you).
export function ConvItem({
  c,
  current,
  onClick,
  now = new Date(),
}: {
  c: Conv
  current: boolean
  onClick: () => void
  now?: Date
}) {
  const live = useContext(LiveRoster)
  const other = c.kind === "dm" ? dmWith(c) : []
  const person = dmPerson(c)
  const icon =
    c.kind === "channel" ? (
      <HashIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
    ) : person ? (
      <UserIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
    ) : c.readOnly ? (
      <EyeIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
    ) : other[0] ? (
      <Face
        who={AgentRef({
          key: other[0].key,
          name: memberName(other[0]),
          status: statusOf(other[0], live),
          live: !["gone", "unknown"].includes(statusOf(other[0], live)),
        })}
        className="size-4 shrink-0"
        noHover
      />
    ) : null
  const label =
    c.kind === "channel"
      ? plainLine(c.name)
      : person
        ? nameOr(plainLine(person.name || c.name))
        : other.map((m) => memberName(m)).join(" ↔ ") || nameOr(c.name)
  const waiting = person ? waitingText(c, label) : ""
  const last =
    c.kind === "dm" &&
    (c.readOnly ||
      (!!person && !waiting) ||
      (!!other[0] && statusOf(other[0], live) === "gone"))
  const { line, when } = last ? lastLine(c, now) : { line: "", when: "" }
  const sub =
    c.kind === "channel"
      ? c.topic
        ? plainLine(c.topic)
        : ""
      : waiting
        ? waiting
        : last
          ? line
          : other[0]
            ? memberStatus(other[0], live)
            : ""
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={current ? "page" : undefined}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] hover:bg-secondary/60",
        current && "bg-secondary",
        c.readOnly ? "text-muted-foreground" : "text-ink-2"
      )}
    >
      {icon}
      <span className="grid min-w-0 flex-1">
        <span className="flex min-w-0 items-baseline gap-2">
          <span
            className={cn(
              "min-w-0 flex-1 truncate",
              c.unread > 0 && "font-medium text-ink"
            )}
          >
            {label}
          </span>
          {when && <When text={when} />}
        </span>
        {sub && (
          <span className="truncate text-[11px] text-muted-foreground">
            {sub}
          </span>
        )}
      </span>
      {c.mentions > 0 ? (
        <span
          className="flex size-5 shrink-0 items-center justify-center rounded-full bg-destructive text-[10px] font-semibold text-destructive-foreground"
          title={`${c.mentions} mentioning you`}
        >
          @
        </span>
      ) : c.unread > 0 && !c.readOnly ? (
        <span className="shrink-0 rounded-full bg-primary px-1.5 text-[10.5px] font-semibold text-primary-foreground tabular-nums">
          {c.unread}
        </span>
      ) : null}
    </button>
  )
}

// lastLine is what a DM row says (card #486): its newest post on one line,
// "owl: Rebased onto yours" for agents talking, and when ("5m ago"), so the
// row says what it is about and when, not which section it is in.
export function lastLine(c: Conv, now: Date): { line: string; when: string } {
  const text = plainLine(c.lastText ?? "")
  const at = parseTime(c.lastAt)
  const when = at ? relativeTime(at, now) : ""
  if (!text) return { line: "", when }
  const by = c.lastBy ? c.members.find((m) => m.key === c.lastBy) : undefined
  const who =
    c.readOnly && by
      ? `${memberName(by)}: `
      : by?.kind === "person"
        ? "You: "
        : ""
  return { line: `${who}${text}`, when }
}

// When is a row's time beside its name; it never truncates.
function When({ text }: { text: string }) {
  return (
    <span className="shrink-0 text-[10.5px] whitespace-nowrap text-muted-foreground tabular-nums">
      {text}
    </span>
  )
}

// BroadcastItem is one message an agent sent to many agents (card #486):
// "owl → 16 agents", its text and when. It unfolds to the DMs it went to.
function BroadcastItem({
  b,
  now,
  open,
  unfolded,
  onToggle,
  onOpen,
}: {
  b: Broadcast
  now: Date
  open: string
  unfolded: boolean
  onToggle: () => void
  onOpen: (id: string) => void
}) {
  const from = b.convs[0].members.find((m) => m.key === b.by)
  const to = broadcastTo(b)
  const when = parseTime(b.lastAt)
  const Chevron = unfolded ? ChevronDownIcon : ChevronRightIcon
  return (
    <div className="grid grid-cols-[minmax(0,1fr)]">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={unfolded}
        className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-muted-foreground hover:bg-secondary/60"
      >
        <EyeIcon className="size-4 shrink-0" aria-hidden />
        <span className="grid min-w-0 flex-1">
          <span className="flex min-w-0 items-baseline gap-2">
            <span className="min-w-0 flex-1 truncate">
              {from ? memberName(from) : "an agent"} → {to.length} agents
            </span>
            {when && <When text={relativeTime(when, now)} />}
          </span>
          <span className="truncate text-[11px]">{plainLine(b.text)}</span>
        </span>
        <Chevron className="size-3 shrink-0" aria-hidden />
      </button>
      {unfolded &&
        b.convs.map((c, i) => (
          <button
            key={c.convId}
            type="button"
            onClick={() => onOpen(c.convId)}
            aria-current={c.convId === open ? "page" : undefined}
            className={cn(
              "truncate rounded-md py-1 pr-2 pl-8 text-left text-[12px] text-muted-foreground hover:bg-secondary/60",
              c.convId === open && "bg-secondary text-ink-2"
            )}
          >
            → {memberName(to[i])}
          </button>
        ))}
    </div>
  )
}

// NewChannel names a new channel (lowercase, no spaces or #) and opens it.
export function channelName(raw: string): string {
  return raw
    .trim()
    .replace(/^#+/, "")
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
}

// It goes in the picked shared space, else Personal (card #461).
function NewChannel({
  space,
  onDone,
}: {
  space: string
  onDone: (id: string) => void
}) {
  const [name, setName] = useState("")
  const [error, setError] = useState("")
  const clean = channelName(name)
  return (
    <form
      className="grid gap-1 px-1 pb-1"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!clean) return
        try {
          onDone(await talkApi.createChannel(clean, null, space || null))
        } catch (err) {
          setError(errorText(err))
        }
      }}
    >
      <input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && onDone("")}
        placeholder="channel-name"
        aria-label="New channel's name"
        className="rounded-md border border-border bg-card px-2 py-1 text-[12.5px] text-ink outline-none focus:border-primary"
      />
      {error && <span className="text-[11px] text-destructive">{error}</span>}
    </form>
  )
}

// NewDm picks someone to message, a person you share a space with (card
// #538) or a live agent; the DM opens (or reopens). Opening one with a
// person writes nothing until the first post.
function NewDm({
  agents,
  onDone,
}: {
  agents: { key: string; name: string; status: string }[]
  onDone: (id: string) => void
}) {
  const [error, setError] = useState("")
  const [people, setPeople] = useState<Person[]>([])
  useEffect(() => {
    let on = true
    talkApi
      .dmPeople()
      .then((p) => on && setPeople(p))
      .catch(() => {})
    return () => {
      on = false
    }
  }, [])
  const live = agents.filter((a) => a.status !== "gone")
  const open = async (m: { kind: "person" | "agent"; key: string }) => {
    try {
      onDone(await talkApi.dm(m))
    } catch (err) {
      setError(errorText(err))
    }
  }
  return (
    <div
      className="grid grid-cols-[minmax(0,1fr)] gap-0.5 rounded-md border border-border bg-card/60 p-1"
      role="listbox"
      aria-label="Message someone"
    >
      {people.length > 0 && (
        <>
          <p className="px-2 pt-0.5 text-[11px] font-medium text-muted-foreground">
            People
          </p>
          {people.map((p) => (
            <button
              key={p.key}
              type="button"
              role="option"
              aria-selected={false}
              className="flex items-center gap-2 rounded px-2 py-1 text-left text-[12.5px] text-ink-2 hover:bg-secondary"
              onClick={() => void open({ kind: "person", key: p.key })}
            >
              <UserIcon
                className="size-4 shrink-0 text-muted-foreground"
                aria-hidden
              />
              <span className="min-w-0 flex-1 truncate">
                {nameOr(plainLine(p.name))}
              </span>
            </button>
          ))}
          <p className="px-2 pt-1 text-[11px] font-medium text-muted-foreground">
            Agents
          </p>
        </>
      )}
      {live.length === 0 && (
        <p className="px-2 py-1 text-[12px] text-muted-foreground">
          No agents are running.
        </p>
      )}
      {live.map((a) => (
        <button
          key={a.key}
          type="button"
          role="option"
          aria-selected={false}
          className="flex items-center gap-2 rounded px-2 py-1 text-left text-[12.5px] text-ink-2 hover:bg-secondary"
          onClick={() => void open({ kind: "agent", key: a.key })}
        >
          <Face
            who={AgentRef({
              key: a.key,
              name: a.name,
              status: a.status,
              live: true,
            })}
            className="size-4"
            noHover
          />
          <span className="min-w-0 flex-1 truncate">{plainLine(a.name)}</span>
          <span className="shrink-0 text-[11px] text-muted-foreground">
            {agentStatus(a).text}
          </span>
        </button>
      ))}
      {error && (
        <span className="px-2 text-[11px] text-destructive">{error}</span>
      )}
    </div>
  )
}

function ThreadItem({
  t,
  current,
  why,
  onClick,
}: {
  t: ThreadSummary
  current: boolean
  why: string
  onClick: () => void
}) {
  const waiting = t.state === "waiting" || t.state === "handed"
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={current ? "page" : undefined}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-ink-2 hover:bg-secondary/60",
        current && "bg-secondary"
      )}
    >
      <MessageSquareTextIcon
        className="size-4 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <span className="grid min-w-0 flex-1">
        <span className={cn("truncate", needsYou(t) && "font-medium text-ink")}>
          {plainLine(t.title)}
        </span>
        <span
          className={cn(
            "truncate text-[11px]",
            waiting ? "text-warning" : "text-muted-foreground"
          )}
        >
          {stateLine(t, why)}
        </span>
      </span>
      {needsYou(t) && (
        <span
          className="size-2 shrink-0 rounded-full bg-primary"
          title="Answered: waiting on you"
        />
      )}
    </button>
  )
}

function Conversation({
  t,
  now,
  narrow,
  onBack,
  onResolved,
}: {
  t: ThreadSummary
  now: Date
  narrow: boolean
  onBack: () => void
  onResolved: () => void
}) {
  const badge = stateBadges[t.state]
  const at = parseTime(t.updatedAt)
  return (
    <section
      aria-label={plainLine(t.title)}
      className={cn(
        // Fills the pane: only the thread's messages scroll (card #564).
        // The column scrolls only when its fixed lines alone outgrow it.
        "flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-y-auto mac:overscroll-none",
        narrow ? "px-1 py-2" : "p-4"
      )}
    >
      <header className="flex min-w-0 shrink-0 items-center gap-2">
        {narrow && (
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label="Back to Talk"
            onClick={onBack}
          >
            <ChevronLeftIcon />
          </Button>
        )}
        <MessageSquareTextIcon
          className="size-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        {t.short && (
          <span className="shrink-0 text-[12px] text-muted-foreground tabular-nums">
            {cardRef(t)}
          </span>
        )}
        <h2 className="min-w-0 flex-1 truncate text-[15px] font-semibold text-ink">
          {plainLine(t.title)}
        </h2>
        {t.private && (
          <Badge
            variant="slate"
            className="h-4 shrink-0 px-1.5 text-[10.5px]"
            title="Only you and your own agents see this thread; it never reaches other members' devices"
          >
            Private
          </Badge>
        )}
        {badge && (
          <Badge
            variant={badge.variant}
            className="h-4 shrink-0 px-1.5 text-[10.5px]"
          >
            {badge.label}
          </Badge>
        )}
        <time
          className="shrink-0 text-[11px] text-muted-foreground tabular-nums"
          dateTime={at?.toISOString()}
        >
          {relativeTime(at, now)}
        </time>
      </header>
      {t.cards.length > 0 && <ThreadCards cards={t.cards} />}
      <ThreadView t={t} now={now} onResolved={onResolved} />
    </section>
  )
}
