// Huddle prototype (card #1038, idea in #1028). THROWAWAY and browser preview
// only: <preview URL>/?huddle. Fake data, no calls, no models, nothing created.
// Like ?gallery it is lazy-loaded from main.tsx and never reachable from the app.
//
//   ?huddle         a fresh huddle, the script starts streaming in
//   ?huddle=mid     paused part-way through (for a look)
//   ?huddle=ended   the after-call view
//
// The design it plays with (#1028, #1029): a huddle is a place, workspace-wide;
// each person has a named office assistant that wakes on an event (its name
// said, its own person asking, a 30-60 s pause digest, the end of the call) and
// writes onto a shared canvas. No cards or tasks exist until the person says so
// after the call.
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  BotIcon,
  CheckCircle2Icon,
  CircleHelpIcon,
  ListChecksIcon,
  MicIcon,
  PauseIcon,
  PencilIcon,
  PhoneOffIcon,
  PlayIcon,
  RotateCcwIcon,
  SendIcon,
  SkipForwardIcon,
  SparklesIcon,
  StickyNoteIcon,
  GavelIcon,
  MessageSquareQuoteIcon,
} from "lucide-react"

import "@xyflow/react/dist/style.css"
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "cn"

// ---------- fake data ----------

type Person = { id: string; name: string; hue: number }
const PEOPLE: Person[] = [
  { id: "p1", name: "Stephen", hue: 250 },
  { id: "p2", name: "Aiko", hue: 20 },
  { id: "p3", name: "Spencer", hue: 150 },
  { id: "p4", name: "Mira", hue: 330 },
]
const YOU = "p1"
// assistant a1 belongs to p1, and so on.
const DEFAULT_NAMES: Record<string, string> = {
  a1: "Pip",
  a2: "Kit",
  a3: "Sable",
  a4: "Wren",
}
const ownerOf = (aid: string) => PEOPLE[Number(aid.slice(1)) - 1]
const personById = (id: string) => PEOPLE.find((p) => p.id === id)!

type Kind = "question" | "answer" | "decision" | "task" | "note"
type Why = "name" | "own" | "digest" | "end"
type Item = {
  id: string
  kind: Kind
  text: string
  by: string // assistant id
  asker: string // person id
  why: Why
  re?: string // an answer's question
}
type Step =
  | { type: "line"; who: string; text: string; secs: number; items?: Item[] }
  | { type: "pause"; secs: number; items?: Item[] }

let n = 0
const it = (
  kind: Kind,
  text: string,
  by: string,
  asker: string,
  why: Why,
  re?: string
): Item => ({ id: `i${++n}`, kind, text, by, asker, why, re })

// {a2} is replaced with assistant a2's current name, so a rename shows
// everywhere (and a line that names an assistant stays true).
const qImport = it(
  "question",
  "Is streaming the import a bigger job than raising the 10 MB limit?",
  "a4",
  "p4",
  "own"
)
const qMigrate = it(
  "question",
  "Do existing imports need a migration?",
  "a4",
  "p4",
  "own"
)
const SCRIPT: Step[] = [
  {
    type: "line",
    who: "p2",
    secs: 20,
    text: "Okay, let's start with the CSV import. Customers keep hitting the 10 MB limit.",
  },
  {
    type: "line",
    who: "p3",
    secs: 15,
    text: "We could stream the file instead of loading it all into memory.",
  },
  {
    type: "line",
    who: "p4",
    secs: 15,
    text: "That's more than a limit bump though, isn't it? How long would it take?",
    items: [qImport],
  },
  {
    type: "line",
    who: "p3",
    secs: 25,
    text: "Maybe three days. {a2}, do we have anyone on the import work already?",
    items: [
      it(
        "answer",
        "Card #412 “CSV import limits” is held by Spencer's agent (it only reads the limit setting). Nobody is on streaming.",
        "a2",
        "p3",
        "name"
      ),
    ],
  },
  {
    type: "pause",
    secs: 40,
    items: [
      it(
        "answer",
        "Streaming the import is about three days (Spencer's estimate).",
        "a4",
        "p4",
        "digest",
        qImport.id
      ),
      it(
        "note",
        "So far: the 10 MB limit hurts customers; streaming is the real fix, ~3 days.",
        "a3",
        "p3",
        "digest"
      ),
    ],
  },
  {
    type: "line",
    who: "p2",
    secs: 25,
    text: "Let's do it: stream it, and keep the 10 MB limit on the web form until it ships. {a2}, log that as a decision.",
    items: [
      it(
        "decision",
        "Build streaming CSV import; keep the 10 MB limit on the web form until it ships.",
        "a2",
        "p2",
        "name"
      ),
    ],
  },
  {
    type: "line",
    who: "p1",
    secs: 15,
    text: "I can write the customer note about the limit. {a1}, make that a task for me.",
    items: [
      it(
        "task",
        "Write the customer note about the 10 MB limit and the streaming fix.",
        "a1",
        "p1",
        "name"
      ),
    ],
  },
  {
    type: "line",
    who: "p4",
    secs: 15,
    text: "Do we need a migration for the imports people already have?",
    items: [qMigrate],
  },
  {
    type: "line",
    who: "p3",
    secs: 20,
    text: "Probably not, they're already stored as files.",
  },
  {
    type: "pause",
    secs: 45,
    items: [
      it(
        "answer",
        "No migration: existing imports are stored as files (Spencer).",
        "a4",
        "p4",
        "digest",
        qMigrate.id
      ),
      it(
        "task",
        "Spike a streaming CSV parser (about three days).",
        "a3",
        "p3",
        "digest"
      ),
    ],
  },
  {
    type: "line",
    who: "p2",
    secs: 20,
    text: "Great. Last thing: who tells support?",
  },
  { type: "line", who: "p1", secs: 10, text: "I'll do it on Monday." },
  { type: "line", who: "p2", secs: 10, text: "Good. Let's end it there." },
]

