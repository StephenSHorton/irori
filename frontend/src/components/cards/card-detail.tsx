import { useState, type FormEvent, type ReactNode } from "react"
import { ArrowLeftIcon, ExternalLinkIcon } from "lucide-react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import {
  agentBusy,
  BlockerState,
  busy,
  cards,
  displayName,
  MaxAnswer,
  MaxComment,
  LinkKind,
  NoteItem,
  over,
  percent,
  TaskState,
  type AgentRef,
  type AnswerResult,
  type CardBlocker,
  type CardDetail,
  type CardOption,
  type CardPR,
  type CardSummary,
  type Step,
  type Told,
} from "@/lib/cards"
import {
  parseTime,
  plainLine,
  relativeTime,
  shortPath,
  shortRepo,
} from "@/lib/irori"
import { errorText, talk, type GoneSession } from "@/lib/talk"
import { answerable, blockerChip, blockerTone, heardLine } from "@/lib/thread"
import { cn } from "@/lib/utils"
import {
  cardTone,
  HolderLine,
  ProgressBar,
  ScopeBadge,
  ShortRef,
  Spinner,
  StatePill,
} from "./bits"
import { Face, LiveDot } from "./faces"
import { Thread } from "./thread"
import { AttachmentList } from "@/components/attachments"
import type { FileRef } from "@/lib/attachments"
import { LinkChip, RichLine, RichText } from "@/components/common/rich-text"
import { useDraft } from "@/lib/drafts"

// Answered is a note answer the person just sent: the note item leaves the
// card once answered, so the panel keeps it to show who heard.
type Answered = { question: string; answer: string; told: Told; to: AgentRef }

