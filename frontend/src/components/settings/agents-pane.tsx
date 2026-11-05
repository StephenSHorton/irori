// AgentsPane is Settings › Agents (card #448, docs/design/spawned-agents.md
// §10, mockup sa-6), as much of it as part 4 needs: your own board's
// switch for agents Irori starts per card, and the model and effort they
// start with, the idle limit, the agents running now, what they may do
// and how each repo's work reaches main, and which of this computer's
// repos are trusted for agents (card #456); and (card #450, part 6) how
// many may run on each device, the daily limit, today's count and cost,
// and a Claude usage limit holding the queue. Only you set these, here; no
// agent's tool can. Agents per card wait on part 7 (card #451).

import { useContext, useId, useState } from "react"
import { FolderIcon, ShieldAlertIcon, ShieldCheckIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { PermissionsSection } from "@/components/settings/agent-permissions"
import { Switch } from "@/components/ui/switch"
import { Badge } from "@/components/ui/badge"
import {
  DAILY_CHOICES,
  costLabel,
  dailyLabel,
  effortLabel,
  endAgent,
  idleLabel,
  useLimits,
  modelLabel,
  runningSince,
  useAgents,
  useRunning,
  useTrusted,
  type AgentsSettings,
  type RunningAgent,
  type TrustedRepo,
} from "@/lib/agents"
import { usageResets } from "@/lib/spawn-line"
import { ConfirmButton } from "@/components/common/confirm-button"
import { SpawnedActions } from "@/components/cards/spawned-actions"
import { agentStatus } from "@/lib/agent-status"
import { LiveRoster } from "@/lib/thread"
import { AgentRef } from "@/lib/models"

export function AgentsPane({ open }: { open: boolean }) {
  const { settings: s, error, set } = useAgents(open)
  const id = useId()
  if (!s) {
    return (
      <p className="text-muted-foreground">
        {error ? `Could not load: ${error}` : "Loading…"}
      </p>
    )
  }
  return (
    <div className="grid gap-4">
      <div className="flex items-start justify-between gap-3">
        <span className="flex flex-col">
          <label htmlFor={id} className="cursor-pointer text-ink">
            Start an agent for each card
          </label>
          <span id={`${id}-hint`} className="text-xs text-muted-foreground">
            On your own board, Irori starts a fresh agent for every card you
            post instead of waiting for a free one, and ends it when the card is
            done.
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
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">
          What a new agent starts with
        </h3>
        <p className="text-[12.5px] text-muted-foreground">
          Every agent Irori starts uses these. One already working keeps what it
          started with.
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
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">Idle limit</h3>
        <p className="text-[12.5px] text-muted-foreground">
          An agent Irori started that waits this long on a question for you
          writes where it left off and goes. Your answer starts a fresh one.
        </p>
        <ChoiceSelect
          label="Idle limit"
          size="sm"
          value={String(s.idleMins)}
          choices={s.idleChoices.map((m) => ({
            value: String(m),
            label: idleLabel(m),
          }))}
          onValueChange={(v) => set({ idleMins: Number(v) })}
          className="w-48 text-[13px]"
        />
      </section>
      <LimitsSection
        open={open}
        settings={s}
        onDaily={(n) => set({ dailyLimit: n })}
      />
      <RunningSection open={open} />
      <PermissionsSection open={open} />
      <TrustedSection open={open} />
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}

// CAP_CHOICES are the caps offered; a device's own number is added when
// it isn't one of them.
const CAP_CHOICES = [1, 2, 3, 4, 6, 8, 10, 12, 16, 20, 24, 32, 48]

// LimitsSection is how many agents run at once on each device, the daily
// limit, today's count and cost, and a usage limit holding the queue
// (card #450, sa-6). Over a device's cap a card queues and says its place;
// a Claude usage limit queues every card until it resets.
function LimitsSection({
  open,
  settings: s,
  onDaily,
}: {
  open: boolean
  settings: AgentsSettings
  onDaily: (n: number) => void
}) {
  const { limits: l, error, setCap } = useLimits(open)
  const daily = DAILY_CHOICES.includes(s.dailyLimit)
    ? DAILY_CHOICES
    : [...DAILY_CHOICES, s.dailyLimit].sort((a, b) => a - b)
  return (
    <section className="grid gap-2">
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
            <p
              role="status"
              className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-[12.5px] text-ink"
            >
              A Claude usage limit is holding new agents (
              {usageResets(new Date(l.heldUntil).toISOString())}). Queued cards
              start then; agents that stopped on it aren't counted as crashes.
            </p>
          )}
          <ul className="grid divide-y divide-border rounded-md border border-border">
            {l.devices.map((d) => {
              const choices =
                d.cap === 0 || CAP_CHOICES.includes(d.cap)
                  ? CAP_CHOICES
                  : [...CAP_CHOICES, d.cap].sort((a, b) => a - b)
              return (
                <li
                  key={d.device}
                  className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
                >
                  <span className="flex min-w-0 flex-col">
                    <span className="truncate text-[13px] text-ink">
                      {d.name}
                    </span>
                    <span className="text-xs text-muted-foreground tabular-nums">
                      {d.cap > 0
                        ? `${d.running} of ${d.cap} running`
                        : `${d.running} running · its own default`}
                    </span>
                  </span>
                  <ChoiceSelect
                    label={`At most on ${d.name}`}
                    size="sm"
                    value={d.cap > 0 ? String(d.cap) : ""}
                    choices={choices
                      .filter((n) => n <= l.maxCap)
                      .map((n) => ({
                        value: String(n),
                        label: `${n} at once`,
                      }))}
                    onValueChange={(v) => setCap(d.device, Number(v))}
                    className="w-36 text-[13px]"
                  />
                </li>
              )
            })}
          </ul>
        </>
      ) : (
        <p className="text-[12.5px] text-muted-foreground">
          {error ? `Could not load: ${error}` : "Loading…"}
        </p>
      )}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="flex flex-col">
          <span className="text-[13px] text-ink">Daily limit</span>
          <span className="text-xs text-muted-foreground">
            Once this many agents started today, new cards wait for you to start
            theirs.
            {l?.dailyReached && " Today's are started."}
          </span>
        </span>
        <ChoiceSelect
          label="Daily limit"
          size="sm"
          value={String(s.dailyLimit)}
          choices={daily.map((n) => ({
            value: String(n),
            label: dailyLabel(n),
          }))}
          onValueChange={(v) => onDaily(Number(v))}
          className="w-48 text-[13px]"
        />
      </div>
      {error && l && <p className="text-[12.5px] text-destructive">{error}</p>}
    </section>
  )
}