// What each assistant writes when the call ends; only those whose `needs`
// lines have been said are written (End huddle early and you get fewer).
const END_ITEMS: { needs: number; item: Item }[] = [
  {
    needs: 11,
    item: it(
      "task",
      "Tell support about the 10 MB limit and the streaming fix (Monday).",
      "a1",
      "p1",
      "end"
    ),
  },
  {
    needs: 6,
    item: it(
      "note",
      "Huddle in one line: limit stays, streaming import is the fix, no migration.",
      "a2",
      "p2",
      "end"
    ),
  },
  {
    needs: 7,
    item: it(
      "question",
      "Who owns the streaming spike? Nobody said.",
      "a3",
      "p3",
      "end"
    ),
  },
]

// ---------- look ----------

const KIND: Record<
  Kind,
  { label: string; icon: typeof StickyNoteIcon; tone: string }
> = {
  question: {
    label: "Question",
    icon: CircleHelpIcon,
    tone: "bg-amber-500/15 text-amber-600 dark:text-amber-400",
  },
  answer: {
    label: "Answer",
    icon: MessageSquareQuoteIcon,
    tone: "bg-sky-500/15 text-sky-600 dark:text-sky-400",
  },
  decision: {
    label: "Decision",
    icon: GavelIcon,
    tone: "bg-violet-500/15 text-violet-600 dark:text-violet-400",
  },
  task: {
    label: "Task",
    icon: ListChecksIcon,
    tone: "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400",
  },
  note: {
    label: "Note",
    icon: StickyNoteIcon,
    tone: "bg-muted text-muted-foreground",
  },
}
const WHY: Record<Why, string> = {
  name: "heard its name",
  own: "its person asked",
  digest: "pause digest",
  end: "end of call",
}

function Dot({ p, className }: { p: Person; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex size-7 shrink-0 items-center justify-center rounded-full text-xs font-semibold text-white",
        className
      )}
      style={{ background: `oklch(0.58 0.14 ${p.hue})` }}
    >
      {p.name[0]}
    </span>
  )
}

function mmss(s: number) {
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
}

const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

// ---------- the prototype ----------

type Phase = "live" | "wrapping" | "ended"
type Typed = { id: string; text: string }
const BASE_MS = 2800
const SPEEDS = [0.5, 1, 2, 4]

