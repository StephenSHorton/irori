// A space's agents switch (card #573, docs/design/spawned-agents.md §9):
// whether Irori starts your agents for the cards you make in that space.
// Off by default. It sits in Settings › Agents and in the space's own
// settings, and the space says so plainly while it's off. A card someone
// gives you isn't under it: that always asks you first.

import { useId } from "react"
import { BotOffIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { useAgents } from "@/lib/agents"

// SpaceAgentSwitch is the switch with its name and what it does now.
export function SpaceAgentSwitch({
  boardId,
  name,
  open,
}: {
  boardId: string
  name: string
  open: boolean
}) {
  const { settings: s, error, set } = useAgents(open, boardId)
  const id = useId()
  if (!s)
    return error ? <p className="text-xs text-destructive">{error}</p> : null
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="flex min-w-0 flex-col">
        <label htmlFor={id} className="cursor-pointer break-words text-ink">
          {name}
        </label>
        <span id={`${id}-hint`} className="text-xs text-muted-foreground">
          {s.on
            ? "On: Irori starts your agent for each card you make here."
            : "Off: Irori doesn't start agents for the cards you make here."}
        </span>
        {error && <span className="text-xs text-destructive">{error}</span>}
      </span>
      <Switch
        id={id}
        checked={s.on}
        aria-describedby={`${id}-hint`}
        onCheckedChange={(v) => set({ on: v })}
        className="mt-0.5"
      />
    </div>
  )
}

// SpaceAgentsOff is the space's own line while its switch is off.
export function SpaceAgentsOff({ boardId }: { boardId: string }) {
  const { settings: s, error, set } = useAgents(true, boardId)
  if (!s || s.on) return null
  return (
    <div
      role="status"
      className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5 px-3 py-2 text-[12.5px] text-muted-foreground glass-panel"
    >
      <BotOffIcon className="size-4 shrink-0" aria-hidden />
      <span className="min-w-0 flex-1 basis-56">
        Your agents are off in this space: Irori won't start one for the cards
        you make here.
      </span>
      {error && <span className="text-destructive">{error}</span>}
      <Button size="xs" variant="outline" onClick={() => set({ on: true })}>
        Turn on
      </Button>
    </div>
  )
}
