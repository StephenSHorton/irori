// NotificationsPage (card #377): every notification Irori showed the
// person, newest first, with Unread and All. Opening an item (a click on
// it, or any of its buttons) marks it read. An item's buttons follow the
// thing it is about as it stands now: the card's blocker or done check
// (Home's own answer row), the message, the agent's ask. Once that is
// resolved, by the person or anyone, the buttons go and the item says
// what happened, who did it and when. A click re-checks too: Irori
// refuses an answer to something already answered, and the row says so.

import { Ago } from "@/components/common/ago"
import { cardRef } from "@/components/cards/bits"
import { PermissionPrompt } from "@/components/cards/permission-prompt"
import { useEffect, useState, type ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Loading } from "@/components/ui/loading"
import {
  BlockerAnswer,
  OwnWords,
  Said,
  useAct,
} from "@/components/cards/needs-you"
import { useCard } from "@/lib/cards"
import { AnswersShape } from "@/components/common/loading-shapes"
import { askKey, askPatch, instant } from "@/lib/optimistic"
import { plainLine } from "@/lib/irori"
import { previewFlag } from "@/lib/preview"
import {
  Kind,
  answerAsk,
  askOutcome,
  asksOnCard,
  blockerOpen,
  blockerOutcome,
  deliveryOutcome,
  kindLabel,
  kindTone,
  readNotifications,
  shown,
  unreadCount,
  waitedSummary,
  type Notification,
  type NotificationFilter,
} from "@/lib/notifications"
import { talk, type Delivery } from "@/lib/talk"
import { go } from "@/lib/nav"
import { cn } from "@/lib/utils"

// PageSize is how many items show before "Show more".
const PageSize = 30

export function NotificationsPage({
  list,
  unreadTotal,
  loading,
  error,
  onOpenCard,
  onOpenMessage,
}: {
  list: Notification[]
  // Every unread item, the ones past the listed newest too (0.11.1), so
  // Mark all read can clear what the badge counts.
  unreadTotal?: number
  loading: boolean
  error: string | null
  onOpenCard: (id: string) => void
  onOpenMessage: (deliveryId: string) => void
}) {
  // ?notifications=all starts on All in the browser preview.
  const [filter, setFilter] = useState<NotificationFilter>(() =>
    previewFlag("notifications") === "all" ? "all" : "unread"
  )
  const [limit, setLimit] = useState(PageSize)
  // An item read while Unread is showing stays until the filter changes,
  // so it doesn't jump out from under the pointer.
  const [kept, setKept] = useState<Set<string>>(() => new Set())
  const unread = unreadCount(list)
  const items =
    filter === "unread"
      ? list.filter((n) => !n.read || kept.has(n.key))
      : shown(list, "all")
  const open = (n: Notification) => {
    if (n.read) return
    setKept((k) => new Set(k).add(n.key))
    void readNotifications([n.key]).catch(() => {})
  }
  return (
    <section aria-labelledby="notifications" className="grid min-w-0 gap-3">
      <div className="flex min-w-0 flex-wrap items-center gap-2 px-1">
        <h2 id="notifications" className="text-base font-semibold">
          Notifications
          {unread > 0 && (
            <span className="font-medium text-muted-foreground">
              {" "}
              · {unread} unread
            </span>
          )}
        </h2>
        <div
          className="ml-auto flex gap-1"
          role="tablist"
          aria-label="Which notifications"
        >
          {(
            [
              ["unread", "Unread"],
              ["all", "All"],
            ] as const
          ).map(([k, label]) => (
            <Button
              key={k}
              type="button"
              size="sm"
              role="tab"
              aria-selected={filter === k}
              variant={filter === k ? "secondary" : "ghost"}
              onClick={() => {
                setFilter(k)
                setKept(new Set())
                setLimit(PageSize)
              }}
            >
              {label}
            </Button>
          ))}
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={unread === 0 && !unreadTotal}
            onClick={() => void readNotifications([]).catch(() => {})}
          >
            Mark all read
          </Button>
        </div>
      </div>
      {error && (
        <p className="px-1 text-sm text-destructive" role="alert">
          Could not load the notifications: {error}
        </p>
      )}
      {/* Until the first read lands, the list's shape (card #598); loading
          never comes back on a reload. */}
      <Loading
        loading={loading}
        label="Loading the notifications"
        className="min-w-0"
      >
        {loading ? (
          <NotificationsShape />
        ) : items.length === 0 ? (
          <p className="px-1 text-[12.5px] text-muted-foreground">
            {filter === "unread" && list.length > 0
              ? "You've read them all. All shows every one."
              : "Nothing yet. Every notification Irori shows you is kept here."}
          </p>
        ) : (
          <ul className="grid min-w-0 gap-2.5" aria-label="Notifications">
            {items.slice(0, limit).map((n) => (
              <li key={n.key} className="min-w-0">
                <Item
                  n={n}

                  onRead={() => open(n)}
                  onOpenCard={onOpenCard}
                  onOpenMessage={onOpenMessage}
                />
              </li>
            ))}
          </ul>
        )}
      </Loading>
      {items.length > limit && (
        <Button
          variant="link"
          size="sm"
          className="justify-self-start px-1"
          onClick={() => setLimit((l) => l + PageSize)}
        >
          Show {Math.min(PageSize, items.length - limit)} more
        </Button>
      )}
    </section>
  )
}

