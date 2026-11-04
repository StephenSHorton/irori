// AgentsPane is Settings › Agents (card #448, docs/design/spawned-agents.md
// §10, mockup sa-6), as much of it as part 4 needs: your own board's
// switch for agents Irori starts per card, and the model and effort they
// start with, the idle limit (card #456) and the agents running now. Only
// you set these, here; no agent's tool can. Caps, the daily limit and
// today's count and cost wait on part 6 (card #450).

import { useId, useState } from "react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { PermissionsSection } from "@/components/settings/agent-permissions"
import { Switch } from "@/components/ui/switch"
import { Badge } from "@/components/ui/badge"
import {
  effortLabel,
  endAgent,
  modelLabel,
  runningSince,
  useAgents,
  useRunning,
  type RunningAgent,
} from "@/lib/agents"
import { ConfirmButton } from "@/components/common/confirm-button"
import { SpawnedActions } from "@/components/cards/spawned-actions"
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
            post instead of waiting for a free one, and ends it when the card
            is done.
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
          Every agent Irori starts uses these. One already working keeps what
          it started with.
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
      <RunningSection open={open} />
      <PermissionsSection open={open} />
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </div>
  )
}

// idleLabel names an idle limit in minutes; 30 is the default.
export function idleLabel(m: number): string {
  const n = m % 60 === 0 ? `${m / 60} hour${m === 60 ? "" : "s"}` : `${m} minutes`
  return m === 30 ? `${n} (default)` : n
}

// RunningSection lists every agent Irori started that is still running,
// and the cards one is starting for (card #448): its card, device, model
// and End. Attach and Logs come with part 12.
function RunningSection({ open }: { open: boolean }) {
  const { rows, error, refresh } = useRunning(open)
  const [now] = useState(() => Date.now())
  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-medium text-ink">
        Running agents{rows.length > 0 && ` · ${rows.length}`}
      </h3>
      <p className="text-[12.5px] text-muted-foreground">
        Agents Irori started for your cards. End asks one to write where it
        left off on its card, then stops it.
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
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-[13px]">
      <span className="grid min-w-0 gap-0.5">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate font-medium text-ink">
            {starting ? "Starting…" : r.name}
          </span>
          <Badge
            variant={starting ? "info" : r.status === "busy" ? "success" : "slate"}
            className="h-4 px-1.5 text-[10.5px]"
          >
            {starting ? "starting" : r.status === "busy" ? "working" : r.status}
          </Badge>
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {r.cardShort} {r.cardTitle}
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {r.device} · {r.model} · {r.effort} · {runningSince(r.startedAt ?? now, now)}
        </span>
      </span>
      {ending ? (
        <span className="text-xs text-muted-foreground">ending</span>
      ) : starting ? null : r.here ? (
        <span className="flex items-center gap-1.5">
        <SpawnedActions
          who={AgentRef({ key: r.key, name: r.name, status: r.status, live: true, where: "This PC" })}
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
        <span className="text-xs text-muted-foreground" title="End it from the device it runs on">
          on {r.device}
        </span>
      )}
    </li>
  )
}
