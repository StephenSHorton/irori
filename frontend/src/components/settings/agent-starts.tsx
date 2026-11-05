// AgentStartsSection is "My agents may start agents on my cards" in Settings
// › Agents (card #575, GitHub #66): the switch and the maximum, for this
// person on all their devices. Agents use the limit and can't change it.

import { useId, useState } from "react"

import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { useAgentStarts } from "@/lib/agents"

export function AgentStartsSection({ open }: { open: boolean }) {
  const { starts: s, error, set } = useAgentStarts(open)
  const id = useId()
  if (!s) return null
  return (
    <section className="grid gap-2">
      <div className="flex items-start justify-between gap-3">
        <span className="flex flex-col">
          <label htmlFor={id} className="cursor-pointer text-ink">
            My agents may start agents on my cards
          </label>
          <span id={`${id}-hint`} className="text-xs text-muted-foreground">
            Lets an agent start, list and end agents on your cards, by the same
            rules as your Start. Whatever those agents go on to start counts
            too. Agents can&apos;t change this.
          </span>
        </span>
        <Switch
          id={id}
          checked={s.on}
          aria-describedby={`${id}-hint`}
          onCheckedChange={(v) => set(v, s.max)}
          className="mt-0.5"
        />
      </div>
      {/* A new saved maximum starts the field over from it. */}
      <MaxField
        key={s.max}
        max={s.max}
        used={s.used}
        ceiling={s.ceiling}
        on={s.on}
        onSave={(n) => set(s.on, n)}
      />
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </section>
  )
}

function MaxField({
  max,
  used,
  ceiling,
  on,
  onSave,
}: {
  max: number
  used: number
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
    <>
      <div className="flex items-center gap-2 text-[13px]">
        <label htmlFor={id} className="text-ink">
          Maximum at once
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
        <span className="text-muted-foreground">
          {used} of {max} in use
        </span>
      </div>
      {!valid && (
        <p className="text-[12.5px] text-destructive">
          Enter a whole number from 0 to {ceiling}.
        </p>
      )}
    </>
  )
}
