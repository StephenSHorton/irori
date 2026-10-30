// App settings: notifications (internal/notify, kept in settings.json) and
// launch at login (the OS registration). Components import from here.

import { useCallback, useEffect, useState } from "react"

import { Events, SettingsService } from "./host"
import { NotifyOptions, type Target } from "./models"
import { inWails } from "./irori"
import { errorText } from "./talk"

export { NotifyOptions }
export type { Target }

// EventOpen is app:open (notify_gui.go): show a tab and select a ref there.
export const EventOpen = "app:open"

// EventLaunchAtLogin is settings:launch-at-login (settings_service.go).
const EventLaunchAtLogin = "settings:launch-at-login"

export const corners = [
  { id: "top-right", label: "Top right" },
  { id: "bottom-right", label: "Bottom right" },
  { id: "top-left", label: "Top left" },
  { id: "bottom-left", label: "Bottom left" },
] as const

const sampleOptions = NotifyOptions({
  enabled: true,
  sound: true,
  volume: 0.6,
  corner: "top-right",
})

export interface SettingsState {
  notify: NotifyOptions | null
  launchAtLogin: boolean | null
  error: string | null
  // A note after the test button: "Sent" or why it couldn't be.
  tested: string | null
  setNotify: (patch: Partial<NotifyOptions>) => Promise<void>
  setLaunchAtLogin: (on: boolean) => Promise<void>
  test: () => Promise<void>
}

// useSettings loads the settings when active (the panel is open) and keeps
// launch at login in step with the tray's checkbox.
export function useSettings(active: boolean): SettingsState {
  // Outside the app (sample data) the controls work on local state only.
  const [notify, setNotifyState] = useState<NotifyOptions | null>(() =>
    inWails() ? null : sampleOptions
  )
  const [launchAtLogin, setLaunch] = useState<boolean | null>(() =>
    inWails() ? null : false
  )
  const [error, setError] = useState<string | null>(null)
  const [tested, setTested] = useState<string | null>(null)

  useEffect(() => {
    if (!active || !inWails()) return
    let live = true
    Promise.all([
      SettingsService.GetNotifications(),
      SettingsService.GetLaunchAtLogin(),
    ])
      .then(([n, l]) => {
        if (!live) return
        setNotifyState(n)
        setLaunch(l)
        setError(null)
      })
      .catch((err) => live && setError(errorText(err)))
    const off = Events.On(EventLaunchAtLogin, (ev) =>
      setLaunch(Boolean(ev.data))
    )
    return () => {
      live = false
      off()
    }
  }, [active])

  const setNotify = useCallback(
    async (patch: Partial<NotifyOptions>) => {
      if (!notify) return
      const next = NotifyOptions({ ...notify, ...patch })
      setNotifyState(next) // the control moves at once; the reply settles it
      setTested(null)
      if (!inWails()) return
      try {
        setNotifyState(await SettingsService.SetNotifications(next))
        setError(null)
      } catch (err) {
        setError(errorText(err))
      }
    },
    [notify]
  )

  const setLaunchAtLogin = useCallback(async (on: boolean) => {
    setLaunch(on)
    if (!inWails()) return
    try {
      await SettingsService.SetLaunchAtLogin(on)
      setError(null)
    } catch (err) {
      setError(errorText(err))
      setLaunch(await SettingsService.GetLaunchAtLogin().catch(() => !on))
    }
  }, [])

  const test = useCallback(async () => {
    if (!inWails()) {
      setTested("Sample data: no card to show here.")
      return
    }
    try {
      await SettingsService.TestNotification()
      setTested("Sent. Look in the corner of your screen.")
    } catch (err) {
      setTested(errorText(err))
    }
  }, [])

  return {
    notify,
    launchAtLogin,
    error,
    tested,
    setNotify,
    setLaunchAtLogin,
    test,
  }
}

// onOpen runs fn for every app:open (a notification's Open or Answer).
// In a plain browser, ?open=<tab>:<ref> (say ?open=inbox:sample-delivery-6)
// sends one, so the dialogs it opens can be tried.
export function onOpen(fn: (t: Target) => void): () => void {
  if (!inWails()) {
    const open = new URLSearchParams(window.location.search).get("open")
    if (!open) return () => {}
    const [tab, ...ref] = open.split(":")
    const id = window.setTimeout(() => fn({ tab, ref: ref.join(":") }), 0)
    return () => window.clearTimeout(id)
  }
  return Events.On<Target>(EventOpen, (ev) => fn(ev.data))
}
