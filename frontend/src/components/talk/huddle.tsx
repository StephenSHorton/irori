// Huddles (#654): voice calls in a channel or a person DM, built from the
// #571 mock-up (call-mock.tsx). Each piece takes the call picture as props
// (lib/call.ts holds it), so the server-rendered tests can feed it
// directly:
// - HuddleButton: the conversation header's Huddle, Join or In huddle.
// - CallBar: the huddle you're in, on every page (the sidebar, or above
//   the phone's tabs).
// - CallPanel: who's in it, how each device reaches you, Mute and Leave.
// - HuddleLine: a Talk list row's huddle that's on.
// - HuddleStarted, HuddleEnded: the posts a huddle leaves.
// - RingBanner: someone started a huddle you can join.
// - HuddleNotice: who reads what you say, before the mic opens (#656).
// - Captions, TranscriptOn: the transcript as it's written (#656).
// Ringing and PhoneCallBar read the store and act, for App.

import { useEffect, useRef, useState, useSyncExternalStore } from "react"
import {
  CircleDotIcon,
  FileTextIcon,
  HeadphonesIcon,
  MicIcon,
  MicOffIcon,
  PhoneOffIcon,
  TriangleAlertIcon,
  XIcon,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogClose,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/dialog"
import {
  canTranscribeHere,
  clockText,
  convLabel,
  dismissRing,
  durationText,
  healthOf,
  healthWord,
  joinHuddle,
  leaveHuddle,
  lengthText,
  namesText,
  personWord,
  setMuted,
  sharedCall,
  subscribeCall,
  transcriptNotice,
  transcriptOn,
  type CallCaptionView,
  type CallMineView,
  type CallPeerView,
  type CallRingView,
  type CallView,
  type Health,
} from "@/lib/call"
import { useNow } from "@/lib/irori"
import { nameOr } from "@/lib/names"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// useCallPart is one part of the call picture (lib/call.ts's store): the
// component renders again only when that part changes, not on every
// call:state. `pick` must give a
// primitive or a piece of the picture itself (never a new object).
export function useCallPart<T>(pick: (s: CallView) => T): T {
  const get = () => pick(sharedCall())
  return useSyncExternalStore(subscribeCall, get, get)
}

// Elapsed is a huddle's running time, ticking each second.
function Elapsed({ since }: { since: number }) {
  const now = useNow(1000)
  return <>{durationText(since, now.getTime())}</>
}

const hues = [
  "var(--device-teal)",
  "var(--device-orange)",
  "var(--device-pink)",
  "var(--device-indigo)",
  "var(--device-lime)",
  "var(--device-cyan)",
]

// hueOf picks a colour for a name, the same one every time.
export function hueOf(name: string): string {
  let h = 0
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) | 0
  return hues[Math.abs(h) % hues.length]
}

// initials is a name's first letter, or its first two words' ("another
// person": "AP").
export function initials(name: string): string {
  const w = name.trim().split(/\s+/)
  return (w.length > 1 ? w[0][0] + w[1][0] : name.slice(0, 1)).toUpperCase()
}

const healthDot: Record<Health, string> = {
  good: "bg-success",
  fair: "bg-warning",
  poor: "bg-destructive",
  off: "bg-muted-foreground/50",
}

// Who a tile shows: a peer, or this device.
type Who = {
  name: string
  device: string
  // How it reaches you, in words ("direct · 26 ms"); "" for this device.
  link: string
  health: Health | null
  speaking: boolean
  muted: boolean
  // Whether what they say is written down (#656); null: not known.
  transcribed: boolean | null
}

function peerWho(p: CallPeerView): Who {
  return {
    name: personWord(p.name),
    device: nameOr(p.device, ""),
    link: healthWord(p),
    health: healthOf(p),
    speaking: p.speaking,
    muted: p.muted,
    transcribed: p.transcribing,
  }
}

function selfWho(m: CallMineView): Who {
  return {
    name: "You",
    device: m.device,
    link: "this device",
    health: null,
    speaking: m.speaking,
    muted: m.muted,
    transcribed: m.transcribing,
  }
}

// whoIn is everyone in the huddle, you first.
function whoIn(m: CallMineView): Who[] {
  return [selfWho(m), ...m.peers.filter((p) => p.phase !== "gone").map(peerWho)]
}

