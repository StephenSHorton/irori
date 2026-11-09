import {
  Fragment,
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
  ExternalLinkIcon,
  CornerDownRightIcon,
  Loader2Icon,
  LocateFixedIcon,
  MessageCircleIcon,
  SquareIcon,
  SquarePenIcon,
  XIcon,
} from "lucide-react"

import { goThere } from "@/components/assistant/go-there"
import { SpotlightLine } from "@/components/assistant/spotlight-line"
import {
  AttachButton,
  AttachmentList,
  PendingFiles,
  useFileAttach,
  withoutSent,
} from "@/components/attachments"
import { AgentCards } from "@/components/cards/agent-hover"
import { RichText } from "@/components/common/rich-text"
import { FromLine } from "./from-line"
import { OwnerDraftLine } from "./owner-draft"
import { AskDraftCard } from "./ask-draft"
import { go as goSection } from "@/lib/nav"
import { Bubble, BubbleContent } from "@/components/ui/bubble"
import { Button } from "@/components/ui/button"
import { Message, MessageContent } from "@/components/ui/message"
import { Textarea } from "@/components/ui/textarea"
import { MicButton } from "@/components/common/mic-button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  answerAsk,
  ask,
  assistantState,
  attachFiles,
  autoAsk,
  liftOver,
  newChat,
  addNote,
  onGo,
  loadOlder,
  postDraft,
  replyText,
  setAssistantOpen,
  setHome,
  spaceOfTag,
  splitReply,
  openDraftPosts,
  startFollowing,
  stopTurn,
  subscribeAssistant,
  suggestions,
  type AssistantFile,
  type AssistantHomeStatus,
  type Box,
  type ChatEntry,
  type Posted,
  type Reply,
  type WorkThread,
} from "@/lib/assistant"
import { OldHistory } from "@/components/assistant/assistant-old-history"
import {
  clearOldHistory,
  startHistoryImport,
  useOldHistory,
} from "@/lib/assistant-import"
import { goLabel, type GoTarget } from "@/lib/goto"
import { stopDivide, takeDrafts, useDivide } from "@/lib/huddle-divide"
import { DivideDrafts } from "./divide-drafts"
import { useBoards } from "@/lib/boards"
import { BlockerState, useCard } from "@/lib/cards"
import { useDraft } from "@/lib/drafts"
import { GIVEN_QUESTION } from "@/lib/needs"
import { byRecommendation } from "@/components/cards/card-detail"
import { composing } from "@/lib/mentions"
import { errorText } from "@/lib/talk"
import { stateDot } from "@/lib/devices"
import { homeView, type HomeView } from "@/lib/assistant-home"
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

  // Take me there (card #784): a reply's go line, by itself once it comes
  // in (lib/assistant.ts finish) or by its button. On a phone the panel
  // gets out of the way first, so the place and its highlight show.
  const goTo = useCallback(
    (t: GoTarget, from?: Element): Promise<string> => {
      if (!ctx) return Promise.resolve("The window isn't ready yet.")
      return goThere(t, {
        // The line starts at the go button tapped, or by itself at the
        // newest reply's (drawn a moment after the reply settles).
        from: from ?? lastGoButton,
        openCard: ctx.onOpen,
        openThread: ctx.onOpenThread ?? ctx.onOpen,
        // Only once the place is found: a go that fails keeps the chat
        // open, saying why.
        before: () => {
          if (narrow && t.kind !== "pr") close(false)
        },
      })
    },
    [ctx, narrow, close]
  )
  useEffect(() => {
    onGo(
      (t) =>
        void goTo(t).then((why) => {
          if (why) addNote(why)
        })
    )
    return () => onGo(null)
  }, [goTo])

  // The chat follows the threads it posted for the window's life (card
  // #785), panel open or not: its lines light the button's dot.
  useEffect(() => startFollowing(), [])

  // Today's history moves into the shared chat on the assistant's computer
  // (card #1105), at load and when the computer changes.
  useEffect(() => startHistoryImport(), [])

  // A window check's ?assistant-ask (lib/assistant.ts autoAsk).
  useEffect(() => {
    autoAsk((p) => cards?.onOpenThread(p.id))
  }, [cards])

  const right = `calc(var(--safe-right) + ${narrow ? 12 : Gap}px)`
  const bottom = (px: number) =>
    `max(calc(var(--safe-bottom) + ${Gap}px), ${px}px)`

  return createPortal(
    <>
      <SpotlightLine />
      {s.open ? (
        <AgentCards.Provider value={cards}>
          <AssistantPanel
            ref={panelRef}
            entries={s.entries}
            streams={s.streams}
            home={s.home}
            more={s.more}
            work={s.work}
            posting={s.posting}
            postFailed={s.postFailed}
            narrow={narrow}
            onAsk={(t, files) => void ask(t, files)}
            onAskAgain={(t, turn) => void ask(t, [], turn)}
            onUsePc={setHome}
            onStop={stopTurn}
            onNew={() => {
              clearOldHistory()
              newChat()
            }}
            onOlder={loadOlder}
            onGo={goTo}
            onPost={async (turn) => {
              const p = await postDraft(turn)
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
          unread={s.unread}
          busy={s.busy}
          onOpen={() => setAssistantOpen(true)}
          style={{ right, bottom: bottom(lift.button + 12) }}
        />
      )}
    </>,
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
            aria-label={
              busy ? "Office Assistant (answering)" : "Office Assistant"
            }
            data-assistant-button=""
            onClick={onOpen}
            style={style}
            className="fixed z-30 flex size-12 items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg ring-1 ring-black/10 transition-[bottom,transform] duration-200 outline-none hover:scale-105 focus-visible:ring-3 focus-visible:ring-ring/50 active:scale-95 motion-reduce:transition-none dark:ring-white/10"
          />
        }
      >
        {/* Filled and white, as the person asked on #782. */}
        <MessageCircleIcon
          className="size-5.5 fill-white text-white"
          aria-hidden
        />
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
      <TooltipContent side="left">Office Assistant</TooltipContent>
    </Tooltip>
  )
}

