// Huddles (#654): voice calls in a channel or a person DM, built from the
// #571 mock-up (call-mock.tsx). Each piece takes the call picture as props
// (lib/call.ts holds it), so the server-rendered tests can feed it
// directly:
// - HuddleButton: the conversation header's Huddle (with its start
//   sheet, StartSheet), Join or In huddle.
// - CallBar: the huddle you're in, on every page (the sidebar, or above
//   the phone's tabs).
// - CallPanel: who's in it, how each device reaches you, Mute, Deafen and
//   Leave.
// - HuddleLine: a Talk list row's huddle that's on.
// - HuddleStarted, HuddleEnded: the posts a huddle leaves.
// - RingBanner: someone started a huddle you can join.
// Ringing and PhoneCallBar read the store and act, for App.

import { useEffect, useState, useSyncExternalStore } from "react"
import {
  HeadphoneOffIcon,
  HeadphonesIcon,
  MicIcon,
  MicOffIcon,
  MonitorUpIcon,
  PhoneOffIcon,
  TriangleAlertIcon,
  XIcon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import {
  convLabel,
  dismissRing,
  durationText,
  healthOf,
  healthWord,
  joinHuddle,
  leaveHuddle,
  lengthText,
  micName,
  namesText,
  personWord,
  setDeafened,
  setMuted,
  sharedCall,
  subscribeCall,
  type CallMineView,
  type CallPeerView,
  type CallRingView,
  type CallView,
  type Health,
} from "@/lib/call"
import { useNow } from "@/lib/irori"
import type { CallShareView } from "@/lib/share"
import { nameOr } from "@/lib/names"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

import { ShareButton } from "./share"

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
function hueOf(name: string): string {
  let h = 0
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) | 0
  return hues[Math.abs(h) % hues.length]
}

// initials is a name's first letter, or its first two words' ("another
// person": "AP").
function initials(name: string): string {
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
  // The mic is off (muted, or deafened).
  muted: boolean
  // Nothing plays there: only this device knows its own.
  deafened: boolean
  // Shares a window or screen in the huddle (#658).
  sharing?: boolean
}

function peerWho(p: CallPeerView): Who {
  return {
    name: personWord(p.name),
    device: nameOr(p.device, ""),
    link: healthWord(p),
    health: healthOf(p),
    speaking: p.speaking,
    muted: p.muted,
    deafened: false,
  }
}

function selfWho(m: CallMineView): Who {
  return {
    name: "You",
    device: m.device,
    link: "this device",
    health: null,
    speaking: m.speaking,
    muted: m.muted || m.deafened,
    deafened: m.deafened,
  }
}

// sharesFrom is whether the share comes from peer `p`: its person, and the
// device when the share names one (your own other device).
function sharesFrom(s: CallShareView, p: CallPeerView): boolean {
  if (s.mine || s.name !== p.name) return false
  return !s.device || s.device === p.device
}

