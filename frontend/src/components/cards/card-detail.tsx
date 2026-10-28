import { useState, type FormEvent, type ReactNode } from "react"
import { ExternalLinkIcon } from "lucide-react"

import { ConfirmButton } from "@/components/common/confirm-button"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  cards,
  displayName,
  MaxAnswer,
  NoteItem,
  percent,
  TaskState,
  type AgentRef,
  type CardBlocker,
  type CardDetail,
  type CardPR,
  type Told,
} from "@/lib/cards"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  shortPath,
  shortRepo,
} from "@/lib/irori"
import { errorText, talk, type GoneSession } from "@/lib/talk"
import { cn } from "@/lib/utils"
import { ProgressBar, ScopeBadge } from "./bits"
import { Face, LiveDot } from "./faces"

// Answered is a note answer the person just sent: the note item leaves the
// card once answered, so the panel keeps it to show who heard.
type Answered = { question: string; answer: string; told: Told; to: AgentRef }

// CardPanel is one card opened beside the grid: the goal, its blockers
// with recommended answers, progress, who's on it, PRs and recent lines.
// Key it by card id, so a note answer shown here stays with its card.
export function CardPanel({
  detail,
  error,
  gone,
  now,
}: {
  detail: CardDetail | null
  error: string | null
  gone: GoneSession[]
  now: Date
}) {
  const [answered, setAnswered] = useState<Answered | null>(null)
  if (!detail)
    return (
      <Card className="px-5 py-6! text-sm text-muted-foreground">
        {error ? `Could not open the card: ${error}` : "Opening the card…"}
      </Card>
    )
  const p = percent(detail.progress)
  const open = detail.blockers.filter((b) => !b.resolved)
  // A card blocked the old way (a reason, no items) has nothing to pick.
  const legacy = detail.openBlockers > 0 && open.length === 0
  return (
    <Card className="gap-0 py-0!">
      <div className="grid gap-2.5 border-b border-border px-5 pt-[18px] pb-3.5">
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <ScopeBadge card={detail} />
          {detail.repoKey && (
            <span className="font-mono" title={detail.repoKey}>
              {shortRepo(detail.repoKey)}
            </span>
          )}
        </div>
        <h2 className="text-xl leading-tight font-semibold tracking-tight text-balance text-ink">
          {plainLine(detail.title) || "(untitled)"}
        </h2>
        {detail.goal && (
          <p className="max-w-[62ch] text-sm whitespace-pre-wrap text-muted-foreground">
            {plainBlock(detail.goal)}
          </p>
        )}
        {detail.note && detail.state !== TaskState.InputRequired && (
          <p className="max-w-[62ch] text-[12.5px] text-muted-foreground">
            <span className="font-medium text-ink-2">Note:</span>{" "}
            {plainLine(detail.note)}
          </p>
        )}
        {detail.doneWhen && (
          <p className="max-w-[62ch] text-[12.5px] text-muted-foreground">
            <span className="font-medium text-ink-2">Done when:</span>{" "}
            {plainLine(detail.doneWhen)}
          </p>
        )}
        <ProgressBar
          progress={detail.progress}
          blocked={detail.openBlockers > 0}
          label={`${p ?? 0}% complete`}
        />
        <div className="flex flex-wrap gap-1.5 text-xs text-muted-foreground">
          {p !== null && <span>{p}% complete</span>}
          {p !== null && detail.openBlockers > 0 && <span>·</span>}
          {detail.openBlockers > 0 && (
            <span className="text-destructive">
              {detail.openBlockers} waiting on a decision
            </span>
          )}
        </div>
      </div>

      {(detail.blockers.length > 0 || legacy || answered) && (
        <Section title="Blockers">
          {legacy && (
            <div className="grid gap-1.5 border border-destructive/50 bg-destructive/10 p-3">
              <div className="text-[13.5px] font-semibold text-ink">
                {plainLine(detail.blockerQuestion)}
              </div>
              <div className="text-xs text-muted-foreground">
                Raised without recommended answers. The agent on it clears it
                when it's sorted.
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

      <Section title="Progress">
        {detail.steps.length ? (
          <ul className="grid gap-1.5 text-[13px]">
            {detail.steps.map((s) => (
              <li
                key={s.id}
                className="grid grid-cols-[16px_minmax(0,1fr)] gap-2"
              >
                <span
                  className={cn(
                    "mt-0.5 grid size-3.5 place-items-center text-[10px]",
                    s.done
                      ? "bg-success text-background"
                      : "ring-1 ring-muted-foreground/50 ring-inset"
                  )}
                  aria-hidden
                >
                  {s.done ? "✓" : ""}
                </span>
                <span
                  className={cn(
                    s.doing && !s.done && "font-medium text-primary",
                    s.done && "text-ink-2"
                  )}
                >
                  <span className="sr-only">
                    {s.done ? "Done: " : s.doing ? "In progress: " : "To do: "}
                  </span>
                  {plainLine(s.text)}
                  {s.doing && !s.done && " · in progress"}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <Note>No steps yet. The agents on it add them as they plan.</Note>
        )}
      </Section>

      <Section title="Who's on it">
        <People detail={detail} gone={gone} />
      </Section>

      <Section title="Pull requests">
        {detail.prs.length ? (
          <div className="grid gap-2">
            {detail.prs.map((pr) => (
              <PRRow key={`${pr.repo}#${pr.number}`} pr={pr} />
            ))}
          </div>
        ) : (
          <Note>
            No PR yet. The agents working on it link one when they open it.
          </Note>
        )}
      </Section>

      <Section title="Recent" last>
        {detail.recent.length ? (
          <div className="grid gap-1 text-[12.5px]">
            {detail.recent.map((a, i) => {
              const at = parseTime(a.at)
              return (
                <div
                  key={`${i}-${String(a.at)}`}
                  className="grid grid-cols-[56px_minmax(0,1fr)] gap-2"
                >
                  <time
                    className="text-muted-foreground tabular-nums"
                    dateTime={at?.toISOString()}
                    title={at?.toLocaleString()}
                  >
                    {relativeTime(at, now)}
                  </time>
                  <span className="text-ink-2">{plainLine(a.text)}</span>
                </div>
              )
            })}
          </div>
        ) : (
          <Note>Nothing yet.</Note>
        )}
      </Section>
    </Card>
  )
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

// BlockerItem is one blocker: the question, context for a newcomer, who
// raised it, and the recommended answers as buttons, with a field for an
// answer in the person's own words. The answer reaches the raiser as a
// wake (it is blocked on the person).
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
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [told, setTold] = useState<Told | null>(null)
  const raised = parseTime(b.at)
  const r = b.resolved

  const send = async (option: number, own: string) => {
    if (busy) return
    if (option < 0 && !own.trim()) {
      setError("Write an answer first.")
      return
    }
    setBusy(true)
    setError(null)
    try {
      const res = await cards.answer(cardId, b.id, option, own)
      if (b.id === NoteItem) onNoteAnswered(own.trim(), res.told)
      setTold(res.told)
      setText("")
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const submit = (e: FormEvent) => {
    e.preventDefault()
    void send(-1, text)
  }

  return (
    <div
      className={cn(
        "grid gap-2 border p-3",
        r
          ? "border-border bg-muted/40 opacity-85"
          : "border-destructive/50 bg-destructive/10"
      )}
    >
      <div className="text-[13.5px] font-semibold text-ink">
        {r && (
          <span className="text-success" aria-label="Resolved: ">
            ✓{" "}
          </span>
        )}
        {plainLine(b.question)}
      </div>
      {b.context && (
        <div className="max-w-[64ch] text-[12.5px] whitespace-pre-wrap text-ink-2">
          {plainBlock(b.context)}
        </div>
      )}
      <div className="text-[11.5px] text-muted-foreground">
        Raised by {name(b.raisedBy)} ·{" "}
        <time dateTime={raised?.toISOString()} title={raised?.toLocaleString()}>
          {relativeTime(raised, now)}
        </time>
        {r &&
          (r.closed ? (
            <>
              {" "}
              · closed by {name(r.by)}
              {r.answer ? `: “${plainLine(r.answer)}”` : ""}
            </>
          ) : (
            <>
              {" "}
              · {name(r.by)} answered: “{plainLine(r.answer)}”
            </>
          ))}
      </div>
      {told && r && <ToldLine told={told} raiser={b.raisedBy} />}
      {!r && (
        <>
          {b.options.length > 0 && (
            <div className="grid gap-1.5">
              {b.options.map((o, i) => (
                <button
                  key={i}
                  type="button"
                  disabled={busy}
                  onClick={() => void send(i, "")}
                  className="grid gap-0.5 border border-border bg-background/60 px-2.5 py-2 text-left transition-[border-color,background-color,transform] duration-100 outline-none hover:border-primary/70 hover:bg-primary/10 focus-visible:ring-1 focus-visible:ring-ring active:scale-[0.99] disabled:opacity-60"
                >
                  <span className="text-[13px] font-medium text-ink">
                    {plainLine(o.answer)}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    <span className="font-medium text-primary">
                      Recommended by {name(o.by)}.
                    </span>{" "}
                    {plainLine(o.why)}
                  </span>
                </button>
              ))}
            </div>
          )}
          <form className="flex gap-1.5" onSubmit={submit}>
            <Input
              value={text}
              maxLength={MaxAnswer}
              onChange={(e) => setText(e.target.value)}
              placeholder={
                b.options.length
                  ? "Or answer in your own words"
                  : "Answer in your own words"
              }
              aria-label={`Your answer to: ${plainLine(b.question)}`}
              className="min-w-0 flex-1"
              disabled={busy}
            />
            <Button type="submit" size="sm" disabled={busy}>
              {busy ? "Sending…" : "Answer"}
            </Button>
          </form>
        </>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

// ToldLine says who heard the answer and how.
function ToldLine({ told, raiser }: { told: Told; raiser: AgentRef }) {
  if (told.error)
    return (
      <p className="text-xs text-destructive">
        Answered, but {name(raiser)} couldn't be told: {told.error}
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
                <span>assigned</span>
              ) : (
                <>
                  {w.live && !w.offline && <LiveDot />}
                  <span>working</span>
                </>
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
      <div className="flex flex-wrap items-center gap-2 text-[13px]">
        <span className="font-mono text-xs text-muted-foreground">
          #{pr.number}
        </span>
        <span className="font-medium text-ink">{plainLine(pr.repo)}</span>
        <Button
          size="xs"
          variant="ghost"
          className="ml-auto"
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
