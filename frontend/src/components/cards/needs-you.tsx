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
import { Asker, cardRef } from "@/components/cards/bits"
import { ConfirmDialog } from "@/components/common/confirm-button"
import {
  BlockerState,
  cards as cardsApi,
  cardStore,
  displayName,
  useCard,
  type CardBlocker,
  type CardSummary,
} from "@/lib/cards"
import { Ago } from "@/components/common/ago"
import { inWails, plainLine } from "@/lib/irori"
import { AgentsService } from "@/lib/host"
import { sampleTrustAndStart } from "@/lib/trust-sample"
import {
  GIVEN_QUESTION,
  goneTitle,
  loopLine,
  NOT_TRUSTED,
  refOf,
  spawnNeed,
  stuckLine,
  WAITING_PERMISSION,
  type Need,
} from "@/lib/needs"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import { PermissionPrompt } from "@/components/cards/permission-prompt"
import { openSettings } from "@/lib/nav"
import { answerable } from "@/lib/thread"
import { useDraft } from "@/lib/drafts"
import { answerKey, answerPatch, instant, isDone } from "@/lib/optimistic"
import {
  allowLifts,
  errorText,
  holdText,
  HoldReason,
  talk,
  startedLine,
  useRepoNames,
  type Delivery,
} from "@/lib/talk"
import { EarlierRound } from "./done-rounds"
import { ProposalsPreview, proposalsPreview } from "./needs-proposals"

