// Screen share in a huddle (#658), from the #571 mock-up's share stage:
// - ShareStage takes the posts' place while a share is on: who shares
//   what, the quality line, Full screen, the picture, and whose view it is.
//   Your own share shows as "You're sharing" with Stop sharing instead.
// - ShareButton is the huddle's Share (desktop only): a picker of this
//   device's displays and windows, or Stop sharing while it's yours.
// lib/share.ts parses and plays the frames; lib/call.ts holds the share.

import { useEffect, useRef, useState } from "react"
import { createPortal } from "react-dom"
import {
  AppWindowIcon,
  MaximizeIcon,
  MinimizeIcon,
  MonitorIcon,
  MonitorOffIcon,
  MonitorUpIcon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { personWord, shareSources } from "@/lib/call"
import {
  askKeyframe,
  canDecode,
  qualityText,
  SharePlayer,
  sortSources,
  sourceLine,
  watchShare,
  whatText,
  type CallShareView,
  type ShareSourceView,
} from "@/lib/share"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"

// useFullScreen is the stage's Full screen: the browser's own where it has
// one for an element, else a fixed overlay over everything that Escape
// leaves (a phone's WebView has none).
function useFullScreen(ref: React.RefObject<HTMLElement | null>) {
  const [native, setNative] = useState(false)
  const [overlay, setOverlay] = useState(false)
  useEffect(() => {
    const on = () => setNative(document.fullscreenElement === ref.current)
    document.addEventListener("fullscreenchange", on)
    return () => document.removeEventListener("fullscreenchange", on)
  }, [ref])
  useEffect(() => {
    if (!overlay) return
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOverlay(false)
    }
    window.addEventListener("keydown", key)
    return () => window.removeEventListener("keydown", key)
  }, [overlay])
  const toggle = () => {
    if (native) {
      void document.exitFullscreen().catch(() => {})
      return
    }
    if (overlay) {
      setOverlay(false)
      return
    }
    const el = ref.current
    if (el && document.fullscreenEnabled && el.requestFullscreen) {
      el.requestFullscreen().catch(() => setOverlay(true))
    } else setOverlay(true)
  }
  return { full: native || overlay, overlay, toggle }
}

// ShareStage is the share on in the huddle, in the posts' place.
export function ShareStage({
  share,
  onStop,
  compact,
}: {
  share: CallShareView
  onStop?: () => void
  // A phone: the lines wrap under the picture's header.
  compact?: boolean
}) {
  if (share.mine) return <MyShare share={share} onStop={onStop} />
  return <TheirShare share={share} compact={compact} />
}

function TheirShare({
  share,
  compact,
}: {
  share: CallShareView
  compact?: boolean
}) {
  const stage = useRef<HTMLDivElement>(null)
  const { full, overlay, toggle } = useFullScreen(stage)
  const name = personWord(share.name)
  const who = share.device ? `${name} on ${share.device}` : name
  const body = (
    <div
      ref={stage}
      aria-label="Shared screen"
      className={cn(
        "flex min-h-0 flex-1 flex-col gap-2 bg-background p-3",
        overlay && "fixed inset-0 z-[1000]"
      )}
    >
      <div
        className={cn(
          "flex items-center gap-2 text-[12.5px]",
          compact && "flex-wrap gap-y-0.5"
        )}
      >
        <MonitorUpIcon className="size-4 shrink-0 text-info" />
        <span
          className={cn(
            "min-w-0 flex-1 text-ink-2",
            compact ? "basis-[calc(100%-1.5rem)]" : "truncate"
          )}
        >
          <span className="font-medium text-ink">{who}</span> {whatText(share)}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
          <span
            className="tabular-nums"
            title={`${share.width} × ${share.height}, about ${share.kbps} kbps`}
          >
            {qualityText(share)}
          </span>
          <Button
            size="icon-xs"
            variant="ghost"
            title={full ? "Leave full screen (Esc)" : "Full screen"}
            aria-label={full ? "Leave full screen" : "Full screen"}
            onClick={toggle}
          >
            {full ? <MinimizeIcon /> : <MaximizeIcon />}
          </Button>
        </span>
      </div>
      <SharePicture key={share.shareId} share={share} />
      {!full && (
        <p className="text-[11.5px] text-muted-foreground">
          Your view only: {name} chose{" "}
          {share.kind === "display" ? "one screen" : "one window"}, and nothing
          else on their screen is sent. Sharing your own replaces theirs after
          they're told.
        </p>
      )}
    </div>
  )
  // Over everything: a glass panel's backdrop filter would hold a fixed
  // element inside it.
  return overlay ? createPortal(body, document.body) : body
}

type Look = "waiting" | "playing" | "unsupported" | "stand-in"

