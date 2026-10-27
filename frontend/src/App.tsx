import { useMemo, useState } from "react"

import { AppHeader } from "@/components/roster/app-header"
import { ActivityPanel } from "@/components/roster/activity-panel"
import { RosterList } from "@/components/roster/roster-list"
import { InboxPanel } from "@/components/inbox/inbox-panel"
import { WorkPanel } from "@/components/work/work-panel"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useNow, useRoster } from "@/lib/irori"
import {
  countsByAgent,
  State,
  taskOpen,
  useGone,
  useInbox,
  useWork,
} from "@/lib/talk"
import { UpdateBanner } from "@/updater"

type Tab = "roster" | "inbox" | "work"

const tabKey = "irori.tab"

function readTab(): Tab {
  // ?tab=inbox opens a tab directly (screenshots, links from the tray).
  const q = new URLSearchParams(window.location.search).get("tab")
  if (q === "roster" || q === "inbox" || q === "work") return q
  try {
    const t = localStorage.getItem(tabKey)
    if (t === "inbox" || t === "work") return t
  } catch {
    // Remembering the tab is a convenience only.
  }
  return "roster"
}

function writeTab(t: Tab) {
  try {
    localStorage.setItem(tabKey, t)
  } catch {
    // ignore
  }
}

// App is the v0.2 screen: who is running (Roster), what they say to each
// other (Inbox), and what they've claimed (Work).
export function App() {
  const roster = useRoster()
  const inbox = useInbox()
  const goneState = useGone()
  const [tab, setTab] = useState<Tab>(readTab)
  const work = useWork(tab === "work")
  const now = useNow()

  const counts = useMemo(
    () => countsByAgent(inbox.deliveries),
    [inbox.deliveries]
  )
  const held = inbox.deliveries.filter((d) => d.state === State.Held).length
  const openTasks = work.tasks.filter((t) => taskOpen(t.state)).length

  return (
    <div className="min-h-svh bg-background">
      <AppHeader roster={roster} />
      <main className="mx-auto flex max-w-5xl flex-col gap-5 px-4 py-5 sm:px-6">
        <UpdateBanner />

        <Tabs
          value={tab}
          onValueChange={(v) => {
            setTab(v as Tab)
            writeTab(v as Tab)
          }}
          className="gap-5"
        >
          <TabsList className="self-start">
            <TabsTrigger value="roster" className="px-3">
              Roster
              <Count n={roster.agents.length} />
            </TabsTrigger>
            <TabsTrigger value="inbox" className="px-3">
              Inbox
              {held > 0 ? <Count n={held} signal /> : null}
            </TabsTrigger>
            <TabsTrigger value="work" className="px-3">
              Work
              <Count n={openTasks} />
            </TabsTrigger>
          </TabsList>

          <TabsContent value="roster" className="flex flex-col gap-6">
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
              <RosterList
                agents={roster.agents}
                counts={counts}
                gone={goneState.gone}
                goneError={goneState.error}
                now={now}
              />
            )}
            <ActivityPanel
              events={roster.recent}
              agents={roster.agents}
              error={roster.recentError}
              now={now}
            />
          </TabsContent>

          <TabsContent value="inbox">
            <InboxPanel
              inbox={inbox}
              agents={roster.agents}
              gone={goneState.gone}
              now={now}
            />
          </TabsContent>

          <TabsContent value="work">
            <WorkPanel
              work={work}
              agents={roster.agents}
              gone={goneState.gone}
              now={now}
            />
          </TabsContent>
        </Tabs>
      </main>
    </div>
  )
}

// Count is a tab's number; signal marks one that wants the person.
function Count({ n, signal }: { n: number; signal?: boolean }) {
  return (
    <span
      className={
        signal
          ? "rounded-full bg-warning-foreground px-1.5 text-[11px] text-warning tabular-nums"
          : "text-xs text-muted-foreground tabular-nums"
      }
    >
      {n}
    </span>
  )
}

export default App
