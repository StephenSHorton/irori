// AgentsPane is Settings › Agents (card #448, docs/design/spawned-agents.md
// §10, mockup sa-6), as much of it as part 4 needs: your own board's
// switch for agents Irori starts per card, and the model and effort they
// start with, the idle limit, the agents running now, what they may do
// and how each repo's work reaches main, and which of this computer's
// repos are trusted for agents (card #456); and every limit on them (cards
// #450, #575, #576): how many may run on each device, the idle limit and the
// crashes and failed starts before Irori asks, and how many your agents
// may start, with today's count and cost and a Claude usage limit holding
// the queue; and the theme new agents' names come from (card #702).
// Only you set these, here; no agent's tool can.

import { useContext, useId, useState, type ReactNode } from "react"
import { FolderIcon, ShieldAlertIcon, ShieldCheckIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { PermissionsSection } from "@/components/settings/agent-permissions"
import {
  AGENT_STARTS_HINT,
  AGENT_STARTS_TITLE,
  AgentStartsControls,
  WholeNumberField,
} from "@/components/settings/agent-starts"
import { NamesThemesSection } from "@/components/settings/names-themes"
import { SpaceAgentSwitch } from "@/components/settings/space-agents"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Badge } from "@/components/ui/badge"
import { Loading } from "@/components/ui/loading"
import {
  TRIES_CHOICES,
  costLabel,
  countLabel,
  effortLabel,
  endAgent,
  idleLabel,
  useLimits,
  modelLabel,
  runningSince,
  useAgentStarts,
  useAgents,
  useRunning,
  useTrusted,
  withChoice,
  type AgentsChange,
  type AgentsSettings,
  type RunningAgent,
  type TrustedRepo,
} from "@/lib/agents"
import { usageResets } from "@/lib/spawn-line"
import { ConfirmButton } from "@/components/common/confirm-button"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import { agentStatus } from "@/lib/agent-status"
import { useBoards } from "@/lib/boards"
import { plainLine } from "@/lib/irori"
import { LiveRoster } from "@/lib/thread"
import { AgentRef } from "@/lib/models"

