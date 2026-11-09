// Assistant's computer, in Settings › Agents (one-assistant.md §6, card 5):
// which of your computers runs the Office Assistant. It only changes the
// choice; the assistant works without anyone touching it (decision 52).

import { useEffect, useState } from "react"

import { ChoiceSelect } from "@/components/common/choice-select"
import {
  refreshHome,
  setHome,
  subscribeAssistant,
  assistantState,
  type AssistantHomeStatus,
} from "@/lib/assistant"
import { stateDot } from "@/lib/devices"
import { relativeTime } from "@/lib/irori"
import { cn } from "@/lib/utils"

const RefreshMs = 5000

export function AssistantComputerSection({ open }: { open: boolean }) {
  const [home, setStatus] = useState<AssistantHomeStatus | null>(
    assistantState().home
  )
  const [error, setError] = useState("")
  useEffect(() => {
    const off = subscribeAssistant(() => setStatus(assistantState().home))
    return off
  }, [])
  useEffect(() => {
    if (!open) return
    void refreshHome()
    const id = window.setInterval(() => void refreshHome(), RefreshMs)
    return () => window.clearInterval(id)
  }, [open])

  if (!home) return null
  const now = new Date()
  const seen = (state: string, ms: number) =>
    state === "connected"
      ? "connected"
      : ms
        ? `seen ${relativeTime(new Date(ms), now)}`
        : "never seen"
  const choices = home.candidates.map((c) => ({
    value: c.device,
    label: `${c.name}${c.here ? " (this device)" : ""} · ${seen(c.state, c.lastSeenMs)}`,
  }))
  // Recorded, but not saying it can answer (Claude Code missing, an older
  // Irori): still listed, so the picker shows what is chosen.
  if (home.device && !choices.some((c) => c.value === home.device))
    choices.unshift({
      value: home.device,
      label: `${home.name || "a computer"} · ${seen(home.state, home.lastSeenMs)}`,
    })
  const dot = home.device ? (home.here ? "connected" : home.state) : ""
  return (
    <section className="grid gap-2" data-setting="agents.assistant-computer">
      <h3 className="text-sm font-medium text-ink">Assistant's computer</h3>
      <p className="text-[12.5px] text-muted-foreground">
        Your assistant runs on one of your computers and answers the chat from
        all your devices. It picks itself the first time; change it here. It
        never moves by itself when that computer is off.
      </p>
      {choices.length ? (
        <div className="flex items-center gap-2 rounded-md border border-border px-3 py-2">
          <span
            className={cn("size-2 shrink-0 rounded-full", stateDot(dot))}
            aria-hidden
          />
          <ChoiceSelect
            label="Assistant's computer"
            size="sm"
            value={home.device}
            choices={choices}
            onValueChange={(v) => {
              setError("")
              void setHome(v).then(setError)
            }}
            className="min-w-0 flex-1 text-[13px]"
          />
        </div>
      ) : (
        <p className="rounded-md border border-border px-3 py-2 text-[13px] text-muted-foreground">
          {home.none === "needs_update"
            ? "None of your computers can answer from your other devices yet. Update Irori on the one that has Claude Code."
            : "None of your computers has Claude Code yet."}
        </p>
      )}
      {error && (
        <p className="text-[12.5px] text-destructive" role="alert">
          {error}
        </p>
      )}
    </section>
  )
}
