// A huddle's page (card #1044; docs/design/call-agents.md §3), "#huddle/
// <callId>": its title (click to edit), where it was, whether it's on or
// how long it ran, Join while it's on, who is in it, and its transcript,
// which follows new lines while the huddle is on. The canvas takes the
// middle later (cards 8 and 9); until then that space says there are no
// notes. HuddleHeader and Roster take their data as props so the
// server-rendered tests can feed them.

import { useEffect, useRef, useState } from "react"
import {
  ChevronLeftIcon,
  FileTextIcon,
  HeadphonesIcon,
  PencilIcon,
  UsersIcon,
  LayoutDashboardIcon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
import { clockText, joinHuddle, personWord } from "@/lib/call"
import {
  huddleName,
  isLive,
  lengthOf,
  peopleText,
  placeText,
  setHuddleTitle,
  useHuddles,
  type TalkHuddle,
} from "@/lib/huddles"
import { useNarrow } from "@/lib/media"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"
import { dayLabel } from "./conversation"
import { HuddleNotice, hueOf, initials, useCallPart } from "./huddle"
import { TranscriptPane } from "./transcript-pane"

// whenOf is when it started: "Today 10:02".
export function whenOf(h: TalkHuddle, now: Date): string {
  if (h.startedAtMs == null) return ""
  return `${dayLabel(new Date(h.startedAtMs), now)} ${clockText(h.startedAtMs)}`
}

// Title is the huddle's name; a click makes it a box. Enter or leaving the
// box saves, Escape drops the change. A refusal (a space that isn't ready)
// is shown under it in the words the host gave.
export function Title({
  h,
  save,
}: {
  h: Pick<TalkHuddle, "title" | "titleByAssistant" | "convKind" | "convName">
  save: (title: string) => Promise<unknown>
}) {
  const [editing, setEditing] = useState(false)
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const box = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editing) box.current?.select()
  }, [editing])
  const commit = async () => {
    const next = text.trim()
    setEditing(false)
    if (next === h.title.trim()) return
    setBusy(true)
    setError("")
    try {
      await save(next)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="min-w-0">
      {editing ? (
        <Input
          ref={box}
          value={text}
          maxLength={120}
          aria-label="Huddle title"
          placeholder="Name this huddle"
          onChange={(e) => setText(e.target.value)}
          onBlur={() => void commit()}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commit()
            if (e.key === "Escape") setEditing(false)
          }}
          className="h-8 text-[15px] font-semibold"
        />
      ) : (
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setText(h.title)
            setEditing(true)
          }}
          title="Rename this huddle"
          className="group flex max-w-full items-center gap-1.5 rounded text-left hover:text-ink"
        >
          <h1
            className={cn(
              "min-w-0 truncate text-[15px] font-semibold",
              h.title.trim() ? "text-ink" : "text-ink-2"
            )}
          >
            {huddleName(h)}
          </h1>
          <PencilIcon
            className="size-3 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 max-md:opacity-100"
            aria-hidden
          />
        </button>
      )}
      {h.titleByAssistant && h.title.trim() && !editing && (
        <p className="text-[11px] text-muted-foreground">
          Named by an assistant. A title you set replaces it.
        </p>
      )}
      {error && (
        <p role="alert" className="text-[11.5px] text-destructive">
          Could not set the title: {error}
        </p>
      )}
    </div>
  )
}

