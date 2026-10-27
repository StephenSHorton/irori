import { useEffect, useRef, useState, type ReactNode } from "react"
import { BellIcon, SettingsIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { corners, useSettings } from "@/lib/settings"
import { UpdateStatusLine } from "@/updater"

// SettingsMenu is the header's gear: notifications (on or off, sound,
// volume, corner, a test card), launch at login, which the tray also
// offers, and the version line with "Check for updates" (or "Restart now"
// once an update is ready, so "Later" on the banner or card never strands it).
export function SettingsMenu() {
  const [open, setOpen] = useState(false)
  const s = useSettings(open)
  const box = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const down = (e: PointerEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false)
    }
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false)
    window.addEventListener("pointerdown", down)
    window.addEventListener("keydown", key)
    return () => {
      window.removeEventListener("pointerdown", down)
      window.removeEventListener("keydown", key)
    }
  }, [open])

  const n = s.notify
  const off = !n?.enabled

  return (
    <div ref={box} className="relative">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Settings"
        aria-expanded={open}
        title="Settings"
        onClick={() => setOpen(!open)}
      >
        <SettingsIcon />
      </Button>
      {open && (
        <div
          role="dialog"
          aria-label="Settings"
          className="absolute top-full right-0 z-30 mt-2 flex w-72 max-w-[calc(100vw-2rem)] flex-col gap-3 rounded-xl border border-border bg-popover p-4 text-sm text-popover-foreground shadow-lg"
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
              <Row label="Volume">
                <input
                  type="range"
                  min={0}
                  max={100}
                  step={5}
                  value={Math.round(n.volume * 100)}
                  disabled={off || !n.sound}
                  aria-label="Volume"
                  onChange={(e) =>
                    s.setNotify({ volume: Number(e.target.value) / 100 })
                  }
                  className="w-32 accent-primary disabled:opacity-50"
                />
              </Row>
              <Row label="Corner">
                <select
                  value={n.corner}
                  disabled={off}
                  aria-label="Corner"
                  onChange={(e) => s.setNotify({ corner: e.target.value })}
                  className="h-7 rounded-md border border-border bg-background px-2 text-[13px] text-ink disabled:opacity-50"
                >
                  {corners.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </select>
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
          <div className="border-t border-border pt-3">
            <UpdateStatusLine />
          </div>
          {n && s.error && (
            <p className="text-xs text-destructive">{s.error}</p>
          )}
        </div>
      )}
    </div>
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
  return (
    <label
      className={`flex items-start gap-2.5 ${disabled ? "opacity-50" : "cursor-pointer"}`}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 size-4 accent-primary"
      />
      <span className="flex flex-col">
        <span className="text-ink">{label}</span>
        {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
      </span>
    </label>
  )
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-ink">{label}</span>
      {children}
    </div>
  )
}
