import { Checkbox } from "@/components/ui/checkbox"
import { stopLabels as L } from "@/lib/stop-labels"
import type { StopWho } from "@/lib/stop-who"

// WhoTicks is the choice of who a Stop reaches (decision 71), kept on this
// computer. Stop here's chevron shows the same two ticks.
export function WhoTicks({
  who,
  counts,
  onChange,
}: {
  who: StopWho
  counts: { cards: number; yours: number }
  onChange: (who: StopWho) => void
}) {
  const tick = (key: keyof StopWho, label: string, hint: string) => (
    <label className="flex items-start gap-2.5 text-sm">
      <Checkbox
        className="mt-0.5"
        checked={who[key]}
        onCheckedChange={(v) => onChange({ ...who, [key]: !!v })}
      />
      <span className="min-w-0">
        <span className="font-medium">{label}</span>
        <span className="block text-xs text-muted-foreground">{hint}</span>
      </span>
    </label>
  )
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-1 text-xs font-medium text-muted-foreground">
        {L.who.legend}
      </legend>
      {tick("cards", L.who.cards(counts.cards), L.who.cardsHint)}
      {tick("yours", L.who.yours(counts.yours), L.who.yoursHint)}
    </fieldset>
  )
}