// SharePicture is the decoded picture on a canvas, fitted inside the
// stage. It watches the share while it's shown, and stops when it goes.
function SharePicture({ share }: { share: CallShareView }) {
  const canvas = useRef<HTMLCanvasElement>(null)
  const [look, setLook] = useState<Look>(() =>
    canDecode() ? "waiting" : "unsupported"
  )
  useEffect(() => {
    const c = canvas.current
    if (!c || !canDecode()) return
    const player = new SharePlayer(c, {
      onPicture: () => setLook((l) => (l === "playing" ? l : "playing")),
      onUnsupported: () => setLook("unsupported"),
      onAskKeyframe: askKeyframe,
    })
    const stop = watchShare(player, () => setLook("stand-in"))
    return () => {
      stop()
      player.close()
    }
  }, [share.shareId])
  return (
    <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-lg border border-border bg-[#0c0c0c] shadow-inner">
      {look === "stand-in" ? (
        <StandInPicture />
      ) : (
        <canvas
          ref={canvas}
          className={cn(
            "size-full object-contain",
            look !== "playing" && "invisible"
          )}
        />
      )}
      {look === "waiting" && (
        <span className="absolute text-[12px] text-[#9d9d9d]">
          Waiting for the picture…
        </span>
      )}
      {look === "unsupported" && (
        <span className="absolute px-4 text-center text-[12.5px] text-[#cccccc]">
          This device can't show shared screens (no video decoder).
        </span>
      )}
    </div>
  )
}

// MyShare is your own share: no picture of your screen, just what's sent
// and Stop sharing.
function MyShare({
  share,
  onStop,
}: {
  share: CallShareView
  onStop?: () => void
}) {
  return (
    <div
      aria-label="Your shared screen"
      className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center"
    >
      <span className="flex size-12 items-center justify-center rounded-full bg-info/15 text-info">
        <MonitorUpIcon className="size-6" />
      </span>
      <div className="grid max-w-[28rem] gap-1">
        <p className="text-[14px] font-semibold text-ink">
          You're {whatText(share)}
        </p>
        <p className="text-[12.5px] text-muted-foreground">
          {share.device ? `From ${share.device}. ` : ""}
          Everyone in the huddle sees{" "}
          {share.kind === "display" ? "this screen" : "this window"} and nothing
          else on your screen. {qualityText(share)}.
        </p>
      </div>
      {onStop && (
        <Button size="sm" variant="destructive" onClick={onStop}>
          <MonitorOffIcon />
          Stop sharing
        </Button>
      )}
    </div>
  )
}

// ShareButton is the huddle's Share: the picker, or Stop sharing while
// the share is yours.
export function ShareButton({
  share,
  onStart,
  onStop,
}: {
  share: CallShareView | null
  onStart: (sourceId: string) => void
  onStop: () => void
}) {
  const [open, setOpen] = useState(false)
  if (share?.mine)
    return (
      <Button
        size="icon-lg"
        variant="secondary"
        className="text-info"
        aria-pressed
        title="Stop sharing"
        aria-label="Stop sharing"
        onClick={onStop}
      >
        <MonitorOffIcon />
      </Button>
    )
  return (
    <Popover open={open} onOpenChange={(o) => setOpen(o)}>
      <PopoverTrigger
        render={
          <Button
            size="icon-lg"
            variant={open ? "secondary" : "ghost"}
            title="Share a window or screen"
            aria-label="Share a window or screen"
          >
            <MonitorUpIcon />
          </Button>
        }
      />
      <PopoverContent
        side="top"
        align="center"
        className="w-[min(22rem,calc(100vw-2rem))] gap-2 p-3"
      >
        <SharePicker
          theirs={share && !share.mine ? personWord(share.name) : ""}
          onPick={(id) => {
            setOpen(false)
            onStart(id)
          }}
        />
      </PopoverContent>
    </Popover>
  )
}

// SharePicker lists what this device can share, its displays first, read
// when it opens.
export function SharePicker({
  theirs,
  onPick,
}: {
  // Who shares now ("" for no one).
  theirs: string
  onPick: (sourceId: string) => void
}) {
  const [list, setList] = useState<ShareSourceView[] | null>(null)
  const [error, setError] = useState("")
  useEffect(() => {
    let on = true
    shareSources()
      .then((l) => on && setList(sortSources(l)))
      .catch((err: unknown) => {
        if (!on) return
        setError(errorText(err))
        setList([])
      })
    return () => {
      on = false
    }
  }, [])
  return (
    <div className="grid gap-2 text-[12.5px]">
      <h3 className="flex items-center gap-2 text-[14px] font-semibold text-ink">
        <MonitorUpIcon className="size-4 shrink-0" />
        Share a window or screen
      </h3>
      <p className="text-muted-foreground">
        {theirs
          ? `${theirs} is sharing now. Sharing yours replaces theirs; they're told.`
          : "Everyone in the huddle sees only what you pick, nothing else on your screen."}
      </p>
      {error && (
        <p role="alert" className="text-[12px] text-destructive">
          {error}
        </p>
      )}
      {list === null ? (
        <p className="py-2 text-muted-foreground">Looking for windows…</p>
      ) : list.length === 0 && !error ? (
        <p className="py-2 text-muted-foreground">Nothing here to share.</p>
      ) : (
        <ul className="-mx-1 grid max-h-[18rem] gap-0.5 overflow-y-auto">
          {list.map((s) => (
            <li key={s.id}>
              <button
                type="button"
                onClick={() => onPick(s.id)}
                className="flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
              >
                {s.kind === "display" ? (
                  <MonitorIcon className="size-4 shrink-0 text-muted-foreground" />
                ) : (
                  <AppWindowIcon className="size-4 shrink-0 text-muted-foreground" />
                )}
                <span className="grid min-w-0">
                  <span className="truncate text-ink">{s.title}</span>
                  <span className="truncate text-[11px] text-muted-foreground">
                    {sourceLine(s)}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

// StandInPicture is the browser preview's picture while there's no
// recorded fixture (public/preview/share-fixture.bin): the #571 mock-up's
// terminal.
function StandInPicture() {
  return (
    <div className="flex size-full min-h-0 flex-col font-mono text-[12px] leading-[1.55] text-[#cccccc]">
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
  )
}