// CardPage is one card on its own page, in the window's full width: a
// header (short ref, title, state, scope, Back), a main column with the
// objective, its blockers and the thread, and a side column with progress,
// who's on it, PRs, links and files. Narrow (below 900px) it is one column
// with the side column after the blockers, and a short progress line under
// the header. Key it by card id, so a note answer shown here stays with its
// card; what the person types is kept outside it (lib/drafts), so leaving
// the page doesn't lose it.
export function CardPage({
  summary,
  detail,
  error,
  gone,
  now,
  onBack,
}: {
  // summary is the card's grid tile, if the grid has it, for the header
  // while the full card loads.
  summary?: CardSummary
  detail: CardDetail | null
  error: string | null
  gone: GoneSession[]
  now: Date
  onBack: () => void
}) {
  const [answered, setAnswered] = useState<Answered | null>(null)
  const back = (
    <Button
      variant="ghost"
      size="sm"
      className="-ml-2 justify-self-start text-muted-foreground"
      onClick={onBack}
      title="Back to the cards (Esc)"
    >
      <ArrowLeftIcon aria-hidden />
      Cards
    </Button>
  )
  if (!detail)
    return (
      <article className="grid gap-3" aria-label="Card" aria-busy={!error}>
        {back}
        {summary ? (
          <PageHeader card={summary} spinning={busy(summary)} />
        ) : (
          !error && (
          <header className="grid gap-2" aria-hidden>
            <span className="h-[18px] w-40 bg-muted/60" />
            <span className="h-8 w-[min(32rem,80%)] bg-muted/40" />
          </header>
          )
        )}
        <Card className="items-start gap-2 px-5 py-5! text-sm">
          {error ? (
            <>
              <p className="font-medium text-ink" role="alert">
                Could not open the card
              </p>
              <p className="text-[12.5px] text-muted-foreground">
                {plainLine(error)}
              </p>
              <Button size="sm" variant="outline" onClick={onBack}>
                Back to the cards
              </Button>
            </>
          ) : (
            <p className="text-muted-foreground">Opening the card…</p>
          )}
        </Card>
      </article>
    )
  const p = percent(detail.progress)
  const open = detail.blockers.filter((b) => !b.resolved)
  // A card blocked the old way (a reason, no items) has nothing to pick.
  const legacy = detail.openBlockers > 0 && open.length === 0
  const spinning = busy(detail)
  // Links already on screen (a PR's, an open blocker's or one of its
  // answers') aren't listed again on the side.
  const shownLinks = new Set(
    detail.prs.map((pr) => `${LinkKind.LinkURL}:${pr.url}`)
  )
  for (const b of open) {
    for (const l of b.links ?? []) shownLinks.add(`${l.kind}:${l.ref}`)
    for (const o of b.options)
      if (o.link) shownLinks.add(`${o.link.kind}:${o.link.ref}`)
  }
  const links = detail.links.filter(
    (l) => !shownLinks.has(`${l.kind}:${l.ref}`)
  )
  const files = cardFiles(detail)
  const showNote = !!detail.note && detail.state !== TaskState.InputRequired
  const hasObjective = !!(detail.goal || detail.doneWhen || showNote)
  const hasBlockers = detail.blockers.length > 0 || legacy || !!answered
  return (
    <article className="grid gap-3" aria-label="Card">
      {back}
      <PageHeader card={detail} spinning={spinning} />
      <ProgressStrip detail={detail} spinning={spinning} />
      <div className="grid items-start gap-4 min-[900px]:grid-cols-[minmax(0,1fr)_minmax(0,360px)] min-[900px]:grid-rows-[auto_1fr] min-[1600px]:grid-cols-[minmax(0,1fr)_minmax(0,400px)]">
        {(hasObjective || hasBlockers) && (
          <Card className="gap-0 py-0! min-[900px]:col-start-1 min-[900px]:row-start-1">
            {hasObjective && (
              <Section title="Objective" last={!hasBlockers}>
                {detail.goal && (
                  <RichText text={detail.goal} className="text-sm text-ink-2" />
                )}
                {showNote && (
                  <p className="text-[12.5px] text-muted-foreground">
                    <span className="font-medium text-ink-2">Note:</span>{" "}
                    <RichLine text={detail.note} />
                  </p>
                )}
                {detail.doneWhen && (
                  <p className="text-[12.5px] text-muted-foreground">
                    <span className="font-medium text-ink-2">Done when:</span>{" "}
                    <RichLine text={detail.doneWhen} />
                  </p>
                )}
              </Section>
            )}
            {hasBlockers && (
              <Section title="Blockers" last>
                {legacy && (
                  <div className="grid gap-1.5 border border-destructive/50 bg-destructive/10 p-3">
                    <div className="text-[13.5px] font-semibold text-ink">
                      <RichLine text={detail.blockerQuestion} />
                    </div>
                    <div className="text-xs text-muted-foreground">
                      Raised without recommended answers. The agent on it
                      clears it when it's sorted.
                    </div>
                  </div>
                )}
                {answered && !open.some((b) => b.id === NoteItem) && (
                  <div className="grid gap-2 border border-border bg-muted/40 p-3 opacity-85">
                    <div className="text-[13.5px] font-semibold text-ink">
                      <span className="text-success" aria-label="Answered: ">
                        ✓{" "}
                      </span>
                      {plainLine(answered.question)}
                    </div>
                    <div className="text-[11.5px] text-muted-foreground">
                      You answered: “{plainLine(answered.answer)}”
                    </div>
                    <ToldLine told={answered.told} raiser={answered.to} />
                  </div>
                )}
                {detail.blockers.map((b) => (
                  <BlockerItem
                    key={b.id}
                    cardId={detail.id}
                    blocker={b}
                    now={now}
                    onNoteAnswered={(answer, told) =>
                      setAnswered({
                        question: b.question,
                        answer,
                        told,
                        to: b.raisedBy,
                      })
                    }
                  />
                ))}
              </Section>
            )}
          </Card>
        )}

        <aside
          aria-label="About the card"
          className="min-w-0 min-[900px]:sticky min-[900px]:top-[4.5rem] min-[900px]:max-h-[calc(100svh-5.5rem)] min-[900px]:overflow-y-auto min-[900px]:col-start-2 min-[900px]:row-span-2 min-[900px]:row-start-1"
        >
          <Card className="gap-0 py-0!">
            <Section title="Progress">
              {p === null ? (
                <HolderLine holder={detail.holder} />
              ) : (
                <ProgressBar
                  progress={detail.progress}
                  label={`${p ?? 0}% complete`}
                  tone={cardTone(detail)}
                />
              )}
              <ProgressLine detail={detail} />
              {detail.steps.length > 0 && (
                <Steps steps={detail.steps} spinning={spinning} />
              )}
            </Section>

            <Section
              title="Who's on it"
              last={!detail.prs.length && !links.length && !files.length}
            >
              <People detail={detail} gone={gone} />
            </Section>

            {detail.prs.length > 0 && (
              <Section
                title="Pull requests"
                last={!links.length && !files.length}
              >
                <div className="grid gap-2">
                  {detail.prs.map((pr) => (
                    <PRRow key={`${pr.repo}#${pr.number}`} pr={pr} />
                  ))}
                </div>
              </Section>
            )}

            {links.length > 0 && (
              <Section title="Links" last={!files.length}>
                <div className="flex flex-wrap gap-1.5">
                  {links.map((l) => (
                    <LinkChip key={`${l.kind}:${l.ref}`} link={l} />
                  ))}
                </div>
              </Section>
            )}

            {files.length > 0 && (
              <Section title="Files" last>
                <AttachmentList attachments={files} compact />
              </Section>
            )}
          </Card>
        </aside>

        <Card className="gap-0 py-0! min-[900px]:col-start-1 min-[900px]:row-start-2">
          <Section title="Thread" last>
            <Thread
              cardId={detail.id}
              items={detail.thread}
              now={now}
              earlier={detail.threadEarlier}
              historyTrimmed={detail.historyTrimmed}
              composer={<CommentBox cardId={detail.id} />}
            />
          </Section>
        </Card>
      </div>
    </article>
  )
}

