import {
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type KeyboardEvent,
  type ReactElement,
  type Ref,
} from "react"
import { createPortal } from "react-dom"
import {
  ArrowUpIcon,
  CheckIcon,
  CornerDownRightIcon,
  Loader2Icon,
  MessageCircleIcon,
  SquareIcon,
  SquarePenIcon,
  XIcon,
} from "lucide-react"

import { AgentCards } from "@/components/cards/agent-hover"
import { RichText } from "@/components/common/rich-text"
import { Bubble, BubbleContent } from "@/components/ui/bubble"
import { Button } from "@/components/ui/button"
import { Message, MessageContent } from "@/components/ui/message"
import { Textarea } from "@/components/ui/textarea"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  answerAsk,
  ask,
  assistantState,
  autoAsk,
  liftOver,
  newChat,
  postDraft,
  replyOn,
  replyText,
  setAssistantOpen,
  splitDraft,
  startFollowing,
  stopAsking,
  subscribeAssistant,
  suggestions,
  type Act,
  type Box,
  type ChatEntry,
  type Posted,
  type Reply,
  type WorkThread,
} from "@/lib/assistant"
import { BlockerState, useCard } from "@/lib/cards"
import { composing } from "@/lib/mentions"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// Ask Irori (card #783): a round button in the bottom-right corner of every
// page that opens into a small chat, and closes back to it keeping the
// chat. It is portalled to <body>, since a glass panel's backdrop-filter
// would trap a fixed child, at z-30: over the page and its header, under
// drawers, dialogs and toasts.
//
// It keeps clear of the bars docked at the bottom (the phone's tabs and
// call bar, a conversation's composer): each carries data-assistant-avoid,
// and the button sits above those that reach into its column. "bar" ones
// (the tabs, the call bar) are kept clear by the open panel too, so the
// way out stays in reach; a composer is covered by the open panel, whose
// own box takes its place.

// The button's size and its gap from the corner (px).
const Size = 48
const Gap = 16
const PanelWidth = 380

// at changes on every move in the window (App's `where`), so the bars are
// measured again on the new page.
export function AssistantChat({ narrow, at }: { narrow: boolean; at: string }) {
  const s = useSyncExternalStore(
    subscribeAssistant,
    assistantState,
    assistantState
  )
  const buttonRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const lift = useLift(narrow, s.open, at, buttonRef, panelRef)
  // Focus goes back to the button when the panel closes by its own close,
  // Escape or a click on the button, not when a card link moved the page.
  const refocus = useRef(false)
  const close = useCallback((back: boolean) => {
    refocus.current = back
    setAssistantOpen(false)
  }, [])
  useEffect(() => {
    if (s.open || !refocus.current) return
    refocus.current = false
    buttonRef.current?.focus({ preventScroll: true })
  }, [s.open])

  // A #ref in a reply opens its card (lib/cards' RichText chips); on a
  // phone the panel then gets out of the way.
  const ctx = useContext(AgentCards)
  const cards = useMemo(
    () =>
      ctx && {
        ...ctx,
        onOpen: (id: string) => {
          if (narrow) close(false)
          ctx.onOpen(id)
        },
        onOpenThread: (id: string) => {
          if (narrow) close(false)
          ;(ctx.onOpenThread ?? ctx.onOpen)(id)
        },
      },
    [ctx, narrow, close]
  )

  // The chat follows the threads it posted for the window's life (card
  // #785), panel open or not: its lines light the button's dot.
  useEffect(() => startFollowing(), [])

  // A window check's ?assistant-ask (lib/assistant.ts autoAsk).
  useEffect(() => {
    autoAsk((p) => cards?.onOpenThread(p.id))
  }, [cards])

  const right = `calc(var(--safe-right) + ${narrow ? 12 : Gap}px)`
  const bottom = (px: number) =>
    `max(calc(var(--safe-bottom) + ${Gap}px), ${px}px)`

  return createPortal(
    s.open ? (
      <AgentCards.Provider value={cards}>
        <AssistantPanel
          ref={panelRef}
          entries={s.entries}
          reply={s.reply}
          work={s.work}
          narrow={narrow}
          onAsk={(t) => void ask(t)}
          onStop={stopAsking}
          onNew={newChat}
          onPost={async (i) => {
            const p = await postDraft(i)
            // As the ask box does, the new thread opens (on a phone the
            // panel gets out of the way).
            cards?.onOpenThread(p.id)
          }}
          onClose={() => close(true)}
          style={
            narrow
              ? {
                  left: "calc(var(--safe-left) + 8px)",
                  right: "calc(var(--safe-right) + 8px)",
                  top: "calc(var(--safe-top) + 8px)",
                  bottom: bottom(lift.panel + 8),
                }
              : {
                  right,
                  bottom: bottom(lift.panel + 12),
                  width: `min(${PanelWidth}px, calc(100vw - 32px))`,
                  height: `min(560px, calc(100svh - ${Math.max(lift.panel + 12, Gap)}px - 72px))`,
                }
          }
        />
      </AgentCards.Provider>
    ) : (
      <AskButton
        ref={buttonRef}
        unread={s.unread || !!s.reply}
        busy={!!s.reply}
        onOpen={() => setAssistantOpen(true)}
        style={{ right, bottom: bottom(lift.button + 12) }}
      />
    ),
    document.body
  )
}

