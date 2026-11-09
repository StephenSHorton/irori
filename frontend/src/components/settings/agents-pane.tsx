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
import { AssistantNameSection } from "@/components/settings/assistant-name"
import { NamesThemesSection } from "@/components/settings/names-themes"
import { WeighsSwitch } from "@/components/settings/settings-parts"
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
  sentBackRate,
  shortModel,
  runningSince,
  useAgentReviews,
  useAgentReviewStarts,
  useAgentStarts,
  useRunHistory,
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
import { go } from "@/lib/nav"
import { LiveRoster } from "@/lib/thread"
import { AgentRef } from "@/lib/models"

export function AgentsPane({ open }: { open: boolean }) {
  const { settings: s, error, loading, set } = useAgents(open)
  // Read in the order someone asks (card #1007): when one starts, what it
  // starts with, what it may do, how many, when Irori stops, then what's
  // running now. Running, permissions and trusted repos read on their own,
  // each with its own skeleton (card #598).
  return (
    <div className="grid grid-cols-[minmax(0,1fr)] gap-5">
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
          <div className="grid gap-5">
            <WhenSection open={open} settings={s} set={set} />
            <WeightsSection settings={s} set={set} />
            <AssistantSection settings={s} set={set} />
            <AssistantNameSection open={open} />
          </div>
        ) : (
          <AgentsShape />
        )}
      </Loading>
      {/* Reads on its own, so it stays when the read above fails. The gap
          pulls it up under the table it picks rows of. */}
      <div className="-mt-3">
        <WeighsSwitch />
      </div>
      <HistorySection open={open} />
      <PermissionsSection open={open} />
      <TrustedSection open={open} />
      {/* Only while it loads: a failed read says so above, with no empty
          headings under it. */}
      {(s || loading) && (
        <Loading loading={loading} label="Loading agent limits">
          {s ? (
            <LimitsSection open={open} settings={s} set={set} />
          ) : (
            <LimitsShape />
          )}
        </Loading>
      )}
      <NamesThemesSection open={open} />
      <RunningSection open={open} />
      {s && error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}

// WhenSection answers "when does Irori start an agent" in one place (card
// #1007, decision 50, docs/design/spawned-agents.md §9): Personal, each
// shared space, and cards given to you, side by side, since the last of
// those is under no switch at all. Nothing here changes when one starts.
function WhenSection({
  open,
  settings: s,
  set,
}: {
  open: boolean
  settings: AgentsSettings
  set: (c: Omit<AgentsChange, "boardId">) => void
}) {
  const { boards } = useBoards()
  const id = useId()
  return (
    <section className="grid gap-2" aria-labelledby={`${id}-when`}>
      <h3 id={`${id}-when`} className="text-sm font-medium text-ink">
        When Irori starts an agent
      </h3>
      <p className="text-[12.5px] text-muted-foreground">
        By itself, only for the cards a switch here covers. Each computer has
        its own switches: what counts is the one on the computer that made the
        card or, for a card made on a phone, the computer that picks it up.
      </p>
      <ul className="grid divide-y divide-border rounded-md border border-border">
        <li
          data-setting="agents.each-card"
          className="flex items-start justify-between gap-3 px-3 py-2"
        >
          <span className="flex min-w-0 flex-col">
            <label htmlFor={id} className="cursor-pointer text-ink">
              Personal
            </label>
            <span id={`${id}-hint`} className="text-xs text-muted-foreground">
              {s.on
                ? "On: Irori starts a fresh agent for each card you post here, and ends it when the card is done."
                : "Off: Irori doesn't start agents for the cards you post here. They wait for a free agent that takes offered cards."}
            </span>
          </span>
          <Switch
            id={id}
            checked={s.on}
            aria-describedby={`${id}-hint`}
            onCheckedChange={(v) => set({ on: v })}
            className="mt-0.5"
          />
        </li>
        {boards.length > 0 && (
          <li data-setting="agents.spaces">
            <ul className="grid divide-y divide-border" aria-label="Spaces">
              {boards.map((b) => (
                <li key={b.boardId} className="px-3 py-2">
                  <SpaceAgentSwitch
                    boardId={b.boardId}
                    name={plainLine(b.name) || "(unnamed)"}
                    open={open}
                  />
                </li>
              ))}
            </ul>
          </li>
        )}
        <li data-setting="agents.given" className="grid gap-0.5 px-3 py-2">
          {/* The badge sits on the title's line so the description gets the
              row's full width (at 390px it was squeezed beside the badge). */}
          <span className="flex items-start justify-between gap-3">
            <span className="min-w-0 text-ink">Cards someone gives you</span>
            <Badge variant="info" className="mt-0.5 shrink-0">
              Always asks
            </Badge>
          </span>
          <span className="text-xs text-muted-foreground">
            When someone in a shared space gives or assigns you a card, Irori
            asks &ldquo;OK to start an agent on this?&rdquo; in Needs you.
            Nothing starts without your yes, whatever the switches above say.
          </span>
        </li>
      </ul>
      <p className="text-xs text-muted-foreground">
        A computer starts agents only for the sides it runs. Set those in{" "}
        <Button
          variant="link"
          size="xs"
          className="h-auto px-0 align-baseline"
          onClick={() => go({ kind: "settings", tab: "devices" })}
        >
          Devices and connections
        </Button>
        .
      </p>
    </section>
  )
}

