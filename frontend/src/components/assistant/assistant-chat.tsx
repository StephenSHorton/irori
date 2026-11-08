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
  ask,
  assistantState,
  liftOver,
  newChat,
  replyText,
  setAssistantOpen,
  stopAsking,
  subscribeAssistant,
  suggestions,
  type Box,
  type ChatEntry,
  type Reply,
} from "@/lib/assistant"
import { composing } from "@/lib/mentions"
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
          narrow={narrow}
          onAsk={(t) => void ask(t)}
          onStop={stopAsking}
          onNew={newChat}
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
      // them room; over it otherwise (they come and go).
      const el = open ? panelRef.current : buttonRef.current
      const top = el?.getBoundingClientRect().top ?? vh
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
  narrow,
  onAsk,
  onStop,
  onNew,
  onClose,
  style,
}: {
  ref?: Ref<HTMLDivElement>
  entries: ChatEntry[]
  reply: Reply | null
  narrow: boolean
  onAsk: (text: string) => void
  onStop: () => void
  onNew: () => void
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
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" })
  }, [last, growing])

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
            Ask about your cards, threads and agents
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
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-3"
        aria-live="polite"
        aria-busy={busy}
      >
        {!entries.length && !reply ? (
          <Starters onPick={send} />
        ) : (
          <div className="flex flex-col gap-4">
            {entries.map((e, i) => (
              <Entry key={i} entry={e} />
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
        plain words. It only reads: it never changes anything. Try:
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

function Entry({ entry }: { entry: ChatEntry }) {
  if (entry.role === "person")
    return (
      <Message align="end">
        <Bubble align="end" className="max-w-[85%]">
          <BubbleContent className="py-1.5 whitespace-pre-wrap">
            {entry.text}
          </BubbleContent>
        </Bubble>
      </Message>
    )
  if (entry.note)
    return (
      <p className="text-[12.5px] text-muted-foreground" data-note="">
        {entry.text}
      </p>
    )
  return (
    <Message>
      <MessageContent className="text-[13.5px] leading-relaxed">
        <RichText bold text={replyText(entry.text)} />
      </MessageContent>
    </Message>
  )
}

// Answering is the reply being written: what it looks at now, then the
// words so far.
function Answering({ reply }: { reply: Reply }) {
  return (
    <Message>
      <MessageContent className="gap-1.5 text-[13.5px] leading-relaxed">
        {(reply.status || !reply.text) && (
          <p className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
            <Loader2Icon
              className="size-3.5 shrink-0 animate-spin motion-reduce:animate-none"
              aria-hidden
            />
            <span className="truncate">
              {reply.status ? `${reply.status}…` : "Thinking…"}
            </span>
          </p>
        )}
        {reply.text && <RichText bold text={replyText(reply.text)} />}
      </MessageContent>
    </Message>
  )
}