function Avatar({ w, size = "md" }: { w: Who; size?: "sm" | "md" }) {
  return (
    <span
      className={cn(
        "relative flex shrink-0 items-center justify-center rounded-full font-semibold text-background",
        size === "sm" ? "size-5 text-[9px]" : "size-10 text-sm"
      )}
      style={{ background: hueOf(w.name) }}
      aria-hidden
    >
      {initials(w.name)}
      {w.health && (
        <span
          className={cn(
            "absolute -right-0.5 -bottom-0.5 rounded-full border-2 border-background",
            size === "sm" ? "size-2" : "size-3",
            healthDot[w.health]
          )}
        />
      )}
    </span>
  )
}

// HuddleButton is a conversation header's way in: Huddle to start one,
// Join when one is on here, or the running time while you're in it.
export function HuddleButton({
  mine,
  live,
  busy,
  onStart,
  onJoin,
}: {
  // This device's huddle when it is in this conversation.
  mine: CallMineView | null
  // How many people are in this conversation's huddle (0: none on).
  live: number
  busy?: boolean
  onStart: () => void
  onJoin: () => void
}) {
  if (mine)
    return (
      <Button
        size="xs"
        variant="secondary"
        className="shrink-0 text-success tabular-nums"
        title="You're in this conversation's huddle"
      >
        <HeadphonesIcon />
        In huddle · <Elapsed since={mine.joinedAtMs} />
      </Button>
    )
  if (live > 0)
    return (
      <Button
        size="xs"
        variant="secondary"
        className="shrink-0 text-success"
        disabled={busy}
        onClick={onJoin}
        title="A huddle is on here: join it"
      >
        <HeadphonesIcon />
        Join · {live}
      </Button>
    )
  return (
    <Button
      size="xs"
      variant="ghost"
      className="shrink-0"
      disabled={busy}
      onClick={onStart}
      title="Start a voice huddle here"
    >
      <HeadphonesIcon />
      Huddle
    </Button>
  )
}

// HuddleLine is a Talk list row's huddle that's on: who's in it.
export function HuddleLine({ names }: { names: string }) {
  return (
    <span className="flex min-w-0 items-center gap-1 text-[11px] text-success">
      <HeadphonesIcon className="size-3 shrink-0" />
      <span className="truncate">Huddle · {names}</span>
    </span>
  )
}

function Tile({ w }: { w: Who }) {
  const line = [w.device, w.link].filter(Boolean).join(" · ")
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 rounded-lg border border-border bg-card/60 p-2",
        w.speaking && "border-success ring-1 ring-success/50"
      )}
    >
      <Avatar w={w} />
      <span className="grid min-w-0 flex-1">
        <span className="truncate text-[13px] font-medium text-ink">
          {w.name}
        </span>
        <span
          className="truncate text-[11px] text-muted-foreground"
          title={w.health ? `${w.name}: ${w.link}` : undefined}
        >
          {line}
        </span>
        {w.transcribed === false && (
          <span
            className="truncate text-[11px] text-muted-foreground italic"
            title={
              w.name === "You"
                ? "What you say isn't written down"
                : `What ${w.name} says isn't written down`
            }
          >
            not transcribed
          </span>
        )}
      </span>
      {w.muted && (
        <MicOffIcon
          className="size-3.5 shrink-0 text-destructive"
          aria-label="muted"
        />
      )}
      {w.speaking && !w.muted && <Speaking />}
    </div>
  )
}

function Speaking() {
  return (
    <span className="flex h-3.5 items-end gap-[2px]" aria-label="speaking">
      {[6, 12, 9, 14].map((h, i) => (
        <span
          key={i}
          className="w-[3px] rounded-sm bg-success"
          style={{ height: h }}
        />
      ))}
    </span>
  )
}

function MuteButton({
  muted,
  onMute,
  size,
}: {
  muted: boolean
  onMute: (on: boolean) => void
  size: "icon-xs" | "icon-lg"
}) {
  return (
    <Button
      size={size}
      variant={
        muted ? "destructive" : size === "icon-lg" ? "secondary" : "ghost"
      }
      aria-pressed={muted}
      title={muted ? "Unmute" : "Mute"}
      aria-label={muted ? "Unmute" : "Mute"}
      onClick={() => onMute(!muted)}
    >
      {muted ? <MicOffIcon /> : <MicIcon />}
    </Button>
  )
}

