// Pieces two Settings panes share (card #1007): a labelled switch, its
// loading stand-in, and a switch that the assistant's settings read and
// write.

import { useEffect, useId, useState } from "react"

import { Loading } from "@/components/ui/loading"
import { Switch } from "@/components/ui/switch"
import { AssistantService } from "@/lib/host"
import { inWails } from "@/lib/irori"
import { errorText } from "@/lib/talk"

// WeighsSwitch is "Weigh new cards" (#965): Irori's assistant weighing each
// new Personal card, on unless they turn it off, on all their devices. It
// sits in Settings › Agents (card #1007), above the table the weight picks
// a row of.
export function WeighsSwitch() {
  return (
    <AssistantSwitch
      setting="agents.weighs"
      label="Weigh new cards"
      hint="In Personal, Irori reads each new card and sets it light, normal or heavy, with a line on why. Its agent waits up to 30 seconds for that. Cards in shared spaces aren't weighed. Applies on all your devices. Undo is on the card; a weight you set is never changed."
      loading="Loading card weights"
      read={AssistantService.Weighs}
      write={AssistantService.WeighsSet}
    />
  )
}

export function AssistantSwitch({
  setting,
  label,
  hint,
  loading,
  read: readOn,
  write,
}: {
  setting: string
  label: string
  hint: string
  loading: string
  read: () => Promise<boolean>
  write: (on: boolean) => Promise<void>
}) {
  const [on, setOn] = useState(true)
  const [read, setRead] = useState(!inWails())
  const [error, setError] = useState("")
  useEffect(() => {
    if (!inWails()) return
    let live = true
    readOn()
      .then((v) => {
        if (!live) return
        setOn(v)
        setRead(true)
      })
      .catch((err: unknown) => {
        if (!live) return
        setError(errorText(err))
        setRead(true)
      })
    return () => {
      live = false
    }
  }, [readOn])
  const set = async (v: boolean) => {
    setError("")
    setOn(v)
    if (!inWails()) return
    try {
      await write(v)
    } catch (err) {
      setError(errorText(err))
      setOn(await readOn().catch(() => !v))
    }
  }
  return (
    <div className="grid gap-1" data-setting={setting}>
      <Loading loading={!read} label={loading}>
        {!read ? (
          <ToggleShape label={label} hint={hint} />
        ) : (
          <Toggle
            label={label}
            hint={hint}
            checked={on}
            onChange={(v) => void set(v)}
          />
        )}
      </Loading>
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

// spot is the row's key for Ask Irori's "take me there" (card #784,
// lib/goto.ts settingSpots): data-setting, which it highlights.
export function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
  spot,
}: {
  label: string
  hint?: string
  checked: boolean
  disabled?: boolean
  onChange: (v: boolean) => void
  spot?: string
}) {
  const id = useId()
  return (
    <div
      data-setting={spot}
      className={`flex items-start justify-between gap-3 ${disabled ? "opacity-50" : ""}`}
    >
      <span className="flex flex-col">
        <label
          htmlFor={id}
          className={disabled ? "text-ink" : "cursor-pointer text-ink"}
        >
          {label}
        </label>
        {hint && (
          <span id={`${id}-hint`} className="text-xs text-muted-foreground">
            {hint}
          </span>
        )}
      </span>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        aria-describedby={hint ? `${id}-hint` : undefined}
        onCheckedChange={(v) => onChange(v)}
        className="mt-0.5"
      />
    </div>
  )
}

// ToggleShape stands in for a Toggle while its setting is read (card #598).
export function ToggleShape({ label, hint }: { label: string; hint?: string }) {
  return (
    <div className="flex items-start justify-between gap-3">
      <span className="flex flex-col">
        <span className="text-ink">{label}</span>
        {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
      </span>
      <span
        skullet-block=""
        className="mt-0.5 block h-5 w-9 shrink-0 rounded-full"
      />
    </div>
  )
}
