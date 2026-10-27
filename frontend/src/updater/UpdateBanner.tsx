import { Download, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import { useUpdater } from "./useUpdater"

// UpdateBanner is the in-window notice that a new Irori is ready. Mount it at
// the top of the main view. The backend never installs on its own, so this is
// where the user decides: "Restart now" launches the installer and quits;
// "Later" hides the bar for this version (the tray keeps a Restart item), and
// the update installs when Irori quits anyway.
//
// It also shows the other visible phases: a thin "Downloading" line while the
// installer is fetched, and "Restarting to update" once it has been launched.
export function UpdateBanner() {
  const { status, dismissed, restartError, restart, dismiss } = useUpdater()
  const v = status.version ? `v${status.version}` : ""

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
        <div className="h-1 overflow-hidden rounded-full bg-muted">
          <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
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
      className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border bg-card px-4 py-3 text-card-foreground"
    >
      <Download className="size-5 shrink-0 text-primary" aria-hidden="true" />
      <div className="min-w-0 flex-1">
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