// whoIn is everyone in the huddle, you first.
function whoIn(m: CallMineView): Who[] {
  const s = m.share ?? null
  return [
    { ...selfWho(m), sharing: !!s?.mine },
    ...m.peers
      .filter((p) => p.phase !== "gone")
      .map((p) => ({ ...peerWho(p), sharing: !!s && sharesFrom(s, p) })),
  ]
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

// HuddleButton is a conversation header's way in: Huddle opens the start
// sheet, Join when one is on here, or the running time while you're in it.
export function HuddleButton({
  mine,
  live,
  busy,
  start,
  error,
  sheetOpen,
  onSheetOpen,
  onStart,
  onJoin,
}: {
  // This device's huddle when it is in this conversation.
  mine: CallMineView | null
  // How many people are in this conversation's huddle (0: none on).
  live: number
  busy?: boolean
  // What the start sheet says about this conversation.
  start: StartWhere
  // Why the last Start failed, shown in the sheet while it's open.
  error?: string
  // The start sheet: open, and opening or closing it.
  sheetOpen: boolean
  onSheetOpen: (open: boolean) => void
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
    <Popover open={sheetOpen} onOpenChange={(o) => onSheetOpen(o)}>
      <PopoverTrigger
        render={
          <Button
            size="xs"
            variant={sheetOpen ? "secondary" : "ghost"}
            className="shrink-0"
            title="Start a voice huddle here"
          >
            <HeadphonesIcon />
            Huddle
          </Button>
        }
      />
      <PopoverContent
        side="bottom"
        align="end"
        className="w-[min(22rem,calc(100vw-2rem))] gap-3 p-4"
      >
        <StartSheet
          start={start}
          busy={busy}
          error={error}
          onStart={onStart}
          onCancel={() => onSheetOpen(false)}
        />
      </PopoverContent>
    </Popover>
  )
}

// StartWhere is the conversation a huddle would start in, as the start
// sheet tells it.
export type StartWhere = {
  // "#suzuri", or the DM's other person.
  label: string
  // channel (in a space), personal (a Personal channel: your own devices)
  // or dm.
  kind: "channel" | "personal" | "dm"
}

// toldText is who a huddle's start reaches: the devices it rings, as the
// Core's call_start does (the space's devices, your own for a Personal
// channel, both of you in a DM). Agents aren't told.
export function toldText(w: StartWhere): string {
  if (w.kind === "personal")
    return "Your other devices get a notice and can join. It's just you here."
  if (w.kind === "dm")
    return `${w.label} gets a notice on each of their devices, and so do your other devices. Either of you can join from any of them.`
  // The ring goes to the whole space, not only the channel's members.
  return `Everyone in ${w.label}'s space gets a notice on each of their devices, and so do your other devices. They can join from any of them; agents aren't told.`
}

// StartSheet is what Huddle opens before anything starts: who gets told,
// the mic it will use, and that no transcript is kept.
export function StartSheet({
  start,
  busy,
  error,
  onStart,
  onCancel,
}: {
  start: StartWhere
  busy?: boolean
  error?: string
  onStart: () => void
  onCancel: () => void
}) {
  // The mic's name, asked once when the sheet opens (null until it
  // answers; "" when it can't say).
  const [mic, setMic] = useState<string | null>(null)
  useEffect(() => {
    let on = true
    micName()
      .then((n) => on && setMic(n))
      .catch(() => on && setMic(""))
    return () => {
      on = false
    }
  }, [])
  return (
    <div className="grid gap-3 text-[12.5px]">
      <div className="grid gap-1">
        <h3 className="flex items-center gap-2 text-[14px] font-semibold text-ink">
          <HeadphonesIcon className="size-4 shrink-0" />
          <span className="min-w-0 truncate">
            {start.kind === "dm"
              ? `Start a huddle with ${start.label}`
              : `Start a huddle in ${start.label}`}
          </span>
        </h3>
        <p className="text-muted-foreground">{toldText(start)}</p>
      </div>
      <div className="grid gap-1.5 rounded-lg border border-border bg-card/60 p-2.5">
        <span className="flex items-center gap-2">
          <MicIcon className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="text-muted-foreground">Microphone</span>
          <span className="ml-auto min-w-0 truncate text-right text-ink">
            {mic === null ? "…" : mic || "the default microphone"}
          </span>
        </span>
        <span className="pl-5.5 text-[11.5px] text-muted-foreground">
          Echo cancelling and noise suppression are on.
        </span>
      </div>
      <p className="rounded-lg border border-border px-2.5 py-2 text-[12px] text-muted-foreground">
        <span className="font-medium text-ink">No transcript.</span> Nothing
        said in a huddle is written down or recorded yet: it is heard live, then
        gone.
      </p>
      {error && <ErrorLine text={error} />}
      <div className="flex items-center justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" disabled={busy} onClick={onStart}>
          <HeadphonesIcon />
          {busy ? "Starting…" : "Start huddle"}
        </Button>
      </div>
    </div>
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
        <span className="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-ink">
          <span className="truncate">{w.name}</span>
          {w.sharing && (
            <MonitorUpIcon
              className="size-3.5 shrink-0 text-info"
              aria-label="sharing"
            />
          )}
        </span>
        <span
          className="truncate text-[11px] text-muted-foreground"
          title={w.health ? `${w.name}: ${w.link}` : undefined}
        >
          {line}
        </span>
      </span>
      {w.deafened ? (
        <HeadphoneOffIcon
          className="size-3.5 shrink-0 text-destructive"
          aria-label="deafened"
        />
      ) : (
        w.muted && (
          <MicOffIcon
            className="size-3.5 shrink-0 text-destructive"
            aria-label="muted"
          />
        )
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

// DeafenButton turns the speaker off (and the mic with it), or back on.
function DeafenButton({
  deafened,
  onDeafen,
  size,
}: {
  deafened: boolean
  onDeafen: (on: boolean) => void
  size: "icon-xs" | "icon-lg"
}) {
  const label = deafened ? "Undeafen" : "Deafen: hear nothing, mic off"
  return (
    <Button
      size={size}
      variant={
        deafened ? "destructive" : size === "icon-lg" ? "secondary" : "ghost"
      }
      aria-pressed={deafened}
      title={label}
      aria-label={deafened ? "Undeafen" : "Deafen"}
      onClick={() => onDeafen(!deafened)}
    >
      {deafened ? <HeadphoneOffIcon /> : <HeadphonesIcon />}
    </Button>
  )
}

// micOff is whether the mic sends nothing: muted, or deafened (Unmute then
// undeafens too, lib/call.ts's setMuted).
function micOff(m: CallMineView): boolean {
  return m.muted || m.deafened
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
// who's in it, how each device reaches you, and Mute, Deafen, Share (a
// desktop's, #658) and Leave.
export function CallPanel({
  mine,
  wide,
  controlsOnly,
  onMute,
  onDeafen,
  onLeave,
  onShare,
  onStopShare,
  onShareNoteSeen,
}: {
  mine: CallMineView
  wide?: boolean
  // Just the controls (and any note): a phone's share stage fills the rest.
  controlsOnly?: boolean
  onMute: (on: boolean) => void
  onDeafen: (on: boolean) => void
  onLeave: () => void
  onShare?: (sourceId: string) => void
  onStopShare?: () => void
  // The share's note line was dismissed (its ×).
  onShareNoteSeen?: () => void
}) {
  const who = whoIn(mine)
  const share = mine.share ?? null
  const controls = (
    <div className="flex items-center justify-center gap-2 border-t border-border px-3 py-2.5">
      <MuteButton muted={micOff(mine)} onMute={onMute} size="icon-lg" />
      <DeafenButton
        deafened={mine.deafened}
        onDeafen={onDeafen}
        size="icon-lg"
      />
      {mine.canShare && onShare && onStopShare && (
        <ShareButton share={share} onStart={onShare} onStop={onStopShare} />
      )}
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
  )
  const notes = (mine.error || mine.shareNote) && (
    <div className="grid gap-1 px-3 pb-2">
      {mine.error && <ErrorLine text={mine.error} />}
      {mine.shareNote && (
        <p className="flex items-start gap-1.5 text-[12px] text-muted-foreground">
          <MonitorUpIcon className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">{mine.shareNote}</span>
          {onShareNoteSeen && (
            <Button
              size="icon-xs"
              variant="ghost"
              className="-my-0.5 shrink-0"
              title="Dismiss"
              aria-label="Dismiss"
              onClick={onShareNoteSeen}
            >
              <XIcon />
            </Button>
          )}
        </p>
      )}
    </div>
  )
  if (controlsOnly)
    return (
      <div aria-label="Huddle" className="shrink-0">
        {notes && <div className="pt-1">{notes}</div>}
        {controls}
      </div>
    )
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
        <span className="ml-auto text-[12px] text-muted-foreground">
          {who.length === 1 ? "just you" : `${who.length} in it`}
        </span>
      </div>
      {notes}
      <div className="grid min-h-0 flex-1 content-start gap-1.5 overflow-y-auto px-3 pb-3">
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
      {controls}
    </aside>
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
  onDeafen,
  onLeave,
}: {
  mine: CallMineView
  folded?: boolean
  onOpen: () => void
  onMute: (on: boolean) => void
  onDeafen: (on: boolean) => void
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
          title={`Huddle · ${where}`}
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
        <MuteButton muted={micOff(mine)} onMute={onMute} size="icon-xs" />
        <DeafenButton
          deafened={mine.deafened}
          onDeafen={onDeafen}
          size="icon-xs"
        />
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
          <MuteButton muted={micOff(mine)} onMute={onMute} size="icon-xs" />
          <DeafenButton
            deafened={mine.deafened}
            onDeafen={onDeafen}
            size="icon-xs"
          />
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

// HuddleEnded is the post a huddle leaves when it ends: how long, and who
// was in it.
export function HuddleEnded({
  durationMs,
  people,
}: {
  durationMs: number
  people: readonly string[]
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
  if (!ring) return null
  return (
    <RingBanner
      ring={ring}
      busy={busy === ring.callId}
      error={error.callId === ring.callId ? error.text : ""}
      onJoin={() => {
        const id = ring.callId
        setBusy(id)
        setError({ callId: "", text: "" })
        joinHuddle(id)
          .then(() => onJoined(ring.convId))
          .catch((err: unknown) =>
            setError({ callId: id, text: errorText(err) })
          )
          .finally(() => setBusy((b) => (b === id ? "" : b)))
      }}
      onDismiss={() => void dismissRing(ring.callId).catch(() => {})}
    />
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
        onDeafen={(on) => void setDeafened(on).catch(() => {})}
        onLeave={() => void leaveHuddle().catch(() => {})}
      />
    </div>
  )
}
