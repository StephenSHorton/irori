// SpawnNotice is the first-run notice for spawned agents (card #448,
// docs/design/spawned-agents.md §11, decision 1 (a)): Irori now starts an
// agent for each card on your own board. It names the switch and the
// trust step, and goes once you keep it on or turn it off, both of which
// record your choice in Settings › Agents.

import { BotIcon } from "lucide-react"

import { openSettings } from "@/components/settings/settings-menu"
import { Button } from "@/components/ui/button"
import { useAgents } from "@/lib/agents"

export function SpawnNotice() {
  const { settings: s, error, set } = useAgents(true)
  if (!s || s.chosen || !s.on) return null
  return (
    <div role="status" className="grid gap-2 px-3 py-2.5 text-sm glass-panel">
      <div className="flex items-start gap-2">
        <BotIcon className="mt-0.5 size-4 shrink-0 text-primary" aria-hidden />
        <div className="grid gap-1">
          <p className="font-medium text-ink">
            Irori now starts an agent for each open card in Personal.
          </p>
          <p className="text-[12.5px] text-muted-foreground">
            You no longer wait for a free agent: open cards nobody holds get one
            now, up to this device's cap, and each new card gets its own. It
            starts on the card, in its repo, and ends once the card is done.
            Each repo must be trusted in Claude Code first; a card whose repo
            isn't shows "can't start" with Trust and start. Change the model or
            turn this off in Settings › Agents.
          </p>
          {error && <p className="text-[12.5px] text-destructive">{error}</p>}
        </div>
      </div>
      <div className="flex flex-wrap justify-end gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => openSettings("agents")}
        >
          Settings › Agents
        </Button>
        <Button variant="outline" size="sm" onClick={() => set({ on: false })}>
          Turn it off
        </Button>
        <Button size="sm" onClick={() => set({ on: true })}>
          Keep it on
        </Button>
      </div>
    </div>
  )
}
