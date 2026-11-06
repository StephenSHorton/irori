// ConversationView is a channel or a DM on the Talk page (card #390,
// design §2, §8; mockups talk-2b, talk-3): the posts, oldest first with a
// line between days, the members with their live status, and the box to
// post. An agent↔agent DM is read-only. Opening one marks it read.

import {
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react"
import {
  ArrowUpRightIcon,
  ChevronLeftIcon,
  EyeIcon,
  HashIcon,
  MessageSquareTextIcon,
  SquarePlusIcon,
  UsersIcon,
} from "lucide-react"

import { MentionTextarea } from "@/components/common/mention-textarea"
import { AgentsNowToggle } from "@/components/common/agents-now-toggle"
import { RichText } from "@/components/common/rich-text"
import { Face } from "@/components/cards/faces"
import { AgentCards } from "@/components/cards/agent-hover"
import { NewCardDialog } from "@/components/cards/new-card-dialog"
import { Button } from "@/components/ui/button"
import { PostRowsShape } from "@/components/common/loading-shapes"
import { Loading } from "@/components/ui/loading"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  agentsNowFor,
  dmWith,
  fromPost,
  talkApi,
  usePosts,
  type Conversation,
  type Member,
  type Post,
} from "@/lib/conversations"
import { AgentRef } from "@/lib/models"
import { composing, MentionBoard, type Mention } from "@/lib/mentions"
import { useAgentsNow } from "@/lib/agents-now"
import { parseTime, plainLine } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { agentStatus } from "@/lib/agent-status"
import { isOthersKey, memberName, nameOr } from "@/lib/names"
import { LiveRoster, type LiveAgent } from "@/lib/thread"
import { cn } from "@/lib/utils"

// statusOf is a member's status now: "person", the live roster's for an
// agent, else its board card's for another member's agent, else "gone";
// agentStatus says it (card #489). Another member's agent this device
// knows nothing about is "unknown", never gone (card #478): it isn't on
// this roster, that's all.
export function statusOf(
  m: Member,
  live: ReadonlyMap<string, { status: string }>
): string {
  if (m.kind === "person") return "person"
  const mine = live.get(m.key)?.status
  if (mine) return mine
  if (m.status) return m.status
  return m.owner || isOthersKey(m.key) ? "unknown" : "gone"
}

// statusWord says a status in words, as agentStatus does (card #489).
export function statusWord(status: string): string {
  return agentStatus({ status }).text
}

// memberStatus is how a member stands in words, with its device's state
// and how long its job has run, as its hover card says it (card #489);
// another member's agent reads "Spencer's agent · idle" (card #478).
export function memberStatus(
  m: Member,
  live: ReadonlyMap<string, LiveAgent>
): string {
  const a = m.kind === "person" ? undefined : live.get(m.key)
  const word = agentStatus(
    { status: statusOf(m, live), deviceState: a?.deviceState },
    a?.statusSince
  ).text
  return withOwner(m, word)
}

// whoseWord is the line under another member's agent: "Spencer's agent ·
// idle", or just "Spencer's agent" when its state isn't known here.
export function whoseWord(m: Member, status: string): string {
  return withOwner(m, statusWord(status))
}

function withOwner(m: Member, word: string): string {
  if (!m.owner) return word
  const whose = `${plainLine(m.owner)}'s agent`
  return word ? `${whose} · ${word}` : whose
}

// dayLabel is the line between days: Today, Yesterday, or the date.
export function dayLabel(at: Date, now: Date): string {
  const d = (x: Date) =>
    new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const days = Math.round((d(now) - d(at)) / 86_400_000)
  if (days === 0) return "Today"
  if (days === 1) return "Yesterday"
  return at.toLocaleDateString(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  })
}

// mentionKinds turns the box's mentions into who they name: person keys
// start "person:", the rest are agents.
export function mentionKinds(ms: readonly Mention[]): { kind: string }[] {
  return ms.map((m) => ({
    kind: m.key.startsWith("person:") ? "person" : "agent",
  }))
}

