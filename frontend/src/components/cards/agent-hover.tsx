import { faceColor, Identicon } from "@/components/cards/faces"
import {
  createContext,
  useContext,
  useState,
  type FormEvent,
  type ReactElement,
} from "react"
import { Popover } from "@base-ui/react/popover"
import { PreviewCard } from "@base-ui/react/preview-card"

import { cardRef } from "@/components/cards/bits"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import {
  deviceVar,
  displayName,
  type AgentRef,
  type CardSummary,
} from "@/lib/cards"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { setAgentOffer } from "@/lib/agents"
import { agentStatus, throughLine } from "@/lib/agent-status"
import { plainLine } from "@/lib/irori"
import { useMedia } from "@/lib/media"
import { errorText, talk } from "@/lib/talk"
import { LiveRoster } from "@/lib/thread"

// AgentCards is what an agent's hover card (card #104) and a #ref in text
// read: the live cards and the threads, and how to open each (card #144).
// App provides it; outside it (the preview of a single component, a test)
// a face has no hover card and a #ref stays text.
export const AgentCards = createContext<{
  cards: CardSummary[]
  onOpen: (id: string) => void
  threads?: readonly {
    id: string
    short: string
    title: string
    tag?: string
    state?: string
  }[]
  onOpenThread?: (id: string) => void
} | null>(null)

const popupClass =
  "z-50 flex w-72 max-w-[calc(100vw-16px)] origin-(--transform-origin) flex-col gap-2.5 rounded-none p-3 mac:bg-popover/90! text-sm text-popover-foreground glass-panel glass-float outline-hidden duration-100 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95"

// MaxRows is how many cards each list shows before "and N more".
const MaxRows = 6

// AgentHover wraps an agent's face (trigger) in a hover card: its name,
// its device, whether it's live, and the cards it holds, is offered, and
// watches, each one click from its page. Its grids are grid-cols-1
// (minmax(0,1fr)) so a long name, path or title truncates inside the
// card's width instead of widening the column past it (card #148).
export function AgentHover({
  who,
  trigger,
}: {
  who: AgentRef
  trigger: ReactElement
}) {
  const ctx = useContext(AgentCards)
  // A touch screen has no hover (card #227): a tap opens the same card.
  const touch = useMedia("(pointer: coarse)")
  if (!ctx || who.status === "person") return trigger
  if (touch)
    return (
      <Popover.Root>
        <Popover.Trigger
          nativeButton={false}
          render={trigger}
          // A face sits on a card tile, which opens on a tap.
          onClick={(e) => e.stopPropagation()}
        />
        <Popover.Portal>
          <Popover.Positioner
            sideOffset={6}
            collisionPadding={8}
            className="isolate z-50"
          >
            <Popover.Popup
              className={popupClass}
              onClick={(e) => e.stopPropagation()}
            >
              <AgentSummary who={who} cards={ctx.cards} onOpen={ctx.onOpen} />
            </Popover.Popup>
          </Popover.Positioner>
        </Popover.Portal>
      </Popover.Root>
    )
  return (
    <PreviewCard.Root>
      <PreviewCard.Trigger render={trigger} delay={250} closeDelay={150} />
      <PreviewCard.Portal>
        <PreviewCard.Positioner sideOffset={6} className="isolate z-50">
          <PreviewCard.Popup
            className={popupClass}
            // The popup is portalled, but React still bubbles its clicks to
            // the face's ancestors (a card tile opens on click).
            onClick={(e) => e.stopPropagation()}
          >
            <AgentSummary who={who} cards={ctx.cards} onOpen={ctx.onOpen} />
          </PreviewCard.Popup>
        </PreviewCard.Positioner>
      </PreviewCard.Portal>
    </PreviewCard.Root>
  )
}

function AgentSummary({
  who,
  cards,
  onOpen,
}: {
  who: AgentRef
  cards: CardSummary[]
  onOpen: (id: string) => void
}) {
  const gone = who.status === "gone"
  const holding: CardSummary[] = []
  const offered: CardSummary[] = []
  const watching: CardSummary[] = []
  for (const c of cards) {
    const w = c.working.find((r) => r.key === who.key)
    if (w) (w.assigned ? offered : holding).push(c)
    else if (c.watching.some((r) => r.key === who.key)) watching.push(c)
  }
  // Irori started it for one card (card #448): the roster says which.
  const spawnedFor = cards.find((c) =>
    c.working.some((r) => r.key === who.key && r.spawned)
  )
  // Card #453: a terminal agent takes offered work only with the person's
  // "Offer it cards" on; an agent Irori started keeps its own rules.
  const live = useContext(LiveRoster).get(who.key)
  const spawned = !!spawnedFor || !!live?.spawned
  const tint = deviceVar(who.color)
  const device = who.device || who.where
  // One wording for every status (card #489), with how long a job has run.
  const state = agentStatus(who, live?.statusSince)
  return (
    <>
      <div className="flex min-w-0 items-center gap-3">
        {who.status !== "person" && (
          // The agent's identicon, big (0.11.1: still, no animation).
          <span
            className="size-[52px] shrink-0"
            style={{ color: faceColor(who.status, tint) }}
          >
            <Identicon seed={who.key} full />
          </span>
        )}
        <div className="grid min-w-0 grid-cols-1 gap-0.5">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate font-medium">{displayName(who)}</span>
            <span
              className={
                state.tone === "muted"
                  ? "shrink-0 text-xs text-muted-foreground"
                  : state.tone === "warn"
                    ? "shrink-0 text-xs text-warning"
                    : "shrink-0 text-xs text-success"
              }
            >
              {state.text}
            </span>
          </div>
          {device && (
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span
                className="inline-block size-2 shrink-0 bg-primary"
                style={tint ? { background: tint } : undefined}
                aria-hidden
              />
              <span className="truncate">
                {plainLine(device)}
                {who.deviceVia ? ` · ${throughLine(who.deviceVia)}` : ""}
              </span>
            </div>
          )}
          {who.where && who.device && who.where !== who.device && (
            <div className="truncate text-xs text-muted-foreground">
              {plainLine(who.where)}
            </div>
          )}
          {spawnedFor && (
            <div className="truncate text-xs text-muted-foreground">
              Irori started it for {cardRef(spawnedFor)}
            </div>
          )}
        </div>
      </div>
      <CardList label="Holding" cards={holding} onOpen={onOpen} />
      <CardList label="Offered" cards={offered} onOpen={onOpen} />
      <CardList label="Watching" cards={watching} onOpen={onOpen} />
      {!holding.length && !offered.length && !watching.length && (
        <p className="text-xs text-muted-foreground">No open cards.</p>
      )}
      {spawnedFor && !gone && (
        // Card #456 (sa-2): Attach, Logs and End for an agent Irori
        // started; Stop is the form below.
        <SpawnedActions
          who={who}
          cardId={spawnedFor.id}
          onOpenCard={onOpen}
          only={["attach", "logs", "end"]}
        />
      )}
      {!gone && !who.offline && who.live && (
        <>
          {live && !spawned && (
            <OfferCards
              who={who}
              on={!!live.offersCards}
              elsewhere={live.here === false ? device || "another device" : ""}
            />
          )}
          <StopAgent who={who} />
        </>
      )}
    </>
  )
}

