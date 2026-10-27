import { AlertTriangle, CheckCircle2, Download, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import { useUpdater } from "./useUpdater"

// UpdateStatusLine is the version line for a settings/about view: the running
// version plus "Check for updates", or "Restart now" when an update is ready
// (so "Later" on the banner never strands the user). On a platform that
// cannot update itself yet (macOS) it says so instead of offering a check.
export function UpdateStatusLine() {
  const {
    status,
    version,
    supported,
    checking,
    upToDate,
    restartError,
    check,
    restart,
  } = useUpdater()
  const v = version || "dev"
  const target = status.version ? `v${status.version}` : ""
  const busy = checking || status.phase === "downloading"

  if (status.phase === "installing") {
    return (
      <div
        className="flex items-center gap-2 text-xs text-muted-foreground"
        role="status"
      >
        <RefreshCw className="size-3.5 animate-spin" aria-hidden="true" />
        Restarting to update{target ? ` to ${target}` : ""}…
      </div>
    )
  }

  return (
    <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
      <span>Irori {v === "dev" ? "(dev build)" : `v${v}`}</span>
      <span className="opacity-40">·</span>
      {!supported ? (
        <span>Updates are not supported on this platform yet</span>
      ) : status.phase === "ready" ? (
        <>
          <span className="flex items-center gap-1 text-primary">
            <Download className="size-3.5" aria-hidden="true" /> {target} is
            ready
          </span>
          <Button
            size="xs"
            onClick={() => void restart()}
            title={`Restart Irori and install ${target}`}
          >
            <RefreshCw aria-hidden="true" /> Restart now
          </Button>
          {restartError ? (
            <span
              className="flex items-center gap-1 text-destructive"
              title={restartError}
            >
              <AlertTriangle className="size-3.5" aria-hidden="true" /> Restart
              failed
            </span>
          ) : null}
        </>
      ) : (
        <>
          <Button
            size="xs"
            variant="ghost"
            disabled={busy || v === "dev"}
            onClick={() => void check()}
            title={
              v === "dev"
                ? "Dev builds never update"
                : "Check GitHub Releases for a newer Irori"
            }
          >
            <RefreshCw
              className={busy ? "animate-spin" : undefined}
              aria-hidden="true"
            />
            {status.phase === "downloading"
              ? `Downloading ${target}…`
              : checking
                ? "Checking…"
                : "Check for updates"}
          </Button>
          {upToDate && !busy ? (
            <span className="flex items-center gap-1 text-primary">
              <CheckCircle2 className="size-3.5" aria-hidden="true" /> Up to
              date
            </span>
          ) : null}
          {status.phase === "error" ? (
            // fail() keeps the version once the check found one, so a set
            // version means the download or verify step failed.
            <span
              className="flex items-center gap-1 text-destructive"
              title={status.error}
            >
              <AlertTriangle className="size-3.5" aria-hidden="true" />
              {status.version ? `Download of ${target} failed` : "Check failed"}
            </span>
          ) : null}
        </>
      )}
    </div>
  )
}
