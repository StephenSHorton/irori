// The transcript a huddle leaves (#656), beside the conversation (or the
// whole page on a phone): each line by who said it, when, and a search.
// TranscriptView draws a transcript it's given, so the server-rendered
// tests can feed it directly; TranscriptPane reads one by its call.

import { useEffect, useState, type ReactNode } from "react"
import { FileTextIcon, SearchIcon, XIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  clockText,
  findLines,
  lengthText,
  loadTranscript,
  personWord,
  whoCanRead,
  type TalkTranscript,
} from "@/lib/call"
import { Events } from "@/lib/host"
import { inWails } from "@/lib/irori"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"
import { dayLabel } from "./conversation"
import { hueOf, initials } from "./huddle"

// whenText is the transcript's start and length: "Today 10:02 · 18 min".
export function whenText(t: TalkTranscript, now: Date): string {
  const parts: string[] = []
  if (t.startedAtMs != null) {
    const at = new Date(t.startedAtMs)
    parts.push(`${dayLabel(at, now)} ${clockText(t.startedAtMs)}`)
  }
  parts.push(t.ended ? lengthText(t.durationMs) : "still on")
  return parts.join(" · ")
}

function Frame({
  wide,
  onClose,
  sub,
  children,
}: {
  wide?: boolean
  // Left out, there is no close button: the huddle page's own column.
  onClose?: () => void
  sub?: string
  children: ReactNode
}) {
  return (
    <aside
      aria-label="Huddle transcript"
      className={cn(
        "flex min-h-0 flex-col",
        wide ? "min-w-0 flex-1" : "w-[340px] shrink-0 border-l border-border"
      )}
    >
      <div className="flex items-center gap-2 px-3 pt-3">
        <FileTextIcon className="size-4 shrink-0 text-ink" />
        <span className="shrink-0 text-[13px] font-semibold text-ink">
          Huddle transcript
        </span>
        {sub && (
          <span className="min-w-0 truncate text-[12px] text-muted-foreground">
            {sub}
          </span>
        )}
        {onClose && (
          <Button
            size="icon-xs"
            variant="ghost"
            className="ml-auto shrink-0"
            title="Close the transcript"
            aria-label="Close the transcript"
            onClick={onClose}
          >
            <XIcon />
          </Button>
        )}
      </div>
      {children}
    </aside>
  )
}

// TranscriptView is a transcript: who can read it, a search, and its
// lines with who said each and when.
export function TranscriptView({
  t,
  now,
  wide,
  onClose,
}: {
  t: TalkTranscript
  now: Date
  wide?: boolean
  onClose?: () => void
}) {
  const [q, setQ] = useState("")
  const shown = findLines(t.lines, q)
  return (
    <Frame wide={wide} onClose={onClose} sub={whenText(t, now)}>
      <div className="grid gap-2 border-b border-border px-3 pt-1 pb-3">
        <span className="text-[11.5px] text-muted-foreground">
          {whoCanRead(t)}
        </span>
        <label className="relative flex items-center">
          <SearchIcon
            className="pointer-events-none absolute left-2 size-3.5 text-muted-foreground"
            aria-hidden
          />
          <Input
            type="search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search the transcript"
            aria-label="Search the transcript"
            className="h-7 pl-7 text-[12.5px]"
          />
        </label>
      </div>
      <div className="grid min-h-0 flex-1 content-start gap-2.5 overflow-y-auto px-3 py-3 text-[12.5px]">
        {t.lines.length === 0 ? (
          <p className="text-muted-foreground">Nothing was written down.</p>
        ) : shown.length === 0 ? (
          <p className="text-muted-foreground">No line has that.</p>
        ) : (
          shown.map((l, i) => {
            const name = personWord(l.name)
            return (
              <div key={`${i}:${l.atMs}`} className="flex gap-2">
                <span
                  className="flex size-5 shrink-0 items-center justify-center rounded-full text-[9px] font-semibold text-background"
                  style={{ background: hueOf(name) }}
                  aria-hidden
                >
                  {initials(name)}
                </span>
                <p className="min-w-0 flex-1 text-ink-2">
                  <span className="font-medium text-ink">{name}</span>{" "}
                  <time
                    className="text-[11px] text-muted-foreground tabular-nums"
                    dateTime={new Date(l.atMs).toISOString()}
                  >
                    {clockText(l.atMs)}
                  </time>
                  <br />
                  <span className="break-words">{l.text}</span>
                </p>
              </div>
            )
          })
        )}
      </div>
    </Frame>
  )
}

// useTranscript reads huddle `callId`'s transcript. With `follow` it reads
// again whenever Talk changes (a new line is a talk row), which is how the
// huddle page follows a huddle that is on; the browser preview, which has
// no rows, asks every few seconds instead.
export function useTranscript(
  callId: string,
  follow = false
): { t?: TalkTranscript; error?: string } {
  const [got, setGot] = useState<{
    callId: string
    t?: TalkTranscript
    error?: string
  } | null>(null)
  useEffect(() => {
    let live = true
    const read = () =>
      loadTranscript(callId)
        .then((t) => live && setGot({ callId, t }))
        .catch(
          (err: unknown) =>
            live &&
            // A failed re-read keeps the lines already shown.
            setGot((g) =>
              g?.callId === callId && g.t
                ? g
                : { callId, error: errorText(err) }
            )
        )
    void read()
    if (!follow) {
      return () => {
        live = false
      }
    }
    const off = inWails()
      ? Events.On("talk:changed", () => void read())
      : () => {}
    const timer = inWails() ? 0 : window.setInterval(() => void read(), 3_000)
    return () => {
      live = false
      off()
      window.clearInterval(timer)
    }
  }, [callId, follow])
  const here = got?.callId === callId ? got : null
  return { t: here?.t, error: here?.error }
}

// TranscriptPane reads huddle `callId`'s transcript and shows it. `follow`
// keeps it current while the huddle is on.
export function TranscriptPane({
  callId,
  now,
  wide,
  follow,
  onClose,
}: {
  callId: string
  now: Date
  wide?: boolean
  follow?: boolean
  onClose?: () => void
}) {
  const { t, error } = useTranscript(callId, follow)
  if (t) return <TranscriptView t={t} now={now} wide={wide} onClose={onClose} />
  return (
    <Frame wide={wide} onClose={onClose}>
      <p
        role={error ? "alert" : undefined}
        className={cn(
          "px-3 py-3 text-[12.5px]",
          error ? "text-destructive" : "text-muted-foreground"
        )}
      >
        {error
          ? `Could not read the transcript: ${error}`
          : "Reading the transcript…"}
      </p>
    </Frame>
  )
}
