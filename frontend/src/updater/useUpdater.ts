import { useCallback, useEffect, useState } from "react"
import { Events as WailsEvents } from "@wailsio/runtime"
import {
  Status,
  UpdateService,
} from "../../bindings/github.com/StephenSHorton/irori/internal/update"
import { previewFlag } from "@/lib/preview"

// The backend's update:status event (internal/update EventUpdateStatus).
export const UPDATE_STATUS_EVENT = "update:status"

// localStorage key for the per-viewer "Later" choice: the dismissed version.
// The banner stays hidden for that version only; a newer release shows again.
const DISMISS_KEY = "irori.update.dismissed"

export type UpdatePhase =
  "idle" | "checking" | "downloading" | "ready" | "installing" | "error"

export interface UpdaterState {
  /** Backend update state (phase/version/notes/error). */
  status: Status
  /** Running version ("dev" in non-release builds; "" until read). */
  version: string
  /** This platform can update itself (Windows only for now). */
  supported: boolean
  /** A check is in flight (manual or the backend's own). */
  checking: boolean
  /** The last manual check finished and found nothing newer. */
  upToDate: boolean
  /** The ready banner was dismissed for status.version ("Later"). */
  dismissed: boolean
  /** Why the last "Restart now" failed ("" if it did not). */
  restartError: string
  /** Manual "Check for updates": checks and downloads, never installs. */
  check: () => Promise<void>
  /** "Restart now": launches the installer and quits Irori. */
  restart: () => Promise<void>
  /** "Later": hide the banner for this version. */
  dismiss: () => void
}

function readDismissed(): string {
  try {
    return window.localStorage.getItem(DISMISS_KEY) ?? ""
  } catch {
    return ""
  }
}

// sampleStatus is the browser preview's ?update=<phase> (lib/preview.ts).
function sampleStatus(): Status | null {
  const phase = previewFlag("update")
  if (phase === null) return null
  return new Status({ phase: phase || "ready", version: "0.4.0" })
}

function toStatus(raw: unknown): Status {
  // createFrom JSON-parses a string payload itself; anything else is idle.
  return Status.createFrom(
    typeof raw === "string" || (raw && typeof raw === "object") ? raw : {}
  )
}

/**
 * useUpdater mirrors the backend update state machine. The backend
 * (internal/update) owns the policy: at startup it checks, downloads and
 * verifies a newer release in the background, then stops at "ready" and
 * announces it. It never installs or quits on its own: the user restarts when
 * they like (banner or tray), or the installer runs when Irori quits.
 *
 * This hook reads GetStatus() on mount, follows update:status events, and
 * exposes the manual check / restart / dismiss actions. It does not start a
 * check on mount; the startup check already does. Outside Wails (a plain
 * browser preview) the binding calls reject: mount-time reads are swallowed,
 * while a manual check shows the failure.
 */
export function useUpdater(): UpdaterState {
  const [status, setStatus] = useState<Status>(
    () => sampleStatus() ?? new Status({ phase: "idle" })
  )
  const [version, setVersion] = useState("")
  const [supported, setSupported] = useState(true)
  const [inFlight, setInFlight] = useState(false)
  const [upToDate, setUpToDate] = useState(false)
  const [dismissedVersion, setDismissedVersion] = useState(readDismissed)
  const [restartError, setRestartError] = useState("")

  useEffect(() => {
    UpdateService.GetVersion()
      .then(setVersion)
      .catch(() => setVersion(""))
    UpdateService.Supported()
      .then(setSupported)
      .catch(() => {})
    UpdateService.GetStatus()
      .then((s) => setStatus(toStatus(s)))
      .catch(() => {})

    const off = WailsEvents.On(UPDATE_STATUS_EVENT, (ev) => {
      const raw: unknown = Array.isArray(ev.data) ? ev.data[0] : ev.data
      const next = toStatus(raw)
      setStatus(next)
      // Any move off idle makes a stale "Up to date" badge wrong.
      if (next.phase !== "idle") setUpToDate(false)
    })
    return () => off()
  }, [])

  const check = useCallback(async () => {
    setInFlight(true)
    setUpToDate(false)
    try {
      const result = toStatus(await UpdateService.CheckAndDownload())
      setStatus(result)
      // "Up to date" is not a backend phase: a finished check that found
      // nothing lands on idle. A checking/downloading result means we raced
      // the startup check; its next update:status event settles it.
      if (result.phase === "idle") setUpToDate(true)
    } catch (e) {
      setStatus(new Status({ phase: "error", error: String(e) }))
    } finally {
      setInFlight(false)
    }
  }, [])

  // The backend keeps the authoritative phase: it reverts to "ready" when the
  // installer fails to launch, so the user can retry or just quit.
  const restart = useCallback(async () => {
    setRestartError("")
    try {
      await UpdateService.RestartToUpdate()
    } catch (e) {
      setRestartError(String(e))
    }
  }, [])

  const dismiss = useCallback(() => {
    const v = status.version
    setDismissedVersion(v)
    try {
      window.localStorage.setItem(DISMISS_KEY, v)
    } catch {
      // a per-viewer convenience; in-memory state still hides the banner
    }
  }, [status.version])

  const checking = inFlight || status.phase === "checking"
  const dismissed = status.version !== "" && dismissedVersion === status.version

  return {
    status,
    version,
    supported,
    checking,
    upToDate,
    dismissed,
    restartError,
    check,
    restart,
    dismiss,
  }
}
