import { useId } from "react"
import { CheckIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  glassStyles,
  oklchToHex,
  resolvedMode,
  swatch,
  themes,
  themeTokens,
  useAppearance,
  type Custom,
  type ModeChoice,
} from "@/lib/appearance"
import { cn } from "@/lib/utils"

const modeChoices: { value: ModeChoice; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
]

const glassChoices = glassStyles.map((g) => ({
  value: g.id,
  label: (
    <span>
      {g.name} <span className="text-muted-foreground">· {g.note}</span>
    </span>
  ),
}))

// AppearancePane is Settings > Appearance (card #275): the theme, your own
// colours over it, light/dark/system and the glass. Saved on this device.
export function AppearancePane() {
  const { appearance: a, set } = useAppearance()
  const mode = resolvedMode(a.mode)
  const base = a.theme === "custom" ? (a.custom?.base ?? "blue") : a.theme
  const baseTheme = themes.find((t) => t.id === base) ?? themes[0]
  // The pickers start from the theme's own accent and its neutrals' hue
  // (at a strength customSeed scales back to the theme's).
  const custom: Custom = a.custom ?? {
    base,
    accent: oklchToHex(themeTokens(baseTheme, "light").primary),
    tint: oklchToHex({
      l: 0.6,
      c: baseTheme.seed.nc / 0.3,
      h: baseTheme.seed.nh,
    }),
  }
  const pick = (patch: Partial<Custom>) =>
    set({ theme: "custom", custom: { ...custom, base, ...patch } })
  const accentId = useId()
  const tintId = useId()

  return (
    <div className="grid gap-4">
      <div className="grid gap-2">
        <span className="text-ink">Theme</span>
        <div
          role="radiogroup"
          aria-label="Theme"
          className="grid grid-cols-2 gap-2 sm:grid-cols-3"
        >
          {themes.map((t) => (
            <Swatch
              key={t.id}
              name={t.name}
              note={t.note}
              colors={swatch(t.id, mode)}
              checked={a.theme === t.id}
              onPick={() => set({ theme: t.id, custom: undefined })}
            />
          ))}
          {a.custom && (
            <Swatch
              name="Custom"
              note={`Your colours over ${baseTheme.name}`}
              colors={swatch("custom", mode, a.custom)}
              checked={a.theme === "custom"}
              onPick={() => set({ theme: "custom" })}
            />
          )}
        </div>
      </div>

      <div className="grid gap-2">
        <span className="text-ink">Your colours</span>
        <p className="text-xs text-muted-foreground">
          Pick an accent and a background tint. Irori keeps the lightness, so
          text stays readable in light and dark.
        </p>
        <div className="flex flex-wrap items-center gap-4">
          <label
            htmlFor={accentId}
            className="flex items-center gap-2 text-[13px] text-ink"
          >
            <input
              id={accentId}
              type="color"
              value={custom.accent}
              onChange={(e) => pick({ accent: e.target.value })}
              className="size-8 cursor-pointer rounded-md border border-border bg-transparent p-0.5"
            />
            Accent
          </label>
          <label
            htmlFor={tintId}
            className="flex items-center gap-2 text-[13px] text-ink"
          >
            <input
              id={tintId}
              type="color"
              value={custom.tint}
              onChange={(e) => pick({ tint: e.target.value })}
              className="size-8 cursor-pointer rounded-md border border-border bg-transparent p-0.5"
            />
            Background tint
          </label>
          <Button
            variant="outline"
            size="sm"
            disabled={!a.custom}
            onClick={() => set({ theme: base, custom: undefined })}
          >
            Reset to {baseTheme.name}
          </Button>
        </div>
      </div>

      <div className="flex items-center justify-between gap-3">
        <span className="text-ink">Light or dark</span>
        <ToggleGroup
          aria-label="Light or dark"
          variant="outline"
          size="sm"
          value={[a.mode]}
          onValueChange={(v: string[]) =>
            v[0] && set({ mode: v[0] as ModeChoice })
          }
        >
          {modeChoices.map((m) => (
            <ToggleGroupItem key={m.value} value={m.value}>
              {m.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>

      <div className="flex items-center justify-between gap-3">
        <span className="text-ink">Glass</span>
        <ChoiceSelect
          label="Glass"
          size="sm"
          value={a.glass}
          choices={glassChoices}
          onValueChange={(v) => set({ glass: v })}
          className="text-[13px]"
        />
      </div>
      <p className="text-xs text-muted-foreground">Saved on this device.</p>
    </div>
  )
}

function Swatch({
  name,
  note,
  colors,
  checked,
  onPick,
}: {
  name: string
  note: string
  colors: { bg: string; accent: string; text: string }
  checked: boolean
  onPick: () => void
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      title={note}
      onClick={onPick}
      className={cn(
        "flex w-full flex-col items-stretch gap-1.5 rounded-lg border p-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring",
        checked
          ? "border-primary ring-1 ring-primary"
          : "border-border hover:border-ring/60"
      )}
    >
      <span
        aria-hidden
        className="flex h-10 w-full items-center gap-1.5 rounded-md border border-border px-2"
        style={{ background: colors.bg }}
      >
        <span
          className="size-4 rounded-full"
          style={{ background: colors.accent }}
        />
        <span
          className="h-1.5 flex-1 rounded-full opacity-70"
          style={{ background: colors.text }}
        />
      </span>
      <span className="flex w-full items-center justify-between gap-1 text-[13px] text-ink">
        {name}
        {checked && <CheckIcon className="size-3.5 text-primary" aria-hidden />}
      </span>
    </button>
  )
}
