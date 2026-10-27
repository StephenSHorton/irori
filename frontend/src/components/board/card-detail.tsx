import { useMemo, useState, type ReactNode } from "react"
import { LinkIcon, LockIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { ConfirmButton } from "@/components/common/confirm-button"
import { Button } from "@/components/ui/button"
import { Card as Panel } from "@/components/ui/card"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  shortRepo,
  untilTime,
  type AgentCard,
} from "@/lib/irori"
import { errorText } from "@/lib/talk"
import {
  board,
  BlockerKind,
  isTerminal,
  isUser,
  MaxComment,
  MaxReason,
  TaskState,
  tellTargets,
  useTaskDetail,
  waitsOnPerson,
  type Card,
  type Comment,
  type LeaseCard,
  type Task,
  type Tell,
  type Told,
} from "@/lib/board"
import {
  blockedLabel,
  displayResource,
  fieldClass,
  LinkChip,
  Pill,
  stateLabel,
  textareaClass,
} from "./bits"

type Names = (key: string) => string

// CardDetail is one card: its facts and links, the person's actions, the
// comments, and a composer with the per-comment "tell the agent" choice
// (decision 18).
export function CardDetail({
  card,
  agents,
  name,
  title,
  now,
}: {
  card: Card
  agents: AgentCard[]
  name: Names
  title: (id: string) => string
  now: Date
}) {
  const { detail, error } = useTaskDetail(card.id)
  // The Board's card is current as of the last board:changed; the detail
  // read may lag it by one event, so the card's own fields win.
  const t: Task = card
  const comments = detail?.comments ?? []
  const open = !isTerminal(t.state)

  return (
    <Panel size="sm" className="gap-0 py-0!">
      <div className="flex flex-col gap-2.5 border-b px-4 pt-4 pb-3">
        <div className="flex flex-wrap items-center gap-1.5">
          <StatePill t={card} />
          {t.shared && <Pill tone="shared">Shared</Pill>}
        </div>
        <h2 className="text-lg leading-snug font-semibold text-balance [overflow-wrap:anywhere] text-ink">
          {plainLine(t.title) || t.id}
        </h2>
        <Facts t={card} name={name} title={title} now={now} />
        <Links t={t} />
        {open && <Actions t={card} agents={agents} name={name} />}
      </div>

      {error && (
        <p className="mx-4 mt-3 rounded-lg bg-destructive/10 px-3 py-1.5 text-xs text-destructive">
          Could not load the comments: {error}
        </p>
      )}
      <Thread comments={comments} name={name} now={now} loading={!detail} />
      <Compose t={t} agents={agents} name={name} />
    </Panel>
  )
}

function StatePill({ t }: { t: Card }) {
  if (t.state === TaskState.Blocked) {
    if (t.resolved) return <Pill tone="done">Blocker cleared</Pill>
    return <Pill tone={waitsOnPerson(t) ? "you" : "block"}>Blocked</Pill>
  }
  if (t.state === TaskState.Completed) return <Pill tone="done">Done</Pill>
  if (isTerminal(t.state))
    return <Pill tone="plain">{stateLabel(t.state)}</Pill>
  if (t.state === TaskState.InputRequired)
    return <Pill tone="you">Needs your input</Pill>
  return <Pill tone="task">{stateLabel(t.state)}</Pill>
}

function Facts({
  t,
  name,
  title,
  now,
}: {
  t: Card
  name: Names
  title: (id: string) => string
  now: Date
}) {
  const assigned = new Set(t.assigned ?? [])
  const holders = t.holders ?? []
  const updated = parseTime(t.updatedAt)
  const rows: Array<[string, ReactNode]> = []
  if (t.state === TaskState.Blocked) {
    rows.push(["Waiting on", blockedLabel(t.blockedBy, name, title)])
    if (t.reason) rows.push(["Why", plainBlock(t.reason)])
  }
  if (t.repoKey)
    rows.push([
      "Repo",
      <span className="font-mono text-xs" title={t.repoKey}>
        {shortRepo(t.repoKey)}
      </span>,
    ])
  rows.push(["Posted by", plainLine(name(t.createdBy))])
  rows.push([
    t.shared ? "Working on it" : "Holder",
    holders.length === 0 ? (
      <span className="text-muted-foreground">nobody yet</span>
    ) : (
      holders
        .map(
          (h) =>
            plainLine(name(h)) +
            (assigned.has(h) ? " (assigned, not picked up yet)" : "")
        )
        .join(", ")
    ),
  ])
  if (t.objective) rows.push(["Objective", plainBlock(t.objective)])
  if (t.doneCriteria) rows.push(["Done when", plainBlock(t.doneCriteria)])
  if (t.note) rows.push(["Note", plainBlock(t.note)])
  if (t.overlaps?.length)
    rows.push([
      "Overlapping leases",
      <span className="text-warning">
        {t.overlaps.map(plainLine).join("; ")}
      </span>,
    ])
  rows.push([
    "Updated",
    <time dateTime={updated?.toISOString()} title={updated?.toLocaleString()}>
      {relativeTime(updated, now)}
    </time>,
  ])
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-[13px]">
      {rows.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="text-muted-foreground">{k}</dt>
          <dd className="min-w-0 [overflow-wrap:anywhere] whitespace-pre-wrap text-ink-2">
            {v}
          </dd>
        </div>
      ))}
    </dl>
  )
}