// ErrorLine is why the huddle isn't working, when the Core says.
function ErrorLine({ text }: { text: string }) {
  return (
    <p
      role="alert"
      className="flex items-start gap-1.5 text-[12px] text-destructive"
    >
      <TriangleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
      <span className="min-w-0">{text}</span>
    </p>
  )
}

// CallPanel is the huddle beside the posts (or the whole page on a phone):
// who's in it, how each device reaches you, and Mute and Leave.
export function CallPanel({
  mine,
  wide,
  onMute,
  onLeave,
}: {
  mine: CallMineView
  wide?: boolean
  onMute: (on: boolean) => void
  onLeave: () => void
}) {
  const who = whoIn(mine)
  const captions = mine.captions.length > 0
  return (
    <aside
      aria-label="Huddle"
      className={cn(
        "flex min-h-0 flex-col",
        wide ? "flex-1" : "w-[300px] shrink-0 border-l border-border"
      )}
    >
      <div className="flex items-center gap-2 px-3 pt-3 pb-2">
        <HeadphonesIcon className="size-4 text-success" />
        <span className="text-[13px] font-semibold text-ink">Huddle</span>
        <span className="text-[12px] text-muted-foreground tabular-nums">
          <Elapsed since={mine.joinedAtMs} />
        </span>
        <span className="ml-auto flex min-w-0 items-center gap-2 text-[12px] text-muted-foreground">
          <span className="truncate">
            {who.length === 1 ? "just you" : `${who.length} in it`}
          </span>
          {transcriptOn(mine) && <TranscriptOn />}
        </span>
      </div>
      {mine.error && (
        <div className="px-3 pb-2">
          <ErrorLine text={mine.error} />
        </div>
      )}
      <div
        className={cn(
          "grid min-h-0 content-start gap-1.5 overflow-y-auto px-3 pb-3",
          captions ? "max-h-[45%] shrink-0" : "flex-1"
        )}
      >
        {who.map((w, i) => (
          <Tile key={i} w={w} />
        ))}
        {who.length === 1 && (
          <p className="px-1 pt-1 text-[12px] text-muted-foreground">
            The others here were told it's on; they can join from any of their
            devices.
          </p>
        )}
      </div>
      {captions && <Captions lines={mine.captions} />}
      <div className="mt-auto flex items-center justify-center gap-2 border-t border-border px-3 py-2.5">
        <MuteButton muted={mine.muted} onMute={onMute} size="icon-lg" />
        <Button
          size="icon-lg"
          variant="destructive"
          title="Leave the huddle"
          aria-label="Leave the huddle"
          onClick={onLeave}
        >
          <PhoneOffIcon />
        </Button>
      </div>
    </aside>
  )
}

// TranscriptOn is the badge a huddle shows while anyone in it is written
// down.
export function TranscriptOn() {
  return (
    <Badge
      variant="destructive"
      className="shrink-0 gap-1 normal-case"
      title="Everyone in the huddle sees this. Each device writes down only its own person's voice."
    >
      <CircleDotIcon className="size-3" />
      Transcript on
    </Badge>
  )
}

// Captions is the transcript as it's written, newest at the bottom; it
// follows the newest line unless you've scrolled up to read.
export function Captions({ lines }: { lines: readonly CallCaptionView[] }) {
  const box = useRef<HTMLDivElement>(null)
  const last = lines[lines.length - 1]
  const lastKey = last ? `${lines.length}:${last.atMs}` : ""
  const follow = useRef(true)
  useEffect(() => {
    const b = box.current
    if (b && follow.current) b.scrollTop = b.scrollHeight
  }, [lastKey])
  return (
    <section
      aria-label="Transcript, as it's written"
      className="flex min-h-0 flex-1 flex-col border-t border-border"
    >
      <h4 className="flex items-center gap-1.5 px-3 pt-2.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        <FileTextIcon className="size-3" />
        Transcript, as it's written
      </h4>
      <div
        ref={box}
        onScroll={(e) => {
          const b = e.currentTarget
          follow.current = b.scrollHeight - b.scrollTop - b.clientHeight < 40
        }}
        aria-live="polite"
        className="grid min-h-0 flex-1 content-start gap-2 overflow-y-auto px-3 py-2 text-[12.5px]"
      >
        {lines.map((l, i) => (
          <p key={`${i}:${l.atMs}`} className="min-w-0 text-ink-2">
            <span className="font-medium text-ink">{personWord(l.name)}</span>{" "}
            <time
              className="text-[11px] text-muted-foreground tabular-nums"
              dateTime={new Date(l.atMs).toISOString()}
            >
              {clockText(l.atMs)}
            </time>
            <br />
            <span className="break-words">{l.text}</span>
          </p>
        ))}
      </div>
    </section>
  )
}