// WeightsSection is what a new agent starts with, by its card's weight
// (card #963): Light, Normal and Heavy, each a model and effort. Normal is
// the board's own pick; Light and Heavy follow it until the person picks
// one (Sonnet at no more than medium, Opus at no less than high).
function WeightsSection({
  settings: s,
  set,
}: {
  settings: AgentsSettings
  set: (c: Omit<AgentsChange, "boardId">) => void
}) {
  const rows: {
    name: string
    model: string
    effort: string
    def: { model: string; effort: string }
    onModel: (v: string) => void
    onEffort: (v: string) => void
  }[] = [
    {
      name: "Light",
      model: s.lightModel,
      effort: s.lightEffort,
      def: { model: "", effort: "" },
      onModel: (v) => set({ lightModel: v }),
      onEffort: (v) => set({ lightEffort: v }),
    },
    {
      name: "Normal",
      model: s.model,
      effort: s.effort,
      def: { model: s.defaultModel, effort: s.defaultEffort },
      onModel: (v) => set({ model: v }),
      onEffort: (v) => set({ effort: v }),
    },
    {
      name: "Heavy",
      model: s.heavyModel,
      effort: s.heavyEffort,
      def: { model: "", effort: "" },
      onModel: (v) => set({ heavyModel: v }),
      onEffort: (v) => set({ heavyEffort: v }),
    },
  ]
  return (
    <section className="grid gap-2" data-setting="agents.model">
      <h3 className="text-sm font-medium text-ink">
        What a new agent starts with
      </h3>
      <p className="text-[12.5px] text-muted-foreground">
        For your Personal cards, on this computer; each computer has its own. A
        card&apos;s weight picks its row, and a card with no weight runs as
        Normal. Agents in a shared space use the defaults. One already working
        keeps what it started with.
      </p>
      <ul className="grid divide-y divide-border rounded-md border border-border">
        <li
          aria-hidden
          className="hidden px-3 py-1.5 text-xs text-muted-foreground sm:grid sm:grid-cols-[4.5rem_minmax(0,1fr)_minmax(0,1fr)] sm:gap-2"
        >
          <span>Weight</span>
          <span>Model</span>
          <span>Effort</span>
        </li>
        {rows.map((r) => (
          <li
            key={r.name}
            className="grid grid-cols-2 items-center gap-2 px-3 py-2 sm:grid-cols-[4.5rem_minmax(0,1fr)_minmax(0,1fr)]"
          >
            <span className="col-span-2 text-[13px] text-ink sm:col-span-1">
              {r.name}
            </span>
            <ChoiceSelect
              label={`${r.name} model`}
              size="sm"
              value={r.model}
              choices={withValue(s.models, r.model).map((m) => ({
                value: m,
                label: modelLabel(m, r.def.model),
              }))}
              onValueChange={r.onModel}
              className="min-w-0 text-[13px]"
            />
            <ChoiceSelect
              label={`${r.name} effort`}
              size="sm"
              value={r.effort}
              choices={withValue(s.efforts, r.effort).map((e) => ({
                value: e,
                label: effortLabel(e, r.def.effort),
              }))}
              onValueChange={r.onEffort}
              className="min-w-0 text-[13px]"
            />
          </li>
        ))}
      </ul>
    </section>
  )
}

