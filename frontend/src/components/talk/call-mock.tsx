// Voice calls, a mock-up (card #571). Only the browser preview shows it,
// behind ?call= (lib/preview.ts): nothing is wired, no audio moves. It shows
// how huddles, their transcript and screen sharing could look, so the person
// can pick what to build before any of it is.
//
//   ?call=start       a channel's Huddle button open: what joining means and
//                     that a transcript is written; netcode has a huddle on
//   ?call=live        in a huddle: who's in it, on which device and link, the
//                     transcript as it's written, and the bar that follows you
//   ?call=share       someone shares a screen: it takes the posts' place
//   ?call=transcript  the huddle ended: its post in the channel, the
//                     transcript beside it, an agent acting on it

import { useState, type ReactNode } from "react"
import {
  BotIcon,
  ChevronDownIcon,
  CircleDotIcon,
  FileTextIcon,
  HeadphonesIcon,
  LaptopIcon,
  MaximizeIcon,
  MicIcon,
  MicOffIcon,
  MonitorIcon,
  MonitorUpIcon,
  PhoneOffIcon,
  SearchIcon,
  SmartphoneIcon,
  SparklesIcon,
  SquarePlusIcon,
  WifiIcon,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { previewFlag } from "@/lib/preview"
import { cn } from "@/lib/utils"

export type CallMock = "start" | "live" | "share" | "transcript"

export function callMock(): CallMock | null {
  // Server-rendered tests have no window.
  if (typeof window === "undefined") return null
  const v = previewFlag("call")
  return v === "start" || v === "live" || v === "share" || v === "transcript"
    ? v
    : null
}

// The channel the mock-up's huddle is in.
export const callChannel = "sample-ch-suzuri"
// A channel where someone else's huddle is on (shown in the list).
export const otherHuddleChannel = "sample-ch-netcode"

type Device = "mac" | "pc" | "phone"

type Who = {
  name: string
  initials: string
  hue: string
  device: Device
  deviceName: string
  // How this device reaches you: direct (one hop) or through a relay.
  link: string
  speaking?: boolean
  muted?: boolean
  sharing?: boolean
  agent?: boolean
}

const people: Who[] = [
  {
    name: "You",
    initials: "S",
    hue: "var(--device-teal)",
    device: "mac",
    deviceName: "MacBook Pro",
    link: "this device",
    speaking: true,
  },
  {
    name: "Spencer",
    initials: "Sp",
    hue: "var(--device-orange)",
    device: "pc",
    deviceName: "Windows PC",
    link: "direct · 26 ms",
  },
  {
    name: "Aiko",
    initials: "A",
    hue: "var(--device-pink)",
    device: "phone",
    deviceName: "Pixel",
    link: "relay · 120 ms",
    muted: true,
  },
  {
    name: "sample-heron",
    initials: "",
    hue: "var(--device-indigo)",
    device: "mac",
    deviceName: "listening",
    link: "reads the transcript",
    agent: true,
  },
]

const deviceIcon = (d: Device) =>
  d === "phone" ? SmartphoneIcon : d === "pc" ? MonitorIcon : LaptopIcon

// Lines as each speaker's own device writes them down.
const lines: { at: string; who: string; text: string; tag?: string }[] = [
  {
    at: "10:02",
    who: "You",
    text: "Okay, quick one: are we tagging 0.10.0 today or waiting on #8?",
  },
  {
    at: "10:03",
    who: "Spencer",
    text: "#8 merged an hour ago. The only thing left is the PC transfer, it still needs the Firewall click.",
  },
  {
    at: "10:04",
    who: "Aiko",
    text: "I can do the click when I'm back at the PC, around two.",
  },
  {
    at: "10:05",
    who: "You",
    text: "Then let's tag at two, right after Aiko's click.",
    tag: "Decision",
  },
  {
    at: "10:07",
    who: "Spencer",
    text: "Someone should rerun the latency test on the PC once it's idle, the spikes looked like load.",
    tag: "To-do",
  },
  {
    at: "10:09",
    who: "You",
    text: "heron, can you make a card for that rerun?",
    tag: "To-do",
  },
]

function Avatar({ w, size = "md" }: { w: Who; size?: "sm" | "md" | "lg" }) {
  const cls =
    size === "lg"
      ? "size-16 text-lg"
      : size === "sm"
        ? "size-5 text-[9px]"
        : "size-10 text-sm"
  return (
    <span
      className={cn(
        "relative flex shrink-0 items-center justify-center rounded-full font-semibold text-background",
        cls,
        w.agent && "border border-dashed border-current bg-transparent"
      )}
      style={w.agent ? { color: w.hue } : { background: w.hue }}
      aria-hidden
    >
      {w.agent ? <BotIcon className="size-1/2" /> : w.initials}
    </span>
  )
}

// HuddleButton is the channel header's way in: Huddle, or the running time
// while you're in it.
export function HuddleButton({ mock }: { mock: CallMock }) {
  if (mock === "live" || mock === "share")
    return (
      <Button
        size="xs"
        variant="secondary"
        className="shrink-0 text-success"
        title="You're in this channel's huddle"
      >
        <HeadphonesIcon />
        In huddle · 4:12
      </Button>
    )
  return (
    <Button
      size="xs"
      variant={mock === "start" ? "secondary" : "ghost"}
      className="shrink-0"
      aria-expanded={mock === "start"}
      title="Start a voice huddle in this channel"
    >
      <HeadphonesIcon />
      Huddle
    </Button>
  )
}

// StartHuddle is what the Huddle button opens: who can join, your mic, and
// the transcript, which is on unless you turn it off (decisions.md #52).
export function StartHuddle({ channel }: { channel: string }) {
  const [transcript, setTranscript] = useState(true)
  return (
    <div className="absolute top-12 right-4 z-20 w-[360px] rounded-xl border border-border bg-background p-4 text-[13px] text-popover-foreground shadow-2xl">
      <h3 className="flex items-center gap-2 text-[15px] font-semibold text-ink">
        <HeadphonesIcon className="size-4" />
        Start a huddle in #{channel}
      </h3>
      <p className="mt-1.5 text-muted-foreground">
        The people in #{channel} get a notice and can join from any of their
        devices, the phone too. Its agents can listen in by reading the
        transcript.
      </p>

      <div className="mt-3 grid gap-2 rounded-lg bg-secondary/50 p-2.5">
        <Row icon={<MicIcon className="size-4" />} label="Microphone">
          <span className="flex items-center gap-1 text-ink-2">
            MacBook Pro Microphone <ChevronDownIcon className="size-3" />
          </span>
        </Row>
        <div className="flex h-1.5 gap-0.5 pl-6" aria-label="Mic level">
          {Array.from({ length: 24 }, (_, i) => (
            <span
              key={i}
              className={cn(
                "flex-1 rounded-sm",
                i < 9 ? "bg-success" : "bg-muted-foreground/25"
              )}
            />
          ))}
        </div>
        <Row icon={<WifiIcon className="size-4" />} label="Noise suppression">
          <span className="text-ink-2">On</span>
        </Row>
      </div>

      <label className="mt-3 flex cursor-pointer gap-2.5 rounded-lg border border-border p-2.5">
        <input
          type="checkbox"
          className="mt-0.5 accent-[var(--primary)]"
          checked={transcript}
          onChange={(e) => setTranscript(e.target.checked)}
        />
        <span className="grid gap-1">
          <span className="font-medium text-ink">Write a transcript</span>
          <span className="text-[12px] text-muted-foreground">
            Each person's own device turns their voice into text, on the device.
            No recording is kept. When the huddle ends the transcript is posted
            in #{channel}, where its people and agents can read and act on it.
            Everyone who joins is told it's on.
          </span>
        </span>
      </label>

      <div className="mt-3 flex justify-end gap-2">
        <Button size="sm" variant="ghost">
          Cancel
        </Button>
        <Button size="sm">
          <HeadphonesIcon />
          Start huddle
        </Button>
      </div>
    </div>
  )
}

function Row({
  icon,
  label,
  children,
}: {
  icon: ReactNode
  label: string
  children: ReactNode
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-muted-foreground">{icon}</span>
      <span className="flex-1 text-muted-foreground">{label}</span>
      {children}
    </div>
  )
}

// HuddleLine is a channel row's live huddle: who's in it, and Join.
export function HuddleLine({ names }: { names: string }) {
  return (
    <span className="flex min-w-0 items-center gap-1 text-[11px] text-success">
      <HeadphonesIcon className="size-3 shrink-0" />
      <span className="truncate">Huddle · {names}</span>
    </span>
  )
}

function TranscriptOn() {
  return (
    <Badge
      variant="destructive"
      className="gap-1 normal-case"
      title="Everyone in the huddle sees this. Each device writes down only its own person's voice."
    >
      <CircleDotIcon className="size-3" />
      Transcript on
    </Badge>
  )
}

function Tile({ w, compact }: { w: Who; compact?: boolean }) {
  const DevIcon = deviceIcon(w.device)
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 rounded-lg border border-border bg-card/60 p-2",
        w.speaking && "border-success ring-1 ring-success/50"
      )}
    >
      <Avatar w={w} size={compact ? "sm" : "md"} />
      <span className="grid min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-ink">
          <span className="truncate">{w.name}</span>
          {w.sharing && (
            <MonitorUpIcon className="size-3.5 shrink-0 text-info" />
          )}
        </span>
        {!compact && (
          <span className="flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground">
            {w.agent ? (
              <span className="truncate">{w.link}</span>
            ) : (
              <>
                <DevIcon className="size-3 shrink-0" />
                <span className="truncate">
                  {w.deviceName} · {w.link}
                </span>
              </>
            )}
          </span>
        )}
      </span>
      {w.muted && <MicOffIcon className="size-3.5 text-destructive" />}
      {w.speaking && <Speaking />}
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