// CallBar is the huddle you're in, wherever you are: above your name in
// the sidebar (an icon with its link's dot when the sidebar is folded), or
// above the tabs on a phone. Its name opens the conversation.
export function CallBar({
  mine,
  folded,
  onOpen,
  onMute,
  onLeave,
}: {
  mine: CallMineView
  folded?: boolean
  onOpen: () => void
  onMute: (on: boolean) => void
  onLeave: () => void
}) {
  const where = convLabel(mine)
  const who = whoIn(mine)
  const peers = mine.peers.filter((p) => p.phase !== "gone")
  // The worst link, for the folded bar's dot.
  const order: Health[] = ["poor", "fair", "off", "good"]
  const worst =
    order.find((h) => peers.some((p) => healthOf(p) === h)) ?? "good"
  if (folded)
    return (
      <div className="mx-2 mb-2 flex flex-col items-center gap-1">
        <button
          type="button"
          onClick={onOpen}
          className="relative flex size-8 items-center justify-center rounded-lg border border-success/40 bg-success/10 text-success hover:bg-success/20"
          title={`Huddle · ${where}${transcriptOn(mine) ? " · Transcript on" : ""}`}
          aria-label={`Huddle in ${where}`}
        >
          <HeadphonesIcon className="size-4" />
          <span
            className={cn(
              "absolute -top-0.5 -right-0.5 size-2.5 rounded-full border-2 border-background",
              mine.error ? "bg-destructive" : healthDot[worst]
            )}
          />
        </button>
        <MuteButton muted={mine.muted} onMute={onMute} size="icon-xs" />
      </div>
    )
  return (
    <div className="mx-2 mb-2 rounded-lg border border-success/40 bg-success/10 p-2">
      <div className="flex items-center gap-2 text-[12.5px]">
        <HeadphonesIcon className="size-4 shrink-0 text-success" />
        <button
          type="button"
          onClick={onOpen}
          className="min-w-0 flex-1 truncate text-left font-medium text-ink hover:underline"
          title={`Open ${where}`}
        >
          Huddle · {where}
        </button>
        {transcriptOn(mine) && (
          <span
            className="flex shrink-0 items-center gap-1 text-[11px] text-destructive"
            title="Transcript on: each device writes down only its own person's voice."
          >
            <CircleDotIcon className="size-3" aria-hidden />
            <span className="sr-only">Transcript on</span>
          </span>
        )}
        <span className="text-[11px] text-muted-foreground tabular-nums">
          <Elapsed since={mine.joinedAtMs} />
        </span>
      </div>
      {mine.error && (
        <div className="mt-1">
          <ErrorLine text={mine.error} />
        </div>
      )}
      <div className="mt-1.5 flex items-center gap-1">
        <div className="flex -space-x-1.5">
          {who.slice(0, 4).map((w, i) => (
            <span key={i} title={w.health ? `${w.name}: ${w.link}` : w.name}>
              <Avatar w={w} size="sm" />
            </span>
          ))}
        </div>
        <span className="ml-1 truncate text-[11px] text-muted-foreground">
          {who.length === 1 ? "just you" : `${who.length} in it`}
        </span>
        <span className="ml-auto flex shrink-0 gap-0.5">
          <MuteButton muted={mine.muted} onMute={onMute} size="icon-xs" />
          <Button
            size="icon-xs"
            variant="ghost"
            className="text-destructive"
            title="Leave the huddle"
            aria-label="Leave the huddle"
            onClick={onLeave}
          >
            <PhoneOffIcon />
          </Button>
        </span>
      </div>
    </div>
  )
}