// RunningSection lists every agent Irori started that is still running,
// and the cards one is starting for (card #448): its card, device, model,
// Logs and End (card #456). Attach is on its card, where the terminal is.
function RunningSection({ open }: { open: boolean }) {
  const { rows, error, refresh } = useRunning(open)
  const [now] = useState(() => Date.now())
  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium text-ink">
        Running agents{rows.length > 0 && ` · ${rows.length}`}
      </h3>
      <p className="text-[12.5px] text-muted-foreground">
        Agents Irori started for your cards. End asks one to write where it left
        off on its card, then stops it.
      </p>
      {rows.length === 0 ? (
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
      ) : starting ? null : r.here ? (
        <span className="flex items-center gap-1.5">
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
          <ConfirmButton
            label="End"
            question={
              <>
                End {r.name} on {r.cardShort}? It writes where it left off on
                the card first, then Irori stops it and removes its session.
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
      ) : (
        <span
          className="text-xs text-muted-foreground"
          title="End it from the device it runs on"
        >
          on {r.device}
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
  const { rows, error, trust } = useTrusted(open)
  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium text-ink">Trusted repos for agents</h3>
      <p className="text-[12.5px] text-muted-foreground">
        An agent Irori starts runs only in a repo Claude Code trusts on that
        computer. These are this computer&apos;s; your other computers each list
        their own.
      </p>
      {rows.length === 0 ? (
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