function Links({ t }: { t: Task }) {
  const [adding, setAdding] = useState(false)
  const [value, setValue] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const links = t.links ?? []

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    try {
      await fn()
      return true
    } catch (err) {
      setError(errorText(err))
      return false
    } finally {
      setBusy(false)
    }
  }

  const add = async () => {
    if (!value.trim() || busy) return
    if (await run(() => board.addLink(t.id, value.trim()))) {
      setValue("")
      setAdding(false)
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-1.5">
        {links.map((l) => (
          <LinkChip
            key={l.ref}
            link={l}
            onRemove={() => void run(() => board.removeLink(t.id, l.ref))}
          />
        ))}
        {!adding && links.length < 16 && (
          <Button size="xs" variant="ghost" onClick={() => setAdding(true)}>
            <LinkIcon aria-hidden />
            Add link
          </Button>
        )}
      </div>
      {adding && (
        <div className="flex flex-wrap items-center gap-1.5">
          <input
            autoFocus
            aria-label="Link"
            value={value}
            placeholder="A Linear, GitHub or Notion URL, a repo path or a commit"
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void add()
              if (e.key === "Escape") setAdding(false)
            }}
            className={cn(fieldClass, "h-7 flex-1 text-[13px]")}
          />
          <Button
            size="xs"
            disabled={!value.trim() || busy}
            onClick={() => void add()}
          >
            Add
          </Button>
          <Button size="xs" variant="ghost" onClick={() => setAdding(false)}>
            Cancel
          </Button>
        </div>
      )}
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  )
}

// Actions are what the person can do to an open card: close its blocker or
// block it, assign it to an agent, mark it done or cancel it.
function Actions({
  t,
  agents,
  name,
}: {
  t: Card
  agents: AgentCard[]
  name: Names
}) {
  const [blocking, setBlocking] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [said, setSaid] = useState<string | null>(null)
  const holders = new Set(t.holders ?? [])
  const assignable = agents.filter((a) => !holders.has(a.key))

  const run = async (fn: () => Promise<string | void>) => {
    setBusy(true)
    setError(null)
    setSaid(null)
    try {
      const s = await fn()
      if (s) setSaid(s)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-1.5">
        {t.state === TaskState.Blocked ? (
          <Button
            size="sm"
            variant={waitsOnPerson(t) ? "default" : "outline"}
            disabled={busy}
            title="Anyone may close a blocker. Its holders hear at their next turn."
            onClick={() =>
              void run(async () => {
                const r = await board.unblock(t.id, "")
                return toldText("Unblocked.", r.told, name)
              })
            }
          >
            {t.resolved ? "Clear the blocker" : "Unblock"}
          </Button>
        ) : (
          <Button
            size="sm"
            variant="outline"
            aria-expanded={blocking}
            disabled={busy}
            onClick={() => setBlocking(!blocking)}
          >
            Block…
          </Button>
        )}
        {assignable.length > 0 && (
          <ChoiceSelect
            label="Assign to an agent"
            size="sm"
            value=""
            disabled={busy}
            onValueChange={(to) => {
              if (!to) return
              void run(async () => {
                const r = await board.assign(t.id, to)
                return toldText(
                  `Assigned to ${plainLine(name(to))}.`,
                  r.told,
                  name
                )
              })
            }}
            choices={[
              { value: "", label: t.shared ? "Add an agent…" : "Assign to…" },
              ...assignable.map((a) => ({
                value: a.key,
                label: `${plainLine(a.name) || a.sessionId.slice(0, 8)}${a.status === "busy" ? " (busy)" : ""}`,
              })),
            ]}
            className="max-w-52 text-[13px]"
          />
        )}
        <span className="ml-auto flex items-center gap-1.5">
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await board.setDone(t.id, false)
              })
            }
          >
            Mark done
          </Button>
          <ConfirmButton
            label="Cancel"
            question="Cancel this card?"
            confirmLabel="Cancel card"
            variant="destructive"
            disabled={busy}
            onConfirm={() => board.setDone(t.id, true)}
          />
        </span>
      </div>
      {blocking && t.state !== TaskState.Blocked && (
        <BlockForm t={t} agents={agents} onDone={() => setBlocking(false)} />
      )}
      {error && <span className="text-xs text-destructive">{error}</span>}
      {said && !error && <span className="text-xs text-success">{said}</span>}
    </div>
  )
}