function Controls({ sharing }: { sharing?: boolean }) {
  return (
    <div className="flex items-center justify-center gap-2 border-t border-border px-3 py-2.5">
      <Button size="icon-lg" variant="secondary" title="Mute (Cmd+Shift+M)">
        <MicIcon />
      </Button>
      <Button size="icon-lg" variant="ghost" title="Deafen">
        <HeadphonesIcon />
      </Button>
      <Button
        size="icon-lg"
        variant={sharing ? "secondary" : "ghost"}
        title="Share your screen"
      >
        <MonitorUpIcon />
      </Button>
      <Button size="icon-lg" variant="destructive" title="Leave the huddle">
        <PhoneOffIcon />
      </Button>
    </div>
  )
}

// CallPanel is the huddle beside the posts (or the whole page on a phone):
// who's in it, the transcript as it's written, and the controls.
export function CallPanel({
  channel,
  wide,
  sharing,
}: {
  channel: string
  wide?: boolean
  sharing?: boolean
}) {
  const who = sharing
    ? people.map((w) =>
        w.name === "Spencer"
          ? { ...w, sharing: true, speaking: true }
          : { ...w, speaking: false }
      )
    : wide
      ? // On a phone, you're on the phone.
        people.map((w) =>
          w.name === "You"
            ? { ...w, device: "phone" as const, deviceName: "Pixel" }
            : w
        )
      : people
  const shown = lines.slice(sharing ? 3 : 0, sharing ? 6 : 5)
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
        <span
          className="text-[13px] font-semibold text-ink"
          title={`Huddle in #${channel}`}
        >
          Huddle
        </span>
        <span className="text-[12px] text-muted-foreground tabular-nums">
          4:12
        </span>
        <span className="ml-auto">
          <TranscriptOn />
        </span>
      </div>
      <div className="grid gap-1.5 px-3">
        {who.map((w) => (
          <Tile key={w.name} w={w} compact={sharing} />
        ))}
      </div>

      <h4 className="mt-4 flex items-center gap-1.5 px-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        <FileTextIcon className="size-3" />
        Transcript, as it's written
      </h4>
      <div className="grid min-h-0 flex-1 content-start gap-2 overflow-y-auto px-3 py-2 text-[12.5px]">
        {shown.map((l) => (
          <p key={l.at + l.who} className="text-ink-2">
            <span className="font-medium text-ink">{l.who}</span>{" "}
            <span className="text-[11px] text-muted-foreground tabular-nums">
              {l.at}
            </span>
            <br />
            {l.text}
          </p>
        ))}
        <p className="text-muted-foreground italic">
          <span className="font-medium text-ink not-italic">
            {sharing ? "Spencer" : "Aiko"}
          </span>{" "}
          {sharing
            ? "so this is the run from this morning, see the p95 column…"
            : "I'll ping here when the click's done, and…"}
        </p>
      </div>
      <Controls sharing={sharing} />
    </aside>
  )
}