export default function HuddleMock() {
  const mode = new URLSearchParams(window.location.search).get("huddle")
  const full = SCRIPT.length
  const startShown = mode === "ended" ? full : mode === "mid" ? 8 : 0
  const [shown, setShown] = useState(startShown) // script steps on screen
  const [itemsShown, setItemsShown] = useState(startShown) // steps whose items are on the canvas
  const [typed, setTyped] = useState<Typed[]>([]) // lines you said, with the script step they follow
  const [typedAt, setTypedAt] = useState<Record<string, number>>({})
  const [extra, setExtra] = useState<Item[]>(
    mode === "ended" ? END_ITEMS.map((e) => e.item) : []
  )
  const [dynWriting, setWriting] = useState<string[]>([])
  const [playing, setPlaying] = useState(mode !== "mid" && mode !== "ended")
  const [speed, setSpeed] = useState(1)
  const [phase, setPhase] = useState<Phase>(mode === "ended" ? "ended" : "live")
  const [names, setNames] = useState(DEFAULT_NAMES)
  const [tab, setTab] = useState<"talk" | "canvas" | "people">("talk")
  const [filter, setFilter] = useState<Kind | "all">("all")
  const [draft, setDraft] = useState("")
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState("")
  const [renameErr, setRenameErr] = useState("")
  const timers = useRef<number[]>([])
  const streamRef = useRef<HTMLDivElement>(null)
  const seq = useRef(0)

  const later = useCallback((fn: () => void, ms: number) => {
    timers.current.push(window.setTimeout(fn, ms))
  }, [])
  useEffect(() => {
    const t = timers.current
    return () => t.forEach(clearTimeout)
  }, [])

  // The script streams in line by line.
  useEffect(() => {
    if (!playing || phase !== "live" || shown >= full) return
    const step = SCRIPT[shown]
    const ms = (step.type === "pause" ? BASE_MS * 1.3 : BASE_MS) / speed
    const t = window.setTimeout(() => setShown((s) => s + 1), ms)
    return () => clearTimeout(t)
  }, [playing, phase, shown, speed, full])

  // An assistant needs a moment to write what it heard.
  useEffect(() => {
    if (itemsShown >= shown) return
    const t = window.setTimeout(() => setItemsShown(shown), 1000 / speed)
    return () => clearTimeout(t)
  }, [shown, itemsShown, speed])
  const scriptWriting = SCRIPT.slice(itemsShown, shown).flatMap((st) =>
    (st.items ?? []).map((x) => x.by)
  )

  const writing = [...new Set([...dynWriting, ...scriptWriting])]
  const scripted = useMemo(
    () => SCRIPT.slice(0, itemsShown).flatMap((s) => s.items ?? []),
    [itemsShown]
  )
  const items = useMemo(() => [...scripted, ...extra], [scripted, extra])
  const elapsed =
    SCRIPT.slice(0, shown).reduce((a, s) => a + s.secs, 0) + typed.length * 12

  // Keep the newest line and canvas item in view while it grows.
  useEffect(() => {
    const el = streamRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [shown, typed.length])

  const nameOf = useCallback((aid: string) => names[aid], [names])
  const fill = useCallback(
    (text: string) =>
      text.replace(/\{(a\d)\}/g, (_, a: string) => names[a] ?? a),
    [names]
  )

  // Every transcript row in order: script steps, then what you typed after each.
  const rows = useMemo(() => {
    const out: (
      { k: "step"; i: number; step: Step } | { k: "typed"; t: Typed }
    )[] = []
    for (let i = 0; i < shown; i++) {
      out.push({ k: "step", i, step: SCRIPT[i] })
      typed
        .filter((t) => (typedAt[t.id] ?? 0) === i + 1)
        .forEach((t) => out.push({ k: "typed", t }))
    }
    typed
      .filter((t) => (typedAt[t.id] ?? 0) === 0)
      .forEach((t) => out.unshift({ k: "typed", t }))
    return out
  }, [shown, typed, typedAt])

  function reply(aid: string, text: string, asker: string, why: Why) {
    const t = text.toLowerCase()
    const body = text.replace(/\s+/g, " ").trim()
    let item: Item
    if (/\b(anyone|who|have we|do we have|is there)\b/.test(t))
      item = it(
        "answer",
        `Two cards touch that: #412 and #418, both held by Spencer's agent. Nobody else is on it. (Prototype: made-up answer.)`,
        aid,
        asker,
        why
      )
    else if (/\b(decid|decision|agree|log that)\b/.test(t))
      item = it("decision", `Decision logged: ${body}`, aid, asker, why)
    else if (/\b(task|todo|to-do|remind|follow up|action)\b/.test(t))
      item = it("task", `Follow up: ${body}`, aid, asker, why)
    else if (t.includes("?"))
      item = it("question", `Open question: ${body}`, aid, asker, why)
    else item = it("note", `Noted: ${body}`, aid, asker, why)
    setWriting((w) => [...new Set([...w, aid])])
    later(() => {
      setExtra((x) => [...x, { ...item, id: `x${++seq.current}` }])
      setWriting((w) => w.filter((a) => a !== aid))
    }, 1100 / speed)
  }

  function say(e?: React.FormEvent) {
    e?.preventDefault()
    const text = draft.trim()
    if (!text || phase !== "live") return
    const id = `t${++seq.current}`
    setTyped((t) => [...t, { id, text }])
    setTypedAt((m) => ({ ...m, [id]: shown }))
    setDraft("")
    // Event-driven wakes: a name said wakes that assistant (whoever says it);
    // otherwise your own question wakes your own assistant.
    const named = Object.keys(names).filter((a) =>
      new RegExp(`\\b${esc(names[a])}\\b`, "i").test(text)
    )
    if (named.length)
      named.forEach((a) =>
        reply(
          a,
          text.replace(new RegExp(`\\b${esc(names[a])}\\b[,:]?`, "i"), ""),
          YOU,
          "name"
        )
      )
    else if (text.includes("?")) reply("a1", text, YOU, "own")
  }

  function endHuddle() {
    if (phase !== "live") return
    setPlaying(false)
    setPhase("wrapping")
    const mine = END_ITEMS.filter((e) => e.needs <= shown).map((e) => e.item)
    setWriting([...new Set(mine.map((i) => i.by))])
    later(() => {
      setExtra((x) => [...x, ...mine])
      setWriting([])
      setPhase("ended")
      setTab("talk")
    }, 1200 / speed)
  }

  function restart() {
    timers.current.forEach(clearTimeout)
    timers.current = []
    setShown(0)
    setItemsShown(0)
    setTyped([])
    setTypedAt({})
    setExtra([])
    setWriting([])
    setPhase("live")
    setPlaying(true)
    setTab("talk")
    setFilter("all")
  }

  function commitRename(aid: string) {
    const v = renameDraft.trim()
    if (!v) return setRenaming(null)
    if (
      Object.entries(names).some(
        ([k, nm]) => k !== aid && nm.toLowerCase() === v.toLowerCase()
      ) ||
      PEOPLE.some((p) => p.name.toLowerCase() === v.toLowerCase())
    )
      return setRenameErr("That name is taken in this huddle.")
    setNames((m) => ({ ...m, [aid]: v.slice(0, 16) }))
    setRenaming(null)
    setRenameErr("")
  }

  // Name highlight in the transcript.
  const nameRe = useMemo(
    () =>
      new RegExp(`\\b(${Object.values(names).map(esc).join("|")})\\b`, "gi"),
    [names]
  )
  const withNames = (text: string) =>
    text.split(nameRe).map((part, i) =>
      i % 2 ? (
        <b key={i} className="rounded bg-primary/15 px-1 text-primary">
          {part}
        </b>
      ) : (
        <span key={i}>{part}</span>
      )
    )

  const speaking =
    phase === "live" && shown > 0 && shown <= full && playing
      ? SCRIPT[shown - 1].type === "line"
        ? (SCRIPT[shown - 1] as { who: string }).who
        : null
      : null

  // ---------- pieces ----------

  const roster = (
    <div className="flex flex-col gap-2">
      {PEOPLE.map((p, i) => {
        const aid = `a${i + 1}`
        const edit = renaming === aid
        return (
          <div
            key={p.id}
            className="flex items-center gap-3 rounded-xl border border-border bg-card/60 p-2.5"
          >
            <span className="relative">
              <Dot p={p} />
              {speaking === p.id && (
                <span className="absolute -right-0.5 -bottom-0.5 size-2.5 animate-pulse rounded-full bg-emerald-500 ring-2 ring-card" />
              )}
            </span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium">
                {p.name}
                {p.id === YOU && (
                  <span className="text-muted-foreground"> (you)</span>
                )}
              </div>
              {edit ? (
                <form
                  className="mt-1 flex gap-1"
                  onSubmit={(e) => {
                    e.preventDefault()
                    commitRename(aid)
                  }}
                >
                  <Input
                    autoFocus
                    aria-label={`New name for ${p.name}'s assistant`}
                    value={renameDraft}
                    maxLength={16}
                    onChange={(e) => {
                      setRenameDraft(e.target.value)
                      setRenameErr("")
                    }}
                    className="h-7 text-xs"
                  />
                  <Button type="submit" size="sm">
                    Save
                  </Button>
                </form>
              ) : (
                <div className="flex items-center gap-1 text-xs text-muted-foreground">
                  <BotIcon className="size-3" />
                  <span>
                    assistant{" "}
                    <b className="font-semibold text-foreground">
                      {nameOf(aid)}
                    </b>
                  </span>
                  {writing.includes(aid) && (
                    <span className="ml-1 animate-pulse text-primary">
                      writing…
                    </span>
                  )}
                </div>
              )}
              {edit && renameErr && (
                <div className="mt-1 text-xs text-destructive">{renameErr}</div>
              )}
            </div>
            {!edit && (
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`Rename ${nameOf(aid)}`}
                onClick={() => {
                  setRenaming(aid)
                  setRenameDraft(nameOf(aid))
                  setRenameErr("")
                }}
              >
                <PencilIcon />
              </Button>
            )}
          </div>
        )
      })}
      <p className="px-1 text-xs text-muted-foreground">
        Say an assistant's name and it answers on the canvas. Rename one and the
        script follows.
      </p>
    </div>
  )

  const transcript = (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        ref={streamRef}
        className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3"
        aria-live="polite"
      >
        {rows.length === 0 && (
          <p className="m-auto text-sm text-muted-foreground">
            Everyone just joined…
          </p>
        )}
        {rows.map((r) => {
          if (r.k === "typed")
            return (
              <Line key={r.t.id} p={personById(YOU)} you>
                {withNames(r.t.text)}
              </Line>
            )
          if (r.step.type === "pause")
            return (
              <div
                key={`s${r.i}`}
                className="flex items-center gap-2 text-xs text-muted-foreground"
              >
                <span className="h-px flex-1 bg-border" />
                {r.step.secs} s of quiet: assistants write a digest
                <span className="h-px flex-1 bg-border" />
              </div>
            )
          return (
            <Line
              key={`s${r.i}`}
              p={personById(r.step.who)}
              you={r.step.who === YOU}
            >
              {withNames(fill(r.step.text))}
            </Line>
          )
        })}
        {shown >= full && phase === "live" && (
          <p className="text-center text-xs text-muted-foreground">
            The script is done. Keep talking, or End huddle.
          </p>
        )}
      </div>
      <form onSubmit={say} className="flex gap-2 border-t border-border p-2.5">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          disabled={phase !== "live"}
          placeholder={`Say something, e.g. “${nameOf("a2")}, do we have anyone on the import work?”`}
          aria-label="Say a line"
          className="min-w-0"
        />
        <Button type="submit" disabled={!draft.trim() || phase !== "live"}>
          <SendIcon /> <span className="hidden sm:inline">Say</span>
        </Button>
      </form>
    </div>
  )

  const canvas = (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap gap-1 border-b border-border p-2">
        {(
          ["all", "question", "answer", "decision", "task", "note"] as const
        ).map((k) => (
          <Button
            key={k}
            size="xs"
            variant={filter === k ? "default" : "outline"}
            onClick={() => setFilter(k)}
          >
            {k === "all"
              ? `All ${items.length}`
              : `${KIND[k].label}s ${items.filter((i) => i.kind === k).length}`}
          </Button>
        ))}
      </div>
      <div className="relative min-h-0 flex-1">
        {items.length === 0 && (
          <p className="absolute inset-0 z-10 m-auto flex items-center justify-center p-6 text-center text-sm text-muted-foreground">
            The canvas is empty. Assistants add to it as the talk goes.
          </p>
        )}
        <ReactFlowProvider>
          <ItemGraph items={items} filter={filter} follow nameOf={nameOf} />
        </ReactFlowProvider>
        {writing.length > 0 && phase !== "ended" && (
          <div className="absolute bottom-2 left-2 z-10 animate-pulse rounded-md border border-border bg-background/90 px-2 py-1 text-xs text-muted-foreground">
            <SparklesIcon className="mr-1 inline size-3" />
            {writing.map(nameOf).join(", ")} writing…
          </div>
        )}
      </div>
    </div>
  )

  // ---------- after the call ----------

  if (phase === "ended")
    return (
      <Shell>
        <AfterCall
          items={items}
          nameOf={nameOf}
          elapsed={elapsed}
          onRestart={restart}
        />
      </Shell>
    )

  return (
    <Shell>
      <header className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-border px-3 py-2.5">
        <div className="min-w-0 basis-full sm:flex-1 sm:basis-0">
          <div className="flex items-center gap-2">
            <span className="size-2 animate-pulse rounded-full bg-red-500" />
            <h1 className="truncate text-sm font-semibold">
              Billing import sync
            </h1>
            <Badge variant="slate">#suzuri</Badge>
          </div>
          <div className="text-xs text-muted-foreground">
            {mmss(elapsed)} · {PEOPLE.length} here ·{" "}
            <span className="hidden sm:inline">
              {Object.values(names).join(", ")} listening
            </span>
            <span className="sm:hidden">assistants listening</span>
          </div>
        </div>
        <Pace
          playing={playing}
          speed={speed}
          setSpeed={setSpeed}
          toggle={() => setPlaying((p) => !p)}
          step={() => setShown((s) => Math.min(full, s + 1))}
          done={shown >= full}
        />
        <Button
          variant="destructive"
          size="sm"
          onClick={endHuddle}
          disabled={phase !== "live"}
        >
          <PhoneOffIcon />{" "}
          {phase === "wrapping" ? "Wrapping up…" : "End huddle"}
        </Button>
      </header>

      <nav className="grid grid-cols-3 border-b border-border md:hidden">
        {(
          [
            ["talk", "Transcript"],
            ["canvas", `Canvas ${items.length}`],
            ["people", "Roster"],
          ] as const
        ).map(([k, l]) => (
          <button
            key={k}
            onClick={() => setTab(k)}
            className={cn(
              "py-2 text-sm",
              tab === k
                ? "border-b-2 border-primary font-medium"
                : "text-muted-foreground"
            )}
          >
            {l}
          </button>
        ))}
      </nav>

      <div className="grid min-h-0 flex-1 md:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] lg:grid-cols-[16rem_minmax(0,1.1fr)_minmax(0,1fr)]">
        <aside
          className={cn(
            "overflow-y-auto border-border p-3 lg:block lg:border-r",
            tab === "people" ? "block" : "hidden"
          )}
        >
          {roster}
        </aside>
        <section
          className={cn(
            "min-h-0 flex-col border-border md:flex md:border-r lg:border-l-0",
            tab === "talk" ? "flex" : "hidden"
          )}
        >
          <Heading>Transcript · live</Heading>
          {transcript}
        </section>
        <section
          className={cn(
            "min-h-0 flex-col md:flex",
            tab === "canvas" ? "flex" : "hidden"
          )}
        >
          <Heading>
            Canvas · written by assistants, nothing is a card yet · drag to
            move, scroll to zoom
          </Heading>
          {canvas}
        </section>
      </div>
    </Shell>
  )
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="fixed inset-0 flex flex-col bg-background text-foreground">
      <div className="bg-amber-500/15 px-3 py-1 text-center text-[11px] text-amber-700 dark:text-amber-300">
        Prototype for #1028 · fake data · no calls, no models · nothing is
        created
      </div>
      {children}
    </div>
  )
}