export function ConversationView({
  conv,
  now,
  narrow,
  onBack,
}: {
  conv: Conversation
  now: Date
  narrow: boolean
  onBack: () => void
}) {
  const live = useContext(LiveRoster)
  const { posts, loading, error } = usePosts(conv.convId)
  // The posts are a skeleton until their first read lands (card #598), not
  // "Nothing here yet"; a reload never brings it back.
  const firstLoad = loading && !error
  const [showMembers, setShowMembers] = useState(
    !narrow && conv.kind === "channel"
  )
  // Make a card (design §4): the post it's made from, while the form is up.
  const [making, setMaking] = useState<Post | null>(null)
  const where = conv.kind === "channel" ? `#${conv.name}` : `a DM`
  const prefill = useMemo(
    () => (making ? fromPost(making.text, where) : undefined),
    [making, where]
  )
  const names = useMemo(() => {
    const m = new Map<string, Member>()
    for (const x of conv.members) m.set(x.key, x)
    return m
  }, [conv.members])

  // Opening it reads it: the cursor moves to the newest post.
  const last = posts[posts.length - 1]?.postId ?? ""
  useEffect(() => {
    if (last && conv.unread > 0)
      void talkApi.read(conv.convId, last).catch(() => {})
  }, [conv.convId, conv.unread, last])

  const bottom = useRef<HTMLDivElement>(null)
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" })
  }, [last])

  const other = conv.kind === "dm" ? dmWith(conv) : []
  const title =
    conv.kind === "channel" ? (
      <>
        <HashIcon
          className="size-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <span className="truncate">{plainLine(conv.name)}</span>
      </>
    ) : conv.readOnly ? (
      <>
        <EyeIcon
          className="size-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <span className="truncate">
          {other.map((m) => memberName(m)).join(" ↔ ")}
        </span>
      </>
    ) : (
      <>
        {other[0] && <Face who={refOf(other[0], live)} />}
        <span className="truncate">
          {other[0] ? memberName(other[0]) : nameOr(conv.name)}
        </span>
        {other[0] && (
          <span className="shrink-0 text-[12px] font-normal text-muted-foreground">
            {memberStatus(other[0], live)}
          </span>
        )}
      </>
    )

  return (
    <section
      aria-label={plainLine(conv.name)}
      className="flex min-h-0 min-w-0 flex-1"
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <header className="flex min-w-0 items-center gap-2 border-b border-border px-4 py-2.5">
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
          {narrow && conv.kind === "channel" ? (
            // A phone keeps the members behind the channel's name (talk-5).
            <h2 className="min-w-0 text-[15px] font-semibold text-ink">
              <button
                type="button"
                className="flex min-w-0 items-center gap-2"
                aria-expanded={showMembers}
                title="Who's in this channel"
                onClick={() => setShowMembers((s) => !s)}
              >
                {title}
              </button>
            </h2>
          ) : (
            <h2 className="flex min-w-0 items-center gap-2 text-[15px] font-semibold text-ink">
              {title}
            </h2>
          )}
          {conv.topic && (
            <span className="min-w-0 truncate text-[12px] text-muted-foreground">
              {plainLine(conv.topic)}
            </span>
          )}
          {conv.kind === "channel" && !narrow && (
            <Button
              size="xs"
              variant={showMembers ? "secondary" : "ghost"}
              className="ml-auto shrink-0"
              aria-pressed={showMembers}
              onClick={() => setShowMembers((s) => !s)}
              title="Who's in this channel"
            >
              <UsersIcon />
              {conv.members.length}
            </Button>
          )}
        </header>
        {narrow && showMembers && conv.kind === "channel" ? (
          <Members members={conv.members} live={live} wide />
        ) : (
          <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)] content-start gap-0.5 overflow-y-auto px-3 py-3 mac:overscroll-none">
            {error && (
              <p className="px-2 text-xs text-destructive" role="alert">
                Could not load the posts: {error}
              </p>
            )}
            <Loading loading={firstLoad} label="Loading the messages">
              {firstLoad ? (
                <PostRowsShape />
              ) : (
                <div className="grid grid-cols-[minmax(0,1fr)] gap-0.5">
                  {posts.length === 0 && !error && (
                    <p className="px-2 py-1 text-[12.5px] text-muted-foreground">
                      {conv.kind === "channel"
                        ? "Nothing here yet. Agents wake only when you @-mention them; the rest read it at their next turn."
                        : conv.readOnly
                          ? "Nothing between them yet."
                          : "Nothing yet. Every message here wakes the agent."}
                    </p>
                  )}
                  {posts.map((p, i) => {
                    const at = parseTime(p.at)
                    const prev = i > 0 ? parseTime(posts[i - 1].at) : null
                    const newDay =
                      at && (!prev || dayLabel(prev, now) !== dayLabel(at, now))
                    return (
                      <div
                        key={p.postId}
                        className="grid grid-cols-[minmax(0,1fr)]"
                      >
                        {newDay && at && <DayLine label={dayLabel(at, now)} />}
                        <PostRow
                          post={p}
                          who={names.get(p.by)}
                          names={names}
                          live={live}
                          onMakeCard={
                            conv.readOnly ? undefined : () => setMaking(p)
                          }
                          channel={conv.kind === "channel" ? conv : undefined}
                        />
                      </div>
                    )
                  })}
                </div>
              )}
            </Loading>
            <div ref={bottom} />
          </div>
        )}
        {conv.readOnly ? (
          <p className="border-t border-border px-4 py-2.5 text-[12px] text-muted-foreground">
            Agents talking to each other: you can read it, not post. Nothing
            between agents is hidden.
          </p>
        ) : (
          <Composer conv={conv} other={other[0]} />
        )}
      </div>
      {showMembers && conv.kind === "channel" && !narrow && (
        <Members members={conv.members} live={live} />
      )}
      <NewCardDialog
        open={!!making}
        from={prefill}
        onClose={() => setMaking(null)}
        onMade={(id) => {
          if (making) void talkApi.linkCard(making.postId, id).catch(() => {})
        }}
      />
    </section>
  )
}

