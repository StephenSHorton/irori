import { useId, useState, type ReactNode } from "react"
import { BellIcon, SettingsIcon } from "lucide-react"

import { ChoiceSelect } from "@/components/common/choice-select"
import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { Slider } from "@/components/ui/slider"
import { Switch } from "@/components/ui/switch"
import { corners, useSettings } from "@/lib/settings"

const cornerChoices = corners.map((c) => ({ value: c.id, label: c.label }))

// SettingsMenu is the header's gear: notifications (on or off, sound,
// volume, corner, a test card) and launch at login, which the tray also
// offers. It is a Popover so the Corner select's own popup, which portals
// out of the panel, does not count as a click outside it.
export function SettingsMenu() {
  const [open, setOpen] = useState(false)
  const s = useSettings(open)

  const n = s.notify
  const off = !n?.enabled

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Settings"
            title="Settings"
          />
        }
      >
        <SettingsIcon />
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={8}
        aria-label="Settings"
        className="w-72 max-w-[calc(100vw-2rem)] gap-3 p-4"
      >
        <h2 className="flex items-center gap-1.5 font-semibold text-ink">
          <BellIcon className="size-4" aria-hidden />
          Notifications
        </h2>
        {!n ? (
          <p className="text-muted-foreground">
            {s.error ? `Could not load: ${s.error}` : "Loading…"}
          </p>
        ) : (
          <>
            <Toggle
              label="Show notifications"
              hint="Held messages, cards blocked on you, and agents asking you to choose."
              checked={n.enabled}
              onChange={(v) => s.setNotify({ enabled: v })}
            />
            <Toggle
              label="Sound"
              checked={n.sound}
              disabled={off}
              onChange={(v) => s.setNotify({ sound: v })}
            />
            <Row label="Volume" disabled={off || !n.sound}>
              <Slider
                min={0}
                max={100}
                step={5}
                value={[Math.round(n.volume * 100)]}
                disabled={off || !n.sound}
                thumbLabel="Volume"
                onValueChange={(v) =>
                  s.setNotify({
                    volume: (Array.isArray(v) ? v[0] : v) / 100,
                  })
                }
                className="w-32"
              />
            </Row>
            <Row label="Corner" disabled={off}>
              <ChoiceSelect
                label="Corner"
                size="sm"
                value={n.corner}
                disabled={off}
                choices={cornerChoices}
                onValueChange={(v) => s.setNotify({ corner: v })}
                className="text-[13px]"
              />
            </Row>
            <div className="flex flex-col gap-1">
              <Button
                variant="outline"
                size="sm"
                disabled={off}
                onClick={() => void s.test()}
              >
                Send a test notification
              </Button>
              {s.tested && (
                <p className="text-xs text-muted-foreground">{s.tested}</p>
              )}
            </div>
          </>
        )}
        <div className="border-t border-border pt-3">
          <Toggle
            label="Launch at login"
            hint="Irori starts in the tray when you sign in."
            checked={Boolean(s.launchAtLogin)}
            disabled={s.launchAtLogin === null}
            onChange={(v) => void s.setLaunchAtLogin(v)}
          />
        </div>
        {n && s.error && <p className="text-xs text-destructive">{s.error}</p>}
      </PopoverContent>
    </Popover>
  )
}

function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string
  hint?: string
  checked: boolean
  disabled?: boolean
  onChange: (v: boolean) => void
}) {
  const id = useId()
  return (
    <div
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

function Row({
  label,
  disabled,
  children,
}: {
  label: string
  disabled?: boolean
  children: ReactNode
}) {
  return (
    <div
      className={`flex items-center justify-between gap-3 ${disabled ? "[&>span]:opacity-50" : ""}`}
    >
      <span className="text-ink">{label}</span>
      {children}
    </div>
  )
}