// NotificationsShape is the list's skeleton: a few items in Item's frame,
// plain markup with no hooks.
function NotificationsShape() {
  const rows = [
    [
      "#12",
      "Fix the boat clipping through the dock",
      "Widened the dock collider, so boats stop at the edge.",
    ],
    [
      "#3",
      "Two-PC lobby join for the multiplayer build",
      "Should the lobby list come from Steam, or from our own server?",
    ],
    ["#18", "Asks and Talk threads get their own agent", ""],
  ]
  return (
    <ul className="grid min-w-0 gap-2.5">
      {rows.map(([ref, title, body]) => (
        <li key={ref} className="grid min-w-0 gap-2 px-4 py-3 glass-panel">
          <div className="flex min-w-0 items-center gap-x-2 text-[12.5px] text-muted-foreground">
            <span className="block h-5 w-20 rounded-md" />
            <span className="font-mono">{ref}</span>
            <span className="ml-auto">2m ago</span>
          </div>
          <h3 className="text-[15px] leading-snug font-semibold">{title}</h3>
          {body && <p className="text-sm text-muted-foreground">{body}</p>}
          <span className="block h-8 w-24 rounded-md" />
        </li>
      ))}
    </ul>
  )
}

const at = (s: string) => {
  const t = Date.parse(s)
  return Number.isNaN(t) ? null : new Date(t)
}

// Item is one notification: its frame, then what it can still do or how
// it ended.
function Item({
  n,
  onRead,
  onOpenCard,
  onOpenMessage,
}: {
  n: Notification
  onRead: () => void
  onOpenCard: (id: string) => void
  onOpenMessage: (deliveryId: string) => void
}) {
  const cardId = n.card?.id ?? n.taskId
  const openCard = () => {
    onRead()
    onOpenCard(cardId)
  }
  return (
    <article
      className={cn(
        "grid min-w-0 gap-2 px-4 py-3 glass-panel",
        n.read && "opacity-80"
      )}
      // Opening it, any click inside, marks it read.
      onClickCapture={onRead}
      onFocusCapture={onRead}
      aria-label={`${n.read ? "" : "Unread: "}${plainLine(n.title)}`}
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
        {!n.read && (
          <span
            className="size-2 shrink-0 rounded-full bg-primary"
            aria-hidden
            title="Unread"
          />
        )}
        <Badge variant={kindTone[n.kind] ?? "secondary"}>
          {kindLabel[n.kind] ?? "Notification"}
        </Badge>
        {n.card && (
          <button
            type="button"
            className="min-w-0 truncate font-mono tabular-nums underline-offset-2 hover:underline"
            title={plainLine(n.card.title)}
            onClick={openCard}
          >
            {n.card.short ? cardRef(n.card) : plainLine(n.card.title)}
          </button>
        )}
        <span
          className="ml-auto shrink-0 tabular-nums"
          title={at(n.at)?.toLocaleString()}
        >
          <Ago at={at(n.at)} />
        </span>
      </div>
      <h3
        className={cn(
          "min-w-0 text-[15px] leading-snug break-words",
          n.read ? "font-medium" : "font-semibold"
        )}
      >
        {plainLine(n.title)}
      </h3>
      {n.body && n.kind !== Kind.DoneCheck && !asksOnCard(n) && (
        <p className="line-clamp-3 text-sm break-words whitespace-pre-line text-muted-foreground">
          {n.body}
        </p>
      )}
      <Live
        n={n}

        onOpenCard={openCard}
        onOpenMessage={() => onOpenMessage(n.deliveryId)}
      />
    </article>
  )
}