// useLift measures the docked bars under the corner: how high the button
// and the panel must sit above the window's bottom. It also tells the
// toasts (components/ui/toast.tsx, --assistant-clear) how much of the
// corner the chat takes, so they stack above the button.
function useLift(
  narrow: boolean,
  open: boolean,
  at: string,
  buttonRef: React.RefObject<HTMLButtonElement | null>,
  panelRef: React.RefObject<HTMLDivElement | null>
): { button: number; panel: number } {
  const [lift, setLift] = useState({ button: 0, panel: 0 })
  useLayoutEffect(() => {
    let frame = 0
    const seen = new Set<Element>()
    const sizes = new ResizeObserver(() => schedule())
    const measure = () => {
      frame = 0
      const vw = window.innerWidth
      const vh = window.innerHeight
      const all = [...document.querySelectorAll("[data-assistant-avoid]")]
      for (const el of all)
        if (!seen.has(el)) {
          seen.add(el)
          sizes.observe(el)
        }
      const box = (el: Element): Box => el.getBoundingClientRect()
      const bars = all.filter(
        (el) => el.getAttribute("data-assistant-avoid") === "bar"
      )
      const button = liftOver(
        all.map(box),
        { height: vh },
        {
          left: vw - Size - 2 * Gap,
          right: vw,
        }
      )
      const panel = liftOver(
        bars.map(box),
        { height: vh },
        {
          left: narrow ? 0 : vw - PanelWidth - 2 * Gap,
          right: vw,
        }
      )
      setLift((l) =>
        l.button === button && l.panel === panel ? l : { button, panel }
      )
      // Toasts stack above the button, or above the panel when it leaves
      // them room; over it otherwise (they come and go). From the lift
      // just measured, not the element's box: the button slides there
      // over 200 ms, and its box would be where it was.
      const safe =
        parseFloat(
          getComputedStyle(document.documentElement).getPropertyValue(
            "--safe-bottom"
          )
        ) || 0
      const sits = (px: number) => Math.max(safe + Gap, px)
      const height = open
        ? (panelRef.current?.getBoundingClientRect().height ?? 0)
        : Size
      const top = open
        ? narrow
          ? 0
          : vh - sits(panel + 12) - height
        : vh - sits(button + 12) - Size
      const clear = vh - top + 8
      document.documentElement.style.setProperty(
        "--assistant-clear",
        `${clear > 0 && top > 180 ? Math.round(clear) : 0}px`
      )
    }
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(measure)
    }
    measure()
    // Bars come and go with the page (a conversation opening, a call
    // starting), so any change to the page measures again, once a frame.
    const changes = new MutationObserver(schedule)
    changes.observe(document.body, { childList: true, subtree: true })
    window.addEventListener("resize", schedule)
    return () => {
      cancelAnimationFrame(frame)
      changes.disconnect()
      sizes.disconnect()
      window.removeEventListener("resize", schedule)
    }
  }, [narrow, open, at, buttonRef, panelRef])
  useEffect(
    () => () => {
      document.documentElement.style.removeProperty("--assistant-clear")
    },
    []
  )
  return lift
}