// Roster is the people in the huddle, each with whether they are in it now.
export function Roster({
  people,
  inNow,
}: {
  people: readonly string[]
  // Who is in it right now (empty once it is over).
  inNow: readonly string[]
}) {
  const now = new Set(inNow.map(personWord))
  if (people.length === 0)
    return (
      <p className="text-[12.5px] text-muted-foreground">
        Nobody is listed yet.
      </p>
    )
  return (
    <ul aria-label="People" className="grid gap-1.5">
      {people.map((p) => {
        const name = personWord(p)
        return (
          <li key={name} className="flex items-center gap-2 text-[13px]">
            <span
              className="flex size-6 shrink-0 items-center justify-center rounded-full text-[10px] font-semibold text-background"
              style={{ background: hueOf(name) }}
              aria-hidden
            >
              {initials(name)}
            </span>
            <span className="min-w-0 truncate font-medium text-ink">
              {name}
            </span>
            {now.has(name) && (
              <span className="ml-auto shrink-0 text-[11.5px] text-success">
                In the huddle
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}

type Tab = "canvas" | "transcript" | "people"

// HuddlePage is huddle `callId`'s page. onBack returns to Talk's list;
// onOpenConv opens the conversation it was in.
export function HuddlePage({
  callId,
  now,
  onBack,
  onOpenConv,
}: {
  callId: string
  now: Date
  onBack: () => void
  onOpenConv: (convId: string) => void
}) {
  const narrow = useNarrow()
  const { huddles, loading, error } = useHuddles()
  const h = huddles.find((x) => x.callId === callId)
  // On now in the picture this device holds, and whether it's in it.
  const onNow = useCallPart((s) => s.live.some((l) => l.callId === callId))
  const livePeople = useCallPart(
    (s) => s.live.find((l) => l.callId === callId)?.people
  )
  const mineHere = useCallPart((s) => s.mine?.callId === callId)
  const [asking, setAsking] = useState(false)
  const [joinError, setJoinError] = useState("")
  const [tab, setTab] = useState<Tab>("transcript")
  const live = !!h && isLive(h)

  if (!h) {
    return (
      <div className="flex min-h-0 flex-1 flex-col glass-panel">
        <Top onBack={onBack} />
        <div className="p-4">
          <Loading loading={loading} label="Loading the huddle">
            <p
              role={error ? "alert" : undefined}
              className={cn(
                "text-[13px]",
                error ? "text-destructive" : "text-muted-foreground"
              )}
            >
              {error
                ? `Could not read the huddle: ${error}`
                : "That huddle isn't here."}
            </p>
          </Loading>
        </div>
      </div>
    )
  }

  const people = h.people
  const roster = (
    <Roster people={people} inNow={live ? (livePeople ?? []) : []} />
  )
  const canvas = (
    <div className="grid content-start gap-3 p-4">
      <p className="rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12.5px] text-muted-foreground">
        No notes from this huddle yet.
      </p>
    </div>
  )
  const transcript = (
    <TranscriptPane callId={callId} now={now} wide follow={live} />
  )

  const join = live ? (
    mineHere ? (
      <Button
        size="xs"
        variant="secondary"
        className="shrink-0 text-success"
        onClick={() => onOpenConv(h.convId)}
        title="You're in this huddle. Open its conversation."
      >
        <HeadphonesIcon />
        You're in it
      </Button>
    ) : (
      <Button
        size="xs"
        variant="secondary"
        className="shrink-0 text-success"
        disabled={!onNow}
        title={
          onNow
            ? "Join this huddle"
            : "This device hasn't heard it lately. It may be over."
        }
        onClick={() => {
          setJoinError("")
          setAsking(true)
        }}
      >
        <HeadphonesIcon />
        Join
      </Button>
    )
  ) : null

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col glass-panel">
      <Top onBack={onBack} />
      <header className="flex flex-wrap items-start gap-x-4 gap-y-2 border-b border-border px-4 pt-1 pb-3">
        <div className="min-w-0 flex-1 basis-60">
          <Title h={h} save={(t) => setHuddleTitle(callId, t)} />
          <p className="mt-0.5 flex flex-wrap items-center gap-x-1.5 text-[12px] text-muted-foreground">
            <button
              type="button"
              className="rounded font-medium text-ink-2 hover:text-ink hover:underline"
              onClick={() => onOpenConv(h.convId)}
              title="Open the conversation it was in"
            >
              {placeText(h)}
            </button>
            {whenOf(h, now) && <span>· {whenOf(h, now)}</span>}
            <span
              className={cn(live && "font-medium text-success")}
              data-live={live ? "on" : "over"}
            >
              · {lengthOf(h)}
            </span>
          </p>
          {people.length > 0 && (
            <p className="mt-0.5 text-[12px] text-muted-foreground">
              {peopleText(people)}
            </p>
          )}
        </div>
        {join}
        {joinError && (
          <p role="alert" className="basis-full text-[12px] text-destructive">
            {joinError}
          </p>
        )}
      </header>
      {narrow ? (
        <>
          <div
            role="tablist"
            aria-label="Huddle"
            className="flex shrink-0 gap-1 border-b border-border px-3 py-1.5"
          >
            {(
              [
                ["canvas", "Canvas", LayoutDashboardIcon],
                ["transcript", "Transcript", FileTextIcon],
                ["people", "People", UsersIcon],
              ] as const
            ).map(([k, label, Icon]) => (
              <button
                key={k}
                type="button"
                role="tab"
                aria-selected={tab === k}
                onClick={() => setTab(k)}
                className={cn(
                  "flex items-center gap-1.5 rounded-md px-2.5 py-1 text-[12.5px]",
                  tab === k
                    ? "bg-secondary font-medium text-ink"
                    : "text-muted-foreground hover:text-ink"
                )}
              >
                <Icon className="size-3.5" aria-hidden />
                {label}
              </button>
            ))}
          </div>
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            {tab === "canvas" && canvas}
            {tab === "transcript" && transcript}
            {tab === "people" && <div className="p-4">{roster}</div>}
          </div>
        </>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
            <section aria-label="People" className="grid gap-2 p-4 pb-0">
              <h2 className="text-[12px] font-medium text-muted-foreground">
                People
              </h2>
              {roster}
            </section>
            {canvas}
          </div>
          <div className="flex w-[380px] shrink-0 flex-col border-l border-border">
            {transcript}
          </div>
        </div>
      )}
      <HuddleNotice
        ask={
          asking
            ? {
                action: "join",
                callId,
                convKind: h.convKind,
                convName: h.convName,
              }
            : null
        }
        onClose={() => setAsking(false)}
        onConfirm={() =>
          void joinHuddle(callId).catch((err: unknown) =>
            setJoinError(errorText(err))
          )
        }
      />
    </div>
  )
}

function Top({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex shrink-0 items-center gap-1 px-2 pt-2">
      <Button size="xs" variant="ghost" onClick={onBack}>
        <ChevronLeftIcon />
        Talk
      </Button>
    </div>
  )
}