function Heading({ children }: { children: React.ReactNode }) {
  return (
    <div className="border-b border-border px-3 py-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  )
}

function Line({
  p,
  you,
  children,
}: {
  p: Person
  you?: boolean
  children: React.ReactNode
}) {
  return (
    <div className="flex gap-2.5">
      <Dot p={p} />
      <div className="min-w-0">
        <div className="text-xs font-medium text-muted-foreground">
          {p.name}
          {you && " (you)"}
        </div>
        <div className="text-sm leading-snug">{children}</div>
      </div>
    </div>
  )
}

function Pace(props: {
  playing: boolean
  speed: number
  setSpeed: (n: number) => void
  toggle: () => void
  step: () => void
  done: boolean
}) {
  return (
    <div className="flex items-center gap-1" aria-label="Pace">
      <Button
        size="icon-sm"
        variant="outline"
        aria-label={props.playing ? "Pause the script" : "Play the script"}
        onClick={props.toggle}
        disabled={props.done}
      >
        {props.playing ? <PauseIcon /> : <PlayIcon />}
      </Button>
      <Button
        size="icon-sm"
        variant="outline"
        aria-label="Next line"
        onClick={props.step}
        disabled={props.done}
      >
        <SkipForwardIcon />
      </Button>
      {SPEEDS.map((s) => (
        <Button
          key={s}
          size="xs"
          variant={props.speed === s ? "default" : "outline"}
          onClick={() => props.setSpeed(s)}
        >
          {s}×
        </Button>
      ))}
    </div>
  )
}

// ---------- the canvas as a graph ----------

// How items hang together: an answer under its question (`re`), a decision
// after the latest answer, a task after the latest decision, a question
// raised at the end after the latest task. Notes stand alone. The columns are
// the depth of that chain, so the canvas reads left to right as the talk goes.
function parentOf(items: Item[]) {
  const parent = new Map<string, string>()
  let answer = "",
    decision = "",
    task = ""
  for (const i of items) {
    if (i.kind === "answer") {
      if (i.re) parent.set(i.id, i.re)
      answer = i.id
    } else if (i.kind === "decision") {
      if (answer) parent.set(i.id, answer)
      decision = i.id
    } else if (i.kind === "task") {
      if (decision) parent.set(i.id, decision)
      task = i.id
    } else if (i.kind === "question" && i.why === "end" && task)
      parent.set(i.id, task)
  }
  return parent
}

const NODE_W = 240
const COL = 280
const ROW = 190

function layoutItems(items: Item[]) {
  const parent = parentOf(items)
  const pos = new Map<string, { col: number; y: number }>()
  const taken = new Set<string>()
  const next: number[] = []
  for (const i of items) {
    const par = parent.get(i.id)
    const pp = par ? pos.get(par) : undefined
    const col = pp ? pp.col + 1 : 0
    let y = pp ? pp.y : (next[col] ?? 0)
    while (taken.has(`${col}:${y}`)) y += 1
    taken.add(`${col}:${y}`)
    next[col] = Math.max(next[col] ?? 0, y + 1)
    pos.set(i.id, { col, y })
  }
  return { parent, pos }
}

type ItemNodeData = { item: Item; nameOf: (a: string) => string }

function ItemNode({ data }: NodeProps<Node<ItemNodeData>>) {
  return (
    <div style={{ width: NODE_W }}>
      <Handle
        type="target"
        position={Position.Left}
        className="!size-1.5 !border-0 !bg-border"
      />
      <ItemCard item={data.item} nameOf={data.nameOf} />
      <Handle
        type="source"
        position={Position.Right}
        className="!size-1.5 !border-0 !bg-border"
      />
    </div>
  )
}
const nodeTypes = { item: ItemNode }

function ItemGraph({
  items,
  filter,
  follow,
  nameOf,
}: {
  items: Item[]
  filter: Kind | "all"
  follow?: boolean
  nameOf: (a: string) => string
}) {
  const { fitView } = useReactFlow()
  const { nodes, edges } = useMemo(() => {
    const { parent, pos } = layoutItems(items)
    const show = (i: Item) => filter === "all" || i.kind === filter
    const shown = new Set(items.filter(show).map((i) => i.id))
    const nodes: Node<ItemNodeData>[] = items.map((item) => {
      const p = pos.get(item.id)!
      return {
        id: item.id,
        type: "item",
        position: { x: p.col * COL, y: p.y * ROW },
        data: { item, nameOf },
        hidden: !shown.has(item.id),
        draggable: true,
      }
    })
    const edges: Edge[] = [...parent].map(([child, par]) => ({
      id: `${par}-${child}`,
      source: par,
      target: child,
      type: "smoothstep",
      animated: true,
      hidden: !shown.has(par) || !shown.has(child),
      markerEnd: { type: MarkerType.ArrowClosed },
      style: { strokeWidth: 1.5 },
    }))
    return { nodes, edges }
  }, [items, filter, nameOf])

  // Live: keep the newest items (and what they hang from) in view, not
  // squeezed small; pan and zoom for the rest. After the call: all of it.
  const count = items.length
  useEffect(() => {
    const t = window.setTimeout(() => {
      const last = items.slice(-4).flatMap((i) => {
        const par = parentOf(items).get(i.id)
        return par ? [{ id: i.id }, { id: par }] : [{ id: i.id }]
      })
      fitView(
        follow && last.length
          ? {
              nodes: last,
              duration: 300,
              padding: 0.15,
              minZoom: 0.6,
              maxZoom: 1,
            }
          : { duration: 300, padding: 0.1, maxZoom: 1 }
      )
    }, 250)
    return () => clearTimeout(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [count, filter, follow, fitView])

  const dark =
    typeof document !== "undefined" &&
    document.documentElement.classList.contains("dark")
  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      colorMode={dark ? "dark" : "light"}
      nodesConnectable={false}
      fitView
      fitViewOptions={{ padding: 0.1, maxZoom: 1 }}
      minZoom={0.2}
      proOptions={{ hideAttribution: true }}
    >
      <Background variant={BackgroundVariant.Dots} gap={18} />
      <Controls showInteractive={false} />
    </ReactFlow>
  )
}

function ItemCard({
  item,
  nameOf,
}: {
  item: Item
  nameOf: (a: string) => string
}) {
  const k = KIND[item.kind]
  const Icon = k.icon
  return (
    <article className="animate-in rounded-xl border border-border bg-card/70 p-3 duration-300 fade-in slide-in-from-bottom-1">
      <div className="mb-1.5 flex flex-wrap items-center gap-1.5">
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10.5px] font-medium tracking-wide uppercase",
            k.tone
          )}
        >
          <Icon className="size-3" />
          {k.label}
        </span>
        {item.kind === "task" && (
          <span className="text-[11px] text-muted-foreground">
            proposed, not a card
          </span>
        )}
        <Badge variant="info" className="ml-auto">
          <SparklesIcon /> written by {nameOf(item.by)}
        </Badge>
      </div>
      <p className="text-sm leading-snug">{item.text}</p>
      <div className="mt-1.5 text-[11px] text-muted-foreground">
        {nameOf(item.by)} ({ownerOf(item.by).name}'s assistant) · asked by{" "}
        {personById(item.asker).name} · {WHY[item.why]}
      </div>
    </article>
  )
}