// AssistantSection is the model and effort of the office assistant's chat
// (card #1014): Sonnet at low until picked. Only the chat: thread names,
// weighing and Rewrite stay on Haiku. The next question uses the change.
function AssistantSection({
  settings: s,
  set,
}: {
  settings: AgentsSettings
  set: (c: Omit<AgentsChange, "boardId">) => void
}) {
  return (
    <section className="grid gap-2" data-setting="agents.assistant">
      <h3 className="text-sm font-medium text-ink">Office assistant</h3>
      <p className="text-[12.5px] text-muted-foreground">
        What the assistant chat answers with. A change applies to your next
        question.
      </p>
      <div className="grid grid-cols-2 gap-2 rounded-md border border-border px-3 py-2">
        <ChoiceSelect
          label="Assistant model"
          size="sm"
          value={s.assistantModel}
          choices={withValue(s.assistantModels, s.assistantModel).map((m) => ({
            value: m,
            label: modelLabel(m, "sonnet"),
          }))}
          onValueChange={(v) => set({ assistantModel: v })}
          className="min-w-0 text-[13px]"
        />
        <ChoiceSelect
          label="Assistant effort"
          size="sm"
          value={s.assistantEffort}
          choices={withValue(s.efforts, s.assistantEffort).map((e) => ({
            value: e,
            label: effortLabel(e, "low"),
          }))}
          onValueChange={(v) => set({ assistantEffort: v })}
          className="min-w-0 text-[13px]"
        />
      </div>
    </section>
  )
}

