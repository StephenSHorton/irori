// NeedsYou is Home's first section (card #261, #259's organization A):
// every done check, decision, held message and agent's message waiting on
// the person, newest first, each answerable on its row. The card's own
// page and the message dialog still have the full forms (mentions, files);
// a row has the everyday answers.

import { useEffect, useState, type FormEvent, type ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { byRecommendation } from "@/components/cards/card-detail"
import { cardRef } from "@/components/cards/bits"
import {
  BlockerState,
  cards as cardsApi,
  displayName,
  useCard,
  type CardBlocker,
  type CardSummary,
} from "@/lib/cards"
import { inWails, plainLine, relativeTime } from "@/lib/irori"
import { AgentsService } from "@/lib/host"
import {
  NOT_TRUSTED,
  spawnNeed,
  WAITING_PERMISSION,
  type Need,
} from "@/lib/needs"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import { openSettings } from "@/components/settings/settings-menu"
import { answerable } from "@/lib/thread"
import {
  allowLifts,
  errorText,
  holdText,
  HoldReason,
  talk,
  type Delivery,
} from "@/lib/talk"

export function NeedsYou({
  needs,
  now,
  name,
  onOpenCard,
  onHeld,
}: {
  needs: Need[]
  now: Date
  // name is an agent's name by key (a held message's addressee).
  name: (key: string) => string
  onOpenCard: (id: string) => void
  // onHeld opens the Held list (the quiet line's See them).
  onHeld: () => void
}) {
  // The quiet line (card #488) is held messages that don't ask the person
  // one by one: it isn't counted, and alone it isn't "something waits".
  const rows = needs.filter((n) => n.kind !== "heldQuiet")
  const quiet = needs.find((n) => n.kind === "heldQuiet")
  return (
    <section aria-labelledby="needs-you" className="grid min-w-0 gap-2.5">
      <h2 id="needs-you" className="px-1 text-base font-semibold">
        Needs you
        {rows.length > 0 && (
          <span className="font-medium text-muted-foreground"> · {rows.length}</span>
        )}
      </h2>
      {rows.length === 0 && (
        <p className="px-1 text-[12.5px] text-muted-foreground">
          Nothing waits on you right now.
        </p>
      )}
      {rows.length > 0 && (
        <ul className="grid min-w-0 gap-2.5" aria-label="What needs you">
          {rows.map((n) => (
            <li key={`${n.kind}:${n.id}`} className="min-w-0">
              {n.kind === "check" || n.kind === "decision" ? (
                <CardNeed need={n} now={now} onOpen={onOpenCard} />
              ) : n.kind === "trust" ? (
                <TrustRow need={n} now={now} onOpen={onOpenCard} />
              ) : n.kind === "review" ? (
                <ReviewNeed need={n} now={now} onOpen={onOpenCard} />
              ) : n.kind === "held" ? (
                <HeldNeed d={n.delivery} now={now} name={name} />
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {quiet?.kind === "heldQuiet" && (
        <HeldQuiet deliveries={quiet.deliveries} onHeld={onHeld} />
      )}
    </section>
  )
}

const kinds = {
  check: { label: "Done check", variant: "success" },
  decision: { label: "Decision", variant: "destructive" },
  review: { label: "Review", variant: "warning" },
  trust: { label: "Trust", variant: "warning" },
  // Card #456 (sa-7): a spawned agent stopped on a permission prompt, and
  // a card whose agents crashed twice.
  permission: { label: "Permission", variant: "warning" },
  crashed: { label: "Crashed", variant: "destructive" },
  message: { label: "Message", variant: "info" },
  held: { label: "Held", variant: "warning" },
} as const

// Row is one item's frame: its kind, where it's from, when, its title.
function Row({
  kind,
  where,
  at,
  now,
  title,
  children,
}: {
  kind: keyof typeof kinds
  where: ReactNode
  at: number
  now: Date
  title: ReactNode
  children: ReactNode
}) {
  const k = kinds[kind]
  return (
    <article className="glass-panel grid min-w-0 gap-2 px-4 py-3">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
        <Badge variant={k.variant}>{k.label}</Badge>
        <span className="min-w-0 truncate">{where}</span>
        <span className="ml-auto shrink-0 tabular-nums">
          {at ? relativeTime(new Date(at), now) : ""}
        </span>
      </div>
      <h3 className="line-clamp-2 min-w-0 text-[15px] leading-snug font-semibold break-words">
        {title}
      </h3>
      {children}
    </article>
  )
}

// useAct runs one action at a time, keeping its error.
export function useAct() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState("")
  const run = async (fn: () => Promise<unknown>, said: string) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await fn()
      setDone(said)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, done, run }
}

// Said is a row's outcome while the list catches up, or its error.
export function Said({ error, done }: { error: string | null; done: string }) {
  if (error)
    return (
      <p className="text-[12.5px] text-destructive" role="alert">
        {error}
      </p>
    )
  if (done)
    return (
      <p className="text-[12.5px] text-muted-foreground" role="status">
        {done}
      </p>
    )
  return null
}

// OwnWords is the fold-out box for an answer, a reply or a reason.
export function OwnWords({
  label,
  placeholder,
  send,
  busy,
  onSend,
  onCancel,
}: {
  label: string
  placeholder: string
  send: string
  busy: boolean
  onSend: (text: string) => void
  onCancel: () => void
}) {
  const [text, setText] = useState("")
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (text.trim()) onSend(text)
  }
  return (
    <form className="flex min-w-0 flex-wrap items-start gap-1.5" onSubmit={submit}>
      <Textarea
        autoFocus
        autoSize
        rows={1}
        value={text}
        aria-label={label}
        placeholder={placeholder}
        className="max-h-40 min-h-9 min-w-[min(100%,16rem)] flex-1 bg-card py-1.5"
        disabled={busy}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault()
            e.currentTarget.form?.requestSubmit()
          }
          if (e.key === "Escape") {
            e.preventDefault()
            onCancel()
          }
        }}
      />
      <Button type="submit" disabled={busy || !text.trim()}>
        {busy ? "Sending…" : send}
      </Button>
      <Button type="button" variant="ghost" onClick={onCancel} disabled={busy}>
        Cancel
      </Button>
    </form>
  )
}

// waitsOnYou: the blockers of a card the person answers now (the rule
// cardSummary counts as open_blockers: open, or replied back to them, and
// not another member's question on a shared board, card #264).
function waitsOnYou(b: CardBlocker): boolean {
  return (
    !b.decider &&
    answerable(b.state || BlockerState.Open) &&
    b.state !== BlockerState.Answered
  )
}

// ReviewNeed is a PR waiting on the person's review (card #347): once,
// with every card that waits on it, and the PR one click away.
function ReviewNeed({
  need,
  now,
  onOpen,
}: {
  need: Extract<Need, { kind: "review" }>
  now: Date
  onOpen: (id: string) => void
}) {
  const { pr, cards } = need
  const where = <span className="font-mono">{pr.repo}</span>
  return (
    <Row kind="review" where={where} at={need.at} now={now} title={`Review PR #${pr.number}`}>
      <p className="text-sm break-words text-muted-foreground">
        {cards.length === 1 ? "A card waits" : `${cards.length} cards wait`} on it:{" "}
        {cards.map((c, i) => (
          <span key={c.id}>
            {i > 0 && ", "}
            <button
              type="button"
              className="font-mono tabular-nums underline-offset-2 hover:underline"
              title={plainLine(c.title)}
              onClick={() => onOpen(c.id)}
            >
              {cardRef(c)}
            </button>
          </span>
        ))}
      </p>
      <span className="flex flex-wrap gap-1.5">
        <Button size="sm" onClick={() => void cardsApi.openURL(pr.url)} title={pr.url}>
          Open PR #{pr.number}
        </Button>
      </span>
    </Row>
  )
}

// TrustRow is a card whose agent can't start because its repo isn't
// trusted for agents on that device (card #449, spawned agents §7 "The
// pre-trust flow"): it says what trusting does, and Trust and start
// records it (from this window only) and starts the agent.
function TrustRow({
  need,
  now,
  onOpen,
}: {
  need: Extract<Need, { kind: "trust" }>
  now: Date
  onOpen: (id: string) => void
}) {
  const c = need.card
  const act = useAct()
  const host = inWails()
  const [asked, setAsked] = useState<TrustInfo | null>(null)
  useEffect(() => {
    if (!host) return
    let live = true
    AgentsService.TrustNeed(c.id)
      .then((n) => live && setAsked(n ?? null))
      .catch(() => live && setAsked(null))
    return () => {
      live = false
    }
  }, [host, c.id, c.cantStart])
  // Without the host (the preview) the reason is all there is.
  const info = host ? asked : trustFromReason(c.cantStart ?? "")
  const folder = info?.folder ?? trustFromReason(c.cantStart ?? "").folder
  const where = (
    <button
      type="button"
      className="font-mono tabular-nums underline-offset-2 hover:underline"
      onClick={() => onOpen(c.id)}
    >
      {cardRef(c)}
    </button>
  )
  return (
    <Row kind="trust" where={where} at={need.at} now={now} title={plainLine(c.title)}>
      <p className="text-sm break-words text-muted-foreground">
        Can't start an agent: <span className="font-mono">{folder}</span> isn't trusted for agents
        {info ? (info.here ? " on this computer" : ` on ${info.device}`) : ""}.
        {info?.what ? ` ${info.what}` : ""}
      </p>
      <span className="flex flex-wrap items-center gap-1.5">
        {info && !info.here ? (
          <span className="text-[12.5px] text-muted-foreground">
            Open Irori on {info.device} to trust it there.
          </span>
        ) : (
          <Button
            size="sm"
            disabled={act.busy || !!act.done}
            onClick={() =>
              void act.run(() => AgentsService.TrustAndStart(c.id), "Trusted. Starting its agent…")
            }
          >
            {act.busy ? "Trusting…" : "Trust and start"}
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => onOpen(c.id)}>
          Open card
        </Button>
      </span>
      <Said error={act.error} done={act.done} />
    </Row>
  )
}

type TrustInfo = { folder: string; device: string; here: boolean; what: string }

// trustFromReason reads `not trusted: <folder> on <device>` (the failed
// spawn's reason) when the host can't be asked.
export function trustFromReason(reason: string): TrustInfo {
  const rest = reason.startsWith(NOT_TRUSTED) ? reason.slice(NOT_TRUSTED.length) : reason
  const at = rest.lastIndexOf(" on ")
  const folder = at > 0 ? rest.slice(0, at) : rest
  return {
    folder,
    device: "this computer",
    here: true,
    what: `Claude Code will run agents in ${folder} on this computer without asking you first. It's the trust Claude Code asks for the first time you open a folder.`,
  }
}

// CardNeed is a card waiting on the person: its done check (Done, or Not
// done with a reason) or each of its decisions (the options, recommended
// first, or an answer in their own words).
function CardNeed({
  need,
  now,
  onOpen,
}: {
  need: Extract<Need, { kind: "check" | "decision" }>
  now: Date
  onOpen: (id: string) => void
}) {
  const c = need.card
  const { detail } = useCard(c.id)
  const spawn = need.kind === "decision" ? spawnNeed(c) : null
  // The permission wait is answered at the agent's prompt, not here.
  const blockers = detail
    ? detail.blockers.filter(
        (b) => waitsOnYou(b) && !(spawn === "permission" && b.question.startsWith(WAITING_PERMISSION)),
      )
    : []
  const where = <Where card={c} />
  return (
    <Row kind={spawn ?? need.kind} where={where} at={need.at} now={now} title={plainLine(c.title)}>
      {spawn === "permission" ? (
        <PermissionWait card={c} onOpen={onOpen} />
      ) : null}
      {spawn === "permission" && blockers.length === 0 ? null : blockers.length === 0 ? (
        <>
          {c.blockerQuestion && (
            <p className="text-sm break-words">{plainLine(c.blockerQuestion)}</p>
          )}
          <span className="flex flex-wrap gap-1.5">
            <Button size="sm" variant="outline" onClick={() => onOpen(c.id)}>
              Open card
            </Button>
          </span>
        </>
      ) : (
        blockers.slice(0, 3).map((b) => (
          <BlockerAnswer
            key={b.id}
            cardId={c.id}
            b={b}
            onOpen={() => onOpen(c.id)}
          />
        ))
      )}
      {blockers.length > 3 && (
        <Button
          size="sm"
          variant="link"
          className="justify-self-start px-0"
          onClick={() => onOpen(c.id)}
        >
          {blockers.length - 3} more on the card
        </Button>
      )}
    </Row>
  )
}

// PermissionWait is "Waiting for your permission" (card #456, sa-7): the
// agent Irori started for the card stopped on a permission prompt and
// nobody is attached. Attach answers it at the prompt; Logs shows what it
// was doing; the board's allow list (Settings › Agents) lets the next one
// run the command without asking.
function PermissionWait({
  card: c,
  onOpen,
}: {
  card: CardSummary
  onOpen: (id: string) => void
}) {
  const who = c.working.find((w) => w.spawned && !w.assigned)
  return (
    <div className="grid min-w-0 gap-1.5">
      <p className="text-sm break-words">
        {who ? displayName(who) : "Its agent"} is stopped on a permission
        prompt nobody is there to answer.
      </p>
      <div className="flex min-w-0 flex-wrap items-start gap-1.5">
        {who && (
          <SpawnedActions
            who={who}
            cardId={c.id}
            onOpenCard={onOpen}
            only={["attach", "logs"]}
          />
        )}
        <Button size="xs" variant="outline" onClick={() => openSettings("agents")}>
          Add to allow-list…
        </Button>
        <Button size="xs" variant="ghost" onClick={() => onOpen(c.id)}>
          Open card
        </Button>
      </div>
    </div>
  )
}

// Where names a card's ref, and its board only when it isn't yours.
function Where({ card: c }: { card: CardSummary }) {
  const ref = cardRef(c)
  const board = c.scope === "board" ? plainLine(c.scopeLabel) : ""
  return (
    <>
      <span className="font-mono tabular-nums">{ref}</span>
      {board && <> · {board}</>}
    </>
  )
}

export function BlockerAnswer({
  cardId,
  b,
  onOpen,
}: {
  cardId: string
  b: CardBlocker
  onOpen: () => void
}) {
  const act = useAct()
  const [own, setOwn] = useState(false)
  const question = plainLine(b.question)
  const send = (option: number, text: string) =>
    void act.run(
      () => cardsApi.answer(cardId, b.id, option, text),
      b.doneCheck
        ? option >= 0
          ? "Marked done."
          : "Sent back to the agent."
        : "Answer sent."
    )
  const replied = b.state === BlockerState.Replied
  return (
    <div className="grid min-w-0 gap-1.5">
      {!b.doneCheck && <p className="text-sm break-words">{question}</p>}
      {b.context && (
        <p className="line-clamp-3 text-[13px] break-words text-muted-foreground">
          {plainLine(b.context)}
        </p>
      )}
      {replied && b.missing.length > 0 && (
        <p className="text-[13px] break-words">
          They still need: {b.missing.map(plainLine).join("; ")}
        </p>
      )}
      {act.done ? null : own ? (
        <OwnWords
          label={`${b.doneCheck ? "Why it isn't done" : "Your answer to"}: ${question}`}
          placeholder={
            b.doneCheck
              ? "Say why it isn't done; it goes back to the agent"
              : "Answer in your own words"
          }
          send={b.doneCheck ? "Not done" : "Answer"}
          busy={act.busy}
          onSend={(t) => send(-1, t)}
          onCancel={() => setOwn(false)}
        />
      ) : (
        <span
          className="flex min-w-0 flex-wrap gap-1.5"
          role="group"
          aria-label={`Answers to: ${question}`}
        >
          {!b.optionsAside &&
            byRecommendation(b.options).map(([o, i], n) => (
              <Button
                key={i}
                size="sm"
                variant={n === 0 && (o.recommended || b.doneCheck) ? "default" : "outline"}
                disabled={act.busy}
                className="h-auto min-h-8 max-w-full py-1 text-left whitespace-normal"
                title={o.why ? plainLine(o.why) : undefined}
                onClick={() => send(i, "")}
              >
                {plainLine(o.answer)}
                {o.recommended && !b.doneCheck && (
                  <span className="opacity-75"> (recommended)</span>
                )}
              </Button>
            ))}
          <Button
            size="sm"
            variant="outline"
            disabled={act.busy}
            onClick={() => setOwn(true)}
          >
            {b.doneCheck ? "Not done…" : replied ? "Reply…" : "Answer…"}
          </Button>
          <Button size="sm" variant="ghost" onClick={onOpen}>
            Open card
          </Button>
        </span>
      )}
      <Said error={act.error} done={act.done} />
    </div>
  )
}

function sender(d: Delivery): string {
  return plainLine(d.message.from.name) || "an agent"
}

const at = (s: string) => {
  const t = Date.parse(s)
  return Number.isNaN(t) ? 0 : t
}

// HeldNeed is a message Irori held from an agent: Release, Dismiss, or
// Allow sender (only for the holds allowing them lifts). Held's History
// stays in the Held list.
function HeldNeed({
  d,
  now,
  name,
}: {
  d: Delivery
  now: Date
  name: (key: string) => string
}) {
  const act = useAct()
  const to = d.to ? name(d.to) : "a repo"
  return (
    <Row
      kind="held"
      where={
        <>
          {sender(d)} → {plainLine(to) || "an agent"}
        </>
      }
      at={at(d.createdAt)}
      now={now}
      title={`Irori held a message: ${holdText(d)}`}
    >
      <p className="line-clamp-3 text-sm break-words whitespace-pre-line">
        {d.message.text}
      </p>
      {!act.done && (
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
          {allowLifts(d) && (
            <Button
              size="sm"
              variant="outline"
              disabled={act.busy}
              onClick={() =>
                void act.run(
                  () => talk.allowAndRelease(d.id),
                  `Allowed ${sender(d)} and released.`
                )
              }
            >
              Allow sender
            </Button>
          )}
        </span>
      )}
      <Said error={act.error} done={act.done} />
    </Row>
  )
}

// HeldQuiet is Needs you's one line for the held messages that don't ask
// the person one by one (card #488): ones between agents, and ones their
// own Pause all held. Release all delivers every one that can go now (a
// message to a repo with no agent there yet stays held); See them opens
// the Held list, where each can be released or dismissed alone.
function HeldQuiet({
  deliveries,
  onHeld,
}: {
  deliveries: Delivery[]
  onHeld: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [said, setSaid] = useState("")
  const paused = deliveries.filter((d) => d.hold === HoldReason.HoldPaused).length
  const between = deliveries.length - paused
  const parts = [
    between ? `${between} between agents` : "",
    paused ? `${paused} while wakes were paused` : "",
  ].filter(Boolean)
  const n = deliveries.length
  const releaseAll = async () => {
    if (busy) return
    setBusy(true)
    setSaid("")
    let left = 0
    for (const d of deliveries) {
      try {
        await talk.release(d.id)
      } catch {
        left++
      }
    }
    setBusy(false)
    setSaid(
      left === 0
        ? n === 1
          ? "Released."
          : `Released all ${n}.`
        : `Released ${n - left}; ${left} can't go yet (See them says why).`
    )
  }
  return (
    <div
      role="group"
      aria-label="Held messages"
      className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 px-1 text-[12.5px] text-muted-foreground"
    >
      <span className="min-w-0">
        <span className="font-medium text-ink">{n} held</span>
        {parts.length > 0 && <>: {parts.join(", ")}</>}
      </span>
      <Button size="xs" variant="outline" disabled={busy} onClick={() => void releaseAll()}>
        {busy ? "Releasing…" : "Release all"}
      </Button>
      <Button size="xs" variant="ghost" onClick={onHeld}>
        See them
      </Button>
      {said && (
        <span role="status" className="basis-full">
          {said}
        </span>
      )}
    </div>
  )
}