// PageHeader is the card page's title block: its short ref, state, scope
// and repo over the title, with the spinner while an agent works on it.
function PageHeader({
  card,
  spinning,
}: {
  card: CardSummary
  spinning?: boolean
}) {
  return (
    <header className="grid gap-2">
      <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
        <ShortRef card={card} />
        <StatePill card={card} />
        {spinning && <Spinner />}
        <ScopeBadge card={card} />
        {card.repoKey && (
          <span className="font-mono" title={card.repoKey}>
            {shortRepo(card.repoKey)}
          </span>
        )}
      </div>
      <h1 className="text-2xl leading-tight font-semibold tracking-tight text-balance text-ink">
        {plainLine(card.title) || "(untitled)"}
      </h1>
    </header>
  )
}

// ProgressLine is the progress figures under the bar: how far along, and
// what it waits on.
function ProgressLine({ detail }: { detail: CardDetail }) {
  const p = percent(detail.progress)
  const parts: ReactNode[] = []
  if (p !== null)
    parts.push(
      <span key="p" className="tabular-nums">
        {detail.progress.done} of {detail.progress.total} steps · {p}%
      </span>
    )
  if (detail.openBlockers > 0)
    parts.push(
      <span key="b" className="text-destructive">
        {detail.openBlockers} waiting on a decision
      </span>
    )
  if (detail.waiting > 0)
    parts.push(<span key="w">{detail.waiting} waiting on an agent</span>)
  if (!parts.length) return null
  return (
    <div className="flex flex-wrap gap-1.5 text-xs text-muted-foreground">
      {parts.flatMap((x, i) =>
        i
          ? [
              <span key={`d${i}`} aria-hidden>
                ·
              </span>,
              x,
            ]
          : [x]
      )}
    </div>
  )
}

// ProgressStrip is the "where is this" glance under the header on a narrow
// window, where the side column comes after the blockers: the bar (or who
// holds it), the figures and the current step. Wide, the side column has
// it all at the top.
function ProgressStrip({
  detail,
  spinning,
}: {
  detail: CardDetail
  spinning: boolean
}) {
  const p = percent(detail.progress)
  const current = detail.steps.find((s) => s.doing && !s.done)
  return (
    <div className="glass-panel grid gap-2 px-4 py-3 min-[900px]:hidden">
      {p === null ? (
        <HolderLine holder={detail.holder} />
      ) : (
        <ProgressBar
          progress={detail.progress}
          label={`${p}% complete`}
          tone={cardTone(detail)}
        />
      )}
      <ProgressLine detail={detail} />
      {current && (
        <div className="grid grid-cols-[16px_minmax(0,1fr)] items-start gap-2 text-[13px]">
          {spinning ? (
            <Spinner className="mt-0.5" label="Being worked on now" />
          ) : (
            <span
              className="mt-0.5 grid size-3.5 place-items-center ring-1 ring-foreground/70 ring-inset"
              aria-hidden
            >
              <span className="size-1.5 bg-primary" />
            </span>
          )}
          <span className="font-medium text-foreground">
            <span className="sr-only">In progress: </span>
            {plainLine(current.text)}
          </span>
        </div>
      )}
    </div>
  )
}

