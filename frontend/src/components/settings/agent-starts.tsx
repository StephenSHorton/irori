// AgentStartsControls is "My agents may start agents on my cards" in
// Settings › Agents › Limits (cards #575, #576, GitHub #66): the switch and
// the maximum, for this person on all their devices. Agents use the limit
// and can't change it.

import { useId, useState } from "react"

import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import type { AgentStartsView } from "@/lib/agents"

export const AGENT_STARTS_TITLE = "My agents may start agents on my cards"

export const AGENT_STARTS_HINT =
  "Lets an agent start, list and end agents on your cards, by the same rules as your Start. Whatever those agents go on to start counts too. Agents can't change this."

export function AgentStartsControls({
  starts: s,
  set,
}: {
  starts: AgentStartsView
  set: (on: boolean, max: number) => void
}) {
  return (
    <span className="flex items-center gap-3">
      {/* A new saved maximum starts the field over from it. */}
      <MaxField
        key={s.max}
        max={s.max}
        ceiling={s.ceiling}
        on={s.on}
        onSave={(n) => set(s.on, n)}
      />
      <Switch
        aria-label={AGENT_STARTS_TITLE}
        checked={s.on}
        onCheckedChange={(v) => set(v, s.max)}
      />
    </span>
  )
}

function MaxField({
  max,
  ceiling,
  on,
  onSave,
}: {
  max: number
  ceiling: number
  on: boolean
  onSave: (n: number) => void
}) {
  const id = useId()
  const [draft, setDraft] = useState(String(max))
  const n = Number(draft)
  const valid =
    draft.trim() !== "" && Number.isInteger(n) && n >= 0 && n <= ceiling
  const save = () => {
    if (valid && n !== max) onSave(n)
    else setDraft(String(max))
  }
  return (
    <span className="flex flex-col items-end gap-1">
      <span className="flex items-center gap-2 text-[13px]">
        <label htmlFor={id} className="text-muted-foreground">
          At most
        </label>
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={0}
          max={ceiling}
          value={draft}
          disabled={!on}
          aria-invalid={!valid}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") save()
          }}
          className="h-8 w-20 text-[13px]"
        />
      </span>
      {!valid && (
        <span className="text-xs text-destructive">
          A whole number from 0 to {ceiling}.
        </span>
      )}
    </span>
  )
}