// blockTarget encodes BlockForm's "on what" choice.
function parseOn(v: string): { on: BlockerKind; ref: string } {
  if (v === "person") return { on: BlockerKind.BlockedOnPerson, ref: "" }
  if (v.startsWith("agent:"))
    return { on: BlockerKind.BlockedOnAgent, ref: v.slice(6) }
  if (v === "external") return { on: BlockerKind.BlockedOnExternal, ref: "" }
  return { on: "" as BlockerKind, ref: "" }
}

function BlockForm({
  t,
  agents,
  onDone,
}: {
  t: Task
  agents: AgentCard[]
  onDone: () => void
}) {
  const [reason, setReason] = useState("")
  const [on, setOn] = useState("person")
  const [external, setExternal] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const trimmed = reason.trim()

  const submit = async () => {
    if (!trimmed || busy) return
    setBusy(true)
    setError(null)
    try {
      const b = parseOn(on)
      await board.block(
        t.id,
        trimmed,
        b.on,
        on === "external" ? external.trim() : b.ref
      )
      onDone()
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="flex flex-col gap-2 rounded-lg bg-muted/60 p-2.5">
      <input
        autoFocus
        aria-label="What's in the way"
        value={reason}
        maxLength={MaxReason}
        placeholder="What's in the way"
        onChange={(e) => setReason(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void submit()
        }}
        className={cn(fieldClass, "w-full")}
      />
      <div className="flex flex-wrap items-center gap-1.5">
        <ChoiceSelect
          label="Waiting on"
          size="sm"
          value={on}
          onValueChange={setOn}
          choices={[
            { value: "person", label: "Waiting on: you" },
            ...agents.map((a) => ({
              value: `agent:${a.key}`,
              label: `Waiting on: ${plainLine(a.name) || a.sessionId.slice(0, 8)}`,
            })),
            { value: "external", label: "Waiting on: something outside Irori" },
            { value: "", label: "Nothing named" },
          ]}
          className="text-[13px]"
        />
        {on === "external" && (
          <input
            aria-label="What, outside Irori"
            value={external}
            placeholder="CI, a review, a URL"
            onChange={(e) => setExternal(e.target.value)}
            className={cn(fieldClass, "h-7 flex-1 text-[13px]")}
          />
        )}
        <span className="ml-auto flex gap-1.5">
          <Button size="xs" variant="ghost" onClick={onDone}>
            Cancel
          </Button>
          <Button
            size="xs"
            disabled={!trimmed || busy}
            onClick={() => void submit()}
          >
            Block
          </Button>
        </span>
      </div>
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  )
}

function initials(label: string): string {
  const words = label
    .replace(/[^\p{L}\p{N} -]/gu, " ")
    .split(/[\s-]+/)
    .filter(Boolean)
  if (words.length === 0) return "·"
  if (words.length === 1) return words[0].slice(0, 2)
  return (words[0][0] + words[1][0]).toLowerCase()
}

function Thread({
  comments,
  name,
  now,
  loading,
}: {
  comments: Comment[]
  name: Names
  now: Date
  loading: boolean
}) {
  if (loading) {
    return (
      <p className="px-4 py-3 text-[13px] text-muted-foreground">
        Reading comments…
      </p>
    )
  }
  if (comments.length === 0) {
    return (
      <p className="px-4 py-3 text-[13px] text-muted-foreground">
        No comments yet.
      </p>
    )
  }
  return (
    <ol className="flex flex-col px-4 py-1">
      {comments.map((c) => {
        const person = isUser(c.by)
        const who = plainLine(name(c.by))
        const at = parseTime(c.at)
        return (
          <li
            key={c.id}
            className="grid grid-cols-[28px_minmax(0,1fr)] gap-2.5 border-t border-border py-2.5 first:border-t-0"
          >
            <span
              aria-hidden
              className={cn(
                "grid size-7 place-items-center rounded-lg text-[11px] font-semibold",
                person
                  ? "bg-primary text-primary-foreground"
                  : "bg-info-foreground font-mono text-info"
              )}
            >
              {person ? "you" : initials(who)}
            </span>
            <div className="min-w-0">
              <div className="flex flex-wrap items-baseline gap-2 text-[12.5px]">
                <b className="text-[13px] font-semibold text-ink" title={c.by}>
                  {person ? "You" : who}
                </b>
                <time
                  className="text-muted-foreground"
                  dateTime={at?.toISOString()}
                  title={at?.toLocaleString()}
                >
                  {relativeTime(at, now)}
                </time>
                {c.wake && (
                  <span className="text-[11.5px] text-warning">
                    woke the agent
                  </span>
                )}
              </div>
              <div className="mt-0.5 text-sm [overflow-wrap:anywhere] whitespace-pre-wrap text-ink-2">
                {plainBlock(c.text)}
              </div>
            </div>
          </li>
        )
      })}
    </ol>
  )
}

const tellOptions: Array<{ id: Tell; label: string }> = [
  { id: "next_turn", label: "at its next turn" },
  { id: "wake", label: "now (wake it)" },
  { id: "none", label: "don't" },
]

// Compose writes a comment as the person. Who hears it is the card's agent
// holders (else the agent that posted it), and when is the person's choice:
// at its next turn by default, now as a wake, or not at all.
function Compose({
  t,
  agents,
  name,
}: {
  t: Task
  agents: AgentCard[]
  name: Names
}) {
  const [text, setText] = useState("")
  const [tell, setTell] = useState<Tell>("next_turn")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [said, setSaid] = useState<string | null>(null)
  const targets = tellTargets(t)
  const answering = waitsOnPerson(t)
  const trimmed = text.trim()
  const tooLong = trimmed.length > MaxComment
  const ready = trimmed !== "" && !tooLong && !busy
  const busyOnes = useMemo(() => {
    const live = new Map(agents.map((a) => [a.key, a]))
    return targets.filter((k) => live.get(k)?.status === "busy")
  }, [agents, targets])
  const who = targets.map((k) => plainLine(name(k))).join(", ")

  const send = async (unblock: boolean) => {
    if (!ready) return
    setBusy(true)
    setError(null)
    setSaid(null)
    try {
      const r = await board.comment(
        t.id,
        trimmed,
        targets.length ? tell : "none"
      )
      let s = toldText("Comment added.", r.told, name)
      if (unblock) {
        await board.unblock(t.id, "")
        s = toldText("Answered and unblocked.", r.told, name)
      }
      setText("")
      setSaid(s)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  let hint: ReactNode
  if (targets.length === 0) {
    hint = "No agent is on this card, so the comment stays on it."
  } else if (tell === "next_turn") {
    hint = `${who} ${targets.length === 1 ? "sees" : "see"} it at the start of the next turn: no extra turn.`
  } else if (tell === "wake") {
    hint = (
      <>
        Starts a turn now: a full turn that re-reads {who}'s context.
        {busyOnes.length > 0 && (
          <b className="font-medium text-warning">
            {" "}
            {busyOnes.map((k) => plainLine(name(k))).join(", ")}{" "}
            {busyOnes.length === 1 ? "is" : "are"} busy now, so the wake waits
            for the current turn to end.
          </b>
        )}
      </>
    )
  } else {
    hint = "Stays on the card. The agent sees it only if it opens the card."
  }

  return (
    <div className="flex flex-col gap-2 border-t px-4 pt-3 pb-4">
      <Textarea
        aria-label="Comment"
        value={text}
        rows={3}
        placeholder={answering ? "Your answer" : "Comment as you"}
        onChange={(e) => {
          setText(e.target.value)
          setSaid(null)
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault()
            void send(false)
          }
        }}
        className={cn(textareaClass, "min-h-18")}
      />
      {targets.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 text-[12.5px]">
          <span className="text-muted-foreground">Tell the agent</span>
          <RadioGroup
            variant="chips"
            name="tell"
            aria-label="When the agent hears it"
            value={tell}
            onValueChange={(v) => setTell(v as Tell)}
            className="w-auto"
          >
            {tellOptions.map((o) => (
              <RadioGroupItem
                key={o.id}
                value={o.id}
                className="h-6 px-2.5 text-[12.5px]"
              >
                {o.label}
              </RadioGroupItem>
            ))}
          </RadioGroup>
        </div>
      )}
      <p className="text-xs text-muted-foreground">{hint}</p>
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={
            tooLong
              ? "text-xs text-destructive tabular-nums"
              : "text-xs text-muted-foreground tabular-nums"
          }
        >
          {trimmed.length}/{MaxComment}
        </span>
        {error && <span className="text-xs text-destructive">{error}</span>}
        {said && !error && <span className="text-xs text-success">{said}</span>}
        <span className="ml-auto flex items-center gap-2">
          {answering && (
            <Button
              size="sm"
              variant="outline"
              disabled={!ready}
              onClick={() => void send(true)}
            >
              Answer & unblock
            </Button>
          )}
          <Button size="sm" disabled={!ready} onClick={() => void send(false)}>
            {busy ? "Posting…" : "Comment"}
          </Button>
        </span>
      </div>
    </div>
  )
}

// toldText is a short line on who heard an action.
function toldText(head: string, told: Told, name: Names): string {
  const to = (told.to ?? []).map((k) => plainLine(name(k))).join(", ")
  let s = head
  if (to && told.mode === "wake") s += ` ${to} wakes with it.`
  else if (to) s += ` ${to} hears at the next turn.`
  if (told.error) s += ` Not everyone was reached: ${told.error}`
  return s
}

// LeaseDetail is a lease card: what it holds, what it blocks, who it
// turned away, and Release.
export function LeaseDetail({
  l,
  name,
  title,
  now,
  onBlocked,
}: {
  l: LeaseCard
  name: Names
  title: (id: string) => string
  now: Date
  onBlocked: (taskId: string) => void
}) {
  const until = parseTime(l.until)
  const since = parseTime(l.acquiredAt)
  return (
    <Panel size="sm" className="gap-0 py-0!">
      <div className="flex flex-col gap-2.5 px-4 pt-4 pb-4">
        <div className="flex items-center gap-1.5">
          <Pill tone="block">Lease</Pill>
        </div>
        <h2 className="flex items-center gap-2 font-mono text-base font-semibold [overflow-wrap:anywhere] text-ink">
          <LockIcon className="size-4 shrink-0 text-warning" aria-hidden />
          {displayResource(plainLine(l.resource))}
        </h2>
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-[13px]">
          <dt className="text-muted-foreground">Held by</dt>
          <dd className="text-ink-2">{plainLine(name(l.holder))}</dd>
          <dt className="text-muted-foreground">Since</dt>
          <dd className="text-ink-2">{relativeTime(since, now)}</dd>
          <dt className="text-muted-foreground">Expires</dt>
          <dd className="text-ink-2">{untilTime(until, now)}</dd>
          {l.stolenFrom && (
            <>
              <dt className="text-muted-foreground">Taken over from</dt>
              <dd className="text-ink-2">{plainLine(name(l.stolenFrom))}</dd>
            </>
          )}
        </dl>
        {(l.blocks?.length ?? 0) > 0 && (
          <div className="flex flex-col gap-1 text-[13px]">
            <span className="text-muted-foreground">Blocks</span>
            {l.blocks!.map((id) => (
              <button
                key={id}
                type="button"
                onClick={() => onBlocked(id)}
                className="self-start text-left text-ink underline-offset-2 hover:underline"
              >
                {title(id) || id}
              </button>
            ))}
          </div>
        )}
        {(l.refused?.length ?? 0) > 0 && (
          <div className="flex flex-col gap-1 text-[13px]">
            <span className="text-muted-foreground">Turned away</span>
            {l.refused!.map((r) => (
              <span key={`${r.by}-${r.resource}`} className="text-ink-2">
                {plainLine(name(r.by))} asked for{" "}
                <span className="font-mono text-xs">
                  {displayResource(plainLine(r.resource))}
                </span>{" "}
                {relativeTime(parseTime(r.at), now)}
              </span>
            ))}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          Leases are advisory claims on files or named resources. Releasing one
          lets the next agent take it; blocked tasks waiting on it clear on
          their own.
        </p>
        <div className="flex justify-end">
          <ConfirmButton
            label="Release"
            question="Release this lease?"
            confirmLabel="Release"
            variant="destructive"
            onConfirm={() => board.releaseLease(l.resource)}
          />
        </div>
      </div>
    </Panel>
  )
}