export function AgentsPane({ open }: { open: boolean }) {
  const { settings: s, error, loading, set } = useAgents(open)
  const id = useId()
  // Running, permissions and trusted repos read on their own, beside the
  // settings rather than after them, each with its own skeleton (card #598).
  return (
    <div className="grid gap-4">
      <Loading
        loading={loading}
        label="Loading agent settings"
        error={
          !s &&
          error && (
            <p className="text-muted-foreground">Could not load: {error}</p>
          )
        }
      >
        {s ? (
          <div className="grid gap-4">
            <div
              className="flex items-start justify-between gap-3"
              data-setting="agents.each-card"
            >
              <span className="flex flex-col">
                <label htmlFor={id} className="cursor-pointer text-ink">
                  Start an agent for each card
                </label>
                <span
                  id={`${id}-hint`}
                  className="text-xs text-muted-foreground"
                >
                  In Personal, Irori starts a fresh agent for every card you
                  post instead of waiting for a free one, and ends it when the
                  card is done.
                </span>
              </span>
              <Switch
                id={id}
                checked={s.on}
                aria-describedby={`${id}-hint`}
                onCheckedChange={(v) => set({ on: v })}
                className="mt-0.5"
              />
            </div>
            <SpacesSection open={open} />
            <section className="grid gap-2" data-setting="agents.model">
              <h3 className="text-sm font-medium text-ink">
                What a new agent starts with
              </h3>
              <p className="text-[12.5px] text-muted-foreground">
                Every agent Irori starts uses these. One already working keeps
                what it started with.
              </p>
              <div className="grid gap-2 sm:grid-cols-2">
                <ChoiceSelect
                  label="Model"
                  showLabel
                  size="sm"
                  value={s.model}
                  choices={s.models.map((m) => ({
                    value: m,
                    label: modelLabel(m, s.defaultModel),
                  }))}
                  onValueChange={(v) => set({ model: v })}
                  className="text-[13px]"
                />
                <ChoiceSelect
                  label="Effort"
                  showLabel
                  size="sm"
                  value={s.effort}
                  choices={s.efforts.map((e) => ({
                    value: e,
                    label: effortLabel(e, s.defaultEffort),
                  }))}
                  onValueChange={(v) => set({ effort: v })}
                  className="text-[13px]"
                />
              </div>
            </section>
            <NamesThemesSection open={open} />
            <LimitsSection open={open} settings={s} set={set} />
          </div>
        ) : (
          <AgentsShape />
        )}
      </Loading>
      <RunningSection open={open} />
      <PermissionsSection open={open} />
      <TrustedSection open={open} />
      {s && error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}

// SpacesSection is each shared space's own switch (card #573): off until
// the person turns it on, and only for cards they make there.
function SpacesSection({ open }: { open: boolean }) {
  const { boards } = useBoards()
  if (boards.length === 0) return null
  return (
    <section
      className="grid gap-2"
      aria-label="Spaces"
      data-setting="agents.spaces"
    >
      <h3 className="text-sm font-medium text-ink">Spaces</h3>
      <p className="text-[12.5px] text-muted-foreground">
        Each space has its own switch, off until you turn it on. It runs your
        agents on your computers, for the cards you make there. A card someone
        gives you always asks you first, whatever the switch says.
      </p>
      <ul className="grid gap-2">
        {boards.map((b) => (
          <li key={b.boardId}>
            <SpaceAgentSwitch
              boardId={b.boardId}
              name={plainLine(b.name) || "(unnamed)"}
              open={open}
            />
          </li>
        ))}
      </ul>
    </section>
  )
}

// LimitRow is one limit: what it is, what it does, and its control. Every
// limit on agents is one row here (card #576), so a new one is one more
// row in LimitsSection's list; "My agents may start agents" (card #575)
// goes there too.
function LimitRow({
  title,
  hint,
  children,
}: {
  title: string
  hint: ReactNode
  children: ReactNode
}) {
  return (
    <li className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
      <span className="flex min-w-0 flex-1 basis-56 flex-col">
        <span className="truncate text-[13px] text-ink">{title}</span>
        <span className="text-xs text-muted-foreground">{hint}</span>
      </span>
      {children}
    </li>
  )
}

// LimitsSection is every limit on the agents Irori starts: how many run at
// once on each device, and below that the idle limit and the crashes and
// failed starts in a row, with today's count and cost and a usage limit
// holding the queue (cards #450, #576, sa-6). Over a device's cap a card queues and says its place; a
// Claude usage limit queues every card until it resets. A change holds from
// the next start, with no restart; agents already running keep going.
function LimitsSection({
  open,
  settings: s,
  set,
}: {
  open: boolean
  settings: AgentsSettings
  set: (c: Omit<AgentsChange, "boardId">) => void
}) {
  const { limits: l, error, setCap, releaseHold } = useLimits(open)
  return (
    <section className="grid gap-2" data-setting="agents.limits">
      <h3 className="text-sm font-medium text-ink">Limits</h3>
      {l ? (
        <>
          <p className="text-[12.5px] text-muted-foreground">
            {l.today === 0
              ? "No agents started today."
              : `${l.today} agent${l.today === 1 ? "" : "s"} started today${
                  l.todayHere !== l.today
                    ? `, ${l.todayHere} on this computer`
                    : ""
                }, using about ${costLabel(l.costUsd)} at API prices${
                  l.todayHere !== l.today ? " here" : ""
                }. On a Claude plan that is plan usage, not a bill.`}
            {l.queued > 0 && ` ${l.queued} queued for a slot.`}
          </p>
          {l.heldUntil != null && (
            <div
              role="status"
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-[12.5px] text-ink"
            >
              <span className="min-w-0 flex-1 basis-56">
                A Claude usage limit is holding new agents on this computer (
                {usageResets(new Date(l.heldUntil).toISOString())}). Queued
                cards start then; agents that stopped on it aren&apos;t counted
                as crashes.
              </span>
              <Button
                size="sm"
                variant="outline"
                title="Starts queued cards now. If the limit still stands, Irori holds again at the first one."
                onClick={releaseHold}
              >
                Try now
              </Button>
            </div>
          )}
          <ul className="grid divide-y divide-border rounded-md border border-border">
            {l.devices.map((d) => (
              <LimitRow
                key={d.device}
                title={d.name}
                hint={
                  <span className="tabular-nums">
                    {d.cap > 0
                      ? `${d.running} of ${d.cap} running`
                      : `${d.running} running · starts no more`}
                  </span>
                }
              >
                {/* A whole number from 0 (starts no more there) to
                    l.maxCap, saved on Enter or blur (card #703). */}
                <WholeNumberField
                  label="At most"
                  ariaLabel={`At most on ${d.name}`}
                  value={d.cap}
                  ceiling={l.maxCap}
                  onSave={(n) => setCap(d.device, n)}
                />
              </LimitRow>
            ))}
          </ul>
        </>
      ) : (
        <p className="text-[12.5px] text-muted-foreground">
          {error ? `Could not load: ${error}` : "Loading…"}
        </p>
      )}
      <ul className="grid divide-y divide-border rounded-md border border-border">
        <LimitRow
          title="Idle limit"
          hint="An agent that waits this long on a question for you writes where it left off and goes. Your answer starts a fresh one."
        >
          <ChoiceSelect
            label="Idle limit"
            size="sm"
            value={String(s.idleMins)}
            choices={s.idleChoices.map((m) => ({
              value: String(m),
              label: idleLabel(m),
            }))}
            onValueChange={(v) => set({ idleMins: Number(v) })}
            className="w-44 text-[13px]"
          />
        </LimitRow>
        <LimitRow
          title="Crashes in a row"
          hint="After this many of a card's agents go away before finishing, Irori asks you before starting another. Each computer has its own number, for the cards it hands out."
        >
          <ChoiceSelect
            label="Crashes in a row"
            size="sm"
            value={String(s.crashLimit)}
            choices={withChoice(TRIES_CHOICES, s.crashLimit).map((n) => ({
              value: String(n),
              label: countLabel(n, s.defaultCrashLimit, "crash", "crashes"),
            }))}
            onValueChange={(v) => set({ crashLimit: Number(v) })}
            className="w-44 text-[13px]"
          />
        </LimitRow>
        <LimitRow
          title="Failed starts in a row"
          hint="After this many tries that couldn't start an agent for a card, Irori asks you before trying again."
        >
          <ChoiceSelect
            label="Failed starts in a row"
            size="sm"
            value={String(s.startTries)}
            choices={withChoice(TRIES_CHOICES, s.startTries).map((n) => ({
              value: String(n),
              label: countLabel(n, s.defaultStartTries, "try", "tries"),
            }))}
            onValueChange={(v) => set({ startTries: Number(v) })}
            className="w-44 text-[13px]"
          />
        </LimitRow>
        <AgentStartsRow open={open} />
      </ul>
      {error && l && <p className="text-[12.5px] text-destructive">{error}</p>}
    </section>
  )
}

// AgentStartsRow is "My agents may start agents on my cards" (card #575):
// the switch and the maximum, with how much of it is in use now.
function AgentStartsRow({ open }: { open: boolean }) {
  const { starts: s, error, set } = useAgentStarts(open)
  if (!s) return null
  return (
    <LimitRow
      title={AGENT_STARTS_TITLE}
      hint={
        <>
          {AGENT_STARTS_HINT}{" "}
          <span className="tabular-nums">
            {s.on ? `${s.used} of ${s.max} in use.` : `Off; ${s.used} in use.`}
          </span>
          {error && <span className="text-destructive"> {error}</span>}
        </>
      }
    >
      <AgentStartsControls starts={s} set={set} />
    </LimitRow>
  )
}

// RunningSection lists every agent Irori started that is still running,
// and the cards one is starting for (card #448): its card, device, model,
// Logs and End (card #456). Attach is on its card, where the terminal is.
function RunningSection({ open }: { open: boolean }) {
  const { rows, error, loading, refresh } = useRunning(open)
  const [now] = useState(() => Date.now())
  return (
    <section className="grid gap-2" data-setting="agents.running">
      <h3 className="text-sm font-medium text-ink">
        Running agents{rows.length > 0 && ` · ${rows.length}`}
      </h3>
      <p className="text-[12.5px] text-muted-foreground">
        Agents Irori started for your cards. End asks one to write where it left
        off on its card, then stops it.
      </p>
      <Loading loading={loading} label="Loading running agents">
        {loading ? (
          <RunningShape />
        ) : rows.length === 0 ? (
          <p className="text-[12.5px] text-muted-foreground">
            {error ? `Could not load: ${error}` : "None running."}
          </p>
        ) : (
          <ul className="grid divide-y divide-border rounded-md border border-border">
            {rows.map((r) => (
              <RunningRow
                key={r.key || `starting-${r.cardId}`}
                r={r}
                now={now}
                onEnded={refresh}
              />
            ))}
          </ul>
        )}
      </Loading>
    </section>
  )
}

function RunningRow({
  r,
  now,
  onEnded,
}: {
  r: RunningAgent
  now: number
  onEnded: () => void
}) {
  const [ending, setEnding] = useState(false)
  const starting = r.status === "starting"
  // The same words as everywhere else (card #489); the live roster knows
  // when a job began.
  const since = useContext(LiveRoster).get(r.key)?.statusSince
  const state = agentStatus(r, since, now)
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-[13px]">
      <span className="grid min-w-0 gap-0.5">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate font-medium text-ink">
            {starting ? "Starting…" : r.name}
          </span>
          <Badge
            variant={
              state.tone === "muted"
                ? "info"
                : state.tone === "working" || state.tone === "job"
                  ? "success"
                  : state.tone === "warn"
                    ? "warning"
                    : "slate"
            }
            className="h-4 px-1.5 text-[10.5px]"
          >
            {state.text}
          </Badge>
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {r.cardShort} {r.cardTitle}
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {r.device} · {r.model} · {r.effort} ·{" "}
          {runningSince(r.startedAt ?? now, now)}
        </span>
      </span>
      {ending ? (
        <span className="text-xs text-muted-foreground">ending</span>
      ) : starting ? null : (
        <span className="flex items-center gap-1.5">
          {r.here ? (
            <SpawnedActions
              who={AgentRef({
                key: r.key,
                name: r.name,
                status: r.status,
                live: true,
                where: r.device,
                here: true,
              })}
              cardId={r.cardId}
              only={["logs"]}
            />
          ) : (
            <span
              className="text-xs text-muted-foreground"
              title="Logs are on the device it runs on"
            >
              on {r.device}
            </span>
          )}
          <ConfirmButton
            label="End"
            question={
              <>
                End {r.name} on {r.cardShort}?{" "}
                {r.here ? "" : `Irori asks ${r.device} to end it. `}It writes
                where it left off on the card first, then Irori stops it and
                removes its session.
              </>
            }
            confirmLabel="End"
            onConfirm={async () => {
              await endAgent(r.key)
              setEnding(true)
              onEnded()
            }}
          />
        </span>
      )}
    </li>
  )
}

// TrustedSection is "Trusted repos for agents" (card #456, sa-6, §7
// Trust): this computer's repos and whether Claude Code trusts each, so an
// agent Irori starts can run there. Trust… records it after you confirm,
// as Trust and start in Needs you does; no agent or tool can. Each of your
// other computers lists its own.
function TrustedSection({ open }: { open: boolean }) {
  const { rows, error, loading, trust } = useTrusted(open)
  return (
    <section className="grid gap-2" data-setting="agents.trusted">
      <h3 className="text-sm font-medium text-ink">Trusted repos for agents</h3>
      <p className="text-[12.5px] text-muted-foreground">
        An agent Irori starts runs only in a repo Claude Code trusts on that
        computer. These are this computer&apos;s; your other computers each list
        their own.
      </p>
      <Loading loading={loading} label="Loading trusted repos">
        {loading ? (
          <TrustedShape />
        ) : rows.length === 0 ? (
          <p className="text-[12.5px] text-muted-foreground">
            {error
              ? `Could not load: ${error}`
              : "No repos on this computer yet."}
          </p>
        ) : (
          <ul className="grid divide-y divide-border rounded-md border border-border">
            {rows.map((r) => (
              <TrustedRow
                key={r.repoKey}
                r={r}
                onTrust={() => trust(r.repoKey)}
              />
            ))}
          </ul>
        )}
      </Loading>
    </section>
  )
}

function TrustedRow({
  r,
  onTrust,
}: {
  r: TrustedRepo
  onTrust: () => Promise<void>
}) {
  const Icon = r.trusted ? ShieldCheckIcon : ShieldAlertIcon
  return (
    <li className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-[13px]">
      <Icon
        aria-hidden
        className={r.trusted ? "size-4 text-success" : "size-4 text-warning"}
      />
      <span className="grid min-w-0 gap-0.5">
        <span className="truncate font-mono text-[12.5px] text-ink">
          {r.label}
        </span>
        <span
          className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground"
          title={r.folder}
        >
          <FolderIcon aria-hidden className="size-3 shrink-0" />
          <span className="truncate">{r.folder}</span>
        </span>
      </span>
      {r.trusted ? (
        <span className="text-xs text-muted-foreground">trusted</span>
      ) : (
        <ConfirmButton
          label="Trust…"
          question={
            <>
              Trust {r.label} for agents? {r.what}
            </>
          }
          confirmLabel="Trust"
          onConfirm={onTrust}
        />
      )}
    </li>
  )
}

// AgentsShape, RunningShape and TrustedShape stand in while the pane's
// reads are on their way (card #598): plain markup in the shape of what
// loads, with no hooks, for Loading to measure. The words only set each
// line's length; skullet-block draws a control's stand-in as one block.
function AgentsShape() {
  return (
    <div className="grid gap-4">
      <div className="flex items-start justify-between gap-3">
        <span className="flex flex-col">
          <span className="text-ink">Start an agent for each card</span>
          <span className="text-xs text-muted-foreground">
            In Personal, Irori starts a fresh agent for every card you post
            instead of waiting for a free one, and ends it when the card is
            done.
          </span>
        </span>
        <span
          skullet-block=""
          className="mt-0.5 block h-5 w-9 shrink-0 rounded-full"
        />
      </div>
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">
          What a new agent starts with
        </h3>
        <p className="text-[12.5px] text-muted-foreground">
          Every agent Irori starts uses these. One already working keeps what it
          started with.
        </p>
        <div className="grid gap-2 sm:grid-cols-2">
          <span className="text-sm font-medium text-ink">Model</span>
          <span skullet-block="" className="block h-7 w-[130px] rounded-lg" />
          <span className="text-sm font-medium text-ink">Effort</span>
          <span skullet-block="" className="block h-7 w-[138px] rounded-lg" />
        </div>
      </section>
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">Idle limit</h3>
        <p className="text-[12.5px] text-muted-foreground">
          An agent Irori started that waits this long on a question for you
          writes where it left off and goes. Your answer starts a fresh one.
        </p>
        <span skullet-block="" className="block h-7 w-48 rounded-lg" />
      </section>
    </div>
  )
}

function RunningShape() {
  const rows = [
    ["sample-heron", "#64 Retry the upload on a dropped connection"],
    ["sample-finch", "#61 Bump the icon set"],
  ]
  return (
    <ul className="grid divide-y divide-border rounded-md border border-border">
      {rows.map(([name, card]) => (
        <li
          key={name}
          className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-[13px]"
        >
          <span className="grid min-w-0 gap-0.5">
            <span className="flex min-w-0 items-center gap-2">
              <span className="font-medium text-ink">{name}</span>
              <span skullet-block="" className="block h-4 w-14 rounded-full" />
            </span>
            <span className="text-xs text-muted-foreground">{card}</span>
            <span className="text-xs text-muted-foreground">
              Macbook Pro (this one) · sonnet · medium · 6 min
            </span>
          </span>
          <span className="flex items-center gap-1.5">
            <span skullet-block="" className="block h-7 w-14 rounded-lg" />
            <span skullet-block="" className="block h-7 w-10 rounded-lg" />
          </span>
        </li>
      ))}
    </ul>
  )
}

function TrustedShape() {
  return (
    <ul className="grid divide-y divide-border rounded-md border border-border">
      {["app-repo", "game-repo"].map((label) => (
        <li
          key={label}
          className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-[13px]"
        >
          <span skullet-block="" className="block size-4 rounded-full" />
          <span className="grid min-w-0 gap-0.5">
            <span className="font-mono text-[12.5px] text-ink">{label}</span>
            <span className="text-xs text-muted-foreground">
              C:/Users/you/projects/{label}
            </span>
          </span>
          <span className="text-xs text-muted-foreground">trusted</span>
        </li>
      ))}
    </ul>
  )
}
