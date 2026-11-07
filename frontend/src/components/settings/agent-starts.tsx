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
      <WholeNumberField
        key={s.max}
        label="At most"
        value={s.max}
        ceiling={s.ceiling}
        disabled={!s.on}
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

// WholeNumberField is a box for a whole number from 0 to ceiling, saved on
// Enter or blur; anything else is refused with a message and, on leaving,
// goes back to the saved value. Give it key={value} so a new saved value
// starts it over. The per-device caps in Limits use it too (card #703).
export function WholeNumberField({
  label,
  ariaLabel,
  value,
  ceiling,
  disabled,
  onSave,
}: {
  label: string
  ariaLabel?: string
  value: number
  ceiling: number
  disabled?: boolean
  onSave: (n: number) => void
}) {
  const id = useId()
  const [draft, setDraft] = useState(String(value))
  const n = Number(draft)
  const valid =
    draft.trim() !== "" && Number.isInteger(n) && n >= 0 && n <= ceiling
  const save = () => {
    if (valid && n !== value) onSave(n)
    else setDraft(String(value))
  }
  return (
    <span className="flex flex-col items-end gap-1">
      <span className="flex items-center gap-2 text-[13px]">
        <label htmlFor={id} className="text-muted-foreground">
          {label}
        </label>
        <Input
          id={id}
          type="number"
          inputMode="numeric"
          min={0}
          max={ceiling}
          step={1}
          value={draft}
          disabled={disabled}
          aria-label={ariaLabel}
          aria-invalid={!valid}
          aria-describedby={valid ? undefined : `${id}-error`}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") save()
          }}
          className="h-8 w-20 text-[13px]"
        />
      </span>
      {!valid && (
        // As wide as the label and box, so the box doesn't move when it shows.
        <span
          id={`${id}-error`}
          className="w-0 min-w-full text-xs text-destructive"
        >
          A whole number from 0 to {ceiling}.
        </span>
      )}
    </span>
  )
}
