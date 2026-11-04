import { useState } from "react"
import { CheckIcon } from "lucide-react"

import { RichText } from "@/components/common/rich-text"
import { Badge } from "@/components/ui/badge"
import { displayName, type CardBlocker } from "@/lib/cards"
import type { CardThreadItem } from "@/lib/models"
import { plainLine } from "@/lib/irori"
import { sentBack, type SentBack } from "@/lib/rounds"
import { cn } from "@/lib/utils"

// Card #476: a done check the person sent back keeps their reason and the
// agent's answer in view. While the agent works on it, the check says
// whether it heard and answered them; the next check opens with that
// exchange ("You said", the agent's answer) above its new proof, so a
// re-raised check never reads like a first ask.

// Signs is the heard / answered pair under the person's Not done.
export function RoundSigns({
  round,
  agent,
  className,
}: {
  round: SentBack
  agent: string
  className?: string
}) {
  const sign = (ok: boolean, yes: string, no: string) => (
    <span
      className={cn(
        "inline-flex items-center gap-1",
        ok ? "text-success" : "text-muted-foreground"
      )}
    >
      {ok ? (
        <CheckIcon className="size-3" aria-hidden />
      ) : (
        <span aria-hidden>○</span>
      )}
      {ok ? yes : no}
    </span>
  )
  return (
    <div
      className={cn(
        "flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px]",
        className
      )}
      role="status"
      aria-label="What the agent did with your reason"
    >
      {sign(round.heard, `${agent} read it`, "Not read yet")}
      {sign(round.answered, `${agent} answered`, "No answer yet")}
    </div>
  )
}

const agentOf = (r: SentBack, fallback: string) =>
  r.replies.length ? displayName(r.replies[r.replies.length - 1].who) : fallback

// SentBackSigns sits on a done check the person sent back (open, waiting on
// the agent): their reason is in the thread below; this says what the
// agent did with it.
export function SentBackSigns({ blocker: b }: { blocker: CardBlocker }) {
  if (!b.doneCheck || b.resolved) return null
  const r = sentBack(b.thread ?? [])
  if (!r) return null
  return (
    <RoundSigns round={r} agent={agentOf(r, displayName(b.sentTo ?? b.raisedBy))} />
  )
}

// EarlierRound opens a re-raised done check: the round the person sent
// back. compact (Needs you) keeps it to a few lines.
export function EarlierRound({
  blocker: b,
  compact = false,
}: {
  blocker: CardBlocker
  compact?: boolean
}) {
  const [proof, setProof] = useState(false)
  const p = b.previous
  if (!p) return null
  const r = sentBack(p.thread ?? [], p.answer)
  if (!r) return null
  const raiser = displayName(b.raisedBy)
  const who = agentOf(r, raiser)
  return (
    <section
      className="grid gap-1.5 rounded-lg border border-border bg-muted/40 px-3 py-2 text-[12.5px]"
      aria-label={`Round ${p.round}: what you said and the agent's answer`}
    >
      <div className="flex flex-wrap items-center gap-1.5 text-[11.5px] text-muted-foreground">
        <Badge variant="slate" className="h-4 px-1.5 text-[10.5px] font-normal">
          Round {p.round}
        </Badge>
        <span>You sent the last check back</span>
      </div>
      <Said who="You said" text={r.reason} compact={compact} />
      {r.replies.length > 0 ? (
        (compact ? r.replies.slice(-1) : r.replies).map((it: CardThreadItem) => (
          <Said
            key={it.id}
            who={`${displayName(it.who)} answered`}
            text={it.text}
            compact={compact}
          />
        ))
      ) : (
        <p className="text-warning">
          {who} didn't answer your reason before asking again.
        </p>
      )}
      {!compact && <RoundSigns round={r} agent={who} />}
      {p.sameProof ? (
        <p className="font-medium text-warning">
          The proof below is the same as the one you sent back.
        </p>
      ) : (
        !compact &&
        p.proof && (
          <div>
            <button
              type="button"
              className="text-[11.5px] text-muted-foreground underline-offset-2 hover:underline"
              aria-expanded={proof}
              onClick={() => setProof((v) => !v)}
            >
              {proof ? "Hide the proof you sent back" : "The proof you sent back"}
            </button>
            {proof && (
              <RichText
                text={p.proof}
                className="mt-1 text-[12px] text-muted-foreground"
              />
            )}
          </div>
        )
      )}
    </section>
  )
}

function Said({
  who,
  text,
  compact,
}: {
  who: string
  text: string
  compact: boolean
}) {
  return (
    <div className="grid gap-0.5">
      <span className="text-[11.5px] font-medium text-muted-foreground">
        {who}
      </span>
      {compact ? (
        <p className="line-clamp-2 break-words text-ink-2">{plainLine(text)}</p>
      ) : (
        <RichText text={text} className="text-ink-2" />
      )}
    </div>
  )
}
