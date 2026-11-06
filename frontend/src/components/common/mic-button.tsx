import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react"
import { LoaderCircleIcon, MicIcon, MicOffIcon } from "lucide-react"

import { composerIcon } from "@/components/attachments/file-attach"
import { Button } from "@/components/ui/button"
import { Popover, PopoverContent } from "@/components/ui/popover"
import { cn } from "@/lib/utils"
import {
  anchorAt,
  compose,
  downloadModel,
  errorLine,
  loadVoiceStatus,
  megabytes,
  newSession,
  onVoiceEvent,
  openVoiceSettings,
  settingsFix,
  sharedVoice,
  spoken,
  startVoice,
  step,
  stopVoice,
  subscribeVoice,
  type Anchor,
  type Session,
} from "@/lib/voice"
import { errorText } from "@/lib/talk"

type Phase = "idle" | "starting" | "listening" | "stopping"

// A press held this long is push-to-talk: letting go stops. A shorter
// press is a tap: it starts, and the next tap stops.
const HoldMs = 500

// MicButton is voice input for one text box (card #240): tap to talk
// (tap again to stop), or hold to talk and let go. The words appear in
// the box live at the caret, a phrase at a time, and the person edits and
// sends as usual; nothing is sent by voice. Typing, or the box being
// cleared by Send, stops listening. Speech is transcribed on this device
// (Whisper); the first press asks before the one-time model download.
export function MicButton({
  target,
  value,
  onText,
  disabled,
}: {
  // The box: its selection decides where the words go.
  target: RefObject<HTMLTextAreaElement | null>
  value: string
  // Puts the box's new text in, with the caret after the words.
  onText: (text: string, caret: number) => void
  disabled?: boolean
}) {
  const voice = useSyncExternalStore(subscribeVoice, sharedVoice)
  const [phase, setPhase] = useState<Phase>("idle")
  const [note, setNote] = useState<
    { kind: "download" } | { kind: "error"; code: string; text: string } | null
  >(null)
  const session = useRef<Session | null>(null)
  const anchor = useRef<Anchor>({ before: "", after: "" })
  // The box's text as dictation last set it; anything else means the
  // person typed or the site cleared it.
  const composed = useRef<string | null>(null)
  const pressedAt = useRef<number | null>(null)
  const button = useRef<HTMLButtonElement>(null)
  // The latest props, for the event handlers (set after each render).
  const onTextRef = useRef(onText)
  const valueRef = useRef(value)
  useLayoutEffect(() => {
    onTextRef.current = onText
    valueRef.current = value
  })

  useEffect(() => {
    void loadVoiceStatus()
  }, [])

  const finish = useCallback(() => {
    session.current = null
    composed.current = null
    setPhase("idle")
  }, [])

  // Every session's events; this box acts on its own session only.
  useEffect(
    () =>
      onVoiceEvent((e) => {
        const s = session.current
        if (!s || e.session !== s.id) return
        const next = step(s, e)
        session.current = next
        const words = spoken(next)
        if (e.kind === "partial" || e.kind === "final" || e.kind === "ended") {
          const c = compose(anchor.current, words)
          if (c.text !== composed.current) {
            composed.current = c.text
            onTextRef.current(c.text, c.caret)
          }
        }
        if (next.error) {
          setNote({
            kind: "error",
            code: next.error.code,
            text: next.error.text,
          })
        }
        if (next.ended) finish()
      }),
    [finish]
  )

  const stop = useCallback(() => {
    const s = session.current
    if (!s) return
    setPhase("stopping")
    void stopVoice(s.id).catch(() => {})
    // ended normally follows within a moment; don't wait on it forever.
    const id = s.id
    window.setTimeout(() => {
      if (session.current?.id === id) finish()
    }, 4000)
  }, [finish])

  // The person typed, or Send cleared the box: stop listening.
  useEffect(() => {
    if (phase !== "listening") return
    if (composed.current !== null && value !== composed.current) stop()
  }, [value, phase, stop])

  // Closing the box (a dialog, leaving the card) stops it.
  useEffect(
    () => () => {
      const s = session.current
      if (s) void stopVoice(s.id).catch(() => {})
    },
    []
  )

  const begin = useCallback(async () => {
    const st = voice.status ?? (await loadVoiceStatus())
    if (st.state === "unsupported") return
    if (st.state === "needs-model") {
      setNote({ kind: "download" })
      return
    }
    const t = target.current
    const v = valueRef.current
    anchor.current = anchorAt(
      v,
      t?.selectionStart ?? v.length,
      t?.selectionEnd ?? v.length
    )
    composed.current = v
    setNote(null)
    setPhase("starting")
    t?.focus()
    try {
      const id = await startVoice()
      session.current = newSession(id)
      setPhase("listening")
    } catch (err) {
      finish()
      const text = errorText(err)
      setNote({ kind: "error", code: codeOf(text), text })
    }
  }, [voice.status, target, finish])

  const fetchModel = useCallback(async () => {
    const ok = await downloadModel()
    if (ok) {
      setNote(null)
      void begin()
    }
  }, [begin])

  const st = voice.status
  if (!st || st.state === "unsupported") return null

  const busy = phase === "starting" || phase === "stopping"
  const live = phase === "listening"
  const dl = voice.download
  const label = live
    ? "Stop voice input"
    : st.state === "needs-model"
      ? "Voice input (downloads the speech model first)"
      : "Voice input: tap to talk, or hold and let go"

  return (
    <Popover
      open={note !== null || !!dl}
      onOpenChange={(open) => {
        if (!open && !dl) setNote(null)
      }}
    >
      <Button
        ref={button}
        type="button"
        size="icon-sm"
        variant="ghost"
        className={cn(
          composerIcon,
          live && "bg-destructive/15 text-destructive hover:bg-destructive/25",
          st.state === "denied" && !live && "text-muted-foreground"
        )}
        title={
          st.state === "denied" && !live ? `${label}. ${st.reason}` : label
        }
        aria-label={label}
        aria-pressed={live}
        disabled={disabled || busy || !!dl}
        // Keep the caret where it is in the box.
        onMouseDown={(e) => e.preventDefault()}
        onPointerDown={(e) => {
          if (e.button !== 0) return
          if (phase === "idle") {
            pressedAt.current = Date.now()
            void begin()
          } else if (live) {
            pressedAt.current = null
            stop()
          }
        }}
        onPointerUp={() => {
          const at = pressedAt.current
          pressedAt.current = null
          if (at !== null && Date.now() - at >= HoldMs) stop()
        }}
        onClick={(e) => {
          // Keyboard (Enter or Space on the button): a toggle.
          if (e.detail !== 0) return
          if (phase === "idle") void begin()
          else if (live) stop()
        }}
      >
        {busy || dl ? (
          <LoaderCircleIcon className="animate-spin" />
        ) : st.state === "denied" && !live ? (
          <MicOffIcon />
        ) : (
          <MicIcon className={cn(live && "animate-pulse")} />
        )}
      </Button>
      <PopoverContent
        anchor={button}
        side="top"
        align="end"
        initialFocus={false}
        className="w-80 gap-2 text-xs"
      >
        {dl ? (
          <DownloadProgress done={dl.done} total={dl.total} />
        ) : note?.kind === "download" ? (
          <>
            <p className="text-foreground">
              Voice input turns speech into text on this device; no audio leaves
              it. It needs the speech model, a one-time{" "}
              {megabytes(st.modelBytes)} download.
            </p>
            {voice.downloadError && (
              <p className="text-destructive" role="alert">
                {voice.downloadError}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button size="xs" variant="ghost" onClick={() => setNote(null)}>
                Not now
              </Button>
              <Button size="xs" onClick={() => void fetchModel()}>
                Download {megabytes(st.modelBytes)}
              </Button>
            </div>
          </>
        ) : note?.kind === "error" ? (
          <>
            <p className="text-destructive" role="alert">
              {errorLine(note.code, note.text)}
            </p>
            <div className="flex justify-end gap-2">
              {settingsFix(note.code) && (
                <Button
                  size="xs"
                  variant="outline"
                  onClick={() =>
                    void openVoiceSettings(note.code).catch(() => {})
                  }
                >
                  Open Settings
                </Button>
              )}
              <Button size="xs" variant="ghost" onClick={() => setNote(null)}>
                OK
              </Button>
            </div>
          </>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}

function DownloadProgress({ done, total }: { done: number; total: number }) {
  const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0
  return (
    <div className="grid gap-1.5" role="status" aria-live="polite">
      <p className="text-foreground">
        Downloading the speech model… {megabytes(done)} of {megabytes(total)}
      </p>
      <div className="h-1.5 w-full overflow-hidden bg-muted">
        <div
          className="h-full bg-primary transition-[width]"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  )
}

// codeOf guesses the error code of a failed start from its text (the
// command rejects with the engine's message only).
function codeOf(text: string): string {
  const t = text.toLowerCase()
  if (t.includes("microphone access") || t.includes("allow"))
    return "mic-denied"
  if (t.includes("microphone")) return "no-mic"
  if (t.includes("model")) return "no-model"
  return "other"
}
