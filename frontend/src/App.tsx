import { useMemo, useState } from "react"

import { AppHeader } from "@/components/roster/app-header"
import { ActivityPanel } from "@/components/roster/activity-panel"
import { RosterList } from "@/components/roster/roster-list"
import { InboxPanel } from "@/components/inbox/inbox-panel"
import { WorkPanel } from "@/components/work/work-panel"
import { DecisionsPanel } from "@/components/capture/decisions-panel"
import {
  HistoryPanel,
  type HistoryFocus,
} from "@/components/capture/history-panel"
import { TimelinePanel } from "@/components/capture/timeline-panel"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useFidelity, useRepos } from "@/lib/capture"
import { useNow, useRoster } from "@/lib/irori"
import {
  countsByAgent,
  isOpen,
  nameLookup,
  State,
  taskOpen,
  useGone,
  useInbox,
  useWork,
} from "@/lib/talk"
import { UpdateBanner } from "@/updater"

const tabs = [
  "roster",
  "inbox",
  "work",
  "timeline",
  "history",
  "decisions",
] as const
type Tab = (typeof tabs)[number]

const tabKey = "irori.tab"

function asTab(v: string | null): Tab | null {
  return tabs.includes(v as Tab) ? (v as Tab) : null
}

function readTab(): Tab {
  // ?tab=inbox opens a tab directly (screenshots, links from the tray).
  const q = asTab(new URLSearchParams(window.location.search).get("tab"))
  if (q) return q
  try {
    return asTab(localStorage.getItem(tabKey)) ?? "roster"
  } catch {
    // Remembering the tab is a convenience only.
    return "roster"
  }
}

function writeTab(t: Tab) {
  try {
    localStorage.setItem(tabKey, t)
  } catch {
    // ignore
  }
}

// App is who is running (Roster), what they say to each other (Inbox), what
// they've claimed (Work), and what they did (Timeline, History, Decisions).
export function App() {
  const roster = useRoster()
  const inbox = useInbox()
  const goneState = useGone()
  const [tab, setTabState] = useState<Tab>(readTab)
  const work = useWork(tab === "work")
  const fidelity = useFidelity()
  const { repos } = useRepos()
  const now = useNow()
  // Cross-tab jumps: a roster card's Timeline link, a commit chip.
  const [agentKey, setAgentKey] = useState("")
  const [focus, setFocus] = useState<{ n: number; f: HistoryFocus | null }>({
    n: 0,
    f: null,
  })

  const setTab = (t: Tab) => {
    setTabState(t)
    writeTab(t)
  }

  const counts = useMemo(
    () => countsByAgent(inbox.deliveries),
    [inbox.deliveries]
  )
  const name = useMemo(
    () =>
      nameLookup(
        roster.agents.map((a): [string, string] => [a.key, a.name]),
        goneState.gone.map((g): [string, string] => [g.key, g.name]),
        inbox.deliveries.map((d): [string, string] => [
          d.message.from.key,
          d.message.from.name ?? "",
        ])
      ),
    [roster.agents, goneState.gone, inbox.deliveries]
  )
  const held = inbox.deliveries.filter((d) => d.state === State.Held).length
  const openTasks = work.tasks.filter((t) => taskOpen(t.state)).length

  const goTimeline = (key: string) => {
    setAgentKey(key)
    setTab("timeline")
  }

  return (
    <div className="min-h-svh bg-background">
      <AppHeader roster={roster} />
      <main className="mx-auto flex max-w-5xl flex-col gap-5 px-4 py-5 sm:px-6">
        <UpdateBanner />

        <Tabs
          value={tab}
          onValueChange={(v) => setTab(v as Tab)}
          className="gap-5"
        >
          <TabsList className="max-w-full self-start overflow-x-auto">
            <TabsTrigger value="roster" className="px-3">
              Roster
              <Count n={roster.agents.length} />
            </TabsTrigger>
            <TabsTrigger value="inbox" className="px-3">
              Inbox
              {held > 0 ? (
                <Count n={held} signal />
              ) : (
                <Count
                  n={inbox.deliveries.filter((d) => isOpen(d.state)).length}
                />
              )}
            </TabsTrigger>
            <TabsTrigger value="work" className="px-3">
              Work
              <Count n={openTasks} />
            </TabsTrigger>
            <TabsTrigger value="timeline" className="px-3">
              Timeline
            </TabsTrigger>
            <TabsTrigger value="history" className="px-3">
              History
            </TabsTrigger>
            <TabsTrigger value="decisions" className="px-3">
              Decisions
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
                fidelity={fidelity}
                onTimeline={goTimeline}
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

          <TabsContent value="timeline">
            <TimelinePanel
              agentKey={agentKey}
              onAgent={setAgentKey}
              agents={roster.agents}
              gone={goneState.gone}
              deliveries={inbox.deliveries}
              repos={repos}
              now={now}
              onCommit={(c) => {
                setFocus((f) => ({
                  n: f.n + 1,
                  f: { repoKey: c.repoKey, sha: c.sha },
                }))
                setTab("history")
              }}
            />
          </TabsContent>

          <TabsContent value="history">
            <HistoryPanel
              key={focus.n}
              repos={repos}
              focus={focus.f}
              name={name}
              onTimeline={goTimeline}
              now={now}
            />
          </TabsContent>

          <TabsContent value="decisions">
            <DecisionsPanel repos={repos} now={now} />
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