// HistorySection is how cards ended by weight and model (card #966), so the
// mapping above rests on what happened: each row counts the attempts you
// answered, Done or Not done.
function HistorySection({ open }: { open: boolean }) {
  const rows = useRunHistory(open)
  return (
    <section className="grid gap-2" data-setting="agents.history">
      <h3 className="text-sm font-medium text-ink">How Personal cards ended</h3>
      <p className="text-[12.5px] text-muted-foreground">
        Each agent&apos;s done check you answered, by what it ran on. A crash or
        a card you send back starts its next agent one weight higher.
      </p>
      {rows.length === 0 ? (
        <p className="text-[12.5px] text-muted-foreground">
          Nothing yet: it fills in as you answer done checks.
        </p>
      ) : (
        <ul className="grid divide-y divide-border rounded-md border border-border">
          {rows.map((r) => (
            <li
              key={`${r.weight}-${r.model}-${r.effort}`}
              className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 gap-y-0.5 px-3 py-2 text-[13px]"
            >
              <span className="min-w-0 text-ink capitalize">
                {r.weight} · {shortModel(r.model, r.effort)}
              </span>
              <span className="text-ink tabular-nums">
                {sentBackRate({ runs: r.runs, sentBack: r.sentBack })} sent back
              </span>
              <span className="col-span-2 text-[12px] text-muted-foreground tabular-nums">
                {r.runs} answered · {r.firstTry} done first try · {r.sentBack}{" "}
                sent back · {r.advised} with the advisor
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

// withValue keeps a stored value in its list even when the list no longer
// offers it, so the select still names what is set.
function withValue(list: string[], v: string): string[] {
  return !v || list.includes(v) ? list : [...list, v]
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
    <div className="grid gap-5">
      <section className="grid gap-2" data-setting="agents.limits">
        <h3 className="text-sm font-medium text-ink">How many at once</h3>
        <p className="text-[12.5px] text-muted-foreground">
          Each computer runs at most this many agents Irori started, Personal
          and spaces together. Any more wait in line.
        </p>
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
              {l.queued > 0 &&
                ` ${l.queued} Personal card${l.queued === 1 ? "" : "s"} queued here.`}
            </p>
            {l.heldUntil != null && (
              <div
                role="status"
                className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-[12.5px] text-ink"
              >
                <span className="min-w-0 flex-1 basis-56">
                  A Claude usage limit is holding new agents on this computer (
                  {usageResets(new Date(l.heldUntil).toISOString())}). Queued
                  cards start then; agents that stopped on it aren&apos;t
                  counted as crashes.
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
        <AgentStartsRow open={open} />
        <AgentReviewsRow open={open} />
        <AgentReviewStartsRow open={open} />
        {error && l && (
          <p className="text-[12.5px] text-destructive">{error}</p>
        )}
      </section>
      <section className="grid gap-2" data-setting="agents.stops">
        <h3 className="text-sm font-medium text-ink">
          When Irori stops and asks
        </h3>
        <ul className="grid divide-y divide-border rounded-md border border-border">
          <LimitRow
            title="Idle limit"
            hint="In Personal, an agent that waits this long on a question for you writes where it left off and goes. Your answer starts a fresh one. This computer's number, for the agents running on it."
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
            hint="In Personal, after this many tries that couldn't start an agent for a card, Irori asks you before trying again. Each computer has its own number."
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
        </ul>
      </section>
    </div>
  )
}

// AgentStartsRow is "My agents may start agents on my cards" (card #575):
// the switch and the maximum, with how much of it is in use now.
function AgentStartsRow({ open }: { open: boolean }) {
  const { starts: s, error, set } = useAgentStarts(open)
  if (!s) return null
  return (
    <ul className="grid divide-y divide-border rounded-md border border-border">
      <LimitRow
        title={AGENT_STARTS_TITLE}
        hint={
          <>
            {AGENT_STARTS_HINT}{" "}
            <span className="tabular-nums">
              {s.on
                ? `${s.used} of ${s.max} in use.`
                : `Off; ${s.used} in use.`}
            </span>
            {error && <span className="text-destructive"> {error}</span>}
          </>
        }
      >
        <AgentStartsControls starts={s} set={set} />
      </LimitRow>
    </ul>
  )
}

// AgentReviewsRow is "Agents review other people's PRs without asking"
// (merge-desk §12.8). On when unset; off makes each review ask the person.
// It governs approving and requesting changes: comments stay allowed.
function AgentReviewsRow({ open }: { open: boolean }) {
  const { on, error, set } = useAgentReviews(open)
  const id = useId()
  if (on === null) return null
  return (
    <ul
      className="grid divide-y divide-border rounded-md border border-border"
      data-setting="agents.reviews"
    >
      <LimitRow
        title="Agents review other people's PRs without asking"
        hint={
          <>
            Across all your computers. Lets an agent approve or request changes
            on a pull request in a repo you work in, under your GitHub login.
            Comments are always allowed. Off, each review asks you first. Agents
            can't change this.
            {error && <span className="text-destructive"> {error}</span>}
          </>
        }
      >
        <Switch
          id={id}
          aria-label="Agents review other people's PRs without asking"
          checked={on}
          onCheckedChange={(v) => void set(v)}
        />
      </LimitRow>
    </ul>
  )
}

// AgentReviewStartsRow is "Start a reviewer on this machine when nobody else
// fits" (merge-desk §12.10). On when unset; off makes a reviewer given to
// you ask "OK to start an agent on this?" first.
function AgentReviewStartsRow({ open }: { open: boolean }) {
  const { on, error, set } = useAgentReviewStarts(open)
  const id = useId()
  if (on === null) return null
  return (
    <ul
      className="grid divide-y divide-border rounded-md border border-border"
      data-setting="agents.review-starts"
    >
      <LimitRow
        title="Start a reviewer on this machine when nobody else fits"
        hint={
          <>
            Across all your computers. When a teammate's pull request has no
            one to review it, Irori may start a read-only reviewer here, in a
            repo you work in. It can comment and review but never merge, push or
            run the code. Off, it asks you first. Agents can't change this.
            {error && <span className="text-destructive"> {error}</span>}
          </>
        }
      >
        <Switch
          id={id}
          aria-label="Start a reviewer on this machine when nobody else fits"
          checked={on}
          onCheckedChange={(v) => void set(v)}
        />
      </LimitRow>
    </ul>
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
    <div className="grid gap-5">
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">
          When Irori starts an agent
        </h3>
        <p className="text-[12.5px] text-muted-foreground">
          By itself, only for the cards a switch here covers. Each computer has
          its own switches: what counts is the one on the computer that made the
          card or, for a card made on a phone, the computer that picks it up.
        </p>
        <ul className="grid divide-y divide-border rounded-md border border-border">
          {["Personal", "Sample space", "Cards someone gives you"].map(
            (title) => (
              <li
                key={title}
                className="flex items-start justify-between gap-3 px-3 py-2"
              >
                <span className="flex flex-col">
                  <span className="text-ink">{title}</span>
                  <span className="text-xs text-muted-foreground">
                    On: Irori starts a fresh agent for each card you post here.
                  </span>
                </span>
                <span
                  skullet-block=""
                  className="mt-0.5 block h-5 w-9 shrink-0 rounded-full"
                />
              </li>
            )
          )}
        </ul>
      </section>
      <section className="grid gap-2">
        <h3 className="text-sm font-medium text-ink">
          What a new agent starts with
        </h3>
        <p className="text-[12.5px] text-muted-foreground">
          For your Personal cards, on this computer; each computer has its own.
          A card&apos;s weight picks its row, and a card with no weight runs as
          Normal.
        </p>
        <span skullet-block="" className="block h-28 w-full rounded-md" />
      </section>
    </div>
  )
}

function LimitsShape() {
  return (
    <div className="grid gap-5">
      {["How many at once", "When Irori stops and asks"].map((title) => (
        <section key={title} className="grid gap-2">
          <h3 className="text-sm font-medium text-ink">{title}</h3>
          <span skullet-block="" className="block h-24 w-full rounded-md" />
        </section>
      ))}
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
