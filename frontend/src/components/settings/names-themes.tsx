// "Agent names" (card #702): which themes a new agent's name comes from, in
// Settings › Agents. Every theme is on by default and the person turns off
// the ones they don't want. When the themes still on have no word free, or
// every theme is off, names come from all of them. Nobody already named is
// renamed. Set here only; no tool can.

import { useId } from "react"

import { Switch } from "@/components/ui/switch"
import { useNamesThemes } from "@/lib/agents"

export function NamesThemesSection({ open }: { open: boolean }) {
  const { view: v, error, set } = useNamesThemes(open)
  const id = useId()
  if (!v)
    return error ? (
      <p className="text-[12.5px] text-destructive">{error}</p>
    ) : null
  return (
    <section
      className="grid gap-2"
      aria-label="Agent names"
      data-setting="agents.names"
    >
      <h3 className="text-sm font-medium text-ink">Agent names</h3>
      <p className="text-[12.5px] text-muted-foreground">
        Each new agent gets a short name nobody has used lately, from the themes
        that are on. When they run out, or every theme is off, names come from
        all of them. Agents already named keep their names.
      </p>
      <ul className="grid gap-2">
        {v.themes.map((t) => (
          <li key={t.id} className="flex items-start justify-between gap-3">
            <span className="flex min-w-0 flex-col">
              <label
                htmlFor={`${id}-${t.id}`}
                className="cursor-pointer break-words text-ink"
              >
                {t.label}
              </label>
              <span
                id={`${id}-${t.id}-hint`}
                className="text-xs text-muted-foreground"
              >
                {t.words.toLocaleString()} names, like {t.sample.join(", ")}
              </span>
            </span>
            <Switch
              id={`${id}-${t.id}`}
              checked={t.on}
              aria-describedby={`${id}-${t.id}-hint`}
              onCheckedChange={(on) => set(t.id, on)}
              className="mt-0.5"
            />
          </li>
        ))}
      </ul>
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </section>
  )
}
