import { useContext, useEffect, useRef, useState, type ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { AttachmentList } from "@/components/attachments"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import {
  cards,
  EditState,
  type AgentRef,
  type CardMessage,
  type CardThreadItem,
} from "@/lib/cards"
import { parseTime, plainBlock, plainLine, relativeTime } from "@/lib/irori"
import { errorText, talk } from "@/lib/talk"
import {
  FocusComment,
  isRevision,
  opensInFull,
  threadEntryId,
  LiveRoster,
  PonderEvery,
  ponderWord,
  StoppedGrace,
  threadLabel,
  ThreadKind,
  visible,
  workingLine,
  StartAgentOn,
  startAgentItem,
  BlockersHere,
  blockerItemId,
} from "@/lib/thread"
import { useReducedMotion } from "@/lib/motion"
import { cn } from "@/lib/utils"
import { Spinner } from "./bits"
import { Face } from "./faces"
import { RichLine, RichText } from "@/components/common/rich-text"

// Thread is a card's thread, oldest first: comments, messages about the
// card ("A -> B") and one muted line per thing that happened, each with
// who wrote it and, once the person has a second device, where. The
// newest page shows first; "Show earlier" opens the rest. A message the
// card cut, doesn't carry the text of yet, or that has files opens in full
// in place (cardId names the card). composer renders under the list,
// newest last: the card's comment box, or a blocker's answer form.
export function Thread({
  cardId,
  items,
  now,
  earlier = 0,
  historyTrimmed,
  empty = "Nothing yet.",
  compact,
  inBlocker,
  composer,
  scroll,
  on,
}: {
  cardId?: string
  items: CardThreadItem[]
  now: Date
  // earlier counts entries older than the ones sent (BoardService.Card
  // sends the newest MaxThreadView).
  earlier?: number
  // historyTrimmed: the card's oldest activity lines are no longer kept.
  historyTrimmed?: boolean
  empty?: ReactNode
  compact?: boolean
  // inBlocker: the items sit under their blocker, so they don't name it.
  inBlocker?: boolean
  composer?: ReactNode
  // scroll caps the entries' height (a class, e.g. a max-h): they scroll
  // by themselves, newest in view, and the composer stays under them
  // (a thread in the Threads panel, card #170).
  scroll?: string
  // on: the agents on this thread (a card's holders, an ask's takers) and
  // whether it is over, for the live "working…" line (card #299).
  // working, when given, is who the card's rule says works on it now
  // (card #317): the line names them, so it agrees with the page.
  on?: {
    agents: readonly AgentRef[]
    over?: boolean
    working?: readonly AgentRef[]
    jobs?: readonly AgentRef[]
  }
}) {
  const [showAll, setShowAll] = useState(false)
  const live = useContext(LiveRoster)
  // "stopped without replying" waits out a grace after the roster changes;
  // tick once it has passed so the line shows without another event.
  const [, setTick] = useState(0)
  const watching = !!on
  useEffect(() => {
    if (!watching) return
    const id = window.setTimeout(
      () => setTick((n) => n + 1),
      StoppedGrace + 100
    )
    return () => window.clearTimeout(id)
  }, [watching, live])
  const status = on
    ? workingLine(
        on.agents,
        items,
        live,
        new Date(),
        on.over,
        on.working,
        on.jobs
      )
    : null
  // A comment a notification opened the card at: shown even when it is
  // older than the first page, scrolled into view once (card #177).
  const focus = useContext(FocusComment)
  const holds = !!focus && items.some((it) => it.id === focus)
  const startOn = useContext(StartAgentOn)
  const startId =
    !inBlocker && cardId && startOn === cardId ? startAgentItem(items) : ""
  const paged = visible(items, showAll)
  const shown =
    holds && !paged.some((it) => it.id === focus) ? visible(items, true) : paged
  const scrolled = useRef("")
  useEffect(() => {
    if (!holds || scrolled.current === focus) return
    scrolled.current = focus
    document
      .getElementById(threadEntryId(focus))
      ?.scrollIntoView({ block: "center" })
  }, [holds, focus])
  const hidden = items.length - shown.length
  const gap = compact ? "gap-1" : "gap-2"
  // The newest entry is last: keep it in view as entries arrive.
  const list = useRef<HTMLDivElement>(null)
  const newest = shown[shown.length - 1]?.id
  useEffect(() => {
    const el = list.current
    if (el) el.scrollTop = el.scrollHeight
  }, [newest])
  const entries = (
    <>
      {hidden > 0 ? (
        <Button
          size="xs"
          variant="ghost"
          className="justify-self-start text-muted-foreground"
          onClick={() => setShowAll(true)}
        >
          Show {hidden} earlier
        </Button>
      ) : (
        (earlier > 0 || historyTrimmed) &&
        items.length > 0 && (
          <p className="text-xs text-muted-foreground">
            {earlier > 0
              ? `${earlier} earlier ${earlier === 1 ? "entry isn't" : "entries aren't"} shown here.`
              : "Earlier history not kept."}
          </p>
        )
      )}
      {shown.length
        ? shown.map((it) => (
            <div
              key={it.id}
              id={threadEntryId(it.id)}
              className={cn(
                "grid scroll-mt-16 grid-cols-[minmax(0,1fr)]",
                holds &&
                  it.id === focus &&
                  "-mx-1.5 rounded-sm bg-primary/10 px-1.5 py-0.5 ring-1 ring-primary/40"
              )}
            >
              <ThreadRow
                cardId={cardId}
                item={it}
                now={now}
                inBlocker={inBlocker}
                startable={it.id === startId}
              />
            </div>
          ))
        : !status && <p className="text-muted-foreground">{empty}</p>}
      {status &&
        (status.tone === "working" ? (
          // Where the next comment will appear (card #317): who is on it,
          // with a word that changes while no running tool says more.
          <p
            role="status"
            aria-label={status.text}
            className="flex items-center gap-2 py-0.5 text-[12.5px] text-ink-2"
          >
            <Spinner className="size-3.5" />
            <span className="min-w-0 truncate">
              <span className="font-medium">{status.who}</span> {status.verb}{" "}
              {status.doing || <PonderWord />}…
            </span>
          </p>
        ) : (
          <p role="status" className="text-[11.5px] text-warning">
            {status.text}
          </p>
        ))}
    </>
  )
  if (scroll) {
    return (
      <div
        className={cn(
          "grid min-w-0 grid-cols-[minmax(0,1fr)] text-[12.5px]",
          gap
        )}
      >
        <div
          ref={list}
          className={cn(
            "grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] content-start overflow-y-auto",
            gap,
            scroll
          )}
        >
          {entries}
        </div>
        {/* In a short window the column scrolls too: the composer sticks
            to its bottom edge, so it is never scrolled out of sight. */}
        {composer && (
          <div className="sticky bottom-0 z-10 bg-background/95 pb-1 backdrop-blur-sm">
            {composer}
          </div>
        )}
      </div>
    )
  }
  return (
    <div
      className={cn(
        "grid min-w-0 grid-cols-[minmax(0,1fr)] text-[12.5px]",
        gap
      )}
    >
      {entries}
      {composer}
    </div>
  )
}

// PonderWord is the working line's changing word (card #317): a new one
// every PonderEvery while it shows; Reduce Motion keeps the first, still.
// Its own component, so only the word re-renders on each change.
function PonderWord() {
  const still = useReducedMotion()
  const [tick, setTick] = useState(0)
  useEffect(() => {
    if (still) return
    const id = window.setInterval(() => setTick((n) => n + 1), PonderEvery)
    return () => window.clearInterval(id)
  }, [still])
  return <span aria-hidden>{ponderWord(still ? 0 : tick)}</span>
}

function ThreadRow({
  cardId,
  item,
  now,
  inBlocker,
  startable,
}: {
  cardId?: string
  item: CardThreadItem
  now: Date
  inBlocker?: boolean
  // "Start an agent with this" (card #452): the person's latest comment
  // on a card with nobody on it.
  startable?: boolean
}) {
  const [started, setStarted] = useState(false)
  const here = useContext(BlockersHere)
  const [full, setFull] = useState<CardMessage | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // released: the person released a dismissed message here; its text
  // joins the card once the inbox lets it through.
  const [released, setReleased] = useState(false)
  // edit: the person's comment no agent read yet (card #105). Modify holds
  // it back from every agent until Save (the new text goes out) or Cancel
  // (it goes as it was). shown is what the person just did, until the
  // card's next load changes the comment.
  const seen = `${item.edit ?? ""}\n${item.text}`
  const [shown, setShown] = useState<{ edit: string; seen: string } | null>(
    null
  )
  const [draft, setDraft] = useState<string | null>(null)
  const edit = shown && shown.seen === seen ? shown.edit : (item.edit ?? "")
  const at = parseTime(item.at)
  const time = (
    <time
      className="text-muted-foreground tabular-nums"
      dateTime={at?.toISOString()}
      title={at?.toLocaleString()}
    >
      {relativeTime(at, now)}
    </time>
  )
  // A revision of its blocker: what the item asks now, under the reply it
  // answers; the current options to pick sit below the thread.
  if (isRevision(item))
    return (
      <div className="grid grid-cols-[56px_minmax(0,1fr)] gap-2">
        {time}
        <span className="flex items-start gap-1.5 text-ink-2">
          <ActorFace who={item.who} />
          <span className="min-w-0">
            <Badge
              variant="warning"
              className="mr-1.5 h-4 px-1.5 align-middle text-[10.5px]"
            >
              Revised
            </Badge>
            <RichLine text={item.text} />
            {item.device && <DeviceTag device={item.device} />}
          </span>
        </span>
      </div>
    )
  // An activity line (card #117): whoever made the change shows their
  // face, and the line stays dim so it reads as a log, not a comment. A
  // line about a blocker the page shows links to it (card #490).
  if (item.kind === ThreadKind.Event)
    return (
      <div className="grid grid-cols-[56px_minmax(0,1fr)] gap-2">
        {time}
        <span className="flex items-start gap-1.5 text-muted-foreground">
          <ActorFace who={item.who} />
          <span className="min-w-0">
            {item.blockerId && !inBlocker && here.has(item.blockerId) ? (
              <button
                type="button"
                className="cursor-pointer rounded-sm text-left underline decoration-muted-foreground/40 underline-offset-2 outline-none hover:text-ink focus-visible:ring-1 focus-visible:ring-ring"
                title="Show this blocker"
                onClick={() =>
                  document
                    .getElementById(blockerItemId(item.blockerId!))
                    ?.scrollIntoView({ block: "center", behavior: "smooth" })
                }
              >
                <RichLine text={item.text} interactive={false} />
              </button>
            ) : (
              <RichLine text={item.text} />
            )}
            {item.device && <DeviceTag device={item.device} />}
          </span>
        </span>
      </div>
    )
  const l = threadLabel(item, inBlocker)
  const body = plainBlock(item.text)
  const open = async () => {
    if (!cardId || busy) return
    setBusy(true)
    setError(null)
    try {
      setFull(await cards.message(cardId, item.id))
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const editAction = async (
    run: () => Promise<void>,
    after: string,
    keepDraft = false
  ) => {
    if (!cardId || busy) return
    setBusy(true)
    setError(null)
    try {
      await run()
      setShown({ edit: after, seen })
      if (!keepDraft) setDraft(null)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const modify = () => {
    if (!cardId) return
    setDraft(item.text)
    void editAction(
      () => cards.modifyComment(cardId, item.id),
      EditState.Editing,
      true
    )
  }
  const cancelEdit = () =>
    cardId &&
    void editAction(() => cards.cancelComment(cardId, item.id), EditState.Can)
  const saveEdit = () => {
    if (!cardId) return
    const text = draft ?? item.text
    if (!text.trim()) {
      setError("Write a comment first.")
      return
    }
    void editAction(
      () => cards.saveComment(cardId, item.id, text),
      EditState.Can
    )
  }
  const release = async () => {
    if (!item.deliveryId || busy) return
    setBusy(true)
    setError(null)
    try {
      await talk.release(item.deliveryId)
      setReleased(true)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="group/row grid grid-cols-[56px_minmax(0,1fr)] gap-2">
      {time}
      <div className="grid min-w-0 gap-1">
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          <Face who={item.who} inline />
          <span className="font-medium text-ink">{plainLine(l.author)}</span>
          {item.to && l.arrow && (
            <>
              <span className="text-muted-foreground" aria-hidden>
                {"->"}
              </span>
              <span className="sr-only">to</span>
              <Face who={item.to} inline />
              <span className="font-medium text-ink">{plainLine(l.arrow)}</span>
            </>
          )}
          {l.device && <DeviceTag device={l.device} />}
          {l.badges.map((b) => (
            <Badge
              key={b.label}
              variant={b.tone}
              className="h-4 px-1.5 text-[10.5px]"
            >
              {b.label}
            </Badge>
          ))}
        </div>
        {full ? (
          <>
            <RichText
              text={full.text}
              mentions={item.mentions}
              className="[overflow-wrap:anywhere] text-ink-2"
            />
            {full.files.length > 0 && (
              <ul className="grid gap-0.5 text-[11px] text-muted-foreground">
                {full.files.map((f, i) => (
                  <li key={i} className="font-mono break-all">
                    {plainLine(f)}
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : item.held && released ? (
          <p className="text-muted-foreground">
            Released. Its text joins the card once it is on its way.
          </p>
        ) : item.held && item.dismissed ? (
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <p className="text-muted-foreground">
              Held, then dismissed: it never reached{" "}
              {l.arrow ? plainLine(l.arrow) : "its recipient"}, and its text
              isn't on the card.
            </p>
            {item.deliveryId && (
              <Button
                size="xs"
                variant="outline"
                disabled={busy}
                onClick={() => void release()}
              >
                Release
              </Button>
            )}
          </div>
        ) : item.held ? (
          <p className="text-muted-foreground">
            Not let through the wake policy yet, so its text isn't on the card.
          </p>
        ) : edit === EditState.Editing ? (
          <form
            className="grid gap-1.5"
            onSubmit={(e) => {
              e.preventDefault()
              saveEdit()
            }}
          >
            <Textarea
              value={draft ?? item.text}
              onChange={(e) => setDraft(e.target.value)}
              aria-label="Edit your comment"
              className="min-h-16 text-[12.5px]"
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault()
                  saveEdit()
                }
                if (e.key === "Escape") {
                  e.preventDefault()
                  cancelEdit()
                }
              }}
            />
            <div className="flex flex-wrap items-center gap-2">
              <Button size="xs" type="submit" disabled={busy}>
                Save
              </Button>
              <Button
                size="xs"
                variant="ghost"
                type="button"
                disabled={busy}
                onClick={cancelEdit}
              >
                Cancel
              </Button>
              <span className="text-xs text-muted-foreground">
                Held back: no agent gets it until you save or cancel.
              </span>
            </div>
          </form>
        ) : (
          body && (
            <RichText
              text={body}
              mentions={item.mentions}
              className="[overflow-wrap:anywhere] text-ink-2"
            />
          )
        )}
        {!!item.missing?.length && (
          <div className="grid gap-0.5 text-ink-2">
            <span className="text-xs text-muted-foreground">Still needed:</span>
            <ul className="ml-4 list-disc">
              {item.missing.map((m, i) => (
                <li key={i}>
                  <RichLine text={m} />
                </li>
              ))}
            </ul>
          </div>
        )}
        <AttachmentList attachments={item.attachments ?? []} />
        {l.waiting.length > 0 && (
          <ul className="grid gap-0.5 text-[11px] text-muted-foreground">
            {l.waiting.map((w) => (
              <li key={w}>{plainLine(w)}</li>
            ))}
          </ul>
        )}
        {cardId && edit === EditState.Can && (
          <Button
            size="xs"
            variant="ghost"
            className="justify-self-start text-muted-foreground"
            disabled={busy}
            title="No agent has read it yet: take it back to edit it"
            onClick={modify}
          >
            Modify
          </Button>
        )}
        {startable && cardId && !started && (
          <Button
            size="xs"
            variant="outline"
            className="justify-self-start"
            disabled={busy}
            title="Nobody is on this card: start an agent, with this comment in its brief"
            onClick={() => {
              setBusy(true)
              setError(null)
              cards
                .startAgent(cardId, full?.text ?? item.text)
                .then(() => setStarted(true))
                .catch((err: unknown) => setError(errorText(err)))
                .finally(() => setBusy(false))
            }}
          >
            Start an agent with this
          </Button>
        )}
        {started && (
          <span className="text-[11px] text-muted-foreground">
            Starting an agent with this comment in its brief.
          </span>
        )}
        {!full && cardId && opensInFull(item) && (
          <Button
            size="xs"
            variant="ghost"
            className="justify-self-start text-muted-foreground"
            disabled={busy}
            onClick={() => void open()}
          >
            {busy ? "Opening…" : "Show the whole message"}
          </Button>
        )}
        {error && <p className="text-xs text-destructive">{error}</p>}
      </div>
    </div>
  )
}

// DeviceTag names the device an entry was written on.
function DeviceTag({ device }: { device: string }) {
  return (
    <Badge
      variant="slate"
      className="ml-1.5 h-4 px-1.5 align-middle font-mono text-[10.5px]"
      title={`Written on ${plainLine(device)}`}
    >
      {plainLine(device)}
    </Badge>
  )
}

// ActorFace is who made an activity line's change (card #117), dimmed to
// match the line; Irori's own lines have none.
function ActorFace({ who }: { who?: AgentRef | null }) {
  if (!who?.key || who.status === "irori") return null
  return <Face who={who} noDot className="-my-0.5 shrink-0 opacity-75" />
}
