import { Badge } from "@/components/ui/badge"
import { SettingsMenu } from "@/components/settings/settings-menu"
import type { Roster } from "@/lib/irori"

// AppHeader is the top bar: the product mark, this device, the version and
// the settings.
export function AppHeader({ roster }: { roster: Roster }) {
  const { device, version, source, agents } = roster
  const busy = agents.filter((a) => a.status === "busy").length

  return (
    <header className="sticky top-0 z-20 glass-panel border-x-0! border-t-0!">
      <div className="mx-auto flex h-14 max-w-7xl items-center gap-3 px-4 sm:px-6">
        <div className="flex items-baseline gap-2">
          <span className="text-lg font-semibold tracking-tight text-ink">
            Irori
          </span>
          <span lang="ja" className="text-sm font-normal text-muted-foreground">
            囲炉裏
          </span>
        </div>
        {source === "sample" && (
          <Badge variant="warning" title="Not connected to the Irori app">
            Sample data
          </Badge>
        )}
        <div className="ml-auto flex min-w-0 items-center gap-3 text-sm text-muted-foreground">
          <span className="hidden text-xs tabular-nums sm:inline">
            {agents.length} {agents.length === 1 ? "agent" : "agents"}
            {busy > 0 && <span className="text-primary"> · {busy} busy</span>}
          </span>
          <span className="truncate text-xs font-medium text-ink-2" title={device?.id}>
            {device?.name || "this device"}
          </span>
          {version && (
            <Badge variant="slate" className="font-mono normal-case">
              {version === "dev" || version === "sample"
                ? version
                : `v${version.replace(/^v/, "")}`}
            </Badge>
          )}
          <SettingsMenu />
        </div>
      </div>
    </header>
  )
}