// refOf is a member as a face needs it, with its live status.
function refOf(m: Member, live: ReadonlyMap<string, LiveAgent>) {
  const status = statusOf(m, live)
  return AgentRef({
    key: m.key,
    name: m.kind === "person" && m.name === "you" ? "You" : memberName(m),
    status,
    live: status !== "gone" && status !== "unknown",
    // Its face's title says "device offline" as the hover does (card #489).
    offline: m.kind !== "person" && live.get(m.key)?.deviceState === "offline",
  })
}

function DayLine({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-2 px-2 py-2 text-[11.5px] text-muted-foreground">
      <span className="h-px flex-1 bg-border" />
      {label}
      <span className="h-px flex-1 bg-border" />
    </div>
  )
}

export function PostRow({
  post,
  who,
  names,
  live,
  onMakeCard,
  channel,
}: {
  post: Post
  who?: Member
  names: ReadonlyMap<string, Member>
  live: ReadonlyMap<string, LiveAgent>
  // Make a card from it; left out where the person can't post.
  onMakeCard?: () => void
  // The channel it's in: a channel message can start a thread (design b2).
  channel?: Conversation
}) {
  const [asking, setAsking] = useState(false)
  // Its @names, so the text shows them as mentions.
  const mentions = post.mentions.map((key) => {
    // "user" is the person on any of their devices (an agent's mention
    // "person", card #395).
    const m: Member =
      names.get(key) ??
      (key === "user" || key.startsWith("user:")
        ? { kind: "person", key, name: "you" }
        : { kind: key.startsWith("person:") ? "person" : "agent", key })
    const ref = refOf(m, live)
    return {
      name: m.kind === "person" && m.name === "you" ? "you" : memberName(m),
      who: ref,
      gone: ref.status === "gone",
    }
  })
  const m: Member = who ?? {
    kind: post.by.startsWith("person:") ? "person" : "agent",
    key: post.by,
  }
  const at = parseTime(post.at)
  const name = m.kind === "person" && m.name === "you" ? "You" : memberName(m)
  // A thread started from a message here leaves this one line (design b2):
  // the thread itself lives in Threads.
  if (post.threadFrom)
    return (
      <div className="ml-9 flex min-w-0 items-center gap-1.5 rounded-md border border-border/70 px-2 py-1 text-[12px] text-muted-foreground">
        <MessageSquareTextIcon className="size-3.5 shrink-0" aria-hidden />
        <span className="shrink-0">{name} started a thread:</span>
        <RichText
          text={post.text}
          className="min-w-0 grid-cols-[minmax(0,1fr)] truncate text-ink-2"
        />
        <span className="shrink-0">· in Threads</span>
      </div>
    )
  // Long-press on a phone, or right-click (card #395, mockup talk-5): the
  // same two actions as the hover buttons, which a touch screen never shows.
  const canThread = !!channel && !!onMakeCard
  const canCard = !!onMakeCard && !post.card
  const row = (
    <div className="group flex gap-2.5 rounded-lg px-2 py-1.5 hover:bg-secondary/40">
      <Face who={refOf(m, live)} className="mt-0.5 shrink-0" />
      <div className="grid min-w-0 flex-1 gap-0.5">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="min-w-0 truncate text-[13px] font-medium text-ink">
            {name}
          </span>
          {m.kind === "agent" && (
            <span className="shrink-0 text-[10.5px] text-muted-foreground uppercase">
              agent
            </span>
          )}
          <time
            className="shrink-0 text-[11px] whitespace-nowrap text-muted-foreground tabular-nums"
            dateTime={at?.toISOString()}
          >
            {at?.toLocaleTimeString(undefined, {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </time>
          {/* Hover buttons only where there's a mouse and room: on a touch
              screen they never show, and below 768 px their hidden width
              cut the sender's name to "sampl…" (card #479). Long-press or
              right-click gives both there. */}
          {channel && onMakeCard && (
            <button
              type="button"
              onClick={() => setAsking((a) => !a)}
              className="ml-auto hidden items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground opacity-0 group-hover:opacity-100 hover:bg-secondary hover:text-ink focus-visible:opacity-100 pointer-fine:md:flex"
              title="Ask about this in a thread: it goes in Threads, and this channel gets a one-line pointer"
            >
              <MessageSquareTextIcon className="size-3.5" aria-hidden />
              Start a thread
            </button>
          )}
          {onMakeCard && !post.card && (
            <button
              type="button"
              onClick={onMakeCard}
              className={cn(
                "hidden items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground opacity-0 group-hover:opacity-100 hover:bg-secondary hover:text-ink focus-visible:opacity-100 pointer-fine:md:flex",
                !channel && "ml-auto"
              )}
              title="Work goes on a card: make one from this message"
            >
              <SquarePlusIcon className="size-3.5" aria-hidden />
              Make a card
            </button>
          )}
        </div>
        <RichText
          text={post.text}
          mentions={mentions}
          className="text-[13px] leading-relaxed [overflow-wrap:anywhere] text-ink-2"
        />
        {post.card && <CardChip id={post.card} />}
        {asking && channel && (
          <ThreadFrom
            post={post}
            author={name}
            channel={channel}
            onDone={() => setAsking(false)}
          />
        )}
      </div>
    </div>
  )
  if (!canThread && !canCard) return row
  return (
    <ContextMenu>
      <ContextMenuTrigger className="[-webkit-touch-callout:none] md:select-text">
        {row}
      </ContextMenuTrigger>
      <ContextMenuContent aria-label="Message actions">
        {canThread && (
          <ContextMenuItem onClick={() => setAsking(true)}>
            <MessageSquareTextIcon aria-hidden />
            Reply in thread
          </ContextMenuItem>
        )}
        {canCard && (
          <ContextMenuItem onClick={onMakeCard}>
            <SquarePlusIcon aria-hidden />
            Make a card
          </ContextMenuItem>
        )}
      </ContextMenuContent>
    </ContextMenu>
  )
}

// threadText is the new thread's ask: the person's question, then the
// message it's about and where it was said.
export function threadText(
  question: string,
  author: string,
  text: string,
  channel: string
): string {
  const quoted = text
    .trim()
    .split(/\r?\n/)
    .map((l) => `> ${l}`)
    .join("\n")
  return `${question.trim()}\n\n${quoted}\n(${author}, in #${channel})`
}

// ThreadFrom asks about a channel message in a new thread (design b2): the
// thread goes in Threads, and the channel gets a pointer to it.
function ThreadFrom({
  post,
  author,
  channel,
  onDone,
}: {
  post: Post
  author: string
  channel: Conversation
  onDone: () => void
}) {
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <form
      className="grid gap-1 pt-1"
      onSubmit={async (e) => {
        e.preventDefault()
        if (!text.trim() || busy) return
        setBusy(true)
        setError(null)
        try {
          await talkApi.threadFrom(
            channel.convId,
            post.postId,
            threadText(text, author, post.text, channel.name)
          )
          onDone()
        } catch (err) {
          setError(errorText(err))
        } finally {
          setBusy(false)
        }
      }}
    >
      <input
        autoFocus
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && onDone()}
        placeholder="Ask about this in a thread…"
        aria-label="Your question for the thread"
        readOnly={busy}
        className="rounded-md border border-border bg-card px-2 py-1 text-[12.5px] text-ink outline-none focus:border-primary"
      />
      {error && <span className="text-[11px] text-destructive">{error}</span>}
    </form>
  )
}

// CardChip is the card made from a message (design §4): one click to it.
function CardChip({ id }: { id: string }) {
  const ctx = useContext(AgentCards)
  const c = ctx?.cards.find((x) => x.id === id)
  return (
    <button
      type="button"
      disabled={!ctx}
      onClick={() => ctx?.onOpen(id)}
      className="flex max-w-full items-center gap-1.5 justify-self-start text-[11.5px] text-muted-foreground hover:text-ink"
    >
      <ArrowUpRightIcon className="size-3.5 shrink-0" aria-hidden />
      Made a card:
      <span className="inline-flex min-w-0 items-center gap-1 rounded-md bg-secondary px-1.5 py-0.5 text-ink-2">
        <span className="font-mono text-primary">
          #{plainLine(c?.short ?? "")}
        </span>
        <span className="truncate">{plainLine(c?.title ?? "a card")}</span>
      </span>
    </button>
  )
}

// Members is a channel's people and agents with how each stands now.
export function Members({
  members,
  live,
  wide = false,
}: {
  members: Member[]
  live: ReadonlyMap<string, LiveAgent>
  // A phone: the whole page under the header, in place of the posts.
  wide?: boolean
}) {
  const sorted = [...members].sort((a, b) =>
    a.kind === b.kind
      ? memberName(a).localeCompare(memberName(b))
      : a.kind === "person"
        ? -1
        : 1
  )
  return (
    <aside
      aria-label="Members"
      className={
        wide
          ? "min-h-0 flex-1 overflow-y-auto px-4 py-3"
          : "hidden w-[220px] shrink-0 overflow-y-auto border-l border-border px-3 py-3 lg:block"
      }
    >
      <h3 className="pb-2 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        Members · {members.length}
      </h3>
      <ul className="grid gap-1.5">
        {sorted.map((m) => {
          const status = statusOf(m, live)
          const gone = status === "gone"
          return (
            <li
              key={m.key}
              className="flex min-w-0 items-center gap-2 text-[12.5px]"
            >
              <Face who={refOf(m, live)} className="shrink-0" />
              <span
                className={cn(
                  "min-w-0 truncate text-ink-2",
                  gone && "text-muted-foreground line-through"
                )}
              >
                {m.kind === "person" && m.name === "you"
                  ? "You"
                  : memberName(m)}
              </span>
              <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">
                {memberStatus(m, live)}
              </span>
            </li>
          )
        })}
      </ul>
      <p className="pt-3 text-[11px] text-muted-foreground">
        @-mention an agent to bring it in.
      </p>
    </aside>
  )
}

function Composer({ conv, other }: { conv: Conversation; other?: Member }) {
  const [text, setText] = useState("")
  const [mentions, setMentions] = useState<Mention[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // A channel post carries the "Agents answer now" toggle (card #391); a DM
  // always wakes the other side, so it keeps the default.
  const agentsNow = useAgentsNow(mentions)
  const channel = conv.kind === "channel"
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const body = text.trim()
    if (!body || busy) return
    setBusy(true)
    setError(null)
    try {
      const now = channel ? agentsNow.on : agentsNowFor(mentionKinds(mentions))
      await talkApi.post(
        conv.convId,
        body,
        now,
        null,
        mentions.map((m) => m.key)
      )
      setText("")
      setMentions([])
      agentsNow.reset()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const placeholder =
    conv.kind === "channel"
      ? `Message #${plainLine(conv.name)}`
      : `Message ${other ? memberName(other) : nameOr(conv.name)}`
  return (
    <form
      className="grid gap-1 border-t border-border px-3 py-2.5"
      onSubmit={(e) => void submit(e)}
    >
      <div className="flex items-start gap-2 p-1.5 glass-panel">
        {/* A space's channel offers its other people's agents too (#461). */}
        <MentionBoard.Provider value={conv.spaceId ?? ""}>
          <MentionTextarea
            autoSize
            rows={1}
            value={text}
            mentions={mentions}
            onChange={(v, ms) => {
              setText(v)
              setMentions(ms)
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
                e.preventDefault()
                e.currentTarget.form?.requestSubmit()
              }
            }}
            placeholder={placeholder}
            aria-label={placeholder}
            className="max-h-40 min-h-9 flex-1 border-transparent bg-transparent shadow-none focus-visible:border-transparent"
            readOnly={busy}
            aria-busy={busy}
          />
        </MentionBoard.Provider>
        <div className="flex h-9 shrink-0 items-center gap-2">
          {channel && (
            <AgentsNowToggle
              on={agentsNow.on}
              onChange={agentsNow.set}
              disabled={busy}
            />
          )}
          <Button type="submit" size="sm" disabled={busy || !text.trim()}>
            {busy ? "Sending…" : "Send"}
          </Button>
        </div>
      </div>
      {error && (
        <p className="px-1 text-xs text-destructive" role="alert">
          {error}
        </p>
      )}
    </form>
  )
}
