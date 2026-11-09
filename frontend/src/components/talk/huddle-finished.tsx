// A huddle that is over, as a list (card #1054; docs/design/call-agents.md
// §9): the chair's summary on top, what it judges answered or superseded
// with Settle these, then the items grouped by kind with open questions and
// proposed tasks first. Divide with my assistant opens the chat on this
// huddle. An item that became a card says so: a link where this person can
// open the card, else "carded by Addam (Personal)".

import { useContext, useState } from "react"
import { CheckIcon, SparklesIcon } from "lucide-react"

import { AgentCards } from "@/components/cards/agent-hover"
import { RichText } from "@/components/common/rich-text"
import { Button } from "@/components/ui/button"
import type { TalkCanvasItem } from "@/gen/bindings"
import { useCardRef } from "@/lib/cards"
import {
  applySettle,
  finishedSections,
  isCarded,
  settleCandidates,
  summaryOf,
} from "@/lib/huddle-divide"
import {
  canSettle,
  clamped,
  kindWord,
  settleWord,
  statusWord,
} from "@/lib/huddle-canvas"
import { setCanvasItem } from "@/lib/huddles"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// Carded is where an item went. A card this person can open is a link; one
// they can't (made on the other person's Personal board) is plain words.
export function Carded({
  item,
  personal,
}: {
  item: Pick<TalkCanvasItem, "card" | "cardByName" | "cardMine">
  // The huddle was a person DM: a card its promoter made without a shared
  // space is on their Personal board.
  personal: boolean
}) {
  const target = useCardRef(item.card)
  const ctx = useContext(AgentCards)
  if (!item.card) return <span>Became a card</span>
  if (target)
    return (
      <button
        type="button"
        className="rounded text-left text-primary hover:underline"
        title={target.title}
        onClick={() => ctx?.onOpen(target.id)}
      >
        Became card {item.card}
      </button>
    )
  const by = item.cardMine ? "you" : item.cardByName
  return (
    <span>
      {by
        ? `Carded by ${by}${personal && !item.cardMine ? " (Personal)" : ""}`
        : "Became a card"}
    </span>
  )
}

function Row({
  item,
  personal,
  busy,
  onSet,
}: {
  item: TalkCanvasItem
  personal: boolean
  busy: boolean
  onSet: (status: "settled" | "dropped") => void
}) {
  const [more, setMore] = useState(false)
  const long = clamped(item.text)
  const word = isCarded(item) ? "" : statusWord(item, 0)
  return (
    <li
      data-item={item.itemId}
      data-status={item.status || "open"}
      className={cn(
        "rounded-lg border border-border px-3 py-2",
        (item.status === "dropped" || isCarded(item)) && "opacity-70"
      )}
    >
      <p
        className={cn(
          "text-[13px] leading-relaxed whitespace-pre-wrap text-ink",
          long && !more && "line-clamp-3"
        )}
      >
        {item.text}
      </p>
      {long && (
        <button
          type="button"
          className="mt-0.5 text-[11.5px] text-muted-foreground hover:text-ink"
          onClick={() => setMore(!more)}
        >
          {more ? "Less" : "More"}
        </button>
      )}
      <p className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-muted-foreground">
        <span>{item.assistant ? item.label : item.byName}</span>
        {item.askerName && item.askerName !== item.byName && (
          <span>asked by {item.askerName}</span>
        )}
        {isCarded(item) ? (
          <span className="font-medium text-ink-2" data-carded="">
            <Carded item={item} personal={personal} />
          </span>
        ) : (
          word && <span className="font-medium text-ink-2">{word}</span>
        )}
        {canSettle(item) && (
          <span className="ml-auto flex gap-1">
            <Button
              size="xs"
              variant="ghost"
              disabled={busy}
              onClick={() => onSet("settled")}
            >
              {settleWord(item.kind)}
            </Button>
            <Button
              size="xs"
              variant="ghost"
              disabled={busy}
              onClick={() => onSet("dropped")}
            >
              Drop
            </Button>
          </span>
        )}
      </p>
    </li>
  )
}

// FinishedCanvas is the canvas of a huddle that is over. Everything it
// writes is the person's own row (settled, dropped); nothing is made by
// itself. onChanged re-reads the transcript after a write.
export function FinishedCanvas({
  callId,
  items,
  personal,
  onDivide,
  onChanged,
}: {
  callId: string
  items: readonly TalkCanvasItem[]
  personal: boolean
  onDivide: () => void
  onChanged: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const summary = summaryOf(items)
  const settle = settleCandidates(items)
  const sections = finishedSections(items)
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true)
    setError("")
    try {
      await f()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
      onChanged()
    }
  }
  return (
    <div
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-4 py-3"
      data-finished=""
    >
      {summary && (
        <section
          aria-label="Summary"
          className="rounded-lg border border-border bg-secondary/40 px-3 py-2.5"
        >
          <p className="flex items-center gap-1.5 text-[11.5px] font-medium text-muted-foreground">
            <SparklesIcon className="size-3.5" aria-hidden />
            Summary
            {summary.assistant && summary.label ? ` · ${summary.label}` : ""}
          </p>
          <div className="mt-1 text-[13px] leading-relaxed whitespace-pre-wrap text-ink">
            <RichText text={summary.text} />
          </div>
        </section>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={onDivide} data-divide="">
          <SparklesIcon />
          Divide with my assistant
        </Button>
        <span className="text-[12px] text-muted-foreground">
          Talk it through, then make the cards you agree on.
        </span>
      </div>
      {settle.length > 0 && (
        <section
          aria-label="Settle these"
          className="rounded-lg border border-border px-3 py-2.5"
          data-settle=""
        >
          <p className="text-[12.5px] font-medium text-ink">
            The summary says {settle.length === 1 ? "this was" : "these were"}{" "}
            answered or superseded
          </p>
          <ul className="mt-1.5 grid gap-1 text-[12.5px] text-ink-2">
            {settle.map((i) => (
              <li key={i.itemId} className="flex gap-1.5">
                <span className="shrink-0 text-muted-foreground">
                  {kindWord(i.kind)}
                </span>
                <span className="min-w-0 truncate">{i.text}</span>
              </li>
            ))}
          </ul>
          <Button
            size="sm"
            variant="secondary"
            className="mt-2"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const r = await applySettle(callId, {
                  settle,
                  drop: [],
                })
                if (r.failed.length)
                  throw new Error(
                    `Settled ${r.set}; ${r.failed.length} could not be: ${r.failed[0].why}`
                  )
              })
            }
          >
            <CheckIcon />
            Settle these
          </Button>
        </section>
      )}
      {error && (
        <p role="alert" className="text-[12px] text-destructive">
          {error}
        </p>
      )}
      {sections.length === 0 && !summary && (
        <p className="text-[12.5px] text-muted-foreground">
          Nothing was written on this huddle's canvas.
        </p>
      )}
      {sections.map((s) => (
        <section key={s.group} aria-label={s.title} data-group={s.group}>
          <h2 className="mb-1.5 text-[12px] font-medium text-muted-foreground">
            {s.title} · {s.items.length}
          </h2>
          <ul className="grid gap-1.5">
            {s.items.map((i) => (
              <Row
                key={i.itemId}
                item={i}
                personal={personal}
                busy={busy}
                onSet={(status) =>
                  void run(() => setCanvasItem(callId, i.itemId, { status }))
                }
              />
            ))}
          </ul>
        </section>
      ))}
    </div>
  )
}