// Outcome is how the thing ended, and when.
function Outcome({ line, when }: { line: string; when?: string }) {
  const t = when ? at(when) : null
  return (
    <p className="text-[13px] break-words text-muted-foreground" role="status">
      {line}
      {t && (
        <span className="tabular-nums">
          {" "}
          · <Ago at={t} />
        </span>
      )}
    </p>
  )
}

function OpenCard({ onOpen }: { onOpen: () => void }) {
  return (
    <span className="flex flex-wrap gap-1.5">
      <Button size="sm" variant="outline" onClick={onOpen}>
        Open card
      </Button>
    </span>
  )
}

function Live({
  n,
  onOpenCard,
  onOpenMessage,
}: {
  n: Notification
  onOpenCard: () => void
  onOpenMessage: () => void
}): ReactNode {
  if (asksOnCard(n)) return <CardLive n={n} onOpen={onOpenCard} />
  if (n.kind === Kind.Ask && n.ask) return <AskLive n={n} />
  // Several messages that waited: they're DMs, so Talk's list (card #482).
  if (waitedSummary(n))
    return (
      <span className="flex flex-wrap gap-1.5">
        <Button
          size="sm"
          variant="outline"
          onClick={() => go({ kind: "talk", id: "" })}
        >
          Open messages
        </Button>
      </span>
    )
  if ((n.kind === Kind.Message || n.kind === Kind.Held) && n.deliveryId)
    return <DeliveryLive n={n} onOpen={onOpenMessage} />
  if (n.taskId)
    return n.card ? (
      <OpenCard onOpen={onOpenCard} />
    ) : (
      <Outcome line="The card is gone" />
    )
  return null
}

// CardLive is a card's blocker, done check or question as it stands now.
function CardLive({ n, onOpen }: { n: Notification; onOpen: () => void }) {
  const { detail, error } = useCard(n.card?.id ?? "")
  if (!n.card) return <Outcome line="The card is gone" />
  if (error) return <OpenCard onOpen={onOpen} />
  // The item's answers are on their way (card #644): their shape now,
  // not a gap that jumps when they come.
  if (!detail)
    return (
      <Loading loading label="Loading the answers">
        <AnswersShape />
      </Loading>
    )
  const b = n.blockerId
    ? detail.blockers.find((x) => x.id === n.blockerId)
    : undefined
  if (b) {
    // "Answer…" opens the card to answer there (card #644); a pick is
    // answered here.
    // A permission prompt is its buttons (card #682).
    if (blockerOpen(b) && b.permission)
      return (
        <>
          <p className="text-sm break-words">{plainLine(b.question)}</p>
          <PermissionPrompt
            cardId={detail.id}
            b={b}
            onOpenCard={() => onOpen()}
            compact
          />
        </>
      )
    if (blockerOpen(b))
      return (
        <BlockerAnswer cardId={detail.id} b={b} onOpen={onOpen} answerOpens />
      )
    return (
      <>
        {!b.doneCheck && (
          <p className="text-sm break-words">{plainLine(b.question)}</p>
        )}
        <Outcome line={blockerOutcome(b)} when={b.resolved?.at} />
        <OpenCard onOpen={onOpen} />
      </>
    )
  }
  // An older card's question without an item: open while the card still
  // asks the person.
  const asks = detail.state === "input_required" || detail.state === "blocked"
  return (
    <>
      {n.body && <p className="text-sm break-words">{plainLine(n.body)}</p>}
      {!asks && (
        <Outcome
          line="Answered; the card has moved on"
          when={detail.updatedAt}
        />
      )}
      <OpenCard onOpen={onOpen} />
    </>
  )
}