// CallBar sits above your name in the sidebar while you're in a huddle, so
// it goes with you to any page.
export function CallBar() {
  return (
    <div className="mx-2 mb-2 rounded-lg border border-success/40 bg-success/10 p-2">
      <div className="flex items-center gap-2 text-[12.5px]">
        <HeadphonesIcon className="size-4 text-success" />
        <span className="min-w-0 flex-1 truncate font-medium text-ink">
          Huddle · #suzuri
        </span>
        <span className="text-[11px] text-muted-foreground tabular-nums">
          4:12
        </span>
      </div>
      <div className="mt-1.5 flex items-center gap-1">
        <div className="flex -space-x-1.5">
          {people.slice(0, 3).map((w) => (
            <Avatar key={w.name} w={w} size="sm" />
          ))}
        </div>
        <span className="ml-1 text-[11px] text-muted-foreground">
          3 people, 1 agent
        </span>
        <span className="ml-auto flex gap-0.5">
          <Button size="icon-xs" variant="ghost" title="Mute">
            <MicIcon />
          </Button>
          <Button
            size="icon-xs"
            variant="ghost"
            className="text-destructive"
            title="Leave"
          >
            <PhoneOffIcon />
          </Button>
        </span>
      </div>
    </div>
  )
}

// HuddleStarted is the line a running huddle leaves in the channel's posts.
export function HuddleStarted() {
  return (
    <div className="mx-2 my-2 flex items-center gap-2 rounded-lg border border-success/40 bg-success/10 px-3 py-2 text-[12.5px]">
      <HeadphonesIcon className="size-4 text-success" />
      <span className="text-ink-2">
        <span className="font-medium text-ink">You</span> started a huddle ·
        Spencer, Aiko and sample-heron joined
      </span>
      <span className="ml-auto">
        <TranscriptOn />
      </span>
    </div>
  )
}