// AskButton is the chat closed: a round button with a dot while a reply
// waits to be read.
function AskButton({
  ref,
  unread,
  busy,
  onOpen,
  style,
}: {
  ref: Ref<HTMLButtonElement>
  unread: boolean
  busy: boolean
  onOpen: () => void
  style: CSSProperties
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            ref={ref}
            type="button"
            aria-label={busy ? "Ask Irori (answering)" : "Ask Irori"}
            data-assistant-button=""
            onClick={onOpen}
            style={style}
            className="fixed z-30 flex size-12 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg ring-1 ring-black/10 transition-[bottom,transform] duration-200 outline-none hover:scale-105 focus-visible:ring-3 focus-visible:ring-ring/50 active:scale-95 motion-reduce:transition-none dark:ring-white/10"
          />
        }
      >
        <MessageCircleIcon className="size-5.5" aria-hidden />
        {unread && (
          <span
            aria-hidden
            className={cn(
              "absolute top-0.5 right-0.5 size-3 rounded-full border-2 border-background bg-success",
              busy && "animate-pulse"
            )}
          />
        )}
      </TooltipTrigger>
      <TooltipContent side="left">Ask Irori</TooltipContent>
    </Tooltip>
  )
}

// The chat's composer keeps what was typed while the panel is closed.
let draft = ""