export function NeedsYou({
  needs,
  name,
  onOpenCard,
  onHeld,
  onOpenThread,
  spaceName,
}: {
  needs: Need[]
  // name is an agent's name by key (a held message's addressee).
  name: (key: string) => string
  onOpenCard: (id: string) => void
  // onHeld opens the Held list (the quiet line's See them).
  onHeld: () => void
  // onOpenThread opens a waiting thread in Talk (card #1001); spaceName is
  // the name of the space a thread is in, "Personal" for "".
  onOpenThread?: (id: string) => void
  spaceName?: (spaceId: string) => string
}) {
  // The quiet line (card #488) is held messages that don't ask the person
  // one by one: it isn't counted, and alone it isn't "something waits".
  const repo = useRepoNames()
  // Start an agent's outcome: its row goes as the message becomes a card.
  const [started, setStarted] = useState("")
  const rows = needs.filter((n) => n.kind !== "heldQuiet")
  const quiet = needs.find((n) => n.kind === "heldQuiet")
  return (
    <section aria-labelledby="needs-you" className="grid min-w-0 gap-2.5">
      <h2 id="needs-you" className="px-1 text-base font-semibold">
        Needs you
        {rows.length > 0 && (
          <span className="font-medium text-muted-foreground">
            {" "}
            · {rows.length}
          </span>
        )}
      </h2>
      {proposalsPreview() && <ProposalsPreview />}
      {rows.length === 0 && !proposalsPreview() && (
        <p className="px-1 text-[12.5px] text-muted-foreground">
          Nothing waits on you right now.
        </p>
      )}
      {rows.length > 0 && (
        <ul className="grid min-w-0 gap-2.5" aria-label="What needs you">
          {rows.map((n) => (
            <li key={`${n.kind}:${n.id}`} className="min-w-0">
              {n.kind === "check" || n.kind === "decision" ? (
                <CardNeed need={n} onOpen={onOpenCard} />
              ) : n.kind === "gone" ? (
                <GoneNeed need={n} onOpen={onOpenCard} />
              ) : n.kind === "noAgent" ? (
                <NoAgentNeed need={n} onOpen={onOpenCard} />
              ) : n.kind === "trust" ? (
                <TrustRow need={n} onOpen={onOpenCard} />
              ) : n.kind === "loop" || n.kind === "stuck" ? (
                <StuckNeed need={n} onOpen={onOpenCard} />
              ) : n.kind === "thread" ? (
                <ThreadNeed
                  need={n}

                  space={spaceName?.(n.thread.spaceId ?? "") ?? ""}
                  onOpen={() => onOpenThread?.(n.id)}
                />
              ) : n.kind === "held" ? (
                <HeldNeed
                  d={n.delivery}

                  name={name}
                  repo={repo}
                  onStarted={setStarted}
                />
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {started && (
        <p role="status" className="px-1 text-[12.5px] text-muted-foreground">
          {started}
        </p>
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
  // Card #520: a wait on an agent that is gone, on a card nobody holds.
  gone: { label: "Gone", variant: "warning" },
  // Card #541: no computer can start its agent. Nothing to decide.
  noAgent: { label: "No agent", variant: "warning" },
  trust: { label: "Trust", variant: "warning" },
  // Card #456 (sa-7): a spawned agent stopped on a permission prompt, and
  // a card whose agents crashed twice.
  permission: { label: "Permission", variant: "warning" },
  crashed: { label: "Crashed", variant: "destructive" },
  // Card #573: another person gave them the card; nothing starts without
  // their yes.
  given: { label: "Given to you", variant: "info" },
  message: { label: "Message", variant: "info" },
  held: { label: "Held", variant: "warning" },
  // Card #1001: a thread waiting on the person, from any space.
  thread: { label: "Thread", variant: "info" },
  // Card #481: waits nothing will end.
  loop: { label: "Stuck", variant: "warning" },
  stuck: { label: "Stuck", variant: "warning" },
} as const

// Row is one item's frame: its kind, where it's from, when, its title.
function Row({
  kind,
  where,
  at,
  title,
  children,
}: {
  kind: keyof typeof kinds
  where: ReactNode
  at: number
  title: ReactNode
  children: ReactNode
}) {
  const k = kinds[kind]
  return (
    <article className="grid min-w-0 gap-2 px-4 py-3 glass-panel">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-muted-foreground">
        <Badge variant={k.variant}>{k.label}</Badge>
        <span className="min-w-0 truncate">{where}</span>
        <span className="ml-auto shrink-0 tabular-nums">
          {at ? <Ago at={new Date(at)} /> : ""}
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
  // said is the outcome's words, or made from fn's answer.
  const run = async <T,>(
    fn: () => Promise<T>,
    said: string | ((r: T) => string)
  ) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      const r = await fn()
      setDone(typeof said === "function" ? said(r) : said)
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
  draft,
}: {
  label: string
  placeholder: string
  send: string
  busy: boolean
  onSend: (text: string) => void
  onCancel: () => void
  // The text kept here instead of in the box (card #644): an instant
  // answer clears it only once saved, and a refused one finds it again.
  draft?: [string, (t: string) => void]
}) {
  const own = useState("")
  const [text, setText] = draft ?? own
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (text.trim()) onSend(text)
  }
  return (
    <form
      className="flex min-w-0 flex-wrap items-start gap-1.5"
      onSubmit={submit}
    >
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
// not another member's question on a shared board, card #264, nor a wait
// on other work, card #481).
function waitsOnYou(b: CardBlocker): boolean {
  return (
    !b.decider &&
    !b.waitsOn &&
    answerable(b.state || BlockerState.Open) &&
    b.state !== BlockerState.Answered
  )
}

// StuckNeed is waits nothing will end (card #481): cards that wait on
// each other, once for the loop ("#392, #396 and #394 wait on each
// other"), or cards waiting on one nobody is on. It says what's stuck
// and opens the cards; dropping a wait or finishing a card is on its
// page.
function StuckNeed({
  need,
  onOpen,
}: {
  need: Extract<Need, { kind: "loop" | "stuck" }>
  onOpen: (id: string) => void
}) {
  const loop = need.kind === "loop"
  const open = loop ? need.ring : [need.end]
  return (
    <Row
      kind={need.kind}
      where={<Where card={need.cards[0]} />}
      at={need.at}

      title={loop ? loopLine(need.ring) : stuckLine(need.end, need.cards)}
    >
      <p className="text-sm break-words text-muted-foreground">
        {loop
          ? "None of them can start until one stops waiting. Drop a wait on one of them, or finish one."
          : `Nobody holds ${refOf(need.end)} and Irori won't hand it out, so the waits on it never end.`}
      </p>
      <span className="flex flex-wrap gap-1.5">
        {open.map((r) => (
          <Button
            key={r.id}
            size="sm"
            variant="outline"
            title={plainLine(r.title)}
            onClick={() => onOpen(r.id)}
          >
            Open {refOf(r)}
          </Button>
        ))}
      </span>
    </Row>
  )
}

// GoneNeed is a card nobody holds whose wait no agent will end (card
// #520): it waits on an agent that is gone, or its gone agent raised an
// outside wait. It sat as an amber line on the card's page, unseen. Hand
// to an agent puts the card back on the Board with what was open said on
// it; Open #N has Stop waiting and the rest.
function GoneNeed({
  need,
  onOpen,
}: {
  need: Extract<Need, { kind: "gone" }>
  onOpen: (id: string) => void
}) {
  const c = need.card
  const g = need.wait
  const { error, done, run } = useAct()
  const [asking, setAsking] = useState(false)
  const name = displayName(g.agent)
  const said = plainLine(g.question)
  const ref = cardRef(c)
  return (
    <Row
      kind="gone"
      where={<Where card={c} />}
      at={need.at}

      title={goneTitle(c, g)}
    >
      <p className="text-sm break-words text-muted-foreground">
        {plainLine(c.title)}
        {said ? `: “${said}”.` : "."}
        {!g.waitedOn && g.on
          ? ` It waits on ${plainLine(g.on)}, and ${name}, who raised it, is gone.`
          : ""}{" "}
        Nobody holds the card, so nothing will move it
        {g.more ? ` (${g.more} more like it)` : ""}.
      </p>
      {g.noHandOn && (
        <p className="text-[12.5px] text-muted-foreground">
          It can't be handed to an agent from here: {plainLine(g.noHandOn)}.
        </p>
      )}
      <span className="flex flex-wrap gap-1.5">
        {!g.noHandOn && (
          <ConfirmDialog
            question={`Hand ${ref} to a new agent? ${name} is gone. Its open items close and are said on the card, and Irori starts an agent for it, or the next free agent takes it.`}
            confirmLabel="Hand to an agent"
            variant="default"
            open={asking}
            onOpenChange={setAsking}
            onConfirm={() =>
              run(
                () => cardsApi.handOn(c.id),
                (r) => r
              )
            }
            onError={() => {}}
            trigger={
              <Button size="sm" onClick={() => setAsking(true)}>
                Hand to an agent
              </Button>
            }
          />
        )}
        <Button size="sm" variant="outline" onClick={() => onOpen(c.id)}>
          Open {ref}
        </Button>
      </span>
      <Said error={error} done={done} />
    </Row>
  )
}

// TrustRow is a card whose agent can't start because its repo isn't
// trusted for agents on that device (card #449, spawned agents §7 "The
// pre-trust flow"): it says what trusting does, and Trust and start
// records it (from this window only) and starts the agent. When the agent
// starts on another device, Ask asks that device to trust it (part 11),
// and there the row says who asked.
function TrustRow({
  need,
  onOpen,
}: {
  need: Extract<Need, { kind: "trust" }>
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
    <Row
      kind="trust"
      where={where}
      at={need.at}

      title={plainLine(c.title)}
    >
      <p className="text-sm break-words text-muted-foreground">
        Can't start an agent: <span className="font-mono">{folder}</span> isn't
        trusted for agents
        {info ? (info.here ? " on this computer" : ` on ${info.device}`) : ""}.
        {info?.what ? ` ${info.what}` : ""}
      </p>
      {info?.here && info.askedBy && (
        <p className="text-[12.5px] text-muted-foreground">
          {info.askedBy} asked you to trust this.
        </p>
      )}
      <span className="flex flex-wrap items-center gap-1.5">
        {info && !info.here ? (
          <Button
            size="sm"
            disabled={act.busy || !!act.done}
            onClick={() =>
              void act.run(
                () => AgentsService.TrustRequest(c.id),
                (d) => `Asked ${d}: confirm it there.`
              )
            }
          >
            {act.busy ? "Asking…" : `Ask ${info.device} to trust it`}
          </Button>
        ) : (
          <Button
            size="sm"
            disabled={act.busy || !!act.done}
            onClick={() =>
              void act.run(
                () =>
                  host
                    ? AgentsService.TrustAndStart(c.id)
                    : sampleTrustAndStart(folder),
                // What happens next (card #717): started, or why not.
                (r) => r?.said || "Trusted."
              )
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

type TrustInfo = {
  folder: string
  device: string
  here: boolean
  what: string
  // The device that asked this one to trust it (part 11).
  askedBy?: string | null
}

// trustFromReason reads `not trusted: <folder> on <device>` (the failed
// spawn's reason) when the host can't be asked.
export function trustFromReason(reason: string): TrustInfo {
  const rest = reason.startsWith(NOT_TRUSTED)
    ? reason.slice(NOT_TRUSTED.length)
    : reason
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
function NoAgentNeed({
  need,
  onOpen,
}: {
  need: Extract<Need, { kind: "noAgent" }>
  onOpen: (id: string) => void
}) {
  const c = need.card
  return (
    <Row
      kind="noAgent"
      where={<Where card={c} />}
      at={need.at}

      title={plainLine(c.title)}
    >
      <p className="text-sm break-words">{c.noAgent}</p>
      <span className="flex flex-wrap gap-1.5">
        <Button size="sm" variant="outline" onClick={() => onOpen(c.id)}>
          Open card
        </Button>
      </span>
    </Row>
  )
}

function ThreadNeed({
  need,
  space,
  onOpen,
}: {
  need: Extract<Need, { kind: "thread" }>
  space: string
  onOpen: () => void
}) {
  const t = need.thread
  return (
    <Row
      kind="thread"
      where={space}
      at={need.at}

      title={plainLine(t.title)}
    >
      {t.noAgent ? (
        <p className="text-sm break-words">{t.noAgent}</p>
      ) : (
        t.lastText && (
          <p className="line-clamp-2 text-sm break-words">
            {plainLine(t.lastText)}
          </p>
        )
      )}
      <span className="flex flex-wrap gap-1.5">
        <Button size="sm" variant="outline" onClick={onOpen}>
          Open thread
        </Button>
      </span>
    </Row>
  )
}

function CardNeed({
  need,
  onOpen,
}: {
  need: Extract<Need, { kind: "check" | "decision" }>
  onOpen: (id: string) => void
}) {
  const c = need.card
  const { detail } = useCard(c.id)
  const spawn = need.kind === "decision" ? spawnNeed(c) : null
  // A permission wait has its own row: the command and its buttons (card
  // #682), never an answer box, since words don't reach the prompt.
  const isPerm = (b: CardBlocker) =>
    !!b.permission || b.question.startsWith(WAITING_PERMISSION)
  const prompts = detail
    ? detail.blockers.filter((b) => waitsOnYou(b) && !!b.permission)
    : []
  const blockers = detail
    ? detail.blockers.filter(
        (b) => waitsOnYou(b) && !(spawn === "permission" && isPerm(b))
      )
    : []
  const who = c.working.find((w) => w.spawned && !w.assigned)
  const where = <Where card={c} />
  return (
    <Row
      kind={spawn ?? need.kind}
      where={where}
      at={need.at}

      title={plainLine(c.title)}
    >
      {spawn === "permission" ? (
        prompts.length > 0 ? (
          prompts.map((b) => (
            <PermissionPrompt
              key={b.id}
              cardId={c.id}
              b={b}
              who={who}
              onOpenCard={onOpen}
              compact
            />
          ))
        ) : (
          <PermissionWait card={c} onOpen={onOpen} />
        )
      ) : null}
      {spawn === "permission" &&
      blockers.length === 0 ? null : blockers.length === 0 ? (
        <>
          {c.blockerQuestion && (
            <p className="text-sm break-words">
              {plainLine(c.blockerQuestion)}
            </p>
          )}
          <span className="flex flex-wrap gap-1.5">
            <Button size="sm" variant="outline" onClick={() => onOpen(c.id)}>
              Open card
            </Button>
          </span>
        </>
      ) : (
        blockers
          .slice(0, 3)
          .map((b) => (
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
        {who ? displayName(who) : "Its agent"} is stopped on a permission prompt
        nobody is there to answer.
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
        <Button
          size="xs"
          variant="outline"
          onClick={() => openSettings("agents")}
        >
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

// BlockerAnswer is an item's answers in a row (Needs you, Notifications).
// A pick or an own-words answer shows at once and saves in the background
// (card #644); a refusal brings the item back, with the words typed still
// in the box, and a toast says why. answerOpens: "Answer…" opens the card
// to answer there instead (Notifications).
export function BlockerAnswer({
  cardId,
  b,
  onOpen,
  answerOpens = false,
}: {
  cardId: string
  b: CardBlocker
  onOpen: () => void
  answerOpens?: boolean
}) {
  const [said, setSaid] = useState("")
  // The same draft as the card page's box for this item; a kept one (a
  // refused answer's words) opens the box it was typed in.
  const words = useDraft(`${cardId}/blocker/${b.id}`)
  const [own, setOwn] = useState(() => !answerOpens && !!words[0])
  const question = plainLine(b.question)
  const send = (option: number, text: string) => {
    const started = instant({
      key: answerKey(cardId, b.id),
      patch: answerPatch(
        cardId,
        b,
        option,
        cardStore.slot(cardId)?.detail?.openBlockers
      ),
      send: () => cardsApi.answer(cardId, b.id, option, text),
      failed: b.doneCheck
        ? "Your answer to the done check wasn't saved"
        : "Your answer wasn't saved",
      onSaved: () => {
        if (option < 0) words[1]("")
      },
      onFailed: () => {
        setSaid("")
        if (option < 0) setOwn(true)
      },
    })
    if (!started) return
    setSaid(
      b.doneCheck
        ? isDone(b, option)
          ? "Marked done."
          : "Sent back to the agent."
        : "Answer sent."
    )
    setOwn(false)
  }
  const replied = b.state === BlockerState.Replied
  return (
    <div className="grid min-w-0 gap-1.5">
      {!b.doneCheck && (
        <div className="grid gap-0.5">
          {/* Irori asks this one in the person's own name (card #573). */}
          {b.question !== GIVEN_QUESTION && <Asker who={b.raisedBy} />}
          <p className="text-sm break-words">{question}</p>
        </div>
      )}
      {b.doneCheck && <EarlierRound blocker={b} compact />}
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
      {said ? null : own ? (
        <OwnWords
          label={`${b.doneCheck ? "Why it isn't done" : "Your answer to"}: ${question}`}
          placeholder={
            b.doneCheck
              ? b.goneAgent
                ? "Say why it isn't done; the next agent on it gets it"
                : "Say why it isn't done; it goes back to the agent"
              : "Answer in your own words"
          }
          send={b.doneCheck ? "Not done" : "Answer"}
          busy={false}
          draft={words}
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
                variant={
                  n === 0 && (o.recommended || b.doneCheck)
                    ? "default"
                    : "outline"
                }
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
          {/* The given question is its two answers: words aren't a yes. */}
          {b.question !== GIVEN_QUESTION && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => (answerOpens ? onOpen() : setOwn(true))}
            >
              {b.doneCheck ? "Not done…" : replied ? "Reply…" : "Answer…"}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onOpen}>
            Open card
          </Button>
        </span>
      )}
      <Said error={null} done={said} />
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

// HeldNeed is a held message that asks the person (asksPerson, card
// #488): their own message no agent in its repo can take yet (Start an
// agent there, since Release has nobody to deliver to), or one from
// another person (Release, Dismiss, or Allow sender where allowing them
// lifts it). Held's History stays in the Held list.
function HeldNeed({
  d,
  name,
  repo,
  onStarted,
}: {
  d: Delivery
  name: (key: string) => string
  repo: (key: string) => string
  // onStarted keeps Start an agent's outcome where the person can read it.
  onStarted: (said: string) => void
}) {
  const act = useAct()
  const unowned = d.hold === HoldReason.HoldUnowned && !d.to
  const mine = d.message.from.key.startsWith("user:")
  const where = plainLine(repo(d.repoKey ?? "")) || "its repo"
  const to = d.to ? plainLine(name(d.to)) || "an agent" : where
  return (
    <Row
      kind="held"
      where={
        <>
          {mine ? "You" : sender(d)} → {to}
        </>
      }
      at={at(d.createdAt)}

      title={holdText(d, { name, repo })}
    >
      <p className="line-clamp-3 text-sm break-words whitespace-pre-line">
        {d.message.text}
      </p>
      {!act.done && (
        <span className="flex flex-wrap gap-1.5">
          {unowned ? (
            mine && (
              <Button
                size="sm"
                disabled={act.busy}
                title={`Makes your message a card in ${where} and gets an agent on it`}
                onClick={() =>
                  void act.run(
                    () => talk.startAgent(d.id),
                    (s) => {
                      const said = startedLine(s)
                      onStarted(said)
                      return said
                    }
                  )
                }
              >
                Start an agent in {where}
              </Button>
            )
          ) : (
            <Button
              size="sm"
              disabled={act.busy}
              onClick={() =>
                void act.run(() => talk.release(d.id), "Released.")
              }
            >
              Release
            </Button>
          )}
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
  // Ones the person's block or allow list holds aren't released in bulk:
  // they chose to stop that sender, so each waits for its own Release (or
  // Allow sender) in the Held list.
  const listed = (d: Delivery) =>
    d.hold === HoldReason.HoldBlocked || d.hold === HoldReason.HoldNotAllowed
  const bulk = deliveries.filter((d) => !listed(d))
  const lists = deliveries.length - bulk.length
  const paused = bulk.filter((d) => d.hold === HoldReason.HoldPaused).length
  const between = bulk.length - paused
  const parts = [
    between ? `${between} between agents` : "",
    paused ? `${paused} while wakes were paused` : "",
    lists ? `${lists} from senders your lists hold back` : "",
  ].filter(Boolean)
  const n = bulk.length
  const releaseAll = async () => {
    if (busy) return
    setBusy(true)
    setSaid("")
    let left = 0
    for (const d of bulk) {
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
        <span className="font-medium text-ink">{deliveries.length} held</span>
        {parts.length > 0 && <>: {parts.join(", ")}</>}
      </span>
      {n > 0 && (
        <Button
          size="xs"
          variant="outline"
          disabled={busy}
          title={
            lists ? "Leaves the ones your block or allow list holds" : undefined
          }
          onClick={() => void releaseAll()}
        >
          {busy ? "Releasing…" : "Release all"}
        </Button>
      )}
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