// ShareStage takes the posts' place while someone shares a screen.
export function ShareStage() {
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-3">
      <div className="flex items-center gap-2 text-[12.5px]">
        <MonitorUpIcon className="size-4 text-info" />
        <span className="text-ink-2">
          <span className="font-medium text-ink">Spencer</span> is sharing a
          window: Windows Terminal
        </span>
        <span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
          1080p · 10 fps · direct
          <Button size="icon-xs" variant="ghost" title="Full screen">
            <MaximizeIcon />
          </Button>
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border bg-[#0c0c0c] font-mono text-[12px] leading-[1.55] text-[#cccccc] shadow-inner">
        <div className="flex items-center gap-2 bg-[#1f1f1f] px-3 py-1.5 text-[11px] text-[#9d9d9d]">
          <span>PowerShell</span>
          <span className="rounded bg-[#0c0c0c] px-2 py-0.5 text-[#cccccc]">
            irori · spike-call
          </span>
        </div>
        <pre className="min-h-0 flex-1 overflow-hidden px-4 py-3 whitespace-pre-wrap">
          {`PS C:\\src\\irori> .\\target\\release\\spike-call dial 6b6c…c16e --secs 60 --label pc-mac
connect  324 ms   path relay → direct after 0.8 s (PC → Mac on the LAN)
t=10s  path=direct  sent=500  lost=0  p50=24.1  p95=41.0  jitter=4.2 ms
t=20s  path=direct  sent=500  lost=0  p50=25.3  p95=`}
          <span className="bg-[#c50f1f] text-white">268.9</span>
          {`  jitter=28.4 ms
t=30s  path=direct  sent=500  lost=0  p50=23.8  p95=44.6  jitter=5.0 ms
t=40s  path=direct  sent=500  lost=0  p50=26.0  p95=`}
          <span className="bg-[#c50f1f] text-white">587.6</span>
          {`  jitter=31.0 ms
t=50s  path=direct  sent=500  lost=0  p50=24.9  p95=39.7  jitter=4.8 ms
t=60s  path=direct  sent=500  lost=0  p50=25.5  p95=42.3  jitter=4.9 ms

SUMMARY label="pc-mac" mode=direct-allowed sent=3000 lost=0 (0.00%)
        rtt p50=25.9 p95=268.9 p99=587.6 max=930.3ms one_way~13ms
PS C:\\src\\irori> `}
          <span className="inline-block h-[1.1em] w-2 translate-y-[2px] bg-[#cccccc]" />
        </pre>
      </div>
      <p className="text-[11.5px] text-muted-foreground">
        Your view only: Spencer chose one window, and nothing else on their
        screen is sent. Sharing your own replaces theirs after they're told.
      </p>
    </div>
  )
}