// AssistantPanel is the chat open: a header, the conversation (or a few
// questions to start with), and the box to ask in. Non-modal: the page
// stays usable beside it on a computer. Escape closes it.
export function AssistantPanel({
  ref,
  entries,
  reply,
  work = {},
  narrow,
  onAsk,
  onStop,
  onNew,
  onPost,
  onClose,
  style,
}: {
  ref?: Ref<HTMLDivElement>
  entries: ChatEntry[]
  reply: Reply | null
  // The work of the threads the chat posted (card #785).
  work?: Record<string, WorkThread>
  narrow: boolean
  onAsk: (text: string) => void
  onStop: () => void
  onNew: () => void
  // onPost posts the drafted thread of the message at that index.
  onPost: (index: number) => Promise<void>
  onClose: () => void
  style?: CSSProperties
}) {
  const [text, setText] = useState(draft)
  const input = useRef<HTMLTextAreaElement>(null)
  const end = useRef<HTMLDivElement>(null)
  useEffect(() => {
    input.current?.focus({ preventScroll: true })
  }, [])
  // The newest message stays in view as the reply comes in.
  const last = entries.length
  const growing = reply ? reply.text.length + reply.status.length : -1
  // A draft just posted shows its "Posted as #N" line, and the cards
  // made from it as they come.
  const posted =
    entries.filter((e) => e.posted).length +
    Object.values(work).reduce((n, w) => n + w.cards.length, 0)
  // Each posted thread's #ref, for a reply sent on it.
  const refs = useMemo(() => {
    const m = new Map<string, string>()
    for (const e of entries)
      if (e.posted)
        m.set(e.posted.id, `${e.posted.tag ?? ""}#${e.posted.short}`)
    return m
  }, [entries])
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" })
  }, [last, growing, posted])
  // Card chips fill in after the text lays out, growing it: while the
  // person is at the end, it stays at the end.
  const scroller = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const box = scroller.current
    const inner = list.current
    if (!box || !inner) return
    let atEnd = true
    const onScroll = () => {
      atEnd = box.scrollHeight - box.scrollTop - box.clientHeight < 48
    }
    const grew = new ResizeObserver(() => {
      if (atEnd) box.scrollTop = box.scrollHeight
    })
    grew.observe(inner)
    box.addEventListener("scroll", onScroll, { passive: true })
    return () => {
      grew.disconnect()
      box.removeEventListener("scroll", onScroll)
    }
  }, [last])

  const busy = !!reply
  const send = (t: string) => {
    if (busy || !t.trim()) return
    onAsk(t)
    draft = ""
    setText("")
  }
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Escape" || e.defaultPrevented || composing(e)) return
    e.preventDefault()
    e.stopPropagation()
    onClose()
  }

  return (
    <div
      ref={ref}
      role="dialog"
      aria-label="Ask Irori"
      data-assistant-panel=""
      onKeyDown={onKey}
      style={style}
      className={cn(
        "glass-float fixed z-30 flex flex-col overflow-hidden text-popover-foreground shadow-2xl glass-panel mac:bg-popover/90!",
        "animate-in duration-150 fade-in-0 zoom-in-95 motion-reduce:animate-none",
        narrow ? "origin-bottom" : "origin-bottom-right"
      )}
    >
      <header className="flex shrink-0 items-start gap-2 border-b border-border px-4 pt-3 pb-2.5">
        <div className="grid min-w-0 flex-1 gap-0.5">
          <h2 className="text-sm leading-tight font-semibold">Ask Irori</h2>
          <p className="text-[12px] leading-snug text-muted-foreground">
            Ask about your cards, threads and agents, or for work
          </p>
        </div>
        <IconTip label="New chat">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label="New chat"
            disabled={!entries.length && !busy}
            onClick={() => {
              onNew()
              input.current?.focus()
            }}
          >
            <SquarePenIcon />
          </Button>
        </IconTip>
        <IconTip label="Close">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label="Close"
            onClick={onClose}
          >
            <XIcon />
          </Button>
        </IconTip>
      </header>

      <div
        ref={scroller}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-3"
        aria-live="polite"
        aria-busy={busy}
      >
        {!entries.length && !reply ? (
          <Starters onPick={send} />
        ) : (
          <div ref={list} className="flex flex-col gap-4">
            {entries.map((e, i) => (
              <Entry
                key={i}
                entry={e}
                work={e.posted ? work[e.posted.id] : undefined}
                threadRef={e.update ? (refs.get(e.update) ?? "") : ""}
                onPost={() => onPost(i)}
              />
            ))}
            {reply && <Answering reply={reply} />}
          </div>
        )}
        <div ref={end} />
      </div>

      <form
        className="shrink-0 border-t border-border p-2.5"
        onSubmit={(e) => {
          e.preventDefault()
          send(text)
        }}
      >
        <div className="flex items-end gap-1.5 rounded-xl border border-border bg-background/60 p-1 focus-within:border-ring">
          <Textarea
            ref={input}
            autoSize
            rows={1}
            value={text}
            onChange={(e) => {
              draft = e.target.value
              setText(e.target.value)
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
                e.preventDefault()
                send(text)
              }
            }}
            placeholder="Ask anything…"
            aria-label="Ask Irori"
            className="max-h-28 min-h-8 flex-1 border-transparent bg-transparent px-2 py-1.5 shadow-none focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent"
          />
          {busy ? (
            <Button
              type="button"
              size="icon"
              variant="secondary"
              className="size-8 shrink-0 rounded-lg"
              aria-label="Stop"
              onClick={onStop}
            >
              <SquareIcon className="size-3.5 fill-current" />
            </Button>
          ) : (
            <Button
              type="submit"
              size="icon"
              className="size-8 shrink-0 rounded-lg"
              aria-label="Send"
              disabled={!text.trim()}
            >
              <ArrowUpIcon />
            </Button>
          )}
        </div>
      </form>
    </div>
  )
}

function IconTip({
  label,
  children,
}: {
  label: string
  children: ReactElement
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  )
}

