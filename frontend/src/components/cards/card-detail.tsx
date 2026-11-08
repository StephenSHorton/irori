import { NotYetButton, WhenLine } from "@/components/cards/order"
import { WeightButton, WeightLine } from "@/components/cards/weight"
import { laterOf } from "@/lib/later"
import { useContext, useState, type FormEvent, type ReactNode } from "react"
import {
  ArrowLeftIcon,
  ChevronDownIcon,
  ExternalLinkIcon,
  PauseIcon,
  PinIcon,
  PinOffIcon,
  PlayIcon,
} from "lucide-react"
import { Loading } from "@/components/ui/loading"
import { MessageRowsShape } from "@/components/common/loading-shapes"

import {
  AttachButton,
  FilesOff,
  PendingFiles,
  useFileAttach,
} from "@/components/attachments"
import { ConfirmButton } from "@/components/common/confirm-button"
import { TitleEdit } from "@/components/common/title-edit"
import { MentionTextarea } from "@/components/common/mention-textarea"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  agentBusy,
  namesLine,
  withLive,
  workingNow,
  runningJobNow,
  BlockerState,
  busy,
  cards,
  openAsk,
  displayName,
  LinkKind,
  NoteItem,
  over,
  overLabel,
  peopleRows,
  percent,
  Holding,
  TaskState,
  type AgentRef,
  type AnswerResult,
  type CardBlocker,
  type CardDetail,
  type CardOption,
  type CardPR,
  type CardReview,
  type CardSubtask,
  type CardSummary,
  type CardWait,
  type Step,
  type ThreadSummary,
  type Told,
  cardStore,
} from "@/lib/cards"
import { emptyLine } from "@/components/talk/threads"
import { ThreadBar } from "./thread-bar"
import {
  parseTime,
  plainLine,
  relativeTime,
  shortPath,
  shortRepo,
} from "@/lib/irori"
import { errorText, talk, type GoneSession } from "@/lib/talk"
import {
  answerable,
  blockerChip,
  blockerTone,
  heardLine,
  isWait,
  LiveRoster,
  StartAgentOn,
  BlockersHere,
  blockerItemId,
} from "@/lib/thread"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import { spawnLine, usageResets } from "@/lib/spawn-line"
import { agentStatus } from "@/lib/agent-status"
import { NotAnswering } from "@/lib/agent-counts"
import { AskedAgain, blockerOutcome } from "@/lib/notifications"
import { PermissionPrompt } from "@/components/cards/permission-prompt"
import { EarlierRound, SentBackSigns } from "./done-rounds"
import { cn } from "@/lib/utils"
import {
  AskLine,
  cardTone,
  HolderLine,
  ProgressBar,
  OwnerChip,
  PrChip,
  ScopeBadge,
  SideBadge,
  ShortRef,
  Spinner,
  StatePill,
  waitLabel,
  Asker,
  cardRef,
} from "./bits"
import { Face, LiveDot } from "./faces"
import { Thread } from "./thread"
import { TerminalSection } from "@/components/term/terminal-section"
import { termsHere } from "@/lib/term"
import {
  TerminalShortcut,
  setTermPanelOpen,
  useTermPanel,
} from "@/lib/term-panel"
import { shortcutKeys } from "@/lib/commands"
import { AttachmentList } from "@/components/attachments"
import type { FileRef } from "@/lib/attachments"
import { LinkChip, RichLine, RichText } from "@/components/common/rich-text"
import { useDraft, useMentionsDraft } from "@/lib/drafts"
import {
  answerKey,
  answerPatch,
  closeKey,
  closePatch,
  instant,
} from "@/lib/optimistic"
import { composing, MentionBoard, useMissed } from "@/lib/mentions"
import { useBoards, usePersonalBoards, type BoardSummary } from "@/lib/boards"
import { previewFlag } from "@/lib/preview"
import { StartPastLimit } from "./start-past-limit"
import {
  MoveBoardButton,
  ShareButton,
  TransferButton,
  WithdrawButton,
} from "./share-dialog"
import { boardsOf, openBoards, Personal } from "@/lib/spaces"
import { cardOwner, sharedBoard } from "@/lib/owners"

// Answered is a note answer the person just sent: the note item leaves the
// card once answered, so the panel keeps it to show who heard.
type Answered = { question: string; answer: string; told: Told; to: AgentRef }

// gist quotes the start of a blocker answer on one summary line: the
// person's answer may be a pasted log (card #265), and the blocker's thread
// shows all of it.
const GistChars = 200
// startable says the person's comment on this card may "Start an agent
// with this" (card #452, spawned-agents §4): their own board's card,
// open, not an ask, and nobody on it. A tag doesn't say shared: a Personal
// board's card has one (card #558).
export function startable(d: CardDetail): boolean {
  if (d.onBoard || d.ask) return false
  if (
    d.state === TaskState.Completed ||
    d.state === TaskState.Canceled ||
    d.state === TaskState.Failed
  )
    return false
  // Nobody at all: a holder shown gone may still be on the card until
  // Irori takes it off, and the Core refuses a card with an agent on it.
  return d.holder.state === Holding.HoldNobody
}

function gist(s: string): string {
  const l = plainLine(s)
  return l.length > GistChars ? `${l.slice(0, GistChars - 1)}…` : l
}

// askedTitle heads what the card is for: a thread's or an open ask's
// question, an agent's own thread's (card #631), or a card's objective.
export function askedTitle(
  d: Pick<CardDetail, "ask">,
  thread?: Pick<ThreadSummary, "openedBy">
): string {
  if (thread) return thread.openedBy ? "Asked" : "You asked"
  return openAsk(d) ? "You asked" : "Objective"
}