// Steps is the card's plan: done ones ticked, the current one marked, and
// spinning while an agent works on the card.
function Steps({ steps, spinning }: { steps: Step[]; spinning: boolean }) {
  return (
    <ul className="grid gap-1.5 text-[13px]">
      {steps.map((s) => {
        const current = s.doing && !s.done
        return (
          <li key={s.id} className="grid grid-cols-[16px_minmax(0,1fr)] gap-2">
            {current && spinning ? (
              <Spinner className="mt-0.5" label="Being worked on now" />
            ) : (
              <span
                className={cn(
                  "mt-0.5 grid size-3.5 place-items-center text-[10px]",
                  s.done
                    ? "bg-success text-background"
                    : s.doing
                      ? "ring-1 ring-foreground/70 ring-inset"
                      : "ring-1 ring-muted-foreground/50 ring-inset"
                )}
                aria-hidden
              >
                {s.done ? (
                  "✓"
                ) : s.doing ? (
                  <span className="size-1.5 bg-primary" />
                ) : (
                  ""
                )}
              </span>
            )}
            <span
              className={cn(
                current && "font-medium text-foreground",
                s.done && "text-ink-2"
              )}
            >
              <span className="sr-only">
                {s.done ? "Done: " : s.doing ? "In progress: " : "To do: "}
              </span>
              {plainLine(s.text)}
            </span>
          </li>
        )
      })}
    </ul>
  )
}

// cardFiles is every file on the card, once each: the ones on its
// blockers and on its thread (blocker replies too). The side column lists
// them by name only; the pictures show where they were posted.
function cardFiles(d: CardDetail): FileRef[] {
  const seen = new Set<string>()
  const out: FileRef[] = []
  const add = (fs: FileRef[] | undefined | null) => {
    for (const f of fs ?? []) {
      if (seen.has(f.sha256)) continue
      seen.add(f.sha256)
      out.push(f)
    }
  }
  for (const b of d.blockers) {
    add(b.attachments)
    for (const it of b.thread) add(it.attachments)
  }
  for (const it of d.thread) add(it.attachments)
  return out
}

function Section({
  title,
  last,
  children,
}: {
  title: string
  last?: boolean
  children: ReactNode
}) {
  return (
    <section
      className={cn(
        "grid gap-2.5 px-5 py-3.5",
        !last && "border-b border-border"
      )}
    >
      <h3 className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {title}
      </h3>
      {children}
    </section>
  )
}

function Note({ children }: { children: ReactNode }) {
  return <p className="text-[12.5px] text-muted-foreground">{children}</p>
}

const name = displayName

// CommentBox is the comment box under a card's thread: the person's
// comment goes to the agents on the card (BoardService.Comment), and the
// line under it says who heard. Wake is on by default: a comment from the
// app is asked now.
function CommentBox({ cardId }: { cardId: string }) {
  const [text, setText] = useDraft(`${cardId}/comment`)
  const [wake, setWake] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [heard, setHeard] = useState<{ line: string; error?: string } | null>(
    null
  )

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (busy) return
    if (!text.trim()) {
      setError("Write a comment first.")
      return
    }
    setBusy(true)
    setError(null)
    try {
      const res = await cards.comment(cardId, text, wake)
      setHeard({
        line: heardLine(res.answers, res.watching, res.told.mode ?? ""),
        error: res.told.error,
      })
      setText("")
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="mt-1 grid gap-1.5" onSubmit={(e) => void submit(e)}>
      <Textarea
        autoSize
        value={text}
        maxLength={MaxComment}
        onChange={(e) => {
          setText(e.target.value)
          setHeard(null)
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey))
            e.currentTarget.form?.requestSubmit()
        }}
        placeholder="Comment on the card"
        aria-label="Your comment on the card"
        className="max-h-48 min-h-10 bg-card"
        disabled={busy}
      />
      <div className="flex items-center justify-between gap-3">
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          <Switch
            size="sm"
            checked={wake}
            disabled={busy}
            onCheckedChange={(v) => setWake(v)}
          />
          Wake now
        </label>
        <Button type="submit" size="sm" disabled={busy}>
          {busy ? "Sending…" : "Comment"}
        </Button>
      </div>
      {heard &&
        (heard.error ? (
          <p className="text-xs text-destructive">
            On the card, but not everyone could be told: {heard.error}
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">{heard.line}</p>
        ))}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

