// "Names from" (card #702): the theme a new agent's name comes from, in
// Settings › Agents. Every theme mixed by default; picking one only narrows
// new names, and when that theme has no word free Irori takes one from
// the others. Nobody already named is renamed. Set here only; no tool can.

import { ChoiceSelect } from "@/components/common/choice-select"
import { useNamesTheme } from "@/lib/agents"

export const NAMES_THEME_ALL = "All themes, mixed"

export function NamesThemeSection({ open }: { open: boolean }) {
  const { view: v, error, set } = useNamesTheme(open)
  if (!v)
    return error ? (
      <p className="text-[12.5px] text-destructive">{error}</p>
    ) : null
  const picked = v.themes.find((t) => t.id === v.theme)
  return (
    <section className="grid gap-2" aria-label="Agent names">
      <h3 className="text-sm font-medium text-ink">Agent names</h3>
      <p className="text-[12.5px] text-muted-foreground">
        Each new agent gets a short name nobody has used lately. Pick a theme to
        take names from it; when it runs out, names come from the others. Agents
        already named keep their names.
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        <ChoiceSelect
          label="Names from"
          showLabel
          size="sm"
          value={v.theme}
          choices={[
            { value: "", label: NAMES_THEME_ALL },
            ...v.themes.map((t) => ({ value: t.id, label: t.label })),
          ]}
          onValueChange={set}
          className="text-[13px]"
        />
      </div>
      <p className="text-[12.5px] text-muted-foreground">
        {picked
          ? `${picked.words} names, like ${picked.sample.join(", ")}.`
          : `${v.themes.reduce((n, t) => n + t.words, 0)} names from ${v.themes
              .map((t) => t.label.toLowerCase())
              .join(", ")}.`}
      </p>
      {error && <p className="text-[12.5px] text-destructive">{error}</p>}
    </section>
  )
}