// ---------- after-call view ----------

function AfterCall({
  items,
  nameOf,
  elapsed,
  onRestart,
}: {
  items: Item[]
  nameOf: (a: string) => string
  elapsed: number
  onRestart: () => void
}) {
  const answered = new Set(items.map((i) => i.re).filter(Boolean))
  // What the assistant would propose: tasks, decisions, and questions nobody answered.
  const proposals = items
    .filter(
      (i) =>
        i.kind === "task" ||
        i.kind === "decision" ||
        (i.kind === "question" && !answered.has(i.id))
    )
    .map((i) => ({ id: i.id, kind: i.kind, text: i.text, asker: i.asker }))
  const [working, setWorking] = useState(false)
  const [pick, setPick] = useState<Record<string, boolean>>({})
  const [titles, setTitles] = useState<Record<string, string>>({})
  const [drafted, setDrafted] = useState(false)
  const [made, setMade] = useState<Record<string, boolean>>({})
  const on = (id: string) => pick[id] !== false
  const myName = nameOf("a1")

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-3xl flex-col gap-5 p-4">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-lg font-semibold">Billing import sync</h1>
            <Badge variant="slate">ended · {mmss(elapsed)}</Badge>
            <Badge variant="slate">#suzuri</Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            The huddle is over. Here is the canvas as it ended. No cards or
            tasks were made during the call; nothing starts until you say so.
          </p>
        </div>

        <div className="flex flex-wrap gap-2">
          {!working && (
            <Button onClick={() => setWorking(true)}>
              <BotIcon /> Work with {myName}
            </Button>
          )}
          <Button variant="outline" onClick={onRestart}>
            <RotateCcwIcon /> Play again
          </Button>
        </div>

        {working && (
          <section className="rounded-xl border border-primary/40 bg-primary/5 p-3">
            <h2 className="flex items-center gap-1.5 text-sm font-semibold">
              <BotIcon className="size-4" /> {myName} proposes{" "}
              {proposals.length} cards
            </h2>
            <p className="mb-2 text-xs text-muted-foreground">
              Untick what you don't want, fix a title, then ask {myName} to
              draft them. Nothing is created until you press Make card, and none
              starts on its own.
            </p>
            <ul className="flex flex-col gap-2">
              {proposals.map((p) => (
                <li
                  key={p.id}
                  className="flex items-start gap-2 rounded-lg border border-border bg-card/70 p-2"
                >
                  <input
                    type="checkbox"
                    className="mt-2 size-4"
                    checked={on(p.id)}
                    disabled={drafted}
                    onChange={(e) =>
                      setPick((m) => ({ ...m, [p.id]: e.target.checked }))
                    }
                    aria-label="Include"
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                      <span className="uppercase">
                        {p.kind === "task"
                          ? "card"
                          : p.kind === "decision"
                            ? "decision note"
                            : "follow-up"}
                      </span>
                      · from {personById(p.asker).name}
                    </div>
                    {drafted ? (
                      <div className="text-sm">{titles[p.id] ?? p.text}</div>
                    ) : (
                      <Input
                        value={titles[p.id] ?? p.text}
                        onChange={(e) =>
                          setTitles((m) => ({ ...m, [p.id]: e.target.value }))
                        }
                        className="mt-0.5 h-8 text-sm"
                        aria-label="Card title"
                      />
                    )}
                  </div>
                  {drafted && on(p.id) && (
                    <Button
                      size="sm"
                      variant={made[p.id] ? "ghost" : "outline"}
                      disabled={made[p.id]}
                      onClick={() => setMade((m) => ({ ...m, [p.id]: true }))}
                    >
                      {made[p.id] ? (
                        <>
                          <CheckCircle2Icon /> Would be a card
                        </>
                      ) : (
                        "Make card"
                      )}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
            {!drafted ? (
              <Button className="mt-3" onClick={() => setDrafted(true)}>
                Ask {myName} to draft {proposals.filter((p) => on(p.id)).length}{" "}
                cards
              </Button>
            ) : (
              <p className="mt-3 text-xs text-muted-foreground">
                Drafted. In the real thing Make card would create it on the
                board; here it only ticks.
              </p>
            )}
          </section>
        )}

        <section>
          <h2 className="mb-2 text-sm font-semibold">
            Finished canvas · {items.length} items
          </h2>
          <div className="h-[36rem] overflow-hidden rounded-xl border border-border">
            <ReactFlowProvider>
              <ItemGraph items={items} filter="all" nameOf={nameOf} />
            </ReactFlowProvider>
          </div>
        </section>
        <div className="flex items-center gap-1 pb-4 text-xs text-muted-foreground">
          <MicIcon className="size-3" /> The transcript stays with the huddle.
        </div>
      </div>
    </div>
  )
}