// HuddleStarted is the post a huddle's start leaves; Join while it's on.
export function HuddleStarted({
  by,
  onJoin,
}: {
  by: string
  // Set while it's on and you aren't in it.
  onJoin?: () => void
}) {
  return (
    <div className="mx-2 my-1.5 flex items-center gap-2 rounded-lg border border-success/40 bg-success/10 px-3 py-2 text-[12.5px]">
      <HeadphonesIcon className="size-4 shrink-0 text-success" />
      <span className="min-w-0 text-ink-2">
        <span className="font-medium text-ink">{by}</span> started a huddle
      </span>
      {onJoin && (
        <Button
          size="xs"
          variant="secondary"
          className="ml-auto shrink-0"
          onClick={onJoin}
        >
          Join
        </Button>
      )}
    </div>
  )
}

// HuddleEnded is the post a huddle leaves when it ends: how long, who was
// in it, and its transcript when it has one.
export function HuddleEnded({
  durationMs,
  people,
  onTranscript,
}: {
  durationMs: number
  people: readonly string[]
  // Set when it has a transcript (#656): opens it.
  onTranscript?: () => void
}) {
  return (
    <div className="mx-2 my-1.5 rounded-lg border border-border bg-card/60 px-3 py-2 text-[13px]">
      <div className="flex items-center gap-2">
        <HeadphonesIcon className="size-4 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-ink">Huddle ended</span>
        <span className="min-w-0 truncate text-[12px] text-muted-foreground">
          {lengthText(durationMs)}
        </span>
      </div>
      {people.length > 0 && (
        <p className="pl-6 text-[12px] text-muted-foreground">
          {namesText(people)} {people.length === 1 ? "was" : "were"} in it.
        </p>
      )}
      {onTranscript && (
        <div className="mt-2 pl-6">
          <Button size="xs" variant="secondary" onClick={onTranscript}>
            <FileTextIcon />
            Transcript
          </Button>
        </div>
      )}
    </div>
  )
}

// RingBanner is a huddle someone started where you can join it: "Spencer
// started a huddle in #netcode · Join / Dismiss".
export function RingBanner({
  ring,
  busy,
  error,
  onJoin,
  onDismiss,
}: {
  ring: CallRingView
  busy?: boolean
  error?: string
  onJoin: () => void
  onDismiss: () => void
}) {
  const where =
    ring.convKind === "channel" ? `in ${convLabel(ring)}` : "with you"
  return (
    <div
      role="status"
      className="mx-auto flex w-full max-w-[640px] flex-wrap items-center gap-2 rounded-lg border border-success/40 bg-success/10 px-3 py-2 text-[13px]"
    >
      <HeadphonesIcon className="size-4 shrink-0 text-success" />
      <span className="min-w-0 flex-1 text-ink-2">
        <span className="font-medium text-ink">{personWord(ring.by)}</span>{" "}
        started a huddle {where}
        {ring.byDevice && ` on ${ring.byDevice}`}
      </span>
      <span className="flex shrink-0 items-center gap-1">
        <Button size="xs" disabled={busy} onClick={onJoin}>
          Join
        </Button>
        <Button
          size="icon-xs"
          variant="ghost"
          title="Dismiss"
          aria-label="Dismiss"
          onClick={onDismiss}
        >
          <XIcon />
        </Button>
      </span>
      {error && (
        <div className="basis-full">
          <ErrorLine text={error} />
        </div>
      )}
    </div>
  )
}

// newestRing is the latest ring here for a huddle this device isn't in.
function newestRing(s: CallView): CallRingView | undefined {
  let best: CallRingView | undefined
  for (const r of s.ringing) {
    if (r.callId === s.mine?.callId) continue
    if (!best || r.atMs > best.atMs) best = r
  }
  return best
}

