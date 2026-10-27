import type { ReactNode } from "react"

import {
  Select,
  SelectContent,
  SelectFieldLabel,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"

export type Choice = { value: string; label: ReactNode }
export type ChoiceGroup = { group: string; choices: Choice[] }

// ChoiceSelect is the app's one-of-a-list control: mokei's Select over a
// flat list of choices, optionally grouped. Values are strings and "" is an
// ordinary choice ("All repos", "No repo"), as it was with <select>. label
// names the control: shown with SelectFieldLabel when showLabel is set,
// otherwise as the trigger's aria-label.
export function ChoiceSelect({
  id,
  label,
  showLabel = false,
  value,
  onValueChange,
  choices,
  disabled,
  size = "default",
  className,
  labelClassName,
}: {
  id?: string
  label: string
  showLabel?: boolean
  value: string
  onValueChange: (value: string) => void
  choices: Array<Choice | ChoiceGroup>
  disabled?: boolean
  size?: "sm" | "default"
  className?: string
  labelClassName?: string
}) {
  const items = choices.flatMap((c) => ("group" in c ? c.choices : [c]))
  return (
    <Select
      items={items}
      value={value}
      disabled={disabled}
      onValueChange={(v) => onValueChange(v ?? "")}
    >
      {showLabel && (
        <SelectFieldLabel
          className={cn("text-sm font-medium text-ink", labelClassName)}
        >
          {label}
        </SelectFieldLabel>
      )}
      <SelectTrigger
        id={id}
        size={size}
        aria-label={showLabel ? undefined : label}
        className={cn(
          "min-w-0 bg-white/70 text-ink dark:bg-white/70 dark:hover:bg-white/70",
          className
        )}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {choices.map((c) =>
          "group" in c ? (
            <SelectGroup key={`group:${c.group}`}>
              <SelectLabel>{c.group}</SelectLabel>
              {c.choices.map((o) => (
                <SelectItem key={o.value} value={o.value}>
                  {o.label}
                </SelectItem>
              ))}
            </SelectGroup>
          ) : (
            <SelectItem key={c.value} value={c.value}>
              {c.label}
            </SelectItem>
          )
        )}
      </SelectContent>
    </Select>
  )
}