// BlockerItem is one blocker: the question, context for a newcomer, who
// raised it, its own thread (the person's answers, the agents' replies)
// and, while the person can answer, the recommended answers as buttons
// with a field for their own words. By state: open and replied are amber
// (they wait on the person); answered is grey with who it went to, never
// a check, the form folded behind "Add to your answer"; resolved has the
// check and the resolver's note; closed keeps its old look. While
// answered or replied the person can also mark it resolved.
function BlockerItem({
  cardId,
  blocker: b,
  now,
  onNoteAnswered,
}: {
  cardId: string
  blocker: CardBlocker
  now: Date
  // The note item (an input_required card's question) leaves the card once
  // answered; this hands its answer to the panel to show.
  onNoteAnswered: (answer: string, told: Told) => void
}) {
  const [text, setText] = useDraft(`${cardId}/blocker/${b.id}`)
  const [note, setNote] = useDraft(`${cardId}/resolve/${b.id}`)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [told, setTold] = useState<Sent | null>(null)
  // A kept draft opens the form it was typed in.
  const [adding, setAdding] = useState(() => !!text)
  const [resolving, setResolving] = useState(() => !!note)
  // A resolved item folds to its one line; a click opens it.
  const [unfolded, setUnfolded] = useState(false)
  const raised = parseTime(b.at)
  const r = b.resolved
  const state = b.state || BlockerState.Open
  const tone = blockerTone(state)
  const chip = blockerChip(b)
  const answered =
    state === BlockerState.Answered || state === BlockerState.Replied
  // Answered: the form waits behind "Add to your answer".
  const showForm =
    answerable(state) && (state !== BlockerState.Answered || adding)

  // run sends an answer (own is its words, "" for a pick), or with own
  // null marks the item resolved.
  const run = async (act: () => Promise<AnswerResult>, own: string | null) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      const res = await act()
      if (b.id === NoteItem && own !== null)
        onNoteAnswered(own.trim(), res.told)
      // Marked resolved: the check says it all.
      const after = res.card.blockers.find((x) => x.id === b.id)
      setTold(
        own === null ? null : { told: res.told, resolved: !!after?.resolved }
      )
      setText("")
      setNote("")
      setAdding(false)
      setResolving(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const send = (option: number, own: string) => {
    if (option < 0 && !own.trim()) {
      setError("Write an answer first.")
      return
    }
    void run(() => cards.answer(cardId, b.id, option, own), own)
  }

  const submit = (e: FormEvent) => {
    e.preventDefault()
    send(-1, text)
  }

  const markResolved = (e: FormEvent) => {
    e.preventDefault()
    void run(() => cards.resolveBlocker(cardId, b.id, note), null)
  }

  const form = showForm ? (
    <div className="grid gap-1.5">
      {b.options.length > 0 && (
        <div
          className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,280px),1fr))] gap-1.5"
          role="group"
          aria-label={`Answers to: ${plainLine(b.question)}`}
        >
          {byRecommendation(b.options).map(([o, i]) => (
            <OptionItem
              key={i}
              option={o}
              busy={busy}
              onPick={() => send(i, "")}
            />
          ))}
        </div>
      )}
      <form className="flex gap-1.5" onSubmit={submit}>
        <Input
          value={text}
          maxLength={MaxAnswer}
          onChange={(e) => setText(e.target.value)}
          placeholder={
            answered
              ? "Answer again, or add what's missing"
              : b.options.length
                ? "Or answer in your own words"
                : "Answer in your own words"
          }
          aria-label={`Your answer to: ${plainLine(b.question)}`}
          className="min-w-0 flex-1 bg-card"
          disabled={busy}
        />
        <Button type="submit" disabled={busy}>
          {busy ? "Sending…" : "Answer"}
        </Button>
      </form>
    </div>
  ) : null

  const by = r
    ? r.closed
      ? `closed by ${name(r.by)}${r.answer ? `: “${plainLine(r.answer)}”` : ""}`
      : r.note
        ? `resolved by ${name(r.by)}: “${plainLine(r.note)}”`
        : `${name(r.by)} answered: “${plainLine(r.answer)}”`
    : ""

  if (r && !unfolded && !told && !error)
    return (
      <button
        type="button"
        onClick={() => setUnfolded(true)}
        aria-expanded={false}
        title="Show the whole item"
        className="flex min-w-0 items-baseline gap-2 border border-border bg-muted/30 px-3 py-2 text-left text-[12.5px] opacity-85 outline-none hover:bg-muted/50 focus-visible:ring-1 focus-visible:ring-ring"
      >
        <span className="shrink-0 text-success" aria-label="Resolved: ">
          ✓
        </span>
        <span className="min-w-0 truncate">
          <span className="font-semibold text-ink">
            {plainLine(b.question)}
          </span>
          <span className="text-muted-foreground"> · {by}</span>
        </span>
      </button>
    )

  return (
    <div
      className={cn(
        "grid gap-2 rounded-xl border p-3",
        tone === "amber" && "border-destructive/30 bg-destructive/5",
        tone === "grey" && "border-border bg-muted/30",
        (tone === "green" || tone === "closed") &&
          "border-border bg-muted/50 opacity-85"
      )}
    >
      <div className="flex flex-wrap items-center gap-1.5 text-[13.5px] font-semibold text-ink">
        {r && (
          <Button
            size="xs"
            variant="ghost"
            aria-expanded
            className="order-last ml-auto font-normal text-muted-foreground"
            onClick={() => setUnfolded(false)}
          >
            Fold
          </Button>
        )}
        <span>
          {r && (
            <span className="text-success" aria-label="Resolved: ">
              ✓{" "}
            </span>
          )}
          <RichLine text={b.question} />
        </span>
        {chip && (
          <Badge
            variant={chip.tone}
            className="h-4 px-1.5 text-[10.5px] font-normal"
          >
            {plainLine(chip.label)}
          </Badge>
        )}
      </div>
      {b.context && (
        <RichText text={b.context} className="text-[12.5px] text-ink-2" />
      )}
      {(b.links?.length ?? 0) > 0 && (
        <div
          className="flex flex-wrap gap-1.5"
          role="group"
          aria-label={`Links for: ${plainLine(b.question)}`}
        >
          {b.links.map((l) => (
            <LinkChip key={`${l.kind}:${l.ref}`} link={l} />
          ))}
        </div>
      )}
      <AttachmentList attachments={b.attachments ?? []} />
      <div className="text-[11.5px] text-muted-foreground">
        Raised by {name(b.raisedBy)} ·{" "}
        <time dateTime={raised?.toISOString()} title={raised?.toLocaleString()}>
          {relativeTime(raised, now)}
        </time>
        {by && ` · ${by}`}
      </div>
      {b.thread.length > 0 ? (
        <Thread
          compact
          inBlocker
          cardId={cardId}
          items={b.thread}
          now={now}
          composer={form}
        />
      ) : (
        form
      )}
      {answered && !busy && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          {state === BlockerState.Answered && !adding && (
            <Button
              size="xs"
              variant="ghost"
              className="text-muted-foreground"
              onClick={() => setAdding(true)}
            >
              Add to your answer
            </Button>
          )}
          {!resolving && (
            <Button
              size="xs"
              variant="link"
              className="h-auto px-0 text-muted-foreground"
              onClick={() => setResolving(true)}
            >
              Mark resolved
            </Button>
          )}
        </div>
      )}
      {resolving && (
        <form className="flex gap-1.5" onSubmit={markResolved}>
          <Input
            value={note}
            maxLength={MaxAnswer}
            onChange={(e) => setNote(e.target.value)}
            placeholder="A note beside the check (optional)"
            aria-label={`Note on resolving: ${plainLine(b.question)}`}
            className="min-w-0 flex-1 bg-card"
            disabled={busy}
          />
          <Button type="submit" disabled={busy}>
            {busy ? "Saving…" : "Mark resolved"}
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={busy}
            onClick={() => {
              setResolving(false)
              setNote("")
            }}
          >
            Cancel
          </Button>
        </form>
      )}
      {told &&
        // Only while it still says what is so: an answer waits on the
        // raiser until it replies or resolves.
        (told.resolved ? !!r : state === BlockerState.Answered) && (
          <ToldLine
            told={told.told}
            raiser={b.raisedBy}
            sent={b.id === NoteItem ? undefined : told.resolved}
          />
        )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

// OptionItem is one answer to pick: the button picks it (the answer and
// who recommends it); its why and link sit beside the button, not in it,
// so their links and code can be clicked without picking.
function OptionItem({
  option: o,
  busy,
  onPick,
}: {
  option: CardOption
  busy: boolean
  onPick: () => void
}) {
  return (
    <div
      className={cn(
        "grid gap-1 border bg-card",
        o.recommended
          ? "border-primary/60 bg-accent/40 ring-1 ring-primary/20"
          : "border-border"
      )}
    >
      <button
        type="button"
        disabled={busy}
        onClick={onPick}
        className="grid gap-0.5 px-2.5 pt-2 text-left outline-none last:pb-2 hover:bg-primary/5 focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-60"
      >
        <span className="flex flex-wrap items-center gap-1.5 text-[13px] font-semibold text-ink">
          <span>
            <RichLine text={o.answer} interactive={false} />
          </span>
          {o.recommended && (
            <Badge variant="info" className="h-4 rounded-full px-1.5 text-[10.5px]">
              Recommended
            </Badge>
          )}
        </span>
        <span className="text-xs text-muted-foreground">
          {o.recommended ? (
            <span className="font-medium text-primary">
              {name(o.by)} recommends this.
            </span>
          ) : (
            <span>Option from {name(o.by)}.</span>
          )}
        </span>
      </button>
      {(o.why || o.link) && (
        <div className="grid gap-1 px-2.5 pb-2 text-xs text-muted-foreground">
          {o.why && <RichText text={o.why} />}
          {o.link && <LinkChip link={o.link} className="justify-self-start" />}
        </div>
      )}
    </div>
  )
}

// byRecommendation puts recommended options first, keeping each one's
// index: the answer sends the index, so the order on screen is free.
function byRecommendation(options: CardOption[]): [CardOption, number][] {
  const all = options.map((o, i): [CardOption, number] => [o, i])
  return [
    ...all.filter(([o]) => o.recommended),
    ...all.filter(([o]) => !o.recommended),
  ]
}

// Sent is what an answer did: who heard, and whether the item resolved.
type Sent = { told: Told; resolved: boolean }

// ToldLine says who heard the answer and how. For a blocker item (sent
// set), it says whether the answer left it open with the raiser or
// resolved it because nobody is running to act on it.
function ToldLine({
  told,
  raiser,
  sent,
}: {
  told: Told
  raiser: AgentRef
  sent?: boolean
}) {
  if (told.error)
    return (
      <p className="text-xs text-destructive">
        Answered, but {name(raiser)} couldn't be told: {told.error}
      </p>
    )
  if (sent === true)
    return (
      <p className="text-xs text-success">
        Resolved: nobody is running to act on it.
      </p>
    )
  if (sent === false)
    return (
      <p className="text-xs text-muted-foreground">
        Sent to {name(raiser)}; it stays open until {name(raiser)} resolves it.
      </p>
    )
  if (!told.to.length)
    return (
      <p className="text-xs text-muted-foreground">
        Answered. Nobody needed telling.
      </p>
    )
  return (
    <p className="text-xs text-success">
      {name(raiser)} {told.mode === "wake" ? "is woken with" : "gets"} your
      answer{told.mode === "wake" ? "" : " at its next turn"} and carries on.
    </p>
  )
}

// People is Who's on it: working agents with their live dot, then
// watchers, each with the machine it's on. A gone agent offers Revive when
// Irori still has its session.
function People({ detail, gone }: { detail: CardDetail; gone: GoneSession[] }) {
  const rows: [AgentRef, "working" | "watching"][] = [
    ...detail.working.map((w): [AgentRef, "working"] => [w, "working"]),
    ...detail.watching.map((w): [AgentRef, "watching"] => [w, "watching"]),
  ]
  if (!rows.length)
    return <Note>Nobody yet. An agent that takes the card shows here.</Note>
  return (
    <div className="grid gap-1.5">
      {rows.map(([w, role]) => (
        <div
          key={`${role}-${w.key}`}
          className="grid grid-cols-[22px_minmax(0,1fr)_auto] items-center gap-2 text-[13px]"
        >
          <Face who={w} watching={role === "watching"} noDot />
          <span className="min-w-0 truncate">
            <span className="text-ink">{name(w)}</span>{" "}
            <small className="text-xs text-muted-foreground">· {w.where}</small>
            {w.offline && (
              <Badge
                variant="slate"
                className="ml-1.5 h-4 px-1.5 text-[10.5px]"
              >
                offline
              </Badge>
            )}
            {w.conflict && (
              <Badge
                variant="warning"
                className="ml-1.5 h-4 px-1.5 text-[10.5px]"
                title={`Two devices leased ${w.conflict} while apart`}
              >
                conflict
              </Badge>
            )}
          </span>
          <span className="flex items-center gap-2 text-xs text-muted-foreground">
            {w.status === "gone" ? (
              <>
                <span>gone</span>
                <Revive who={w} gone={gone} />
              </>
            ) : role === "working" ? (
              w.assigned ? (
                w.waiting ? (
                  <span
                    className="text-warning"
                    title="The hand-over is still queued or held: it hasn't reached this agent"
                  >
                    waiting for it
                  </span>
                ) : (
                  <span>assigned</span>
                )
              ) : (
                <PersonStatus card={detail} who={w} />
              )
            ) : (
              <span>watching</span>
            )}
          </span>
        </div>
      ))}
    </div>
  )
}

// PersonStatus is a holder's word in Who's on it, by the card's spinner
// rule: "working" with its live dot only while the card spins for this
// agent; nothing once the card is over; else "on it".
function PersonStatus({ card, who }: { card: CardDetail; who: AgentRef }) {
  if (busy(card) && agentBusy(who))
    return (
      <>
        <LiveDot />
        <span>working</span>
      </>
    )
  if (over(card)) return null
  return <span>on it</span>
}

// Revive runs claude --bg --resume for a gone agent's session after a
// confirm. Only sessions Irori still lists as gone (with a transcript) can.
function Revive({ who, gone }: { who: AgentRef; gone: GoneSession[] }) {
  const [done, setDone] = useState(false)
  const g = gone.find((x) => x.key === who.key)
  if (!g?.transcript) return null
  // Revived: no second button, or a quick second click could start the
  // same session again before the roster sees it.
  if (done) return <span>revived</span>
  const where = shortPath(g.cwd)
  return (
    <ConfirmButton
      label="Revive"
      question={
        <>
          Run <code className="font-mono">claude --bg --resume</code> for{" "}
          {name(who)} in <span className="font-mono">{where}</span>?
          {!g.trusted && (
            <span className="block text-warning">
              Claude Code may refuse: this folder isn't marked trusted.
            </span>
          )}
        </>
      }
      confirmLabel="Revive"
      onConfirm={async () => {
        await talk.revive(g.sessionId)
        setDone(true)
      }}
    />
  )
}

// PRRow is one linked pull request with what the ledger knows of it: who
// opened it, reviewed it or touched it. CI and review state come with v0.4.
function PRRow({ pr }: { pr: CardPR }) {
  const roles: [string, AgentRef[]][] = [
    ["Opened by", pr.openedBy],
    ["Reviewed by", pr.reviewedBy],
    ["Also touched by", pr.touchedBy],
  ]
  const known = roles.filter(([, who]) => who.length > 0)
  return (
    <div className="grid gap-1.5 border border-border bg-background/40 px-3 py-2.5">
      <div className="flex min-w-0 items-center gap-2 text-[13px]">
        <span className="shrink-0 font-mono text-xs text-muted-foreground">
          #{pr.number}
        </span>
        <span
          className="min-w-0 truncate font-medium text-ink"
          title={plainLine(pr.repo)}
        >
          {plainLine(pr.repo)}
        </span>
        <Button
          size="xs"
          variant="ghost"
          className="ml-auto shrink-0"
          onClick={() => void cards.openURL(pr.url)}
          title={pr.url}
        >
          {plainLine(pr.label) || `#${pr.number}`}
          <ExternalLinkIcon aria-hidden />
        </Button>
      </div>
      <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
        {known.length ? (
          known.map(([label, who]) => (
            <span key={label}>
              {label} {who.map(name).join(", ")}
            </span>
          ))
        ) : (
          <span>No agent here has worked on it yet.</span>
        )}
      </div>
    </div>
  )
}