// threadAgents are the agents on a card's thread: its holders, and on a
// Talk thread (card #780) the agent that opened it, those told and those
// that answered, once each.
export function threadAgents(
  d: Pick<CardDetail, "working">,
  thread?: Pick<ThreadSummary, "openedBy" | "told" | "answeredBy">
): AgentRef[] {
  if (!thread) return d.working
  const all = [
    ...d.working,
    ...(thread.openedBy ? [thread.openedBy] : []),
    ...thread.told,
    ...thread.answeredBy,
  ]
  return all.filter((a, i) => all.findIndex((b) => b.key === a.key) === i)
}

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
  onOpen,
  thread,
  pane,
  why,
  onResolved,
}: {
  // summary is the card's grid tile, if the grid has it, for the header
  // while the full card loads.
  summary?: CardSummary
  detail: CardDetail | null
  error: string | null
  gone: GoneSession[]
  now: Date
  onBack: () => void
  // onOpen opens another card's page: a subtask's parent, a parent's
  // subtasks.
  onOpen?: (id: string) => void
  // thread is the card's Talk thread when it is one (card #780): its bar
  // takes the ask's Close line, with Resolve and Agents answer now.
  thread?: ThreadSummary
  // pane shows the page in Talk's pane (card #780): Back goes to Talk's
  // list, on a narrow window only.
  pane?: { narrow: boolean }
  // Why nobody is free to take a waiting thread (card #172).
  why?: string
  onResolved?: () => void
}) {
  const [answered, setAnswered] = useState<Answered | null>(null)
  // The done check lifted to the top (card #140) stays there once answered
  // (Not done, or Done), so its told line and state aren't lost to a move.
  const [lifted, setLifted] = useState<{ card: string; id: string } | null>(
    null
  )
  const { boards } = useBoards()
  // The live roster's status over the card's (card #317): the same busy
  // rule Home's Agent progress reads, as of now.
  const live = useContext(LiveRoster)
  // In Talk's pane the list is beside it, so Back shows only on a narrow
  // window, where the thread has the whole screen (card #780).
  const back = pane ? (
    pane.narrow && (
      <Button
        variant="ghost"
        size="sm"
        className="-ml-2 self-start justify-self-start text-muted-foreground"
        onClick={onBack}
        aria-label="Back to Talk"
      >
        <ArrowLeftIcon aria-hidden />
        Talk
      </Button>
    )
  ) : (
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
  const L = pane ? PaneLayout : PageLayout
  // While the card loads (card #598), the header comes from the grid's
  // tile when there is one, and the body is a skeleton in the page's own
  // shape; the same Loading (key "body") then reveals the page in place.
  if (!detail)
    return (
      <MentionBoard.Provider value="">
        <article className="grid gap-3" aria-label="Card">
          {back}
          {summary && <WorkingRow card={withLive(summary, live)} />}
          {summary ? (
            <PageHeader
              card={summary}
              spinning={busy(withLive(summary, live))}
            />
          ) : (
            !error && (
              <Loading loading label="Loading the card">
                <CardHeaderShape />
              </Loading>
            )
          )}
          {error ? (
            <Card className="items-start gap-2 px-5 py-5! text-sm">
              <p className="font-medium text-ink" role="alert">
                Could not open the card
              </p>
              <p className="text-[12.5px] text-muted-foreground">
                {plainLine(error)}
              </p>
              <Button size="sm" variant="outline" onClick={onBack}>
                Back to the cards
              </Button>
            </Card>
          ) : (
            <Loading key="body" loading label="Loading the card">
              <CardBodyShape />
            </Loading>
          )}
        </article>
      </MentionBoard.Provider>
    )
  const p = percent(detail.progress)
  const open = detail.blockers.filter((b) => !b.resolved)
  // A card blocked the old way (a reason, no items) has nothing to pick.
  const legacy = detail.openBlockers > 0 && open.length === 0
  const now_ = withLive(detail, live)
  const spinning = busy(now_)
  // Links already on screen (a PR's, an open blocker's or one of its
  // answers') aren't listed again on the side.
  const shownLinks = new Set(
    detail.prs.map((pr) => `${LinkKind.LinkURL}:${pr.url}`)
  )
  for (const b of open) {
    // A wait's line shows no links: they stay in the side list.
    if (!isWait(b))
      for (const l of b.links ?? []) shownLinks.add(`${l.kind}:${l.ref}`)
    for (const o of b.options)
      if (o.link) shownLinks.add(`${o.link.kind}:${o.link.ref}`)
  }
  const links = detail.links.filter(
    (l) => !shownLinks.has(`${l.kind}:${l.ref}`)
  )
  const files = cardFiles(detail)
  const showNote = !!detail.note && detail.state !== TaskState.InputRequired
  // Ready for you (cards #112, #140): an agent finished and brought proof.
  // Its done check leads the page: screenshots, the proof, then Done and
  // Not done; it isn't listed again under Blockers.
  const readyNow = detail.ready ? open.find((b) => b.doneCheck) : undefined
  if (readyNow && (lifted?.card !== detail.id || lifted.id !== readyNow.id))
    setLifted({ card: detail.id, id: readyNow.id })
  // The blockers this page shows (card #490): thread lines link to them.
  const blockersHere = new Set(detail.blockers.map((b) => b.id))
  const ready =
    readyNow ??
    (lifted?.card === detail.id
      ? detail.blockers.find((b) => b.id === lifted.id)
      : undefined)
  const blockers = detail.blockers.filter((b) => b !== ready)
  const hasObjective = !!(detail.goal || detail.doneWhen || showNote)
  const hasBlockers = blockers.length > 0 || legacy || !!answered
  const asked = askedTitle(detail, thread)
  const about = (
    <AboutCard
      detail={detail}
      live={now_}
      p={p}
      links={links}
      files={files}
      gone={gone}
      now={now}
      spinning={spinning}
      onOpen={onOpen}
    />
  )
  // A thread's own lines in its thread (card #780): no answer yet says
  // why, the reply box is a reply, and a resolved one takes none.
  const threadOn = {
    agents: threadAgents(detail, thread),
    over: thread ? thread.state === "resolved" : over(detail),
  }
  const composer =
    thread?.state === "resolved" ? undefined : (
      <CommentBox
        cardId={detail.id}
        noFiles={!!detail.onBoard}
        reply={!!thread}
      />
    )
  const statusBar = thread ? (
    <ThreadBar
      t={thread}
      ask={detail.ask}
      spawn={detail}
      newest={detail.thread[detail.thread.length - 1]?.id ?? ""}
      why={why}
      onResolved={onResolved}
    />
  ) : (
    <>
      {openAsk(detail) ? (
        <AskStatus detail={detail} onClosed={onBack} />
      ) : (
        <ProgressStrip detail={detail} spinning={spinning} />
      )}
      {!over(detail) && !openAsk(detail) && (
        <CloseBar detail={detail} ready={!!readyNow} />
      )}
      {over(detail) &&
        (detail.sharedTo && !detail.sharedTo.withdrawn ? (
          <SharedToBar detail={detail} onOpen={onOpen} />
        ) : (
          <ReopenBar detail={detail} />
        ))}
    </>
  )
  const header = (
    <PageHeader
      card={detail}
      spinning={spinning}
      onOpen={onOpen}
      small={!!pane}
      thread={!!thread}
      renamable
      extra={
        <>
          <BoardActions
            detail={detail}
            boards={boards}
            onOpen={onOpen}
            onBack={onBack}
          />
          <WeightButton card={detail} />
          <NotYetButton card={detail} />
          {!detail.onBoard && <PauseButton card={detail} />}
          {!detail.onBoard && (
            <DeleteButton detail={detail} onDeleted={onBack} />
          )}
        </>
      }
    />
  )
  const mainCard = (ready || hasObjective || hasBlockers) && (
    <Card className={cn("gap-0 py-0!", L.main)}>
      {ready && (
        <Section
          title={readyNow ? "Ready for you" : "Done check"}
          last={!hasObjective && !hasBlockers}
        >
          <BlockerItem
            cardId={detail.id}
            blocker={ready}
            now={now}
            noFiles={!!detail.onBoard}
            onNoteAnswered={() => {}}
          />
        </Section>
      )}
      {hasObjective && (
        <Section title={asked} last={!hasBlockers}>
          {detail.goal && (
            <RichText
              text={detail.goal}
              mentions={detail.ask?.mentions}
              className="text-sm text-ink-2"
            />
          )}
          {detail.onBoard && (
            <p className="text-[12.5px] text-muted-foreground">
              <span className="font-medium text-ink-2">Visible to:</span>{" "}
              {detail.onBoard.visibleTo.map(plainLine).join(", ")}
            </p>
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
      {hasBlockers && !laterOf(detail) && (
        <Section title="Blockers" last>
          {legacy && (
            <div className="grid gap-1.5 rounded-lg border border-destructive/50 bg-destructive/10 p-3">
              <div className="text-[13.5px] font-semibold text-ink">
                <RichLine text={detail.blockerQuestion} />
              </div>
              <div className="text-xs text-muted-foreground">
                Raised without recommended answers. The agent on it clears it
                when it's sorted.
              </div>
            </div>
          )}
          {answered && !open.some((b) => b.id === NoteItem) && (
            <div className="grid gap-2 rounded-lg border border-border bg-muted/40 p-3 opacity-85">
              <div className="text-[13.5px] font-semibold text-ink">
                <span className="text-success" aria-label="Answered: ">
                  ✓{" "}
                </span>
                {plainLine(answered.question)}
              </div>
              <div className="text-[11.5px] text-muted-foreground">
                You answered: “{gist(answered.answer)}”
              </div>
              <ToldLine told={answered.told} raiser={answered.to} />
            </div>
          )}
          {blockers
            .filter((b) => !isWait(b))
            .map((b) => (
              <BlockerItem
                key={b.id}
                cardId={detail.id}
                blocker={b}
                now={now}
                noFiles={!!detail.onBoard}
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
          {blockers.filter(isWait).map((b) => (
            <WaitItem
              key={b.id}
              cardId={detail.id}
              blocker={b}
              onOpen={onOpen}
            />
          ))}
        </Section>
      )}
    </Card>
  )
  const threadSection = (
    <BlockersHere.Provider value={blockersHere}>
      <StartAgentOn.Provider value={startable(detail) ? detail.id : ""}>
        <Thread
          cardId={detail.id}
          items={detail.thread}
          now={now}
          starting={!!detail.starting}
          earlier={detail.threadEarlier}
          historyTrimmed={detail.historyTrimmed}
          on={threadOn}
          empty={thread ? emptyLine(thread) : undefined}
          composer={composer}
        />
      </StartAgentOn.Provider>
    </BlockersHere.Provider>
  )
  return (
    <MentionBoard.Provider value={detail.onBoard?.id ?? ""}>
      <article
        className={cn("grid gap-3", L.article)}
        aria-label="Card"
        data-card-page={detail.id}
      >
        {back}
        <WorkingRow card={now_} />
        {header}
        <Loading key="body" loading={false} label="Loading the card">
          <div className="grid gap-3">
            {statusBar}
            <div className={cn("grid items-start gap-4", L.grid)}>
              {mainCard}
              <aside
                aria-label="About the card"
                className={cn("min-w-0", L.aside)}
              >
                {about}
              </aside>
              <div className={cn("grid gap-4", L.thread)}>
                <TerminalCard cardId={detail.id} />
                <Card className="gap-0 py-0!">
                  <Section title="Thread" last>
                    {threadSection}
                  </Section>
                </Card>
              </div>
            </div>
          </div>
        </Loading>
      </article>
    </MentionBoard.Provider>
  )
}

// PageLayout is the card page's two columns, by the window's width;
// PaneLayout the same in Talk's pane (card #780), by the pane's own width,
// since Threads' list takes part of the window.
const PageLayout = {
  article: "",
  grid: "min-[900px]:grid-cols-[minmax(0,1fr)_minmax(0,360px)] min-[900px]:grid-rows-[auto_1fr] min-[1600px]:grid-cols-[minmax(0,1fr)_minmax(0,400px)]",
  main: "min-[900px]:col-start-1 min-[900px]:row-start-1",
  aside:
    "min-[900px]:sticky min-[900px]:top-0 min-[900px]:col-start-2 min-[900px]:row-span-2 min-[900px]:row-start-1 min-[900px]:max-h-[calc(100svh-5.5rem)] min-[900px]:overflow-y-auto",
  thread: "min-[900px]:col-start-1 min-[900px]:row-start-2",
}
const PaneLayout = {
  article: "@container",
  grid: "@min-[820px]:grid-cols-[minmax(0,1fr)_minmax(0,340px)] @min-[820px]:grid-rows-[auto_1fr] @min-[1200px]:grid-cols-[minmax(0,1fr)_minmax(0,380px)]",
  main: "@min-[820px]:col-start-1 @min-[820px]:row-start-1",
  aside:
    "@min-[820px]:sticky @min-[820px]:top-0 @min-[820px]:col-start-2 @min-[820px]:row-span-2 @min-[820px]:row-start-1 @min-[820px]:max-h-[calc(100svh-8rem)] @min-[820px]:overflow-y-auto",
  thread: "@min-[820px]:col-start-1 @min-[820px]:row-start-2",
}

// AboutCard is the page's side column: Progress, Who's on it, Review,
// PRs, Links and Files.
function AboutCard({
  detail,
  live,
  p,
  links,
  files,
  gone,
  now,
  spinning,
  onOpen,
}: {
  detail: CardDetail
  live: CardDetail
  p: number | null
  links: CardDetail["links"]
  files: FileRef[]
  gone: GoneSession[]
  now: Date
  spinning: boolean
  onOpen?: (id: string) => void
}) {
  return (
    <Card className="gap-0 py-0!">
      <Section title="Progress">
        {detail.ask && openAsk(detail) ? (
          <AskLine ask={detail.ask} spawn={detail} />
        ) : p === null ? (
          <HolderLine holder={detail.holder} />
        ) : (
          <ProgressBar
            progress={detail.progress}
            label={`${p ?? 0}% complete`}
            tone={cardTone(detail)}
          />
        )}
        <ProgressLine detail={detail} />
        {detail.subtaskList?.length > 0 ? (
          <>
            <Subtasks list={detail.subtaskList} onOpen={onOpen} />
            {detail.steps.length > 0 && (
              <details className="text-xs text-muted-foreground">
                <summary className="cursor-pointer">
                  Steps done before the split
                </summary>
                <Steps
                  steps={detail.steps}
                  spinning={false}
                  people={detail.working}
                />
              </details>
            )}
          </>
        ) : (
          detail.steps.length > 0 && (
            <Steps
              steps={detail.steps}
              spinning={spinning}
              people={detail.working}
            />
          )
        )}
        <WhereLines detail={detail} now={now} />
      </Section>

      <Section
        title="Who's on it"
        last={
          !detail.reviews.length &&
          !detail.prs.length &&
          !links.length &&
          !files.length
        }
      >
        <People detail={live} gone={gone} />
      </Section>

      {detail.reviews.length > 0 && (
        <Section
          title="Review"
          last={!detail.prs.length && !links.length && !files.length}
        >
          <div className="grid gap-2">
            {detail.reviews.map((r) => (
              <ReviewRow key={r.reviewer.key} review={r} now={now} />
            ))}
          </div>
        </Section>
      )}

      {detail.prs.length > 0 && (
        <Section title="Pull requests" last={!links.length && !files.length}>
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
  )
}

// WorkingRow is the card page's top line while an agent works on it (card
// #317): who, with Agent progress's spinner (a still dot under Reduce
// Motion), by the same busy rule as Home, so the two never disagree.
// Nothing while nobody is.
function WorkingRow({ card }: { card: CardSummary }) {
  const live = useContext(LiveRoster)
  const busyNow = workingNow(card)
  // Card #431: between turns, but a background job it started still runs.
  const who = busyNow.length ? busyNow : runningJobNow(card)
  if (!who.length) return <QueueRow card={card} />
  const names = namesLine(who.map(displayName))
  // Card #489: a job reads as on the hover card, with how long one has run.
  const doing = busyNow.length
    ? "working on this card now"
    : agentStatus(
        { status: "shell" },
        who.length === 1 ? live.get(who[0].key)?.statusSince : undefined
      ).text
  return (
    <p
      role="status"
      className="flex min-w-0 items-center gap-2 rounded-lg border border-info/40 bg-info/10 px-3 py-2 text-[13px] text-ink"
    >
      <Spinner className="size-4" />
      <span className="min-w-0 truncate">
        <span className="font-semibold">{names}</span>{" "}
        {who.length > 1 ? "are" : "is"} {doing}
      </span>
    </p>
  )
}

// QueueRow is the card page's top line while nobody is on it and Irori is
// getting it an agent (spawned-agents §10, card #450): starting, its place
// in a device's queue, held on a usage limit until its reset (with Start
// an agent), or why one can't start. Nothing otherwise.
//
// An open ask's line says that an agent is starting or queued (card
// #739), so the page doesn't say it twice; why one can't start, and a
// usage limit's reset with Start an agent, still show here in full.
function QueueRow({ card }: { card: CardSummary }) {
  const line = spawnLine(card)
  if (!line) return null
  if (openAsk(card) && line.tone === "quiet" && !card.usageLimit) return null
  const resets = card.usageLimit ? usageResets(card.usageLimit) : ""
  return (
    <p
      role="status"
      className={cn(
        "flex min-w-0 flex-wrap items-center gap-2 rounded-lg border px-3 py-2 text-[13px] text-ink",
        line.tone === "warn"
          ? "border-warning/40 bg-warning/10"
          : "border-border bg-muted/40"
      )}
    >
      <span className="min-w-0">
        {line.text.charAt(0).toUpperCase() + line.text.slice(1)}
        {resets && ` (${resets})`}
      </span>
      <StartPastLimit id={card.id} state={card} />
    </p>
  )
}

// AskStatus is an open ask's line under the page header: where it stands
// (as on its tile) and Close for the person, with an optional note. Once
// an agent takes it, it talks it through on the thread below and ends it
// as a card, a change to another card, or nothing.
function AskStatus({
  detail,
  onClosed,
}: {
  detail: CardDetail
  onClosed: () => void
}) {
  const [note, setNote] = useDraft(`${detail.id}/close-ask`)
  if (!detail.ask) return null
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 glass-panel">
      <AskLine ask={detail.ask} spawn={detail} className="min-w-48 flex-1" />
      <Input
        value={note}
        onChange={(e) => setNote(e.target.value)}
        maxLength={300}
        placeholder="Why close it (optional)"
        aria-label="Why you close the ask"
        className="h-8 w-56 text-xs"
      />
      <ConfirmButton
        label="Close"
        question="Close this ask? Nothing comes of it, and the agents stop hearing about it."
        confirmLabel="Close it"
        onConfirm={async () => {
          await cards.closeAsk(detail.id, note)
          setNote("")
          onClosed()
        }}
      />
    </div>
  )
}

// CloseBar is a card in play's line for the person to end it by hand
// (card #57): Done or Canceled, with an optional note the agents on it
// hear. A parent ends with its subtasks: Canceled cancels them too, and
// Done waits for them (Coord refuses it while one is open).
// CloseBar closes a card by hand. While it is Ready for you, the done
// check at the top has Done, so only Cancel stays here (card #140).
function CloseBar({ detail, ready }: { detail: CardDetail; ready?: boolean }) {
  const [note, setNote] = useDraft(`${detail.id}/close`)
  // The card closes on the screen at once and saves in the background
  // (card #644); a refusal (an open subtask, say) reopens it, puts the
  // note back and says why in a toast.
  // async only for ConfirmButton: it resolves at once, so the dialog
  // closes without waiting on the save.
  const close = (state: TaskState) => async () => {
    const kept = note
    const started = instant({
      key: closeKey(detail.id),
      patch: closePatch(detail.id, state),
      send: () => cards.close(detail.id, state, kept),
      failed:
        state === TaskState.Canceled
          ? "The card wasn't canceled"
          : "The card wasn't marked done",
      onFailed: () => setNote(kept),
    })
    if (started) setNote("")
  }
  const kids = (detail.subtaskList?.length ?? 0) > 0
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2.5 glass-panel">
      <span className="min-w-40 flex-1 text-[13px] text-muted-foreground">
        Close it by hand
      </span>
      <Input
        value={note}
        onChange={(e) => setNote(e.target.value)}
        maxLength={300}
        placeholder="Why (optional)"
        aria-label="Why you close the card"
        className="h-8 w-56 text-xs"
      />
      {!ready && (
        <ConfirmButton
          label="Done"
          question="Mark this card done? It moves to Closed, and whoever is on it hears."
          confirmLabel="Mark done"
          onConfirm={close(TaskState.Completed)}
        />
      )}
      <ConfirmButton
        label="Cancel card"
        question={
          kids
            ? "Cancel this card and its open subtasks? They move to Closed, and whoever is on them hears."
            : "Cancel this card? It moves to Closed, and whoever is on it hears."
        }
        confirmLabel="Cancel it"
        cancelLabel="Keep it"
        onConfirm={close(TaskState.Canceled)}
      />
    </div>
  )
}

// DeleteButton deletes a card for good (card #57), e.g. one opened by
// mistake: it leaves the Board on every device and agents can't reach it;
// the signed log keeps it. Its subtasks stay, as cards of their own.
function DeleteButton({
  detail,
  onDeleted,
}: {
  detail: CardDetail
  onDeleted: () => void
}) {
  const kids = (detail.subtaskList?.length ?? 0) > 0
  return (
    <ConfirmButton
      label="Delete"
      variant="destructive"
      question={`Delete ${detail.short ? cardRef(detail) : "this card"} for good? It disappears from the Board on all your devices and agents can no longer reach it. It can't be undone.${kids ? " Its subtasks stay, as cards of their own." : ""}`}
      confirmLabel="Delete it"
      onConfirm={async () => {
        await cards.remove(detail.id)
        onDeleted()
      }}
    />
  )
}

// useOnClosedBoard says whether the card sits on a closed board (card
// #690), in a space or in Personal.
function useOnClosedBoard(detail: CardDetail): boolean {
  const { boards } = useBoards()
  const { boards: personal } = usePersonalBoards()
  if (detail.onBoard || detail.board) {
    const space = boards.find((b) => b.boardId === detail.board)
    if (!space) return false
    const id = detail.spaceBoard || space.boardId
    return !!boardsOf(space).find((x) => x.boardId === id)?.closed
  }
  const id = detail.spaceBoard || Personal
  return !!personal.find((x) => x.boardId === id)?.closed
}

// ReopenBar is a finished card's line under the page header: how it ended
// and Reopen, which brings it back with whoever held it. A comment on it
// reopens it too when someone holds it. A parent whose subtasks have all
// ended can't come back (its state follows them, Coord.Reopen refuses it):
// it says to reopen a subtask instead, with no button.
function ReopenBar({ detail }: { detail: CardDetail }) {
  const held = detail.working.length > 0
  const followsSubtasks =
    (detail.subtaskList?.length ?? 0) > 0 && detail.subtaskList.every(over)
  const boardClosed = useOnClosedBoard(detail)
  if (boardClosed) {
    // A closed board is read-only (card #690): its cards stay done, and a
    // comment doesn't reopen them.
    return (
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 glass-panel">
        <span className="min-w-48 flex-1 text-[13px] text-muted-foreground">
          {overLabel(detail)}. Its board is closed: reopen the board to reopen
          this card.
        </span>
      </div>
    )
  }
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 glass-panel">
      <span className="min-w-48 flex-1 text-[13px] text-muted-foreground">
        {overLabel(detail)}
        {followsSubtasks
          ? ". Its subtasks have all ended; reopen one to bring it back."
          : held
            ? ". A comment reopens it for whoever held it."
            : ". Nobody holds it; reopening puts it back on the grid, open."}
      </span>
      {!followsSubtasks && (
        <ConfirmButton
          label="Reopen"
          question="Reopen this card? It goes back on the grid with whoever held it, and they hear."
          confirmLabel="Reopen it"
          onConfirm={() => cards.reopen(detail.id)}
        />
      )}
    </div>
  )
}

// SharedToBar is a private card's line once it was shared to a board
// (v0.5 §5.2): where it went, and a way there. It is closed, and writes to
// it are refused, so it offers no Reopen; Withdraw on the shared card
// brings it back.
function SharedToBar({
  detail,
  onOpen,
}: {
  detail: CardDetail
  onOpen?: (id: string) => void
}) {
  const s = detail.sharedTo
  if (!s) return null
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 glass-panel">
      <span className="min-w-48 flex-1 text-[13px] text-muted-foreground">
        Shared to {plainLine(s.boardName) || "a space"} as{" "}
        <span className="font-mono text-ink">{plainLine(s.ref)}</span>. Work it
        there; this private card is closed.
      </span>
      {onOpen && (
        <Button variant="outline" size="sm" onClick={() => onOpen(s.taskId)}>
          Open {plainLine(s.ref)}
        </Button>
      )}
    </div>
  )
}

// BoardActions are the header's board buttons (v0.5 §8): Share to board on
// a private card still in play, when there is a board to share to (a
// parent takes its open subtasks along; a subtask goes with its parent,
// card #180); Withdraw on a shared card, for its creator or an owner (a
// parent's, which brings its subtasks back too).
function BoardActions({
  detail,
  boards,
  onOpen,
  onBack,
}: {
  detail: CardDetail
  boards: BoardSummary[]
  onOpen?: (id: string) => void
  onBack: () => void
}) {
  if (detail.onBoard) {
    if (over(detail)) return null
    const shared = sharedBoard(detail, boards)
    return (
      <>
        {detail.givenNo && <StartGiven id={detail.id} />}
        {detail.onBoard.canTransfer && shared && (
          <TransferButton
            cardId={detail.id}
            owner={cardOwner(detail, boards)}
            members={shared.members ?? []}
            initialOpen={previewFlag("transfer") === detail.id}
          />
        )}
        {shared && !detail.parent && !openAsk(detail) && (
          <SpaceMove detail={detail} space={shared} />
        )}
        {/* The private card the work carries on in, else back to the grid:
            a withdrawn card is hidden and can't be reopened. */}
        {detail.onBoard.canWithdraw && !detail.parent && (
          <WithdrawButton
            cardId={detail.id}
            visibleTo={detail.onBoard.visibleTo}
            family={(detail.subtasks ?? 0) > 0}
            onWithdrawn={(id) => (id && onOpen ? onOpen(id) : onBack())}
          />
        )}
      </>
    )
  }
  if (over(detail) || detail.sharedTo || openAsk(detail) || detail.parent)
    return null
  return (
    <>
      <PersonalMove detail={detail} />
      <ShareButton
        cardId={detail.id}
        boards={boards}
        initialOpen={previewFlag("share") === detail.id}
        onShared={(id) => onOpen?.(id)}
      />
    </>
  )
}

// SpaceMove is Move on a shared card (card #558): to another of its
// space's boards, never a closed one (card #690).
function SpaceMove({
  detail,
  space,
}: {
  detail: CardDetail
  space: BoardSummary
}) {
  const all = boardsOf(space)
  const current =
    !detail.spaceBoard || detail.spaceBoard === space.boardId
      ? all[0].boardId
      : detail.spaceBoard
  const live = openBoards(all)
  if (!live.some((b) => b.boardId !== current)) return null
  return (
    <MoveBoardButton
      id={detail.id}
      boards={live}
      current={current}
      family={(detail.subtasks ?? 0) > 0}
      initialOpen={previewFlag("move") === detail.id}
    />
  )
}

// PersonalMove is Move on a private card (card #563): to another of
// Personal's boards that isn't closed, once there is one.
function PersonalMove({ detail }: { detail: CardDetail }) {
  const { boards: all } = usePersonalBoards()
  const current = detail.spaceBoard || Personal
  const boards = openBoards(all)
  if (!boards.some((b) => b.boardId !== current)) return null
  return (
    <MoveBoardButton
      id={detail.id}
      boards={boards}
      current={current}
      family={(detail.subtasks ?? 0) > 0}
      initialOpen={previewFlag("move") === detail.id}
    />
  )
}

// PageHeader is the card page's title block: its short ref, state, scope
// and repo over the title, with the spinner while an agent works on it.
// extra are buttons beside Pin (the board's, BoardActions).
// StartGiven takes back a Not now on a card given to you (card #573): the
// question comes back here, or it says which device to press it on.
function StartGiven({ id }: { id: string }) {
  const [error, setError] = useState<string | null>(null)
  return (
    <>
      <Button
        size="sm"
        variant="outline"
        onClick={() => {
          setError(null)
          cards.startAgent(id, "").catch((e: Error) => setError(e.message))
        }}
      >
        Start an agent…
      </Button>
      {error && (
        <span role="alert" className="text-[12.5px] text-destructive">
          {error}
        </span>
      )}
    </>
  )
}

function PageHeader({
  card,
  spinning,
  onOpen,
  extra,
  small,
  thread,
  renamable,
}: {
  card: CardSummary
  spinning?: boolean
  onOpen?: (id: string) => void
  extra?: ReactNode
  // small is its title in Talk's pane (card #780), under the list's size.
  small?: boolean
  // thread: the card is a Talk thread (card #780).
  thread?: boolean
  // The person may rename it here (card #779): the loaded page, not the
  // summary shown while it loads.
  renamable?: boolean
}) {
  return (
    <header className="grid gap-2" data-card-head="">
      <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
        <ShortRef card={card} />
        <StatePill card={card} thread={thread} />
        {spinning && <Spinner />}
        <ScopeBadge card={card} />
        <SideBadge card={card} />
        <OwnerChip card={card} />
        <PrChip card={card} />
        {card.repoKey && (
          <span className="font-mono" title={card.repoKey}>
            {shortRepo(card.repoKey)}
          </span>
        )}
        {/* A shared card pins in your own log, yours alone (card #161). */}
        <PinButton card={card} />
        {extra}
      </div>
      {renamable ? (
        <TitleEdit
          title={card.title}
          save={(t) => cards.rename(card.id, t)}
          label={thread ? "Thread title" : "Card title"}
          editing={previewFlag("rename") === card.id}
          inputClassName="text-base"
          className={cn(
            "leading-tight font-semibold tracking-tight text-balance text-ink",
            small ? "text-lg" : "text-2xl"
          )}
        >
          {plainLine(card.title) || "(untitled)"}
        </TitleEdit>
      ) : (
        <h1
          className={cn(
            "leading-tight font-semibold tracking-tight text-balance text-ink",
            small ? "text-lg" : "text-2xl"
          )}
        >
          {plainLine(card.title) || "(untitled)"}
        </h1>
      )}
      {card.parent && (
        <p className="text-sm text-muted-foreground">
          Part of{" "}
          <button
            type="button"
            className="text-ink underline-offset-2 hover:underline disabled:no-underline"
            disabled={!onOpen}
            onClick={() => card.parent && onOpen?.(card.parent.id)}
          >
            <span className="font-mono">{cardRef(card.parent)}</span>{" "}
            {plainLine(card.parent.title)}
          </button>
        </p>
      )}
      <WhenLine card={card} onOpen={onOpen} />
      <WeightLine card={card} />
      {card.fromThread && (
        <p className="text-sm text-muted-foreground">
          From the thread{" "}
          <button
            type="button"
            className="text-ink underline-offset-2 hover:underline disabled:no-underline"
            disabled={!onOpen}
            onClick={() => card.fromThread && onOpen?.(card.fromThread.id)}
          >
            <span className="font-mono">{cardRef(card.fromThread)}</span>{" "}
            {plainLine(card.fromThread.title)}
          </button>
        </p>
      )}
    </header>
  )
}

// PinButton pins or unpins the card: pinned cards show in their own group
// above the grid, in the person's order (drag to reorder there).
function PinButton({ card }: { card: CardSummary }) {
  // The page reloads when the pin row folds; until then busy holds the
  // button.
  const [busy, setBusy] = useState(false)
  const pinned = !!card.pinned
  return (
    <Button
      variant="ghost"
      size="sm"
      className="ml-auto h-7 gap-1.5 text-xs"
      disabled={busy}
      aria-pressed={pinned}
      title={
        pinned
          ? "Unpin: back to the regular list"
          : "Pin: keep it in the Pinned group at the top"
      }
      onClick={async () => {
        setBusy(true)
        try {
          await cards.pin(card.id, !pinned)
        } finally {
          setBusy(false)
        }
      }}
    >
      {pinned ? <PinOffIcon aria-hidden /> : <PinIcon aria-hidden />}
      {pinned ? "Unpin" : "Pin"}
    </Button>
  )
}

// PauseButton pauses an open card of the person's own board, or resumes it
// (card #207): pausing stops every holder and refuses their tools, except
// Irori's own, until Resume. While paused it says so.
function PauseButton({ card }: { card: CardSummary }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const paused = !!card.paused
  const closed =
    card.state === TaskState.Completed ||
    card.state === TaskState.Failed ||
    card.state === TaskState.Canceled
  if (closed && !paused) return null
  return (
    <>
      <Button
        variant={paused ? "destructive" : "ghost"}
        size="sm"
        className="h-7 gap-1.5 text-xs"
        disabled={busy}
        aria-pressed={paused}
        title={
          paused
            ? "Resume: its holders' tools come back and they carry on"
            : "Pause: stop every holder now; they may only answer you on the card until you resume"
        }
        onClick={async () => {
          setBusy(true)
          setError(null)
          try {
            await cards.pause(card.id, !paused)
          } catch (err) {
            setError(errorText(err))
          } finally {
            setBusy(false)
          }
        }}
      >
        {paused ? <PlayIcon aria-hidden /> : <PauseIcon aria-hidden />}
        {paused ? "Paused · Resume" : "Pause"}
      </Button>
      {error && <span className="text-xs text-destructive">{error}</span>}
    </>
  )
}

// Subtasks is a parent's list of its subtasks: each with its short ref,
// title, state, holder and progress, opening its page on a click.
function Subtasks({
  list,
  onOpen,
}: {
  list: CardSubtask[]
  onOpen?: (id: string) => void
}) {
  return (
    <ul className="grid gap-1" aria-label="Subtasks">
      {list.map((k) => {
        const p = percent(k.progress)
        return (
          <li key={k.id}>
            <button
              type="button"
              className="grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-0.5 px-1.5 py-1 text-left text-[13px] hover:bg-accent disabled:hover:bg-transparent"
              disabled={!onOpen}
              onClick={() => onOpen?.(k.id)}
            >
              <span className="font-mono text-xs text-muted-foreground">
                {cardRef(k)}
              </span>
              <span
                className={cn(
                  "truncate",
                  over(k) ? "text-muted-foreground line-through" : "text-ink"
                )}
              >
                {plainLine(k.title) || "(untitled)"}
              </span>
              <span
                className={cn(
                  "text-[11px] tracking-wide uppercase",
                  k.blocked ? "text-destructive" : "text-muted-foreground"
                )}
              >
                {k.blocked ? "blocked" : k.state.replace("_", " ")}
              </span>
              <span />
              <span className="col-span-2 flex items-center gap-2 text-[11px] text-muted-foreground">
                <HolderLine holder={k.holder} className="text-[11px]" />
                {p !== null && <span className="tabular-nums">{p}%</span>}
              </span>
            </button>
          </li>
        )
      })}
    </ul>
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
        {detail.progress.done} of {detail.progress.total}{" "}
        {detail.subtasks ? "subtasks done" : "steps"} · {p}%
      </span>
    )
  if (detail.subtaskBlocked && !detail.openBlockers)
    parts.push(
      <span key="s" className="text-warning">
        blocked by {plainLine(detail.subtaskBlocked)}
      </span>
    )
  if (detail.openBlockers > 0)
    parts.push(
      <span key="b" className="text-destructive">
        {detail.openBlockers} waiting on a decision
      </span>
    )
  if (detail.waitingOn && !laterOf(detail))
    parts.push(
      <span key="o">{waitLabel(detail).replace(/^Waiting/, "waiting")}</span>
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
    <div className="grid gap-2 px-4 py-3 glass-panel min-[900px]:hidden">
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
// Steps is the card's checklist; a step with an owner (card #86) names
// the agent doing it.
function Steps({
  steps,
  spinning,
  people,
}: {
  steps: Step[]
  spinning: boolean
  people: readonly AgentRef[]
}) {
  const ownerName = (key: string) => {
    const w = people.find((p) => p.key === key)
    return w ? name(w) : key
  }
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
              {s.owner && (
                <small className="ml-1.5 text-xs text-muted-foreground">
                  · {ownerName(s.owner)}
                </small>
              )}
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

// WhereLines is where the card's work is and where its holder left off
// (card #137): the branch, its last commit and whether it's pushed, the
// worktree and the device, then the holder's last line on what's done and
// what's next. So whoever picks the card up after a crash can find it.
function WhereLines({ detail, now }: { detail: CardDetail; now: Date }) {
  const w = detail.where
  const l = detail.leftOff
  if (!w && !l) return null
  return (
    <div className="grid gap-1.5 border-t border-border pt-2.5 text-xs">
      {l && (
        <p className="text-muted-foreground">
          <span className="font-medium text-ink-2">Left off:</span>{" "}
          {plainLine(l.text)}
          <span className="text-muted-foreground/70">
            {" "}
            · {name(l.by)}, {relativeTime(parseTime(l.at), now)}
          </span>
        </p>
      )}
      {w && (
        <div className="grid gap-0.5 text-muted-foreground">
          <p className="flex flex-wrap items-center gap-x-1.5">
            <span className="font-medium text-ink-2">Where:</span>
            <span className="font-mono text-ink-2">
              {plainLine(w.branch) || "detached"}
            </span>
            {w.head && (
              <span className="font-mono">
                @ {plainLine(w.head).slice(0, 7)}
              </span>
            )}
            <Badge variant={w.pushed ? "success" : "warning"}>
              {w.pushed ? "Pushed" : "Not pushed"}
            </Badge>
          </p>
          <p
            className="truncate font-mono text-[11px]"
            title={plainLine(w.worktree)}
          >
            {plainLine(w.worktree)}
          </p>
          <p className="text-[11px]">
            on {plainLine(w.who.where) || "this device"} · {name(w.who)} · seen{" "}
            {relativeTime(parseTime(w.at), now)}
          </p>
        </div>
      )}
    </div>
  )
}

// TerminalCard is the card's live terminal on this device (card #369), in
// the window only: none in a plain browser or on a phone. While the
// bottom terminal panel is open it shows these shells instead (card #734):
// a shell has one view, so the section only points there.
function TerminalCard({ cardId }: { cardId: string }) {
  const panel = useTermPanel()
  if (!termsHere()) return null
  return (
    <Card className="gap-0 py-0!">
      <Section title="Terminal" last>
        {panel.open ? (
          <p className="flex flex-wrap items-center gap-x-2 text-[12.5px] text-muted-foreground">
            This card's shells are in the terminal panel below (
            {shortcutKeys(TerminalShortcut).join("+")}).
            <button
              type="button"
              className="underline underline-offset-2 hover:text-foreground"
              onClick={() => setTermPanelOpen(false)}
            >
              Show them here
            </button>
          </p>
        ) : (
          <TerminalSection cardId={cardId} />
        )}
      </Section>
    </Card>
  )
}

// CardHeaderShape and CardBodyShape are the card page's skeleton (card
// #598): plain markup in the header's and body's shape, with no hooks, for
// Loading to measure. The words only set line lengths.
function CardHeaderShape() {
  return (
    <header className="grid gap-2">
      <span className="text-[12.5px] text-muted-foreground">#598 · irori</span>
      <h1 className="text-2xl font-semibold text-ink">
        Loading states across the app
      </h1>
    </header>
  )
}

function CardBodyShape() {
  return (
    <div className="grid gap-3">
      <div className="grid gap-2 px-4 py-3 glass-panel min-[900px]:hidden">
        <span className="text-[13px] text-ink-2">2 of 5 steps done</span>
        <span skullet-block="" className="block h-1 bg-muted" />
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2.5 glass-panel">
        <span className="min-w-40 flex-1 text-[13px] text-muted-foreground">
          Close it by hand when the work is done.
        </span>
        <span skullet-block="" className="block h-8 w-56 rounded-md" />
      </div>
      <div className="grid items-start gap-4 min-[900px]:grid-cols-[minmax(0,1fr)_minmax(0,360px)] min-[900px]:grid-rows-[auto_1fr] min-[1600px]:grid-cols-[minmax(0,1fr)_minmax(0,400px)]">
        <Card className="gap-0 py-0! min-[900px]:col-start-1 min-[900px]:row-start-1">
          <Section title="Objective" last>
            <p className="text-sm text-ink-2">
              Move the main loading spots onto one wrapper of ours, so a page
              keeps its shape while its data loads and nothing jumps when it
              arrives.
            </p>
            <p className="text-[12.5px] text-muted-foreground">
              Done when: the main spots use it, in both themes.
            </p>
          </Section>
        </Card>
        <aside className="min-w-0 min-[900px]:col-start-2 min-[900px]:row-span-2 min-[900px]:row-start-1">
          <Card className="gap-0 py-0!">
            <Section title="Progress">
              <span skullet-block="" className="block h-1 bg-muted" />
              <p className="text-[12.5px] text-muted-foreground">
                2 of 5 steps · updated 4 min ago
              </p>
              <ul className="grid gap-1.5 text-[12.5px] text-ink-2">
                <li>List every loading state</li>
                <li>Add the wrapper</li>
                <li>Move the main spots onto it</li>
              </ul>
            </Section>
            <Section title="Who's on it" last>
              <span className="flex items-center gap-2 text-[12.5px] text-ink-2">
                <span
                  skullet-block=""
                  className="block size-6 shrink-0 rounded-full"
                />
                sample-heron · working
              </span>
            </Section>
          </Card>
        </aside>
        <div className="grid gap-4 min-[900px]:col-start-1 min-[900px]:row-start-2">
          <Card className="gap-0 py-0!">
            <Section title="Thread" last>
              <MessageRowsShape rows={4} />
            </Section>
          </Card>
        </div>
      </div>
    </div>
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
        // minmax(0,1fr): a wide row (a file chip) shrinks inside the card
        // instead of widening the column past its edge.
        "grid grid-cols-[minmax(0,1fr)] gap-2.5 px-5 py-3.5",
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
// line under it says who heard. "Answer when ready" is the default: a comment
// from the app is asked now (card #1042's picker).
// reply makes it a thread's reply box (card #116): a reply always wakes
// the agents in the thread, so the picker steps aside.
// Files go with it (card #124): the clip, a drop or a pasted screenshot,
// and whoever hears the comment gets them. A shared board's card takes
// none (noFiles): its rows may name only files offered on the board.
export function CommentBox({
  cardId,
  reply = false,
  noFiles = false,
}: {
  cardId: string
  reply?: boolean
  noFiles?: boolean
}) {
  const [text, setText] = useDraft(`${cardId}/comment`)
  const [mentions, setMentions] = useMentionsDraft(`${cardId}/comment`)
  const [missed, setMissed] = useMissed()
  const [how, setHow] = useState<SendHow>("ready")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [heard, setHeard] = useState<{ line: string; error?: string } | null>(
    null
  )
  const attach = useFileAttach(busy || noFiles, noFiles ? SharedNoFiles : "")

  // how (card #1042): "interrupt" is card #207's stop, the holders' running
  // turns cut and this their next prompt; "note" files it without waking.
  const submit = async (e: FormEvent | null) => {
    e?.preventDefault()
    if (busy) return
    if (!text.trim()) {
      setError("Write a comment first.")
      return
    }
    setBusy(true)
    setError(null)
    try {
      const res = await cards.comment(
        cardId,
        text,
        reply ? true : how === "interrupt" ? "stop" : how === "ready",
        mentions,
        attach.files
      )
      setHeard({
        line: heardLine(res.answers, res.watching, res.told.mode ?? ""),
        error: res.told.error,
      })
      setMissed(res.mentioned)
      setText("")
      setMentions([])
      attach.clear()
      setHow("ready")
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form
      className="mt-1 grid gap-1.5"
      onSubmit={(e) => void submit(e)}
      {...attach.drop}
    >
      <div className="flex items-start gap-1.5">
        <MentionTextarea
          card={cardId}
          autoSize
          voice="main"
          value={text}
          mentions={mentions}
          onChange={(v, ms) => {
            setText(v)
            setMentions(ms)
            setHeard(null)
          }}
          onPaste={(e) => {
            if (attach.onPaste(e)) setHeard(null)
          }}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter is a new line (not while an IME is
            // composing, where Enter picks a candidate).
            if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
              e.preventDefault()
              e.currentTarget.form?.requestSubmit()
            }
          }}
          placeholder={reply ? "Reply…" : "Comment on the card…"}
          aria-label={reply ? "Your reply" : "Your comment on the card"}
          className="max-h-48 min-h-9 min-w-0 flex-1 bg-card"
          disabled={busy}
        />
      </div>
      <PendingFiles
        attach={attach}
        disabled={busy}
        label="Files with the comment"
        className="flex flex-wrap gap-2"
      />
      <div className="flex items-center justify-between gap-3">
        <span />
        <span className="flex items-center gap-1">
          {noFiles ? (
            <FilesOff why={SharedNoFiles} />
          ) : (
            <>
              {attach.input}
              <AttachButton attach={attach} disabled={busy} />
            </>
          )}
          <span className="flex items-center">
            <Button
              type="submit"
              size="sm"
              disabled={busy}
              className={reply ? undefined : "rounded-r-none"}
            >
              {busy ? "Sending…" : "Send"}
            </Button>
            {!reply && <SendPicker how={how} onChange={setHow} disabled={busy} />}
          </span>
        </span>
      </div>
      {heard &&
        (heard.error ? (
          <p className="text-xs text-destructive">
            On the card, but not everyone could be told: {heard.error}
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">{heard.line}</p>
        ))}
      {missed && <p className="text-xs text-destructive">{missed}</p>}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  )
}

type SendHow = "ready" | "interrupt" | "note"

const SendHows: { key: SendHow; label: string; why: string }[] = [
  {
    key: "ready",
    label: "Answer when ready",
    why: "Starts the agent if it is idle; a busy one reads it at its next step",
  },
  {
    key: "interrupt",
    label: "Interrupt",
    why: "Cuts the running turn; the agent reads this now",
  },
  {
    key: "note",
    label: "Just a note",
    why: "Files it on the card without starting the agent",
  },
]

// SendPicker is the small picker beside Send (card #1042): how this
// comment reaches the agents. It resets to "Answer when ready" after a send.
function SendPicker({
  how,
  onChange,
  disabled,
}: {
  how: SendHow
  onChange: (h: SendHow) => void
  disabled?: boolean
}) {
  const now = SendHows.find((h) => h.key === how) ?? SendHows[0]
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled}
        aria-label={`How it is sent: ${now.label}`}
        title={now.why}
        className="inline-flex h-8 items-center gap-1 rounded-r-lg border-l border-primary-foreground/25 bg-primary px-2 text-xs font-medium whitespace-nowrap text-primary-foreground outline-none hover:bg-primary/90 focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
      >
        {now.label}
        <ChevronDownIcon className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuRadioGroup
          value={how}
          onValueChange={(v) => onChange(v as SendHow)}
        >
          {SendHows.map((h) => (
            <DropdownMenuRadioItem key={h.key} value={h.key}>
              <span className="grid gap-0.5">
                <span>{h.label}</span>
                <span className="text-xs text-muted-foreground">{h.why}</span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

// SharedNoFiles is why a shared board's card takes no files from the
// window: a board row may name only files offered on that board.
const SharedNoFiles =
  "Files don't go on a shared space's cards from the app yet; an agent can attach them for you."
// NoteNoFiles is why the card's question takes none: its answer is a
// line on the card, not a comment.
const NoteNoFiles =
  "Files go with a comment: add them in the comment box below."

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
  noFiles = false,
  onNoteAnswered,
}: {
  cardId: string
  blocker: CardBlocker
  now: Date
  // A shared board's card: the answer box takes no files (card #195).
  noFiles?: boolean
  // The note item (an input_required card's question) leaves the card once
  // answered; this hands its answer to the panel to show.
  onNoteAnswered: (answer: string, told: Told) => void
}) {
  const [text, setText] = useDraft(`${cardId}/blocker/${b.id}`)
  const [mentions, setMentions] = useMentionsDraft(`${cardId}/blocker/${b.id}`)
  const [missed, setMissed] = useMissed()
  const [note, setNote] = useDraft(`${cardId}/resolve/${b.id}`)
  const [noteMentions, setNoteMentions] = useMentionsDraft(
    `${cardId}/resolve/${b.id}`
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [told, setTold] = useState<Sent | null>(null)
  // Files go with an answer (card #195), not with the card's question.
  const filesOff = noFiles
    ? SharedNoFiles
    : b.id === NoteItem
      ? NoteNoFiles
      : ""
  const attach = useFileAttach(busy || !!filesOff, filesOff)
  // A kept draft opens the form it was typed in.
  const [adding, setAdding] = useState(() => !!text)
  const [resolving, setResolving] = useState(() => !!note)
  // A resolved item folds to its one line; a click opens it.
  const [unfolded, setUnfolded] = useState(false)
  // Set-aside options (card #90) stay folded under "Earlier options".
  const [earlier, setEarlier] = useState(false)
  const raised = parseTime(b.at)
  const r = b.resolved
  const state = b.state || BlockerState.Open
  const tone = blockerTone(state)
  const chip = blockerChip(b)
  const answered =
    state === BlockerState.Answered || state === BlockerState.Replied
  // The person answered in their own words: the options are set aside
  // until the agent offers new ones, and the box continues the talk.
  const aside = !!b.optionsAside
  const replying = aside || answered
  // Answered: the form waits behind "Add to your answer", unless the
  // options are set aside, when the reply box is the way on.
  const showForm =
    answerable(state) && (state !== BlockerState.Answered || adding || aside)

  // heard shows what a saved answer's reply says: who heard it, and
  // whether it closed the item. own is its words, "" for a pick, null for
  // Mark resolved (the check says it all).
  const heard = (res: AnswerResult, own: string | null) => {
    if (b.id === NoteItem && own !== null) onNoteAnswered(own.trim(), res.told)
    setTold(
      own === null
        ? null
        : { told: res.told, resolved: res.resolved, irori: !!res.iroriActs }
    )
    setMissed(res.mentioned)
  }

  // run sends an answer and waits for it: one with files (an upload can
  // fail on its own), the card's question, or Mark resolved (own null).
  const run = async (act: () => Promise<AnswerResult>, own: string | null) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      heard(await act(), own)
      setText("")
      setMentions([])
      attach.clear()
      setNote("")
      setNoteMentions([])
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
      setError(replying ? "Write a reply first." : "Write an answer first.")
      return
    }
    // A picked option carries no text, so no mentions.
    const ms = option < 0 ? mentions : []
    const files = attach.files
    if (files.length || b.id === NoteItem) {
      void run(
        () => cards.answer(cardId, b.id, option, own, true, ms, files),
        own
      )
      return
    }
    // Anything else shows at once and saves in the background (card
    // #644); a refusal takes it back, puts the words back in the box and
    // says why in a toast.
    const kept = { text, mentions }
    const started = instant({
      key: answerKey(cardId, b.id),
      patch: answerPatch(
        cardId,
        b,
        option,
        cardStore.slot(cardId)?.detail?.openBlockers
      ),
      send: () => cards.answer(cardId, b.id, option, own, true, ms),
      failed: b.doneCheck
        ? "Your answer to the done check wasn't saved"
        : "Your answer wasn't saved",
      onSaved: (res) => heard(res, own),
      onFailed: () => {
        if (option >= 0) return
        setText(kept.text)
        setMentions(kept.mentions)
        setAdding(true)
      },
    })
    if (!started) {
      setError("Your last answer to this is still saving; send again then.")
      return
    }
    setError(null)
    setTold(null)
    if (option < 0) {
      setText("")
      setMentions([])
      setAdding(false)
    }
  }

  const submit = (e: FormEvent) => {
    e.preventDefault()
    send(-1, text)
  }

  const markResolved = (e: FormEvent) => {
    e.preventDefault()
    void run(() => cards.resolveBlocker(cardId, b.id, note, noteMentions), null)
  }

  const form = showForm ? (
    <div className="grid gap-1.5">
      {aside && b.options.length > 0 && (
        <div className="grid gap-1">
          <Button
            type="button"
            size="xs"
            variant="ghost"
            aria-expanded={earlier}
            className="justify-self-start px-0 font-normal text-muted-foreground"
            onClick={() => setEarlier((v) => !v)}
          >
            {earlier ? "▾" : "▸"} Earlier options ({b.options.length})
          </Button>
          {earlier && (
            <ul
              className="grid gap-1 border-l border-border pl-3 text-xs text-muted-foreground"
              aria-label={`Earlier options for: ${plainLine(b.question)}`}
            >
              {b.options.map((o, i) => (
                <li key={i}>
                  <RichLine text={o.answer} />
                  {o.recommended && ` (recommended by ${name(o.by)})`}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {!aside && b.options.length > 0 && (
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
      <form
        className="flex items-start gap-1.5"
        onSubmit={submit}
        {...attach.drop}
      >
        <MentionTextarea
          card={cardId}
          autoSize
          voice
          rows={1}
          value={text}
          mentions={mentions}
          onChange={(v, ms) => {
            setText(v)
            setMentions(ms)
          }}
          onPaste={(e) => void attach.onPaste(e)}
          onKeyDown={(e) => {
            // Enter sends, as the one-line field did; Shift+Enter is a
            // new line.
            if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
              e.preventDefault()
              e.currentTarget.form?.requestSubmit()
            }
          }}
          placeholder={
            replying
              ? "Say more, or describe another option"
              : b.doneCheck
                ? b.goneAgent
                  ? "Or say why it isn't done; the next agent on it gets it"
                  : "Or say why it isn't done; it goes back to the agent"
                : b.options.length
                  ? "Or answer in your own words"
                  : "Answer in your own words"
          }
          aria-label={`${replying ? "Your reply on" : "Your answer to"}: ${plainLine(b.question)}`}
          className="max-h-40 min-h-9 min-w-0 flex-1 bg-card py-1.5"
          disabled={busy}
        />
        {filesOff ? (
          <FilesOff why={filesOff} />
        ) : (
          <>
            {attach.input}
            <AttachButton attach={attach} disabled={busy} />
          </>
        )}
        <Button type="submit" disabled={busy}>
          {busy
            ? "Sending…"
            : replying
              ? "Reply"
              : b.doneCheck
                ? "Not done"
                : "Answer"}
        </Button>
      </form>
      <PendingFiles
        attach={attach}
        disabled={busy}
        label="Files with the answer"
        className="flex flex-wrap gap-2"
      />
    </div>
  ) : null

  const by = r
    ? b.doneCheck && r.note === AskedAgain
      ? blockerOutcome(b)
      : r.closed
        ? `closed by ${name(r.by)}${r.answer ? `: “${gist(r.answer)}”` : ""}`
        : r.note
          ? `resolved by ${name(r.by)}: “${gist(r.note)}”`
          : `${name(r.by)} answered: “${gist(r.answer)}”`
    : ""

  if (r && !unfolded && !told && !error)
    return (
      <button
        type="button"
        id={blockerItemId(b.id)}
        onClick={() => setUnfolded(true)}
        aria-expanded={false}
        title="Show the whole item"
        className="flex min-w-0 items-baseline gap-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-left text-[12.5px] opacity-85 outline-none hover:bg-muted/50 focus-visible:ring-1 focus-visible:ring-ring"
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

  // A spawned agent's permission prompt (card #682): what waits and its
  // buttons; no answer box or thread, since words don't reach a prompt.
  if (b.permission)
    return (
      <div
        id={blockerItemId(b.id)}
        className={cn(
          "grid scroll-mt-16 gap-2 rounded-xl border p-3",
          r
            ? "border-border bg-muted/50 opacity-85"
            : "border-destructive/30 bg-destructive/5"
        )}
      >
        <Asker who={b.raisedBy} className="-mb-1.5" />
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
        </div>
        <PermissionPrompt cardId={cardId} b={b} />
        <div className="text-[11.5px] text-muted-foreground">
          <time
            dateTime={raised?.toISOString()}
            title={raised?.toLocaleString()}
          >
            {relativeTime(raised, now)}
          </time>
          {by && ` · ${by}`}
        </div>
      </div>
    )

  return (
    <div
      id={blockerItemId(b.id)}
      className={cn(
        "grid scroll-mt-16 gap-2 rounded-xl border p-3",
        tone === "amber" &&
          (b.doneCheck
            ? "border-success/40 bg-success/5"
            : "border-destructive/30 bg-destructive/5"),
        tone === "grey" && "border-border bg-muted/30",
        (tone === "green" || tone === "closed") &&
          "border-border bg-muted/50 opacity-85"
      )}
    >
      {!b.doneCheck && <Asker who={b.raisedBy} className="-mb-1.5" />}
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
      {b.doneCheck && <EarlierRound blocker={b} />}
      {b.doneCheck && <AttachmentList attachments={b.attachments ?? []} />}
      {b.doneCheck && b.previous && b.context && (
        <span className="text-[11.5px] font-medium text-muted-foreground">
          New proof
        </span>
      )}
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
      {!b.doneCheck && <AttachmentList attachments={b.attachments ?? []} />}
      {!r && b.goneAgent && <GoneRaiser cardId={cardId} who={b.goneAgent} />}
      <div className="text-[11.5px] text-muted-foreground">
        {/* A question names its asker above it (Asker); a done check
            says here who raised it. */}
        {b.doneCheck && <>Raised by {name(b.raisedBy)} · </>}
        <time dateTime={raised?.toISOString()} title={raised?.toLocaleString()}>
          {relativeTime(raised, now)}
        </time>
        {/* Another member decides (card #264); anyone can still answer. */}
        {b.decider && !r && ` · for ${name(b.decider)} to decide`}
        {by && ` · ${by}`}
      </div>
      <SentBackSigns blocker={b} />
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
          {state === BlockerState.Answered && !adding && !aside && (
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
          <MentionTextarea
            card={cardId}
            autoSize
            voice
            rows={1}
            value={note}
            mentions={noteMentions}
            onChange={(v, ms) => {
              setNote(v)
              setNoteMentions(ms)
            }}
            onKeyDown={(e) => {
              // Enter saves, as the one-line field did.
              if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
                e.preventDefault()
                e.currentTarget.form?.requestSubmit()
              }
            }}
            placeholder="A note beside the check (optional)"
            aria-label={`Note on resolving: ${plainLine(b.question)}`}
            className="max-h-40 min-h-9 min-w-0 flex-1 bg-card py-1.5"
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
              setNoteMentions([])
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
            done={!!b.doneCheck}
            irori={told.irori}
          />
        )}
      {missed && <p className="text-xs text-destructive">{missed}</p>}
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
        "grid gap-1 rounded-lg border bg-card",
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
            <Badge
              variant="info"
              className="h-4 rounded-full px-1.5 text-[10.5px]"
            >
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
export function byRecommendation(
  options: CardOption[]
): [CardOption, number][] {
  const all = options.map((o, i): [CardOption, number] => [o, i])
  return [
    ...all.filter(([o]) => o.recommended),
    ...all.filter(([o]) => !o.recommended),
  ]
}

// Sent is what an answer did: who heard, and whether the item resolved.
type Sent = { told: Told; resolved: boolean; irori: boolean }

// ToldLine says who heard the answer and how. For a blocker item (sent
// set), it says whether the answer left it open with the raiser or
// resolved it because nobody is running to act on it. Irori's own asks
// (irori) resolve too, but Irori reads the answer itself (card #531).
function ToldLine({
  told,
  raiser,
  sent,
  done,
  irori,
}: {
  told: Told
  raiser: AgentRef
  sent?: boolean
  // A done check (card #112): resolved means you said Done.
  done?: boolean
  irori?: boolean
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
        {done
          ? "Done: the card is closed."
          : irori
            ? "Sent to Irori: it starts the agent, or asks again."
            : told.to.length
              ? "Resolved: the agent on the card gets your answer."
              : "Resolved: nobody is running to act on it."}
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
  // One row per agent (card #490): a holder that also waits, or a watcher
  // waiting on this card, shows its waits under its one row.
  const rows = peopleRows(detail)
  if (!rows.length)
    return <Note>Nobody yet. An agent that takes the card shows here.</Note>
  return (
    <div className="grid gap-1.5">
      {!rows.some((r) => r.role !== "waiting") && (
        <Note>Nobody yet. An agent that takes the card shows here.</Note>
      )}
      {rows.map(({ who: w, role, stepTitles, waits }) =>
        role === "waiting" ? (
          <WaitRow key={w.key} wait={waits[0]} more={waits.slice(1)} />
        ) : (
          <div
            key={w.key}
            className="grid grid-cols-[22px_minmax(0,1fr)_auto] items-center gap-2 text-[13px]"
          >
            <Face who={w} watching={role === "watching"} noDot />
            <span className="min-w-0 truncate">
              <span className="text-ink">{name(w)}</span>{" "}
              <small className="text-xs text-muted-foreground">
                · {w.where}
              </small>
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
                  {!w.spawned && <Revive who={w} gone={gone} />}
                </>
              ) : role === "working" && stepTitles.length ? (
                <span
                  className="max-w-[14rem] min-w-0 truncate"
                  title={`Owns ${stepTitles.map((t) => `“${t}”`).join(", ")}: on the card until they're done`}
                >
                  on {stepTitles[0]}
                  {stepTitles.length > 1 ? ` +${stepTitles.length - 1}` : ""}
                </span>
              ) : role === "working" ? (
                w.assigned && w.status === NotAnswering ? (
                  // Card #699: not a slow agent about to take it up.
                  <span className="text-warning">isn't answering</span>
                ) : w.assigned && w.offline ? (
                  <span className="text-warning">its device is offline</span>
                ) : w.assigned ? (
                  w.waiting ? (
                    <span
                      className="text-warning"
                      title="The hand-over is still queued or held: it hasn't reached this agent"
                    >
                      waiting for it
                    </span>
                  ) : (
                    <span title={w.handoff || undefined}>
                      offered, not taken up yet
                    </span>
                  )
                ) : (
                  <PersonStatus card={detail} who={w} />
                )
              ) : (
                <span>watching</span>
              )}
            </span>
            {role === "working" &&
              w.spawned &&
              !w.assigned &&
              w.status !== "gone" && (
                // Card #456 (sa-2): an agent Irori started for this card, with
                // its device and Attach, Logs, Stop, End. No Revive: a fresh
                // agent picks the card up from Where and Left off.
                <div className="col-span-3 grid gap-1 pl-[30px]">
                  <span className="text-xs text-muted-foreground">
                    Irori started it for this card
                  </span>
                  {!over(detail) && (
                    <SpawnedActions who={w} cardId={detail.id} />
                  )}
                </div>
              )}
            {role === "working" && w.assigned && w.handoff && (
              <p className="col-span-3 pl-[30px] text-xs break-words whitespace-pre-wrap text-muted-foreground">
                Hand-off note: {w.handoff}
              </p>
            )}
            {waits.map((x) => (
              <p
                key={x.id}
                className="col-span-3 pl-[30px] text-xs text-muted-foreground"
                title="Irori wakes this agent once when it happens"
              >
                {waitWords(x)}
              </p>
            ))}
          </div>
        )
      )}
    </div>
  )
}

// waitWords is one wait as Who's on it says it (card #87).
function waitWords(w: CardWait): string {
  return w.onThis ? `waits ${w.text}` : `waiting on ${w.text}`
}

// WaitItem is a blocker that waits on other work (card #481): a card, a
// lease, an agent or something outside. It ends by itself or by whoever
// does that work, so it is a grey line naming what it waits on, linked
// when it's a card, with nothing to answer and outside the red count.
// Stop waiting closes it, for a wait that will never end (a loop of
// cards waiting on each other, or one nobody is on).
function WaitItem({
  cardId,
  blocker: b,
  onOpen,
}: {
  cardId: string
  blocker: CardBlocker
  onOpen?: (id: string) => void
}) {
  const w = b.waitsOn
  if (!w) return null
  const card = w.taskId
  // The agent it waits on is gone and nobody holds the card (card #485):
  // nothing moves it, so it is amber and can be handed to a new agent.
  const gone = !!b.goneAgent
  return (
    <div
      className={cn(
        "grid grid-cols-[minmax(0,1fr)_auto] items-start gap-x-2 gap-y-0.5 rounded-lg border px-3 py-2",
        gone ? "border-warning/40 bg-warning/10" : "border-border bg-muted/30"
      )}
      aria-label={`Waiting on ${w.label}${gone ? " (gone)" : ""}`}
    >
      <div className="min-w-0 text-[13px] text-muted-foreground">
        Waiting on{" "}
        {card ? (
          <button
            type="button"
            className="text-ink underline-offset-2 hover:underline disabled:no-underline"
            disabled={!onOpen}
            onClick={() => onOpen?.(card)}
          >
            <span className="font-mono">{plainLine(w.label)}</span>
            {w.title && <> {plainLine(w.title)}</>}
          </button>
        ) : (
          <span className="text-ink">
            <RichLine text={w.label} />
          </span>
        )}
        {gone && <span className="font-medium text-warning"> (gone)</span>}
      </div>
      <span className="row-span-2 flex items-start gap-1.5">
        {b.goneAgent && <HandOnButton cardId={cardId} who={b.goneAgent} />}
        <ConfirmButton
          label="Stop waiting"
          variant="outline"
          question={`Stop waiting on ${plainLine(w.label)}? The card no longer waits on it, and whoever is on it hears.`}
          confirmLabel="Stop waiting"
          onConfirm={async () => {
            await cards.resolveBlocker(
              cardId,
              b.id,
              `Stopped waiting on ${plainLine(w.label)}`
            )
          }}
        />
      </span>
      {b.question && (
        <div className="min-w-0 text-xs text-muted-foreground">
          {plainLine(b.question)}
        </div>
      )}
      {b.context && (
        <div className="col-span-2 min-w-0 text-xs text-muted-foreground">
          <RichLine text={b.context} />
        </div>
      )}
      {gone && (
        <div className="col-span-2 min-w-0 text-xs text-muted-foreground">
          Nobody holds this card, so nothing will move it.
        </div>
      )}
    </div>
  )
}

// HandOnButton is "Hand to an agent" (card #485) on an item whose agent is
// gone, on a card nobody holds: the card goes back on the Board with what
// was open said on it, and Irori starts an agent for it.
function HandOnButton({ cardId, who }: { cardId: string; who: AgentRef }) {
  return (
    <ConfirmButton
      label="Hand to an agent"
      variant="outline"
      question={`Hand this card to a new agent? ${displayName(who)} is gone. Its open items close and are said on the card, and Irori starts an agent for it, or the next free agent takes it.`}
      confirmLabel="Hand to an agent"
      onConfirm={() => cards.handOn(cardId)}
    />
  )
}

// GoneRaiser is the amber line under an item whose agent is gone on a card
// nobody holds (card #485): an answer still reaches someone, the agent the
// card goes to next.
function GoneRaiser({ cardId, who }: { cardId: string; who: AgentRef }) {
  return (
    <div
      className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-warning/40 bg-warning/10 px-2.5 py-1.5 text-[12.5px] text-foreground"
      role="status"
    >
      <span className="min-w-0">
        <span className="font-medium text-warning">
          Waiting on {displayName(who)} (gone)
        </span>
        . Nobody holds this card: your answer goes to the agent it goes to next.
      </span>
      <HandOnButton cardId={cardId} who={who} />
    </div>
  )
}

// WaitRow is one wait on the card page (card #87): an agent waiting on
// this card ("waits until done"), or a holder of this card waiting on
// another card or a branch. Irori wakes the agent once when it fires.
function WaitRow({ wait, more = [] }: { wait: CardWait; more?: CardWait[] }) {
  return (
    <div
      className="grid grid-cols-[22px_minmax(0,1fr)_auto] items-center gap-2 text-[13px]"
      title="Irori wakes this agent once when it happens"
    >
      <Face who={wait.who} watching noDot />
      <span className="min-w-0 truncate">
        <span className="text-ink">{name(wait.who)}</span>{" "}
        <small className="text-xs text-muted-foreground">
          · {[wait, ...more].map(waitWords).join("; ")}
        </small>
      </span>
      <span className="text-xs text-muted-foreground">waiting</span>
    </div>
  )
}

// PersonStatus is a holder's word in Who's on it, by the card's spinner
// rule: "working" with its live dot only while the card spins for this
// agent; nothing once the card is over; else its status as everywhere else
// (card #489: "running a job", "not answering"), or "on it".
function PersonStatus({ card, who }: { card: CardDetail; who: AgentRef }) {
  const since = useContext(LiveRoster).get(who.key)?.statusSince
  if (busy(card) && agentBusy(who))
    return (
      <>
        <LiveDot />
        <span>working</span>
      </>
    )
  if (over(card)) return null
  return <span>{agentStatus(who, since).text || "on it"}</span>
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
const reviewStates: Record<
  string,
  { label: string; variant: "info" | "success" | "warning" }
> = {
  requested: { label: "requested", variant: "info" },
  approved: { label: "approved", variant: "success" },
  changes: { label: "changes asked", variant: "warning" },
}

/** One reviewer's review of a private card (#88): its state, who asked,
 *  and the verdict comment once there is one. */
function ReviewRow({ review, now }: { review: CardReview; now: Date }) {
  const st = reviewStates[review.state] ?? reviewStates.requested
  return (
    <div className="grid gap-1.5 rounded-lg border border-border bg-background/40 px-3 py-2.5">
      <div className="grid grid-cols-[22px_minmax(0,1fr)_auto] items-center gap-2 text-[13px]">
        <Face who={review.reviewer} noDot />
        <span className="min-w-0 truncate">
          <span className="text-ink">{name(review.reviewer)}</span>{" "}
          <small className="text-xs text-muted-foreground">
            · asked by {name(review.askedBy)}
            {review.pr ? ` · ${plainLine(review.pr)}` : ""}
          </small>
        </span>
        <Badge
          variant={st.variant}
          className="h-5 px-2 text-[11px]"
          title={relativeTime(parseTime(review.at), now)}
        >
          {st.label}
        </Badge>
      </div>
      {review.text && (
        <p className="text-xs whitespace-pre-wrap text-muted-foreground">
          {review.text}
        </p>
      )}
    </div>
  )
}

function PRRow({ pr }: { pr: CardPR }) {
  const roles: [string, AgentRef[]][] = [
    ["Opened by", pr.openedBy],
    ["Reviewed by", pr.reviewedBy],
    ["Also touched by", pr.touchedBy],
  ]
  const known = roles.filter(([, who]) => who.length > 0)
  return (
    <div className="grid gap-1.5 rounded-lg border border-border bg-background/40 px-3 py-2.5">
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
        <PRStateChip pr={pr} />
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
      {pr.error && (
        <p className="text-xs text-muted-foreground">{plainLine(pr.error)}</p>
      )}
    </div>
  )
}

// PRStateChip is how GitHub last said the PR stands (card #208): Irori
// asks with your own gh while an agent here holds the card.
function PRStateChip({ pr }: { pr: CardPR }) {
  const chip = (
    variant: "success" | "slate" | "warning" | "info",
    text: string
  ) => (
    <Badge variant={variant} className="h-4 shrink-0 px-1.5 text-[10.5px]">
      {text}
    </Badge>
  )
  if (pr.state === "MERGED") return chip("success", "Merged")
  if (pr.state === "CLOSED") return chip("slate", "Closed")
  if (pr.state !== "OPEN") return null
  if (pr.review === "CHANGES_REQUESTED")
    return chip("warning", "Changes requested")
  if (pr.review === "APPROVED") return chip("info", "Open · approved")
  return chip("info", "Open")
}
