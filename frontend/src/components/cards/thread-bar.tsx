// ThreadBar is a Talk thread's line on its ask card view (card #780), in
// place of the ask's Close: where it stands, Private and its state, the
// cards made from it, Resolve (or Reopen) and "Agents answer now". Talk's
// pane and the card's own page show the same view, so both have it.

import { useEffect, useState } from "react"

import { Badge } from "@/components/ui/badge"
import { Face } from "@/components/cards/faces"
import { Button } from "@/components/ui/button"
import { AgentsNowToggle } from "@/components/common/agents-now-toggle"
import {
  OpenedBy,
  stateBadges,
  stateLine,
  ThreadCards,
} from "@/components/talk/threads"
import {
  displayName,
  threads as threadsApi,
  type CardAsk,
  type ThreadSummary,
} from "@/lib/cards"
import type { SpawnState } from "@/lib/spawn-line"
import { errorText } from "@/lib/talk"
import { AskLine } from "./bits"
import { StartPastLimit } from "./start-past-limit"

export function ThreadBar({
  t,
  ask,
  spawn,
  newest = "",
  why = "",
  onResolved,
}: {
  t: ThreadSummary
  // The card's ask while it is open: its line says who took it, and with
  // spawn, an agent starting for it (card #739).
  ask?: CardAsk | null
  spawn?: SpawnState
  // The newest item in its thread: open, what it holds is read.
  newest?: string
  // Why nobody is free to take a waiting thread (card #172).
  why?: string
  onResolved?: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const resolved = t.state === "resolved"
  const badge = stateBadges[t.state]
  // Open and in view, what it holds now is read (card #719): Talk's count
  // drops it until an agent writes again. A reply that lands while the
  // window is in the background waits until it is back in front.
  useEffect(() => {
    if (resolved) return
    const read = () => {
      if (document.visibilityState === "visible" && document.hasFocus())
        void threadsApi.read(t.id).catch(() => {})
    }
    read()
    window.addEventListener("focus", read)
    return () => window.removeEventListener("focus", read)
  }, [t.id, newest, resolved])
  const act = (f: (id: string) => Promise<void>) => async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await f(t.id)
    } catch (err) {
      setError(errorText(err))
    } finally {
      setBusy(false)
    }
  }
  const resolve = act(async (id) => {
    await threadsApi.resolve(id)
    onResolved?.()
  })
  const reopen = act(threadsApi.reopen)
  const setAgentsNow = (on: boolean) =>
    void act((id) => threadsApi.setAgentsNow(id, on))()
  return (
    <div
      className="grid min-w-0 gap-2 px-4 py-3 glass-panel"
      aria-label="The thread"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <div className="flex min-w-48 flex-1 items-center gap-2">
          {t.private && (
            <Badge
              variant="slate"
              className="h-4 shrink-0 px-1.5 text-[10.5px]"
              title="Only you and your own agents see this thread; it never reaches other members' devices"
            >
              Private
            </Badge>
          )}
          {badge && (
            <Badge
              variant={badge.variant}
              className="h-4 shrink-0 px-1.5 text-[10.5px]"
            >
              {badge.label}
            </Badge>
          )}
          {/* A resolved one's badge says it all. */}
          {resolved ? null : ask ? (
            <AskLine ask={ask} spawn={spawn} className="min-w-0 flex-1" />
          ) : (
            <span className="min-w-0 flex-1 truncate text-[12.5px] text-muted-foreground">
              {stateLine(t, why)}
            </span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-3">
          {!resolved && (
            <AgentsNowToggle
              on={t.agentsNow}
              onChange={setAgentsNow}
              disabled={busy}
            />
          )}
          {resolved ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => void reopen()}
              title="Bring it back: your next reply goes to the agents that answered"
            >
              Reopen
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => void resolve()}
              title="Done with it: the thread closes"
            >
              Resolve
            </Button>
          )}
        </div>
      </div>
      {t.openedBy && <OpenedBy who={t.openedBy} />}
      {t.told.length > 0 && (
        <span className="flex flex-wrap items-center gap-1 text-[11px] text-muted-foreground">
          {t.state === "waiting" ? "Asked first:" : "Told:"}
          {t.told.map((w) => (
            <Badge
              key={w.key}
              variant="slate"
              className="h-4 gap-1 px-1 text-[10.5px]"
            >
              <Face who={w} inline className="size-3 text-(--face-tint)" />
              {displayName(w)}
            </Badge>
          ))}
        </span>
      )}
      {t.cards.length > 0 && <ThreadCards cards={t.cards} />}
      {t.state === "waiting" && (
        <StartPastLimit id={t.id} state={t} size="xs" />
      )}
      {error && (
        <span className="text-xs text-destructive" role="alert">
          {error}
        </span>
      )}
    </div>
  )
}
