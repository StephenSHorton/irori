// A huddle's page (card #1044; docs/design/call-agents.md §3), "#huddle/
// <callId>": its title (click to edit), where it was, whether it's on or
// how long it ran, Join while it's on, who is in it, and its transcript,
// which follows new lines while the huddle is on. The canvas (card #1052)
// has the middle: people and their assistants across the top, the notes
// under whoever asked, an Add box and, while the viewer's own assistant is
// connected, an Ask box. Roster takes its data as props so the
// server-rendered tests can feed them.

import { useToday } from "@/lib/clock"
import { lazy, Suspense, useEffect, useRef, useState } from "react"
import {
  ChevronLeftIcon,
  FileTextIcon,
  HeadphonesIcon,
  PencilIcon,
  PlusIcon,
  SendIcon,
  SparklesIcon,
  UsersIcon,
  LayoutDashboardIcon,
} from "lucide-react"

import { Button } from "@/components/ui/button"
import { FinishedCanvas } from "./huddle-finished"
import { Input } from "@/components/ui/input"
import { Loading } from "@/components/ui/loading"
import { ask as askAssistantChat, setAssistantOpen } from "@/lib/assistant"
import { DivideAsk, startDivide } from "@/lib/huddle-divide"
import { agentsReady, clockText, joinHuddle, personWord } from "@/lib/call"
import {
  Kinds,
  askAvailable,
  emptyWords,
  rosterRows,
  type AddKind,
  type RosterRow,
} from "@/lib/huddle-canvas"
import {
  addCanvasItem,
  askAssistant,
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
import { previewFlag } from "@/lib/preview"
import { errorText } from "@/lib/talk"
import { cn } from "@/lib/utils"
import { dayLabel } from "./conversation"
import { HuddleNotice, hueOf, initials, useCallPart } from "./huddle"
import { TranscriptView, useTranscript } from "./transcript-pane"

// The canvas is its own chunk: React Flow loads when a huddle page opens.
const HuddleCanvas = lazy(() => import("@/components/canvas/huddle"))

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

const stateDot: Record<RosterRow["state"], string> = {
  connected: "bg-success",
  notes: "bg-success",
  reconnecting: "bg-warning",
  off: "bg-muted-foreground",
  "": "bg-transparent",
}

// Roster is the people in the huddle, each with whether they are in it now
// and, while it is on, their assistant's state: connected, taking notes,
// reconnecting or not connected, and what is wrong with their lines.
export function Roster({
  people,
  inNow,
  rows = [],
}: {
  people: readonly string[]
  // Who is in it right now (empty once it is over).
  inNow: readonly string[]
  rows?: readonly RosterRow[]
}) {
  const now = new Set(inNow.map(personWord))
  const names = [
    ...new Set([...people.map(personWord), ...rows.map((r) => r.name)]),
  ]
  if (names.length === 0)
    return (
      <p className="text-[12.5px] text-muted-foreground">
        Nobody is listed yet.
      </p>
    )
  return (
    <ul aria-label="People" className="grid gap-2">
      {names.map((name) => {
        const row = rows.find((r) => r.name === name)
        return (
          <li key={name} className="grid gap-0.5 text-[13px]">
            <div className="flex items-center gap-2">
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
            </div>
            {row && row.words && (
              <p
                className="flex items-start gap-1.5 pl-8 text-[12px] text-muted-foreground"
                data-assistant-state={row.state}
              >
                <span
                  aria-hidden
                  className={cn(
                    "mt-1.5 size-1.5 shrink-0 rounded-full",
                    stateDot[row.state]
                  )}
                />
                <span>
                  {row.mine ? "Your assistant" : "Assistant"}: {row.words}
                </span>
              </p>
            )}
            {row?.notes.map((n) => (
              <p key={n} className="pl-8 text-[11.5px] text-muted-foreground">
                {n}
              </p>
            ))}
          </li>
        )
      })}
    </ul>
  )
}

// useAgentsOff is whether the huddle's space can't carry agents and the
// canvas live (some device there is too old: card #1047).
function useAgentsOff(convId: string | undefined): boolean {
  const [off, setOff] = useState(false)
  useEffect(() => {
    if (!convId) return
    let live = true
    void agentsReady(convId)
      .then((ok) => live && setOff(!ok))
      .catch(() => live && setOff(false))
    return () => {
      live = false
    }
  }, [convId])
  return off
}

const kindLabel: Record<AddKind, string> = {
  note: "Note",
  question: "Question",
  decision: "Decision",
  task: "Task",
}

// AddItem is a note, question, decision or task of the person's own.
export function AddItem({
  add,
}: {
  add: (kind: AddKind, text: string) => Promise<unknown>
}) {
  const [kind, setKind] = useState<AddKind>("note")
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const submit = async () => {
    const v = text.trim()
    if (!v || busy) return
    setBusy(true)
    setError("")
    try {
      await add(kind, v)
      setText("")
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="flex flex-wrap items-center gap-1.5"
      aria-label="Add to the canvas"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <select
        aria-label="Kind"
        value={kind}
        onChange={(e) => setKind(e.target.value as AddKind)}
        className="h-8 rounded-md border border-input bg-transparent px-2 text-[12.5px] text-ink"
      >
        {Kinds.map((k) => (
          <option key={k} value={k}>
            {kindLabel[k]}
          </option>
        ))}
      </select>
      <Input
        value={text}
        maxLength={2000}
        aria-label="Add to the canvas"
        placeholder="Add a note, question, decision or task"
        onChange={(e) => setText(e.target.value)}
        className="h-8 min-w-0 flex-1 basis-40 text-[12.5px]"
      />
      <Button
        type="submit"
        size="sm"
        variant="secondary"
        disabled={busy || !text.trim()}
      >
        <PlusIcon />
        Add
      </Button>
      {error && (
        <p role="alert" className="basis-full text-[11.5px] text-destructive">
          {error}
        </p>
      )}
    </form>
  )
}

// AskBox puts a question to the viewer's own assistant. The answer comes
// back on the canvas as an item.
export function AskBox({ ask }: { ask: (text: string) => Promise<unknown> }) {
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const submit = async () => {
    const v = text.trim()
    if (!v || busy) return
    setBusy(true)
    setError("")
    try {
      await ask(v)
      setText("")
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="flex flex-wrap items-center gap-1.5"
      aria-label="Ask your assistant"
      onSubmit={(e) => {
        e.preventDefault()
        void submit()
      }}
    >
      <SparklesIcon
        className="size-4 shrink-0 text-muted-foreground"
        aria-hidden
      />
      <Input
        value={text}
        maxLength={2000}
        aria-label="Ask your assistant"
        placeholder="Ask your assistant about this huddle"
        onChange={(e) => setText(e.target.value)}
        className="h-8 min-w-0 flex-1 basis-40 text-[12.5px]"
      />
      <Button type="submit" size="sm" disabled={busy || !text.trim()}>
        <SendIcon />
        Ask
      </Button>
      {error && (
        <p role="alert" className="basis-full text-[11.5px] text-destructive">
          {error}
        </p>
      )}
    </form>
  )
}

type Tab = "canvas" | "transcript" | "people"

// HuddlePage is huddle `callId`'s page. onBack returns to Talk's list;
// onOpenConv opens the conversation it was in.
export function HuddlePage({
  callId,
  onBack,
  onOpenConv,
}: {
  callId: string
  onBack: () => void
  onOpenConv: (convId: string) => void
}) {
  const today = useToday()
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
  // ?huddletab=people (the browser preview) opens a phone's other tabs.
  const [tab, setTab] = useState<Tab>(() => {
    const f = previewFlag("huddletab")
    return f === "people" || f === "transcript" ? f : "canvas"
  })
  const live = !!h && isLive(h)
  // One read of the transcript feeds the canvas, the roster and the
  // transcript pane, and follows Talk's changes (the canvas changes too
  // when another member settles or moves an item).
  const { t, error: tError, reload } = useTranscript(callId, true)
  const agentsOff = useAgentsOff(h?.convId)
  const [writeError, setWriteError] = useState("")
  // A huddle that is over opens on its finished canvas (card #1054); the
  // freeform canvas stays one tap away.
  const [view, setView] = useState<"finished" | "canvas">("finished")

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
  const rows = t ? rosterRows(t, people, { live, agentsOff }) : []
  const roster = (
    <Roster
      people={people}
      inNow={live ? (livePeople ?? []) : []}
      rows={rows}
    />
  )
  const canAsk = !!t && askAvailable(t, live)
  const empty = !!t && t.canvas.length === 0
  const finished = !live && !!t && t.canvas.length > 0 && view === "finished"
  const divide = () => {
    startDivide(callId, huddleName(h))
    setAssistantOpen(true)
    void askAssistantChat(DivideAsk)
  }
  const canvas = (
    <div className="flex min-h-0 flex-1 flex-col">
      {!live && !!t && t.canvas.length > 0 && (
        <div
          role="group"
          aria-label="Canvas view"
          className="flex shrink-0 gap-1 px-4 pt-3"
        >
          {(
            [
              ["finished", "Finished"],
              ["canvas", "Canvas"],
            ] as const
          ).map(([k, label]) => (
            <button
              key={k}
              type="button"
              aria-pressed={view === k}
              onClick={() => setView(k)}
              className={cn(
                "rounded-md px-2.5 py-1 text-[12.5px]",
                view === k
                  ? "bg-secondary font-medium text-ink"
                  : "text-muted-foreground hover:text-ink"
              )}
            >
              {label}
            </button>
          ))}
        </div>
      )}
      {finished && t && (
        <FinishedCanvas
          callId={callId}
          items={t.canvas}
          personal={h.convKind === "dm"}
          onDivide={divide}
          onChanged={reload}
        />
      )}
      {!finished && agentsOff && (
        <p
          role="status"
          className="mx-4 mt-3 rounded-lg border border-border px-3 py-2 text-[12.5px] text-muted-foreground"
        >
          Agents and the canvas are off: someone's Irori in this space needs
          updating.
        </p>
      )}
      {!finished && (
        <div className="relative min-h-[340px] flex-1">
          {t ? (
            <Suspense
              fallback={
                <p className="p-4 text-[12.5px] text-muted-foreground">
                  Laying out the canvas…
                </p>
              }
            >
              <HuddleCanvas
                callId={callId}
                t={t}
                people={people}
                onError={setWriteError}
                onChanged={reload}
              />
            </Suspense>
          ) : (
            <p
              role={tError ? "alert" : undefined}
              className="p-4 text-[12.5px] text-muted-foreground"
            >
              {tError
                ? `Could not read the canvas: ${tError}`
                : "Reading the canvas…"}
            </p>
          )}
          {empty && !agentsOff && (
            <p className="pointer-events-none absolute inset-x-4 top-3 rounded-lg border border-dashed border-border bg-background/80 px-3 py-3 text-center text-[12.5px] text-muted-foreground">
              {emptyWords({
                live,
                agentsOff,
                anyAssistant: !!t?.assistants.some(
                  (a) => a.state !== "not connected"
                ),
              })}
            </p>
          )}
        </div>
      )}
      {t && (
        <div className="grid gap-2 border-t border-border px-4 py-3 max-md:pr-16">
          {writeError && (
            <p role="alert" className="text-[11.5px] text-destructive">
              {writeError}
            </p>
          )}
          {canAsk && (
            <AskBox
              ask={async (text) => {
                setWriteError("")
                await askAssistant(callId, text)
                reload()
              }}
            />
          )}
          <AddItem
            add={async (kind, text) => {
              setWriteError("")
              await addCanvasItem(callId, kind, text)
              reload()
            }}
          />
        </div>
      )}
    </div>
  )
  const transcript = t ? (
    <TranscriptView t={t} wide />
  ) : (
    <p
      role={tError ? "alert" : undefined}
      className="px-3 py-3 text-[12.5px] text-muted-foreground"
    >
      {tError
        ? `Could not read the transcript: ${tError}`
        : "Reading the transcript…"}
    </p>
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
            {whenOf(h, today) && <span>· {whenOf(h, today)}</span>}
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
          <div className="flex min-w-0 flex-1 flex-col">{canvas}</div>
          <div className="flex w-[380px] shrink-0 flex-col border-l border-border">
            <section
              aria-label="People"
              className="grid max-h-[40%] shrink-0 gap-2 overflow-y-auto border-b border-border p-4"
            >
              <h2 className="text-[12px] font-medium text-muted-foreground">
                People
              </h2>
              {roster}
            </section>
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
