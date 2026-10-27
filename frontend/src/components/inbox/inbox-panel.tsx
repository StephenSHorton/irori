import { useEffect, useMemo, useRef, useState } from "react"
import {
  ArrowRightIcon,
  ChevronRightIcon,
  InboxIcon,
  PauseIcon,
  PlayIcon,
} from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Textarea } from "@/components/ui/textarea"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  parseTime,
  plainBlock,
  plainLine,
  relativeTime,
  shortPath,
  shortRepo,
  type AgentCard,
} from "@/lib/irori"
import {
  errorText,
  HoldReason,
  isOpen,
  MaxText,
  nameLookup,
  State,
  talk,
  type Delivery,
  type GoneSession,
  type InboxState,
} from "@/lib/talk"

type Filter = "all" | "held" | "open" | "closed"

const filters: Array<{ id: Filter; label: string }> = [
  { id: "all", label: "All" },
  { id: "held", label: "Needs you" },
  { id: "open", label: "Open" },
  { id: "closed", label: "Closed" },
]

function matches(d: Delivery, f: Filter): boolean {
  switch (f) {
    case "held":
      return d.state === State.Held
    case "open":
      return isOpen(d.state)
    case "closed":
      return !isOpen(d.state)
  }
  return true
}

// InboxPanel is every message with its delivery state, the held items that
// wait on the person, and a composer. focusId (a delivery id, from a
// notification's Open) is scrolled to, opened and marked.
export function InboxPanel({
  inbox,
  agents,
  gone,
  now,
  focusId,
}: {
  inbox: InboxState
  agents: AgentCard[]
  gone: GoneSession[]
  now: Date
  focusId?: string
}) {
  const [filter, setFilter] = useState<Filter>("all")
  const { deliveries, policy } = inbox
  const held = deliveries.filter((d) => d.state === State.Held).length
  const name = useMemo(
    () =>
      nameLookup(
        agents.map((a): [string, string] => [a.key, a.name]),
        gone.map((g): [string, string] => [g.key, g.name]),
        deliveries.map((d): [string, string] => [
          d.message.from.key,
          d.message.from.name,
        ])
      ),
    [agents, gone, deliveries]
  )
  const shown = deliveries.filter((d) => matches(d, filter))

  return (
    <div className="flex flex-col gap-4">
      <Composer agents={agents} gone={gone} />

      <div className="flex flex-wrap items-center gap-2">
        <ToggleGroup
          aria-label="Filter messages"
          value={[filter]}
          onValueChange={([v]) => v && setFilter(v)}
        >
          {filters.map((f) => (
            <ToggleGroupItem
              key={f.id}
              value={f.id}
              className="h-7 px-2.5 text-[13px] dark:data-pressed:border-transparent dark:data-pressed:bg-background"
            >
              {f.label}
              {f.id === "held" && held > 0 && (
                <span className="rounded-full bg-warning-foreground px-1.5 text-[11px] text-warning tabular-nums">
                  {held}
                </span>
              )}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <PauseToggle paused={policy?.paused ?? false} />
      </div>

      {policy?.paused && (
        <p className="rounded-lg bg-warning-foreground px-4 py-2 text-sm text-ink-2">
          Wakes are paused. New messages are held until you resume.
        </p>
      )}

      {inbox.error && (
        <p className="rounded-lg bg-destructive/10 px-4 py-2 text-sm text-destructive">
          Could not load the inbox: {inbox.error}
        </p>
      )}

      {inbox.loading ? (
        <p className="px-1 text-sm text-muted-foreground">Reading the inbox…</p>
      ) : shown.length === 0 ? (
        <EmptyInbox filter={filter} />
      ) : (
        <Card size="sm" className="gap-0 py-0!">
          <ul className="divide-y divide-border">
            {shown.map((d) => (
              <DeliveryRow
                key={d.id}
                d={d}
                name={name}
                now={now}
                focused={d.id === focusId}
              />
            ))}
          </ul>
        </Card>
      )}
    </div>
  )
}

function PauseToggle({ paused }: { paused: boolean }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <span className="ml-auto flex items-center gap-2">
      {error && <span className="text-xs text-destructive">{error}</span>}
      <Button
        size="sm"
        variant={paused ? "default" : "outline"}
        aria-pressed={paused}
        disabled={busy}
        onClick={async () => {
          setBusy(true)
          setError(null)
          try {
            await talk.setPaused(!paused)
          } catch (err) {
            setError(errorText(err))
          } finally {
            setBusy(false)
          }
        }}
      >
        {paused ? <PlayIcon aria-hidden /> : <PauseIcon aria-hidden />}
        {paused ? "Resume wakes" : "Pause all wakes"}
      </Button>
    </span>
  )
}

const holdText: Record<string, string> = {
  [HoldReason.HoldBlocked]: "Sender is on the block list",
  [HoldReason.HoldNotAllowed]: "Sender isn't on the allow list",
  [HoldReason.HoldBypassReceiver]:
    "Recipient runs with bypassed permissions and the sender doesn't",
  [HoldReason.HoldPaused]: "Wakes were paused",
  [HoldReason.HoldHopLimit]: "Too many agent-to-agent hops in a row",
  [HoldReason.HoldUnowned]: "No agent in this repo yet; waiting for one",
}

// canAllow: only these holds are lifted by allowing the sender. A block
// outranks the allow list, and a pause or hop limit isn't about the sender.
function canAllow(h: HoldReason | undefined): boolean {
  return h === HoldReason.HoldNotAllowed || h === HoldReason.HoldBypassReceiver
}

function DeliveryRow({
  d,
  name,
  now,
  focused,
}: {
  d: Delivery
  name: (key: string) => string
  now: Date
  focused?: boolean
}) {
  const [open, setOpen] = useState(Boolean(focused))
  const row = useRef<HTMLLIElement>(null)
  useEffect(() => {
    if (focused) row.current?.scrollIntoView({ block: "center" })
  }, [focused])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const m = d.message
  const from =
    plainLine(m.from.kind === "user" ? "you" : m.from.name) || name(m.from.key)
  const to = d.to
    ? plainLine(name(d.to))
    : d.repoKey
      ? d.repoKey.startsWith("root:")
        ? shortRepo(d.repoKey)
        : `repo ${shortRepo(d.repoKey)}`
      : "nobody yet"
  const at = parseTime(d.createdAt)
  const preview = plainLine(m.text)
  const held = d.state === State.Held
  // Unowned repo work waits for a desk; the person can only drop it.
  const unowned = d.hold === HoldReason.HoldUnowned

  const act = (fn: (id: string) => Promise<void>) => async () => {
    setBusy(true)
    setError(null)
    try {
      await fn(d.id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <li
      ref={row}
      className={
        [
          held ? "bg-warning-foreground/35" : "",
          focused ? "ring-2 ring-ring/60 ring-inset" : "",
        ]
          .filter(Boolean)
          .join(" ") || undefined
      }
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="group flex w-full flex-col gap-1 px-4 py-2.5 text-left outline-none focus-visible:bg-muted/60"
      >
        <span className="flex min-w-0 items-center gap-1.5">
          <ChevronRightIcon
            className="size-3.5 shrink-0 text-muted-foreground transition-transform group-aria-expanded:rotate-90"
            aria-hidden
          />
          <span className="truncate font-medium text-ink" title={m.from.key}>
            {from}
          </span>
          <ArrowRightIcon
            className="size-3.5 shrink-0 text-muted-foreground"
            aria-hidden
          />
          <span className="truncate text-ink-2" title={d.to || d.repoKey}>
            {to}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-2">
            <time
              className="text-xs text-muted-foreground tabular-nums"
              dateTime={at?.toISOString()}
              title={at?.toLocaleString()}
            >
              {relativeTime(at, now)}
            </time>
            <StateBadge state={d.state} />
          </span>
        </span>
        <span
          className={
            open ? "hidden" : "truncate pl-5 text-[13px] text-muted-foreground"
          }
        >
          {preview || "(empty message)"}
        </span>
        {held && d.hold && (
          <span className="pl-5 text-xs text-warning">
            {holdText[d.hold] ?? plainLine(d.hold)}
          </span>
        )}
      </button>

      {open && (
        <div className="flex flex-col gap-2 px-4 pb-3 pl-9">
          <div className="max-h-96 overflow-y-auto rounded-lg bg-muted/60 px-3 py-2 font-mono text-[12.5px] leading-relaxed break-words whitespace-pre-wrap text-ink-2">
            {plainBlock(m.text) || "(empty message)"}
          </div>
          <dl className="flex flex-wrap gap-x-4 gap-y-0.5 text-xs text-muted-foreground">
            {m.reason && <Meta label="reason" value={plainLine(m.reason)} />}
            {d.via && <Meta label="via" value={d.via} />}
            <Meta label="hop" value={String(m.hop)} />
            {d.note && <Meta label="note" value={plainLine(d.note)} />}
            {d.for && <Meta label="for" value={plainLine(d.for)} />}
            {d.movedTo && <Meta label="moved to" value={name(d.movedTo)} />}
            <Meta label="id" value={d.id} mono />
          </dl>
        </div>
      )}

      {held && (
        <div className="flex flex-wrap items-center justify-end gap-1.5 px-4 pb-2.5">
          {error && (
            <span className="mr-auto text-xs text-destructive">{error}</span>
          )}
          <Button
            size="xs"
            variant="ghost"
            disabled={busy}
            onClick={act(talk.dismiss)}
          >
            Dismiss
          </Button>
          {canAllow(d.hold) && (
            <Button
              size="xs"
              variant="outline"
              disabled={busy}
              title={`Adds ${m.from.key} to the allow list`}
              onClick={act(talk.allowAndRelease)}
            >
              Allow sender & deliver
            </Button>
          )}
          {!unowned && (
            <Button size="xs" disabled={busy} onClick={act(talk.release)}>
              Deliver
            </Button>
          )}
        </div>
      )}
    </li>
  )
}

function Meta({
  label,
  value,
  mono,
}: {
  label: string
  value: string
  mono?: boolean
}) {
  return (
    <span className="flex gap-1">
      <dt>{label}</dt>
      <dd className={mono ? "font-mono text-ink-2" : "text-ink-2"}>{value}</dd>
    </span>
  )
}

// StateBadge: held is the one signal that wants the person, so it gets the
// warning tone; work in flight is info; finished is quiet.
export function StateBadge({ state }: { state: State }) {
  switch (state) {
    case State.Held:
      return <Badge variant="warning">held</Badge>
    case State.Queued:
      return <Badge variant="slate">queued</Badge>
    case State.Delivered:
    case State.Engaged:
      return <Badge variant="info">{state}</Badge>
    case State.Answered:
      return <Badge variant="success">answered</Badge>
    case State.Deferred:
      return <Badge variant="slate">deferred</Badge>
  }
  return (
    <Badge variant="outline" className="text-muted-foreground">
      {plainLine(state) || "unknown"}
    </Badge>
  )
}

function EmptyInbox({ filter }: { filter: Filter }) {
  return (
    <Card className="items-center px-6 py-10! text-center">
      <InboxIcon className="size-7 text-muted-foreground" aria-hidden />
      <div className="flex max-w-md flex-col gap-1.5">
        <h2 className="text-base font-medium text-ink">
          {filter === "held" ? "Nothing needs you" : "No messages yet"}
        </h2>
        <p className="text-muted-foreground">
          {filter === "held"
            ? "Messages that policy holds back wait here for Deliver or Dismiss."
            : "Messages between agents, and the ones you send, show up here with where each one is."}
        </p>
      </div>
    </Card>
  )
}

// Composer sends a message from the person to one agent or to a repo's
// front desk (the earliest attached live agent in it).
function Composer({
  agents,
  gone,
}: {
  agents: AgentCard[]
  gone: GoneSession[]
}) {
  const [target, setTarget] = useState("")
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [sent, setSent] = useState(false)

  const repos = useMemo(() => {
    const keys = new Set<string>()
    for (const a of agents) if (a.repoKey) keys.add(a.repoKey)
    return [...keys].sort()
  }, [agents])
  const live = useMemo(() => new Set(agents.map((a) => a.key)), [agents])
  const goneOnly = gone.filter((g) => !live.has(g.key))

  const trimmed = text.trim()
  const tooLong = trimmed.length > MaxText
  const valid =
    target === "" ||
    target.startsWith("repo:") ||
    live.has(target.slice(6)) ||
    goneOnly.some((g) => `agent:${g.key}` === target)
  const ready = valid && target !== "" && trimmed !== "" && !tooLong && !busy

  const send = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    setSent(false)
    try {
      const [kind, ...rest] = target.split(":")
      const value = rest.join(":")
      await talk.send(
        kind === "agent" ? value : "",
        kind === "repo" ? value : "",
        trimmed
      )
      setText("")
      setSent(true)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card size="sm" className="gap-2 px-4">
      <div className="flex flex-wrap items-center gap-2">
        <ChoiceSelect
          id="composer-to"
          label="Message"
          showLabel
          value={valid ? target : ""}
          onValueChange={(v) => {
            setTarget(v)
            setSent(false)
          }}
          choices={[
            { value: "", label: "Choose an agent or a repo…" },
            ...(agents.length > 0
              ? [
                  {
                    group: "Agents",
                    choices: agents.map((a) => ({
                      value: `agent:${a.key}`,
                      label: `${plainLine(a.name) || a.sessionId.slice(0, 8)} · ${shortPath(a.cwd)}`,
                    })),
                  },
                ]
              : []),
            ...(repos.length > 0
              ? [
                  {
                    group: "Repos (front desk)",
                    choices: repos.map((r) => ({
                      value: `repo:${r}`,
                      label: shortRepo(r),
                    })),
                  },
                ]
              : []),
            ...(goneOnly.length > 0
              ? [
                  {
                    group: "Gone (waits until it's back)",
                    choices: goneOnly.map((g) => ({
                      value: `agent:${g.key}`,
                      label: plainLine(g.name) || g.sessionId.slice(0, 8),
                    })),
                  },
                ]
              : []),
          ]}
          className="flex-1 sm:max-w-sm"
        />
      </div>
      <Textarea
        aria-label="Message text"
        value={text}
        rows={3}
        placeholder="Write to the agent. It arrives as a message from you."
        onChange={(e) => {
          setText(e.target.value)
          setSent(false)
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault()
            void send()
          }
        }}
        className="min-h-18 bg-white/70 text-sm text-ink dark:bg-white/70"
      />
      <div className="flex items-center gap-3">
        <span
          className={
            tooLong
              ? "text-xs text-destructive tabular-nums"
              : "text-xs text-muted-foreground tabular-nums"
          }
        >
          {trimmed.length}/{MaxText}
        </span>
        {error && <span className="text-xs text-destructive">{error}</span>}
        {sent && !error && (
          <span className="text-xs text-success">Sent. It shows below.</span>
        )}
        <span className="ml-auto flex items-center gap-2">
          <span className="hidden text-xs text-muted-foreground sm:inline">
            Ctrl+Enter
          </span>
          <Button size="sm" disabled={!ready} onClick={() => void send()}>
            {busy ? "Sending…" : "Send"}
          </Button>
        </span>
      </div>
    </Card>
  )
}