// Starters is the empty chat: what it is for, and a few questions to tap.
function Starters({ onPick }: { onPick: (q: string) => void }) {
  return (
    <div className="flex h-full flex-col justify-end gap-3 pb-1">
      <p className="text-[13px] text-muted-foreground">
        It looks through your boards, cards, threads and agents and answers in
        plain words. Ask it for work and it drafts a thread for you to post; it
        changes nothing by itself. Try:
      </p>
      <div className="flex flex-col items-start gap-1.5">
        {suggestions.map((q) => (
          <Button
            key={q}
            type="button"
            variant="outline"
            size="sm"
            className="h-auto max-w-full rounded-full py-1.5 text-left whitespace-normal"
            onClick={() => onPick(q)}
          >
            {q}
          </Button>
        ))}
      </div>
    </div>
  )
}

function Entry({
  entry,
  work,
  threadRef,
  onPost,
}: {
  entry: ChatEntry
  // A posted draft's thread, as last read.
  work?: WorkThread
  // An update's thread, as the chat cites it.
  threadRef: string
  onPost: () => Promise<void>
}) {
  if (entry.role === "person")
    return (
      <Message align="end">
        <Bubble align="end" className="max-w-[85%]">
          <BubbleContent className="py-1.5 whitespace-pre-wrap">
            {entry.to && (
              <span className="mb-0.5 block text-[11.5px] opacity-75">
                Reply on {entry.to}
              </span>
            )}
            {entry.text}
          </BubbleContent>
        </Bubble>
      </Message>
    )
  if (entry.update) return <Update entry={entry} threadRef={threadRef} />
  if (entry.note)
    return (
      <p className="text-[12.5px] text-muted-foreground" data-note="">
        {entry.text}
      </p>
    )
  // A draft cut short (Stopped, or a reply that never closed it) shows
  // with no Post thread: it isn't the whole thread.
  const { text, draft, partial } = splitDraft(entry.text)
  return (
    <Message>
      <MessageContent className="gap-2 text-[13.5px] leading-relaxed">
        {text && <RichText bold text={replyText(text)} />}
        {draft && (
          <DraftThread
            draft={draft}
            posted={entry.posted}
            work={work}
            onPost={partial ? undefined : onPost}
            cut={partial}
          />
        )}
      </MessageContent>
    </Message>
  )
}

