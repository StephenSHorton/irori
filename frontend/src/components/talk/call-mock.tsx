// Voice calls, the parts still a mock-up (card #571). Only the browser
// preview shows them, behind ?call= (lib/preview.ts): nothing is wired. The
// huddle itself is real now (#654, huddle.tsx; ?call=live and ?call=start
// show it on lib/call.ts's stand-in, and its transcripts are real since
// #656, ?call=transcript); what's left here is what a huddle could grow
// into:
//
//   ?call=share       someone shares a screen: it takes the posts' place,
//                     beside the huddle with its transcript

import {
  BotIcon,
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
  SmartphoneIcon,
} from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { previewFlag } from "@/lib/preview"
import { cn } from "@/lib/utils"

// share is this file's mock-up; transcript opens the real transcript of
// #suzuri's earlier huddle (conversation.tsx).
export type CallMock = "share" | "transcript"

export function callMock(): CallMock | null {
  // Server-rendered tests have no window.
  if (typeof window === "undefined") return null
  const v = previewFlag("call")
  return v === "share" || v === "transcript" ? v : null
}

// The channel the mock-up's huddle is in.
export const callChannel = "sample-ch-suzuri"

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

function DevIcon({ device }: { device: Device }) {
  const cls = "size-3 shrink-0"
  if (device === "phone") return <SmartphoneIcon className={cls} />
  if (device === "pc") return <MonitorIcon className={cls} />
  return <LaptopIcon className={cls} />
}

// Lines as each speaker's own device writes them down.
const lines: { at: string; who: string; text: string }[] = [
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
  },
  {
    at: "10:07",
    who: "Spencer",
    text: "Someone should rerun the latency test on the PC once it's idle, the spikes looked like load.",
  },
  {
    at: "10:09",
    who: "You",
    text: "heron, can you make a card for that rerun?",
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
                <DevIcon device={w.device} />
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

// MockCallPanel is the huddle beside a shared screen: who's in it, the
// transcript as it's written, and the controls (none wired).
export function MockCallPanel({
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