// OfferCards is the person's "Offer it cards" on a terminal agent (card
// #453). The switch shows the change at once; roster:changed confirms it,
// and an error puts it back. Each device decides for its own agents (card
// #492): for an agent elsewhere it only shows what that device chose.
function OfferCards({
  who,
  on,
  elsewhere,
}: {
  who: AgentRef
  on: boolean
  elsewhere: string
}) {
  const [want, setWant] = useState<boolean | null>(null)
  const [error, setError] = useState("")
  // The roster caught up: show it again.
  if (want !== null && want === on) setWant(null)
  const id = `offer-${who.key}`
  const set = (v: boolean) => {
    setWant(v)
    setError("")
    setAgentOffer(who.key, v).catch((err: unknown) => {
      setWant(null)
      setError(errorText(err))
    })
  }
  return (
    <div className="grid grid-cols-1 gap-1">
      <div className="flex items-start justify-between gap-3">
        <span className="grid min-w-0 gap-0.5">
          <label htmlFor={id} className="cursor-pointer text-xs font-medium">
            Offer it cards
          </label>
          <span id={`${id}-hint`} className="text-xs text-muted-foreground">
            {elsewhere
              ? `It runs on ${plainLine(elsewhere)}: turn this on or off there.`
              : "Irori hands it unheld cards, asks and threads when it's idle. Off: it works only on what you give it."}
          </span>
        </span>
        <Switch
          id={id}
          size="sm"
          checked={want ?? on}
          disabled={!!elsewhere}
          aria-describedby={`${id}-hint`}
          onCheckedChange={(v) => set(v)}
          className="mt-0.5"
        />
      </div>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

// StopAgent is Stop on an agent (card #207): its running turn is cut and
// what the person writes (or "Stop and wait for me.") is its next prompt.
function StopAgent({ who }: { who: AgentRef }) {
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [line, setLine] = useState<{ ok: boolean; text: string } | null>(null)
  const stop = async (e: FormEvent) => {
    e.preventDefault()
    if (busy) return
    setBusy(true)
    setLine(null)
    try {
      await talk.stop(who.key, text)
      setText("")
      setLine({
        ok: true,
        text: `${displayName(who)} is stopped and reads this first.`,
      })
    } catch (err) {
      setLine({ ok: false, text: errorText(err) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="grid grid-cols-1 gap-1" onSubmit={(e) => void stop(e)}>
      <div className="flex min-w-0 gap-1">
        <Input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Why (optional)"
          aria-label={`Why you stop ${displayName(who)}`}
          className="h-7 min-w-0 text-xs"
          disabled={busy}
        />
        <Button
          type="submit"
          size="sm"
          variant="destructive"
          className="h-7 shrink-0 text-xs"
          disabled={busy}
          title="Cut its running turn now: it reads this first and answers you before anything else"
        >
          Stop
        </Button>
      </div>
      {line && (
        <p
          className={
            line.ok
              ? "text-xs text-muted-foreground"
              : "text-xs text-destructive"
          }
        >
          {line.text}
        </p>
      )}
    </form>
  )
}

function CardList({
  label,
  cards,
  onOpen,
}: {
  label: string
  cards: CardSummary[]
  onOpen: (id: string) => void
}) {
  if (!cards.length) return null
  const more = cards.length - MaxRows
  return (
    <div className="grid grid-cols-1 gap-1">
      <div className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        {label} · {cards.length}
      </div>
      <ul className="grid grid-cols-1 gap-0.5">
        {cards.slice(0, MaxRows).map((c) => (
          <li key={c.id}>
            <button
              type="button"
              className="flex w-full items-baseline gap-1.5 px-1 py-0.5 text-left hover:bg-secondary"
              onClick={() => onOpen(c.id)}
            >
              <span className="shrink-0 font-mono text-xs text-muted-foreground">
                {cardRef(c)}
              </span>
              <span className="truncate">{plainLine(c.title)}</span>
            </button>
          </li>
        ))}
      </ul>
      {more > 0 && (
        <p className="px-1 text-xs text-muted-foreground">and {more} more</p>
      )}
    </div>
  )
}