// Ringing is the newest huddle ringing here, as a banner over the page;
// `onJoined` opens its conversation once you're in.
export function Ringing({ onJoined }: { onJoined: (convId: string) => void }) {
  const ring = useCallPart(newestRing)
  // A Join's progress and failure belong to the ring it was for, so the
  // next ring starts clean.
  const [busy, setBusy] = useState("")
  const [error, setError] = useState({ callId: "", text: "" })
  // The notice before the mic opens (#656), for the ring it was asked on.
  const [asking, setAsking] = useState<CallRingView | null>(null)
  if (!ring && !asking) return null
  const join = (r: CallRingView) => {
    const id = r.callId
    setBusy(id)
    setError({ callId: "", text: "" })
    joinHuddle(id)
      .then(() => onJoined(r.convId))
      .catch((err: unknown) => setError({ callId: id, text: errorText(err) }))
      .finally(() => setBusy((b) => (b === id ? "" : b)))
  }
  return (
    <>
      {ring && (
        <RingBanner
          ring={ring}
          busy={busy === ring.callId}
          error={error.callId === ring.callId ? error.text : ""}
          onJoin={() => setAsking(ring)}
          onDismiss={() => void dismissRing(ring.callId).catch(() => {})}
        />
      )}
      <HuddleNotice
        ask={
          asking && {
            action: "join",
            convKind: asking.convKind,
            convName: asking.convName,
          }
        }
        onConfirm={() => asking && join(asking)}
        onClose={() => setAsking(null)}
      />
    </>
  )
}

// NoticeAsk is a huddle about to open the mic: starting one, or joining.
export type NoticeAsk = {
  action: "start" | "join"
  convKind: string
  convName: string
}

// NoticeText is the notice's words, with the badge's dot when what you say
// is written down.
export function NoticeText({ text }: { text: string }) {
  const on = text.startsWith("This huddle is transcribed")
  return (
    <span className="flex items-start gap-2 text-left">
      {on ? (
        <CircleDotIcon
          className="mt-0.5 size-4 shrink-0 text-destructive"
          aria-hidden
        />
      ) : (
        <MicIcon
          className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
      )}
      <span className="min-w-0">{text}</span>
    </span>
  )
}

// HuddleNotice asks before the mic opens (#656): who reads what you say,
// or that your voice isn't written down. Start or Join runs onConfirm;
// Cancel or Escape just closes it.
export function HuddleNotice({
  ask,
  onConfirm,
  onClose,
}: {
  // The huddle it asks about; null: closed.
  ask: NoticeAsk | null
  onConfirm: () => void
  onClose: () => void
}) {
  const transcribeOn = useCallPart((s) => s.transcribeOn)
  const [canTranscribe, setCanTranscribe] = useState(true)
  const open = !!ask
  useEffect(() => {
    if (!open) return
    let live = true
    void canTranscribeHere()
      .then((ok) => live && setCanTranscribe(ok))
      .catch(() => {})
    return () => {
      live = false
    }
  }, [open])
  const confirmRef = useRef<HTMLButtonElement>(null)
  const a = ask
  const text = a
    ? transcriptNotice({
        convKind: a.convKind,
        convName: a.convName,
        transcribeOn,
        canTranscribe,
      })
    : ""
  const start = a?.action === "start"
  return (
    <AlertDialog open={open} onOpenChange={(next) => !next && onClose()}>
      <AlertDialogContent initialFocus={confirmRef}>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {start ? "Start a huddle" : "Join the huddle"}
            {a &&
              ` ${a.convKind === "channel" ? "in" : "with"} ${convLabel(a)}`}
          </AlertDialogTitle>
          <AlertDialogDescription render={<div />}>
            <NoticeText text={text} />
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogClose render={<Button size="sm" variant="ghost" />}>
            Cancel
          </AlertDialogClose>
          <Button
            ref={confirmRef}
            size="sm"
            onClick={() => {
              onClose()
              onConfirm()
            }}
          >
            <HeadphonesIcon />
            {start ? "Start" : "Join"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

// PhoneCallBar is the huddle you're in above a phone's tabs, except on
// its own conversation, where the panel is the page.
export function PhoneCallBar({
  here,
  onOpen,
}: {
  // The conversation open now ("" for none).
  here: string
  onOpen: (convId: string) => void
}) {
  const mine = useCallPart((s) => s.mine)
  if (!mine || mine.convId === here) return null
  return (
    <div className="pt-2">
      <CallBar
        mine={mine}
        onOpen={() => onOpen(mine.convId)}
        onMute={(on) => void setMuted(on).catch(() => {})}
        onLeave={() => void leaveHuddle().catch(() => {})}
      />
    </div>
  )
}
