import { useState } from "react"
import { AlertTriangle, Download, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import { isAndroid, NEEDS_INSTALL_PERMISSION, updateErrorLine } from "./android"
import { useUpdater, type UpdaterState } from "./useUpdater"

// UpdateBanner is the in-window notice that a new Irori is ready. Mount it at
// the top of the main view. The backend never installs on its own, so this is
// where the user decides: "Restart now" launches the installer and quits;
// "Later" hides the bar for this version (the tray keeps a Restart item), and
// the update installs when Irori quits anyway.
//
// It also shows the other visible phases: a thin "Downloading" line while the
// installer is fetched, and "Restarting to update" once it has been launched.
export function UpdateBanner() {
  const u = useUpdater()
  const { status, dismissed, restartError, restart, dismiss } = u
  const v = status.version ? `v${status.version}` : ""

  // A phone updates through Android's install screen (card #270).
  if (isAndroid()) return <AndroidBanner u={u} />

  if (status.phase === "downloading") {
    return (
      <div
        role="status"
        aria-live="polite"
        aria-label={`Downloading Irori ${v}`}
        className="mb-4 flex flex-col gap-1.5 text-xs text-muted-foreground"
      >
        <span className="flex items-center gap-2">
          <Download className="size-3.5" aria-hidden="true" />
          Downloading Irori {v}…
        </span>
        <div className="relative h-1 overflow-hidden bg-muted">
          <div className="absolute inset-y-0 w-1/4 animate-[irori-sweep_1.1s_ease-in-out_infinite] bg-primary" />
        </div>
      </div>
    )
  }

  if (status.phase === "installing") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="mb-4 flex items-center gap-2 text-xs text-muted-foreground"
      >
        <RefreshCw className="size-3.5 animate-spin" aria-hidden="true" />
        Restarting to update…
      </div>
    )
  }

  if (status.phase !== "ready" || dismissed) return null

  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={`Irori ${v} is ready to install`}
      className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 text-card-foreground glass-panel"
    >
      <Download className="size-5 shrink-0 text-primary" aria-hidden="true" />
      <div className="min-w-0 flex-1 basis-56">
        <div className="text-sm font-medium">Irori {v} is ready.</div>
        <div className="text-xs text-muted-foreground">
          Restart now to update, or keep working. It installs when you quit
          Irori.
        </div>
        {restartError ? (
          <div className="mt-0.5 text-xs text-destructive" title={restartError}>
            Couldn't start the installer. Try again, or quit Irori to install.
          </div>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button
          size="sm"
          onClick={() => void restart()}
          title={`Restart Irori and install ${v}`}
        >
          <RefreshCw aria-hidden="true" /> Restart now
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={dismiss}
          title="Keep working; it installs when you quit Irori"
        >
          Later
        </Button>
      </div>
    </div>
  )
}

// AndroidBanner is the phone's update bar (card #270). The check never
// downloads: "Irori vX is available" waits for the person's Update, which
// downloads the APK, checks it against SHA256SUMS and opens Android's install
// screen. Android then replaces the app and Irori starts on the new version
// with its data. A cancelled install screen leaves Install to tap again; a
// bad hash or a failed download says so, with Try again.
function AndroidBanner({ u }: { u: UpdaterState }) {
  const { status, dismissed, restart, dismiss } = u
  const v = status.version ? `v${status.version}` : ""
  const [busy, setBusy] = useState(false)
  const go = () => {
    setBusy(true)
    void restart().finally(() => setBusy(false))
  }

  if (status.phase === "downloading") {
    return (
      <div
        role="status"
        aria-live="polite"
        aria-label={`Downloading Irori ${v}`}
        className="mb-4 flex flex-col gap-1.5 text-xs text-muted-foreground"
      >
        <span className="flex items-center gap-2">
          <Download className="size-3.5" aria-hidden="true" />
          Downloading Irori {v}…
        </span>
        <div className="relative h-1 overflow-hidden bg-muted">
          <div className="absolute inset-y-0 w-1/4 animate-[irori-sweep_1.1s_ease-in-out_infinite] bg-primary" />
        </div>
      </div>
    )
  }

  // An error with a version is a failed download or check of that update;
  // without one, the check itself failed (nothing to show on the bar).
  const failed = status.phase === "error" && !!status.version
  if (!["available", "ready"].includes(status.phase) && !failed) return null
  if (dismissed && !failed) return null

  const ready = status.phase === "ready"
  const permission = ready && status.error === NEEDS_INSTALL_PERMISSION
  const title = failed
    ? `Irori ${v} didn't install.`
    : ready
      ? `Irori ${v} is downloaded.`
      : `Irori ${v} is available.`
  const line = failed
    ? updateErrorLine(status.error)
    : permission
      ? status.error
      : ready
        ? status.error ||
          "Install opens Android's install screen. Irori restarts on the new version with your data kept."
        : "Update downloads it, checks it, then opens Android's install screen."
  const action = failed ? "Try again" : ready ? "Install" : "Update"

  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={title}
      className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 text-card-foreground glass-panel"
    >
      {failed ? (
        <AlertTriangle
          className="size-5 shrink-0 text-destructive"
          aria-hidden="true"
        />
      ) : (
        <Download className="size-5 shrink-0 text-primary" aria-hidden="true" />
      )}
      <div className="min-w-0 flex-1 basis-56">
        <div className="text-sm font-medium">{title}</div>
        <div
          className={
            failed || permission
              ? "text-xs text-warning"
              : "text-xs text-muted-foreground"
          }
          title={failed ? status.error : undefined}
        >
          {line}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button size="sm" disabled={busy} onClick={go}>
          <Download aria-hidden="true" /> {action}
        </Button>
        {!failed && (
          <Button size="sm" variant="ghost" onClick={dismiss}>
            Later
          </Button>
        )}
      </div>
    </div>
  )
}