// DraftThread is a thread the assistant drafted when asked for work: the
// text the thread will carry and a Post thread button. One tap posts it
// as the ask box does; once posted it says which thread it became.
function DraftThread({
  draft,
  posted,
  work,
  onPost,
  cut,
}: {
  draft: string
  posted?: Posted
  // What came of it since (card #785): who took it, the cards made.
  work?: WorkThread
  // Without it there is no Post thread (the draft is being written).
  onPost?: () => Promise<void>
  // The draft was cut short.
  cut?: boolean
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const cards = useContext(AgentCards)
  const post = async () => {
    if (busy || posted || !onPost) return
    setBusy(true)
    setError("")
    try {
      await onPost()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div
      className="grid gap-2 rounded-lg border border-border bg-background/60 p-2.5"
      data-assistant-draft=""
    >
      <p className="text-[11.5px] font-medium tracking-wide text-muted-foreground uppercase">
        Draft thread
      </p>
      {/* A long draft scrolls in its box, so Post thread stays in reach. */}
      <div className="max-h-44 overflow-y-auto overscroll-contain text-[13px]">
        <RichText text={draft} />
      </div>
      {posted ? (
        <>
          <p className="flex flex-wrap items-center gap-1.5 text-[12.5px] text-muted-foreground">
            <CheckIcon className="size-3.5 text-success" aria-hidden />
            Posted{posted.short ? ` as #${posted.short}` : ""}.
            {cards && (
              <Button
                type="button"
                variant="link"
                size="sm"
                className="h-auto p-0 text-[12.5px]"
                onClick={() => (cards.onOpenThread ?? cards.onOpen)(posted.id)}
              >
                Open it
              </Button>
            )}
          </p>
          {work && <WorkLine work={work} />}
        </>
      ) : onPost ? (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            size="sm"
            disabled={busy}
            onClick={() => void post()}
          >
            {busy && (
              <Loader2Icon
                className="animate-spin motion-reduce:animate-none"
                aria-hidden
              />
            )}
            Post thread
          </Button>
          {error && (
            <span className="text-[12px] text-destructive" role="alert">
              {error}
            </span>
          )}
        </div>
      ) : cut ? (
        <p className="text-[12.5px] text-muted-foreground">
          The draft was cut short. Ask again for the whole thread.
        </p>
      ) : null}
    </div>
  )
}

// Answering is the reply being written: what it looks at now, then the
// words so far, and a thread it is drafting (posted only once it's done).
function Answering({ reply }: { reply: Reply }) {
  const { text, draft, partial } = splitDraft(reply.text)
  const status = reply.status || (partial ? "Drafting a thread" : "")
  return (
    <Message>
      <MessageContent className="gap-1.5 text-[13.5px] leading-relaxed">
        {(status || !text) && (
          <p className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
            <Loader2Icon
              className="size-3.5 shrink-0 animate-spin motion-reduce:animate-none"
              aria-hidden
            />
            <span className="truncate">
              {status ? `${status}…` : "Thinking…"}
            </span>
          </p>
        )}
        {text && <RichText bold text={replyText(text)} />}
        {draft && <DraftThread draft={draft} />}
      </MessageContent>
    </Message>
  )
}

// WorkLine is what came of a posted thread so far (card #785), under its
// "Posted" line: who took it, then the cards made from it as live chips
// (each shows its state and steps, and opens the card).
function WorkLine({ work }: { work: WorkThread }) {
  if (!work.cards.length)
    return (
      <p className="text-[12.5px] text-muted-foreground" data-assistant-work="">
        {work.closed
          ? "Resolved."
          : work.taker
            ? `${work.taker} is on it.`
            : "Waiting for an agent to take it."}
      </p>
    )
  return (
    <div
      className="grid gap-1 text-[12.5px] text-muted-foreground"
      data-assistant-work=""
    >
      <span>Its {work.cards.length === 1 ? "card" : "cards"}:</span>
      <RichText text={work.cards.map((c) => c.ref).join(" ")} />
    </div>
  )
}

// Update is a line the chat says about a thread it posted (card #785):
// the line, its cards as chips, and what it lets the person do.
function Update({ entry, threadRef }: { entry: ChatEntry; threadRef: string }) {
  return (
    <div
      className="grid min-w-0 gap-1.5 border-l-2 border-primary/40 pl-2.5 text-[13px] leading-relaxed [overflow-wrap:anywhere]"
      data-assistant-update=""
    >
      <RichText bold text={replyText(entry.text)} />
      {entry.act && <ActRow act={entry.act} threadRef={threadRef} />}
    </div>
  )
}

function ActRow({ act, threadRef }: { act: Act; threadRef: string }) {
  if (act.kind === "answer")
    return <AnswerAsk card={act.card} blocker={act.blocker} />
  return <ReplyOnThread thread={act.thread} threadRef={threadRef} to={act.to} />
}

// AnswerAsk is a card's open item answered from the chat, as its card
// page does: its options as buttons (the recommended one first, filled),
// and own words. Once answered it says so; the card has the rest.
function AnswerAsk({ card, blocker }: { card: string; blocker: string }) {
  const { detail } = useCard(card)
  const b = detail?.blockers.find((x) => x.id === blocker)
  const [busy, setBusy] = useState(-2)
  const [error, setError] = useState("")
  const [words, setWords] = useState<string | null>(null)
  if (!b) return null
  if (b.resolved || b.state === BlockerState.Resolved)
    return <Muted>Settled.</Muted>
  if (b.state === BlockerState.Closed) return <Muted>Closed.</Muted>
  if (b.state === BlockerState.Answered)
    return <Muted>Answered. Its agent has it now.</Muted>
  const send = async (i: number, text = "") => {
    if (busy !== -2) return
    setBusy(i)
    setError("")
    try {
      await answerAsk(card, blocker, i, text)
      setWords(null)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(-2)
    }
  }
  const options = b.optionsAside ? [] : b.options
  return (
    <div className="grid gap-1.5" data-assistant-answer="">
      {options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {options.map((o, i) => (
            <Button
              key={i}
              type="button"
              size="sm"
              variant={o.recommended ? "default" : "outline"}
              className="h-auto max-w-full py-1 text-left whitespace-normal"
              disabled={busy !== -2}
              title={o.why || undefined}
              onClick={() => void send(i)}
            >
              {busy === i && (
                <Loader2Icon
                  className="animate-spin motion-reduce:animate-none"
                  aria-hidden
                />
              )}
              {o.answer}
            </Button>
          ))}
        </div>
      )}
      <WordsBox
        open={words !== null}
        text={words ?? ""}
        label={
          b.doneCheck
            ? "Say what's missing"
            : options.length
              ? "Answer in your own words"
              : "Answer"
        }
        busy={busy === -1}
        onOpen={() => setWords("")}
        onText={setWords}
        onCancel={() => setWords(null)}
        onSend={(t) => void send(-1, t)}
      />
      {error && (
        <span className="text-[12px] text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  )
}

// ReplyOnThread sends the person's reply on a thread the chat posted:
// its agents hear it, as from the thread's own box.
function ReplyOnThread({
  thread,
  threadRef,
  to,
}: {
  thread: string
  threadRef: string
  to: string
}) {
  const [words, setWords] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const send = async (t: string) => {
    if (busy) return
    setBusy(true)
    setError("")
    try {
      await replyOn(thread, threadRef, t)
      setWords(null)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="grid gap-1.5" data-assistant-reply="">
      <WordsBox
        open={words !== null}
        text={words ?? ""}
        label={to ? `Reply to ${to}` : "Reply"}
        busy={busy}
        onOpen={() => setWords("")}
        onText={setWords}
        onCancel={() => setWords(null)}
        onSend={(t) => void send(t)}
      />
      {error && (
        <span className="text-[12px] text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  )
}

// WordsBox is a link that opens into a small box for the person's own
// words: Enter sends, Escape closes it (not the chat).
function WordsBox({
  open,
  text,
  label,
  busy,
  onOpen,
  onText,
  onCancel,
  onSend,
}: {
  open: boolean
  text: string
  label: string
  busy: boolean
  onOpen: () => void
  onText: (t: string) => void
  onCancel: () => void
  onSend: (t: string) => void
}) {
  if (!open)
    return (
      <Button
        type="button"
        variant="link"
        size="sm"
        className="h-auto justify-self-start p-0 text-[12.5px]"
        onClick={onOpen}
      >
        <CornerDownRightIcon aria-hidden />
        {label}
      </Button>
    )
  const send = () => {
    if (!busy && text.trim()) onSend(text)
  }
  return (
    <div className="flex items-end gap-1.5 rounded-lg border border-border bg-background/60 p-1 focus-within:border-ring">
      <Textarea
        autoFocus
        autoSize
        rows={1}
        value={text}
        aria-label={label}
        placeholder={`${label}…`}
        onChange={(e) => onText(e.target.value)}
        onKeyDown={(e) => {
          if (composing(e)) return
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault()
            send()
          } else if (e.key === "Escape") {
            e.preventDefault()
            e.stopPropagation()
            onCancel()
          }
        }}
        className="max-h-24 min-h-7 flex-1 border-transparent bg-transparent px-1.5 py-1 text-[13px] shadow-none focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent"
      />
      <Button
        type="button"
        size="icon"
        className="size-7 shrink-0 rounded-md"
        aria-label="Send"
        disabled={busy || !text.trim()}
        onClick={send}
      >
        {busy ? (
          <Loader2Icon
            className="animate-spin motion-reduce:animate-none"
            aria-hidden
          />
        ) : (
          <ArrowUpIcon />
        )}
      </Button>
    </div>
  )
}

function Muted({ children }: { children: string }) {
  return <p className="text-[12.5px] text-muted-foreground">{children}</p>
}