// entryKey is a message's React key: the same message object keeps it as
// the cap drops older ones (an index would move an open answer box onto
// another card's question). capHistory and the follower keep objects.
const entryKeys = new WeakMap<ChatEntry, number>()
let nextEntryKey = 0
function entryKey(e: ChatEntry): number {
  let k = entryKeys.get(e)
  if (k === undefined) {
    k = nextEntryKey++
    entryKeys.set(e, k)
  }
  return k
}

// The chat's composer keeps what was typed, and the files waiting, while
// the panel is closed. uploading is files on their way to the store: it
// outlives a panel closed meanwhile, and says how it went ("" or why not).
let draft = ""
let draftFiles: File[] = []
let uploading: Promise<string> | null = null
// The files that upload is carrying: only these leave the box when it ends.
let uploadingFiles: File[] = []
// Why the last upload failed, for a panel closed while it did.
let uploadFailed = ""

// AssistantPanel is the chat open: a header, the conversation (or a few
// questions to start with), and the box to ask in. Non-modal: the page
// stays usable beside it on a computer. Escape closes it.
export function AssistantPanel({
  ref,
  entries,
  streams = {},
  home = null,
  more = false,
  work = {},
  posting,
  postFailed,
  narrow,
  onAsk,
  onAskAgain,
  onUsePc,
  onStop,
  onNew,
  onOlder,
  onGo,
  onPost,
  onClose,
  style,
}: {
  ref?: Ref<HTMLDivElement>
  entries: ChatEntry[]
  // The replies being written, by turn (only where the turn runs).
  streams?: Readonly<Record<string, Reply>>
  // Where the assistant runs (assistant_home_status).
  home?: AssistantHomeStatus | null
  // Older entries remain beyond those shown.
  more?: boolean
  // The work of the threads the chat posted (card #785).
  work?: Record<string, WorkThread>
  // The turns whose draft a tap is posting, and why a post failed.
  posting?: ReadonlySet<string>
  postFailed?: Readonly<Record<string, string>>
  narrow: boolean
  onAsk: (text: string, files?: AssistantFile[]) => void
  // Ask again: the same words, as a new question that repeats this turn.
  onAskAgain?: (text: string, turn: string) => void
  // Use <computer> instead: the person's pick, "" or why it failed.
  onUsePc?: (device: string) => Promise<string>
  // Stop one question.
  onStop: (turn: string) => void
  onNew: () => void
  onOlder?: () => void
  // onGo goes where a reply's go line says (card #784), its line from the
  // button tapped: "" or why not.
  onGo: (t: GoTarget, from?: Element) => Promise<string>
  // onPost posts the drafted thread of the reply to that turn.
  onPost: (turn: string) => Promise<void>
  onClose: () => void
  style?: CSSProperties
}) {
  const [text, setText] = useState(draft)
  // Files on their way: the box counts as busy until this settles. A
  // promise, so an upload over before the effect below runs still counts.
  const [going, setGoing] = useState(uploading)
  const sending = going !== null
  const [sendError, setSendError] = useState(uploadFailed)
  const view = homeView(home)
  // Now, for "seen 2h ago": the status is read again every 5 s.
  const input = useRef<HTMLTextAreaElement>(null)
  // Where dictation left the caret, applied once the new text is in.
  const pendingCaret = useRef<number | null>(null)
  useLayoutEffect(() => {
    const at = pendingCaret.current
    pendingCaret.current = null
    if (at !== null) input.current?.setSelectionRange(at, at)
  }, [text])
  const end = useRef<HTMLDivElement>(null)
  // Answer this is hidden while the question has an open draft.
  const draftedPosts = openDraftPosts(entries)
  const firstPending = entries.find((e) => e.pending)?.turn
  useEffect(() => {
    input.current?.focus({ preventScroll: true })
  }, [])
  // The newest message stays in view as the reply comes in.
  const last = entries.length
  const growing = Object.values(streams).reduce(
    (n, r) => n + r.text.length + r.status.length,
    0
  )
  // A draft just posted shows its "Posted as #N" line, and the cards
  // made from it as they come.
  const posted =
    entries.filter((e) => e.posted).length +
    Object.values(work).reduce((n, w) => n + w.cards.length, 0)
  // Whether the person is at the end. Lines the follower adds (card #785)
  // come at any time: they keep the end in view only while the person is
  // there, never pulling them away from an older line they read or answer.
  const atEnd = useRef(true)
  const newest = entries[entries.length - 1]
  const mine = newest?.role === "person"
  useEffect(() => {
    if (atEnd.current || mine) end.current?.scrollIntoView({ block: "end" })
  }, [last, growing, posted, mine])
  // Card chips fill in after the text lays out, growing it: while the
  // person is at the end, it stays at the end.
  const scroller = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const box = scroller.current
    if (!box) return
    const onScroll = () => {
      atEnd.current = box.scrollHeight - box.scrollTop - box.clientHeight < 48
    }
    box.addEventListener("scroll", onScroll, { passive: true })
    return () => box.removeEventListener("scroll", onScroll)
  }, [])
  useEffect(() => {
    const box = scroller.current
    const inner = list.current
    if (!box || !inner) return
    const grew = new ResizeObserver(() => {
      if (atEnd.current) box.scrollTop = box.scrollHeight
    })
    grew.observe(inner)
    return () => grew.disconnect()
  }, [last])

  // The composer stays on while questions wait or run: only files on
  // their way hold it.
  const old = useOldHistory()
  const attach = useFileAttach(
    sending,
    "Add files once the last ones are sent.",
    draftFiles
  )
  const pending = attach.files
  const discardFiles = attach.discard
  useEffect(() => {
    draftFiles = pending
  }, [pending])
  useEffect(() => {
    if (!going) return
    let here = true
    void going.then((why) => {
      if (!here) return
      setGoing(null)
      setSendError(why)
      setText(draft)
      if (!why) discardFiles(uploadingFiles)
    })
    return () => {
      here = false
    }
  }, [going, discardFiles])
  // The box empties only once the files are stored: a failure keeps the
  // words and the files, and says why.
  const upload = (t: string, files: File[]) => {
    uploadingFiles = files
    uploading = attachFiles(files).then(
      (refs) => {
        uploading = null
        draft = ""
        draftFiles = withoutSent(draftFiles, files)
        onAsk(t, refs)
        return ""
      },
      (err: unknown) => {
        uploading = null
        uploadFailed = errorText(err)
        return uploadFailed
      }
    )
    setGoing(uploading)
  }
  const send = (t: string, files: File[] = []) => {
    if (uploading || attach.reading || !view.composer) return
    if (!t.trim() && !files.length) return
    uploadFailed = ""
    setSendError("")
    if (files.length) return upload(t, files)
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
      aria-label="Office Assistant"
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
          <h2 className="text-sm leading-tight font-semibold">
            Office Assistant
          </h2>
          <p className="text-[12px] leading-snug text-muted-foreground">
            Ask about your cards, threads and agents, or for work
          </p>
          {view.where && (
            <p
              className="flex items-center gap-1.5 text-[12px] leading-snug text-muted-foreground"
              data-assistant-where=""
            >
              <span
                className={cn(
                  "size-2 shrink-0 rounded-full",
                  stateDot(view.dot)
                )}
                data-dot={view.dot}
                aria-hidden
              />
              <span className="truncate">{view.where}</span>
            </p>
          )}
        </div>
        <IconTip label="New chat">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label="New chat"
            disabled={sending || (!entries.length && !old.length)}
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

      <DivideBar />

      <div
        ref={scroller}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-3"
        aria-live="polite"
        aria-busy={Object.keys(streams).length > 0}
      >
        <OldHistory entries={old} narrow={narrow} />
        {!entries.length ? (
          <Starters
            onPick={(q) => send(q, pending)}
            disabled={sending || attach.reading}
          />
        ) : (
          <div ref={list} className="flex flex-col gap-4">
            {more && onOlder && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="self-center text-[12.5px] text-muted-foreground"
                onClick={onOlder}
              >
                Earlier
              </Button>
            )}
            {entries.map((e) => (
              <Fragment key={e.id ?? entryKey(e)}>
                <Entry
                  entry={e}
                  drafted={draftedPosts.has(e.from?.postId ?? "")}
                  onTalk={(conv) => {
                    if (narrow) onClose()
                    goSection({ kind: "talk", id: conv })
                  }}
                  onGo={onGo}
                  work={e.posted ? work[e.posted.id] : undefined}
                  onPost={() => onPost(e.turn ?? "")}
                  posting={!!e.turn && !!posting?.has(e.turn)}
                  failed={e.turn ? postFailed?.[e.turn] : undefined}
                  onAskAgain={
                    e.retry && e.turn && onAskAgain
                      ? () => {
                          const q = entries.find(
                            (x) => x.turn === e.turn && x.role === "person"
                          )
                          if (q) onAskAgain(q.text, e.turn as string)
                        }
                      : undefined
                  }
                />
                {e.pending && e.turn && (
                  <Waiting
                    turn={e.turn}
                    stream={streams[e.turn]}
                    home={home}
                    oldest={e.turn === firstPending}
                    onStop={() => onStop(e.turn as string)}
                  />
                )}
              </Fragment>
            ))}
          </div>
        )}
        <div ref={end} />
      </div>

      <HomeLine view={view} onUsePc={onUsePc} />

      <form
        className="grid shrink-0 gap-1.5 border-t border-border p-2.5"
        onSubmit={(e) => {
          e.preventDefault()
          send(text, pending)
        }}
        {...attach.drop}
      >
        <div className="flex items-end gap-1.5 rounded-xl border border-border bg-background/60 p-1 focus-within:border-ring">
          <Textarea
            ref={input}
            autoSize
            rows={1}
            value={text}
            readOnly={sending || !view.composer}
            onChange={(e) => {
              draft = e.target.value
              setText(e.target.value)
            }}
            onPaste={(e) => {
              if (attach.onPaste(e)) setSendError("")
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !composing(e)) {
                e.preventDefault()
                send(text, pending)
              }
            }}
            placeholder={view.composer ? "Ask anything…" : view.placeholder}
            aria-label="Ask the Office Assistant"
            className="max-h-28 min-h-8 flex-1 border-transparent bg-transparent px-2 py-1.5 shadow-none focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent"
          />
          {attach.input}
          {/* Centred on the 32 px send button the row bottom-aligns to. */}
          <div className="flex h-8 shrink-0 items-center">
            <AttachButton attach={attach} disabled={sending} />
            <MicButton
              target={input}
              value={text}
              onText={(t, at) => {
                // Same as typing: the module draft and the text both take it.
                draft = t
                pendingCaret.current = at
                setText(t)
              }}
            />
          </div>
          {
            <Button
              type="submit"
              size="icon"
              className="size-8 shrink-0 rounded-lg"
              aria-label={
                sending
                  ? "Sending"
                  : attach.reading
                    ? "Reading the picture"
                    : "Send"
              }
              disabled={
                sending ||
                attach.reading ||
                !view.composer ||
                (!text.trim() && !pending.length)
              }
            >
              {sending || attach.reading ? (
                <Loader2Icon
                  className="animate-spin motion-reduce:animate-none"
                  aria-hidden
                />
              ) : (
                <ArrowUpIcon />
              )}
            </Button>
          }
        </div>
        <PendingFiles
          attach={attach}
          disabled={sending}
          label="Files with the message"
          className="flex max-h-44 flex-wrap gap-2 overflow-y-auto px-1"
        />
        {sendError && (
          <p className="px-1 text-xs text-destructive" role="alert">
            {sendError}
          </p>
        )}
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
function Starters({
  onPick,
  disabled,
}: {
  onPick: (q: string) => void
  disabled: boolean
}) {
  return (
    <div className="flex h-full flex-col justify-end gap-3 pb-1">
      <p className="text-[13px] text-muted-foreground">
        It looks through your boards, cards, threads and agents and answers in
        plain words. Ask it for work and it posts a thread for your agents, or
        waits if you say to check with you first; it changes nothing else. Try:
      </p>
      <div className="flex flex-col items-start gap-1.5">
        {suggestions.map((q) => (
          <Button
            key={q}
            type="button"
            variant="outline"
            size="sm"
            className="h-auto max-w-full rounded-full py-1.5 text-left whitespace-normal"
            disabled={disabled}
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
  drafted,
  onGo,
  work,
  onPost,
  posting,
  failed,
  onAskAgain,
  onTalk,
}: {
  entry: ChatEntry
  // The owner's-OK draft for this line's question is shown (card #1064).
  drafted: boolean
  // Open in Talk, on a labelled line (card #1062).
  onTalk: (conv: string) => void
  // Its draft is being posted by a tap.
  posting: boolean
  // Why the post failed.
  failed?: string
  // Ask again, on a note of a turn that was skipped, failed or stopped.
  onAskAgain?: () => void
  onGo: (t: GoTarget, from?: Element) => Promise<string>
  // A posted draft's thread, as last read.
  work?: WorkThread
  onPost: () => Promise<void>
}) {
  if (entry.role === "person")
    return (
      <Message align="end">
        {/* A chip's Save and Open need the width, or its name is squeezed
          out on a phone. */}
        <Bubble
          align="end"
          className={entry.files?.length ? "max-w-full" : "max-w-[85%]"}
        >
          {!!entry.files?.length && (
            <AttachmentList
              attachments={entry.files}
              className="[&>div]:justify-end"
            />
          )}
          {(entry.text || entry.to) && (
            <BubbleContent className="py-1.5 whitespace-pre-wrap">
              {entry.to && (
                <span className="mb-0.5 block text-[11.5px] opacity-75">
                  Reply on {entry.to}
                </span>
              )}
              {entry.text}
            </BubbleContent>
          )}
        </Bubble>
      </Message>
    )
  // Someone else's words: before anything that parses a reply.
  if (entry.from)
    return <FromLine entry={entry} onTalk={onTalk} drafted={drafted} />
  if (entry.ownerDraft) return <OwnerDraftLine entry={entry} />
  if (entry.update) return <Update entry={entry} />
  if (entry.changed)
    return (
      <div className="text-[12.5px] font-medium" data-changed="">
        <RichText text={entry.text} />
      </div>
    )
  if (entry.note)
    return (
      <p className="text-[12.5px] text-muted-foreground" data-note="">
        {entry.text}
        {onAskAgain && (
          <Button
            type="button"
            variant="link"
            size="sm"
            className="ml-1 h-auto p-0 text-[12.5px]"
            onClick={onAskAgain}
          >
            Ask again
          </Button>
        )}
      </p>
    )
  // A thread posting by itself shows that, not Post thread (card #988).
  // A draft cut short (Stopped, or a failure before it closed) shows with
  // no Post thread: it isn't the whole thread. A reply that ended by itself
  // without closing it has the whole draft.
  const { text, draft, partial, go, cards, settle, ask, space } = splitReply(
    entry.text,
    false,
    entry.cutOff
  )
  return (
    <Message>
      <MessageContent className="gap-2 text-[13.5px] leading-relaxed">
        {text && <RichText bold text={replyText(text)} />}
        {(cards.length > 0 || settle) && (
          <DivideDrafts cards={cards} settle={settle} />
        )}
        {go && <GoButton target={go} onGo={onGo} />}
        {ask && (
          <AskDraftCard
            key={`${ask.person}|${ask.space}|${ask.text}`}
            ask={entry.cutOff ? { ...ask, partial: true } : ask}
          />
        )}
        {entry.looked && <LookedAt label={entry.looked} />}
        {draft && (
          <DraftThread
            draft={draft}
            posted={entry.posted}
            posting={posting}
            failed={failed}
            work={work}
            onPost={partial ? undefined : onPost}
            cut={partial}
            space={space}
          />
        )}
      </MessageContent>
    </Message>
  )
}

// DivideBar says the chat is on a huddle (card #1054): each question it asks
// carries the huddle, and nothing is made until a draft's button is tapped.
function DivideBar() {
  const divide = useDivide()
  if (!divide) return null
  return (
    <div
      className="flex shrink-0 items-center gap-2 border-b border-border bg-secondary/40 px-4 py-1.5 text-[12px]"
      data-divide-bar=""
    >
      <span className="grid min-w-0 flex-1 gap-0.5">
        <span className="truncate">
          Dividing <span className="font-medium">{divide.title}</span>
        </span>
        <span className="text-muted-foreground">
          Nothing is made until you tap.
        </span>
      </span>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Stop dividing this huddle"
        onClick={stopDivide}
      >
        <XIcon />
      </Button>
    </div>
  )
}

// LookedAt is the small line under a reply that read the screen (card
// #1009): it names what the assistant looked at. The text itself stays with
// the question's row.
function LookedAt({ label }: { label: string }) {
  return (
    <p
      className="truncate text-[12px] text-muted-foreground"
      data-assistant-looked=""
    >
      Looked at: {label || "your screen"}
    </p>
  )
}

// lastGoButton is the newest reply's go button, if the chat shows one.
function lastGoButton(): Element | null {
  const all = document.querySelectorAll(
    "[data-assistant-panel] [data-assistant-go]"
  )
  return all.length ? all[all.length - 1] : null
}

// GoButton takes the person to what the reply's go line names (card
// #784), and again on every tap: the chat went there by itself once.
function GoButton({
  target,
  onGo,
}: {
  target: GoTarget
  onGo: (t: GoTarget, from?: Element) => Promise<string>
}) {
  const [error, setError] = useState("")
  const Icon = target.kind === "pr" ? ExternalLinkIcon : LocateFixedIcon
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="h-auto min-h-7 max-w-full justify-start py-1 text-left whitespace-normal"
        data-assistant-go=""
        onClick={(e) => {
          setError("")
          void onGo(target, e.currentTarget).then(setError)
        }}
      >
        <Icon aria-hidden />
        {goLabel(target)}
      </Button>
      {error && (
        <span className="text-[12px] text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  )
}

// DraftThread is a thread the assistant wrote when asked for work: the
// text the thread carries, and how it stands. Most post by themselves
// (card #988). One that waits (check-first, a failed post, or one restored
// from history that never posted) has Post thread: one tap posts it as the
// ask box does. Once posted it says which thread it became.
function DraftThread({
  draft,
  posted,
  posting,
  failed,
  work,
  onPost,
  cut,
  space,
}: {
  draft: string
  posted?: Posted
  // It is posting by itself.
  posting?: boolean
  // Why it couldn't post by itself (Post thread retries).
  failed?: string
  // What came of it since (card #785): who took it, the cards made.
  work?: WorkThread
  // Without it there is no Post thread (the draft is being written).
  onPost?: () => Promise<void>
  // The draft was cut short.
  cut?: boolean
  // The board tag of the shared space it is for (card #1198); "" or
  // none: Personal.
  space?: string
}) {
  const [busy, setBusy] = useState(false)
  const { boards } = useBoards()
  const where = space ? spaceOfTag(space, boards) : null
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
        {posted ? "Thread" : "Draft thread"}
      </p>
      {space && (
        <p className="text-[12px] text-muted-foreground" data-draft-space="">
          {where
            ? `For the shared space “${where.name}”.`
            : `For a space tagged ${space}, which isn't here.`}
        </p>
      )}
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
      ) : posting ? (
        <p className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
          <Loader2Icon
            className="size-3.5 animate-spin motion-reduce:animate-none"
            aria-hidden
          />
          Posting…
        </p>
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
          {!busy && (error || failed) && (
            <span className="text-[12px] text-destructive" role="alert">
              Couldn't post it: {error || failed}
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

// HomeLine is what the chat says above the box when the assistant's
// computer is away or can't answer yet (design §6), with Use <computer>
// instead when another one can take over.
function HomeLine({
  view,
  onUsePc,
}: {
  view: HomeView
  onUsePc?: (device: string) => Promise<string>
}) {
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState("")
  if (!view.line) return null
  const pc = view.usePc
  return (
    <div
      className="shrink-0 border-t border-border bg-muted/40 px-4 py-2 text-[12.5px] leading-snug text-muted-foreground"
      data-assistant-away=""
      role="status"
    >
      {view.line}
      {pc && onUsePc && (
        <Button
          type="button"
          variant="link"
          size="sm"
          className="ml-1 h-auto p-0 text-[12.5px]"
          disabled={busy}
          onClick={() => {
            setBusy(true)
            setFailed("")
            void onUsePc(pc.device).then((why) => {
              setBusy(false)
              setFailed(why)
            })
          }}
        >
          Use {pc.name} instead
        </Button>
      )}
      {failed && (
        <span className="ml-1 text-destructive" role="alert">
          {failed}
        </span>
      )}
    </div>
  )
}

// Waiting is what shows under a question with no reply yet: the reply as
// it streams where the turn runs, else which computer has it (design §5).
function Waiting({
  turn,
  stream,
  home,
  oldest,
  onStop,
}: {
  turn: string
  stream?: Reply
  home: AssistantHomeStatus | null
  oldest: boolean
  onStop: () => void
}) {
  const name = home?.name || "the assistant's computer"
  const connected = home?.state === "connected"
  let line = ""
  if (!stream) {
    if (home?.here) line = "Waiting for the question above…"
    else if (!home?.device) line = "Waiting for the assistant's computer…"
    else if (connected && oldest) line = `${name} is answering…`
    else line = `Waiting for ${name}`
  }
  return (
    <div className="flex flex-col gap-1" data-assistant-waiting={turn}>
      {stream ? (
        <Answering reply={stream} />
      ) : (
        <p className="flex items-center gap-1.5 text-[12.5px] text-muted-foreground">
          <Loader2Icon
            className="size-3.5 shrink-0 animate-spin motion-reduce:animate-none"
            aria-hidden
          />
          {line}
        </p>
      )}
      <Button
        type="button"
        variant="link"
        size="sm"
        className="h-auto self-start p-0 text-[12.5px] text-muted-foreground"
        aria-label="Stop this question"
        onClick={onStop}
      >
        <SquareIcon className="size-3 fill-current" aria-hidden />
        Stop
      </Button>
    </div>
  )
}

// Answering is the reply being written: what it looks at now, then the
// words so far, and a thread it is drafting (posted only once it's done).
function Answering({ reply }: { reply: Reply }) {
  const { text, draft, partial, ask } = splitReply(reply.text, true)
  // A Divide reply's blocks are cut from the text as they are written.
  const drafting = takeDrafts(reply.text).partial
  const status =
    reply.status ||
    (drafting
      ? "Drafting cards"
      : partial
        ? "Drafting a thread"
        : ask
          ? "Drafting a question"
          : "")
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
          : work.answered
            ? "Answered. A card made from it shows here."
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
function Update({ entry }: { entry: ChatEntry }) {
  // Sending unmounts the box that had focus: focus stays in the panel, on
  // this line, so Escape still closes it.
  const row = useRef<HTMLDivElement>(null)
  const settle = useCallback(() => {
    if (row.current?.contains(document.activeElement))
      row.current.focus({ preventScroll: true })
  }, [])
  return (
    <div
      ref={row}
      tabIndex={-1}
      className={
        entry.quiet
          ? "grid min-w-0 gap-1.5 border-l-2 border-border pl-2.5 text-[13px] leading-relaxed [overflow-wrap:anywhere] text-muted-foreground outline-none"
          : "grid min-w-0 gap-1.5 border-l-2 border-primary/40 pl-2.5 text-[13px] leading-relaxed [overflow-wrap:anywhere] outline-none"
      }
      data-assistant-update=""
    >
      <RichText bold text={replyText(entry.text)} />
      {entry.act && (
        <AnswerAsk
          card={entry.act.card}
          blocker={entry.act.blocker}
          settle={settle}
        />
      )}
    </div>
  )
}

// AnswerAsk is a card's open item answered from the chat, as its card
// page does: its options as buttons (the recommended one first, filled),
// and own words. Once answered it says so; the card has the rest. Words
// typed are kept with the card page's own draft of the same answer.
function AnswerAsk({
  card,
  blocker,
  settle,
}: {
  card: string
  blocker: string
  settle: () => void
}) {
  const { detail } = useCard(card)
  const b = detail?.blockers.find((x) => x.id === blocker)
  const [busy, setBusy] = useState(-2)
  const [error, setError] = useState("")
  const [text, setText] = useDraft(`${card}/blocker/${blocker}`)
  const [open, setOpen] = useState(() => text !== "")
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
      settle()
      setText("")
      setOpen(false)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(-2)
    }
  }
  // As the card page: the recommended answer (or Done) first and filled.
  const options = b.optionsAside ? [] : byRecommendation(b.options)
  // Irori's own "OK to start an agent on this?" takes its buttons only.
  const words = b.question !== GIVEN_QUESTION
  return (
    <div className="grid gap-1.5" data-assistant-answer="">
      {options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {options.map(([o, i], n) => (
            <Button
              key={i}
              type="button"
              size="sm"
              variant={
                n === 0 && (o.recommended || b.doneCheck)
                  ? "default"
                  : "outline"
              }
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
      {words && (
        <WordsBox
          open={open}
          text={text}
          label={
            b.doneCheck
              ? "Say what's missing"
              : options.length
                ? "Answer in your own words"
                : "Answer"
          }
          busy={busy === -1}
          onOpen={() => setOpen(true)}
          onText={setText}
          onCancel={() => setOpen(false)}
          onSend={(t) => void send(-1, t)}
        />
      )}
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
