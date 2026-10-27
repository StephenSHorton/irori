import { AppHeader } from "@/components/roster/app-header"
import { ActivityPanel } from "@/components/roster/activity-panel"
import { RosterList } from "@/components/roster/roster-list"
import { useNow, useRoster } from "@/lib/irori"
import { UpdateBanner } from "@/updater"

// App is the v0.1 screen: who is running on this device, and what the log saw.
export function App() {
  const roster = useRoster()
  const now = useNow()

  return (
    <div className="min-h-svh bg-background">
      <AppHeader roster={roster} />
      <main className="mx-auto flex max-w-5xl flex-col gap-6 px-4 py-6 sm:px-6">
        <UpdateBanner />

        {roster.error && (
          <p className="rounded-lg bg-destructive/10 px-4 py-2 text-sm text-destructive">
            Could not load the roster: {roster.error}
          </p>
        )}

        {roster.loading ? (
          <p className="px-1 text-sm text-muted-foreground">
            Looking for agents…
          </p>
        ) : (
          <RosterList agents={roster.agents} now={now} />
        )}

        <ActivityPanel
          events={roster.recent}
          agents={roster.agents}
          error={roster.recentError}
          now={now}
        />
      </main>
    </div>
  )
}

export default App