// AskLive is an agent's ask: its buttons while it waits, else its answer.
// A pick shows as the answer at once and saves in the background (card
// #644); a refusal (answered on another device, out of time) brings the
// buttons back and says why in a toast.
function AskLive({ n }: { n: Notification }) {
  const a = n.ask!
  const ask = n.key.startsWith("ask:") ? n.key.slice(4) : ""
  if (!a.open) return <Outcome line={askOutcome(a)} />
  const pick = (l: string) =>
    instant({
      key: askKey(ask),
      patch: askPatch(n.key, l),
      send: () => answerAsk(ask, l),
      failed: "Your answer wasn't sent",
    })
  return (
    <span className="flex flex-wrap gap-1.5" role="group" aria-label="Answers">
      {a.actions.map((l, i) => (
        <Button
          key={l}
          size="sm"
          variant={i === 0 ? "default" : "outline"}
          disabled={!ask}
          onClick={() => void pick(l)}
        >
          {l}
        </Button>
      ))}
    </span>
  )
}

// DeliveryLive is a message to the person, or one Irori held, as it
// stands now.
function DeliveryLive({ n, onOpen }: { n: Notification; onOpen: () => void }) {
  const held = n.kind === Kind.Held
  const [d, setD] = useState<Delivery | null>(null)
  const [gone, setGone] = useState(false)
  useEffect(() => {
    let live = true
    talk
      .get(n.deliveryId)
      .then((x) => live && setD(x))
      .catch(() => live && setGone(true))
    return () => {
      live = false
    }
    // n is re-read on every inbox change, so its object changes then.
  }, [n])
  const act = useAct()
  const [replying, setReplying] = useState(false)
  if (gone) return <Outcome line="The message is gone" />
  if (!d) return null
  const ended = deliveryOutcome(d, held)
  if (ended) return <Outcome line={ended} when={d.decidedAt || d.updatedAt} />
  if (act.done) return <Said error={null} done={act.done} />
  if (held)
    return (
      <>
        <span className="flex flex-wrap gap-1.5">
          <Button
            size="sm"
            disabled={act.busy}
            onClick={() => void act.run(() => talk.release(d.id), "Released.")}
          >
            Release
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={act.busy}
            onClick={() => void act.run(() => talk.dismiss(d.id), "Dismissed.")}
          >
            Dismiss
          </Button>
        </span>
        <Said error={act.error} done={act.done} />
      </>
    )
  const from = plainLine(d.message.from.name) || "the agent"
  return (
    <>
      {replying ? (
        <OwnWords
          label={`Your reply to ${from}`}
          placeholder={`Reply to ${from}`}
          send="Reply"
          busy={act.busy}
          onSend={(t) => void act.run(() => talk.reply(d.id, t), "Replied.")}
          onCancel={() => setReplying(false)}
        />
      ) : (
        <span className="flex flex-wrap gap-1.5">
          <Button
            size="sm"
            disabled={act.busy}
            onClick={() => setReplying(true)}
          >
            Reply…
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={act.busy}
            onClick={() =>
              void act.run(() => talk.markRead(d.id), "Marked read.")
            }
          >
            Mark read
          </Button>
          <Button size="sm" variant="ghost" onClick={onOpen}>
            Open
          </Button>
        </span>
      )}
      <Said error={act.error} done={act.done} />
    </>
  )
}