// HuddleEnded is the post a huddle leaves when it ends: who was in it, what
// was decided, and the transcript one click away.
export function HuddleEnded({ onOpen }: { onOpen?: () => void }) {
  return (
    <div className="mx-2 my-1.5 rounded-lg border border-border bg-card/60 p-3 text-[13px]">
      <div className="flex items-center gap-2">
        <HeadphonesIcon className="size-4 shrink-0 text-muted-foreground" />
        <span className="shrink-0 font-medium text-ink">Huddle ended</span>
        <span className="min-w-0 truncate text-[12px] text-muted-foreground">
          10:02 to 10:20 · 18 min
        </span>
      </div>
      <p className="pl-6 text-[12px] text-muted-foreground">
        You, Spencer and Aiko talked; sample-heron listened.
      </p>
      <div className="mt-2 grid gap-1 pl-6 text-ink-2">
        <p className="flex items-center gap-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          <SparklesIcon className="size-3" />
          Summary by sample-heron
        </p>
        <p>
          <Badge variant="info" className="mr-1.5 normal-case">
            Decision
          </Badge>
          Tag 0.10.0 at two, right after Aiko's Firewall click on the PC.
        </p>
        <p>
          <Badge variant="warning" className="mr-1.5 normal-case">
            To-do
          </Badge>
          Rerun the latency test on the PC once it's idle.
        </p>
      </div>
      <div className="mt-2.5 flex gap-1.5 pl-6">
        <Button size="xs" variant="secondary" onClick={onOpen}>
          <FileTextIcon />
          Transcript · 38 lines
        </Button>
        <Button size="xs" variant="ghost">
          <SquarePlusIcon />
          Make a card
        </Button>
      </div>
    </div>
  )
}

// TranscriptPane is the whole transcript beside the channel: each line by
// who said it, and what an agent picked out of it.
export function TranscriptPane({ wide }: { wide?: boolean }) {
  return (
    <aside
      aria-label="Huddle transcript"
      className={cn(
        "flex min-h-0 flex-col",
        wide ? "flex-1" : "w-[340px] shrink-0 border-l border-border"
      )}
    >
      <div className="grid gap-1 border-b border-border px-3 py-3">
        <span className="flex items-center gap-2 text-[13px] font-semibold text-ink">
          <FileTextIcon className="size-4" />
          Huddle transcript
          <span className="font-normal text-muted-foreground">
            Today 10:02 · 18 min
          </span>
        </span>
        <span className="text-[11.5px] text-muted-foreground">
          Who can read it: the people and agents of #suzuri. Agents read it like
          any post in the channel.
        </span>
        <span className="mt-1 flex items-center gap-2 rounded-md border border-border px-2 py-1 text-[12px] text-muted-foreground">
          <SearchIcon className="size-3.5" />
          Search the transcript
        </span>
      </div>
      <div className="grid min-h-0 flex-1 content-start gap-2.5 overflow-y-auto px-3 py-3 text-[12.5px]">
        {lines.map((l) => {
          const w = people.find((p) => p.name === l.who)!
          return (
            <div key={l.at + l.who} className="flex gap-2">
              <Avatar w={w} size="sm" />
              <p className="min-w-0 flex-1 text-ink-2">
                <span className="font-medium text-ink">{l.who}</span>{" "}
                <span className="text-[11px] text-muted-foreground tabular-nums">
                  {l.at}
                </span>
                {l.tag && (
                  <Badge
                    variant={l.tag === "Decision" ? "info" : "warning"}
                    className="ml-1.5 normal-case"
                  >
                    {l.tag}
                  </Badge>
                )}
                <br />
                {l.text}
              </p>
            </div>
          )
        })}
        <p className="text-center text-[11px] text-muted-foreground">
          … 32 more lines
        </p>
      </div>
      <div className="flex items-center gap-1.5 border-t border-border px-3 py-2">
        <Button size="xs" variant="secondary">
          <SquarePlusIcon />
          Make a card from it
        </Button>
        <Button size="xs" variant="ghost">
          <BotIcon />
          Ask an agent
        </Button>
        <Button size="xs" variant="ghost" className="ml-auto text-destructive">
          Delete
        </Button>
      </div>
    </aside>
  )
}
