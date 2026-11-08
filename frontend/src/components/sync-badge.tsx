import { Spinner } from "@/components/ui/spinner"
import { useSyncing } from "@/lib/sync"
import { cn } from "@/lib/utils"

// SyncBadge (card #1089) is the small badge at the header's top left that
// shows this device syncing with a peer. It is always in the page, drawn
// from opacity 0: it fades in when syncing starts and back to 0 when it
// ends, so there is nothing to shift when it appears. The host holds the
// flag a second past the last row, so a short sync still shows. With
// reduced motion it appears and goes without fading, and the spinner is
// still. It sits over the header's top edge, out of the way of the menu
// button on a phone (which it clears by sitting to its right).
export function SyncBadge({ hasMenu }: { hasMenu: boolean }) {
  const syncing = useSyncing()
  return (
    <span
      data-slot="sync-badge"
      data-syncing={syncing}
      aria-hidden={!syncing}
      title="Syncing with your other devices"
      className={cn(
        "pointer-events-none absolute top-0.5 z-10 inline-flex h-4 items-center gap-1 rounded-full bg-muted px-1.5 text-[10px] font-medium text-muted-foreground",
        "transition-[opacity,visibility] duration-300 ease-out motion-reduce:transition-none",
        hasMenu ? "left-11 md:left-2" : "left-2",
        syncing ? "visible opacity-100" : "invisible opacity-0"
      )}
    >
      <Spinner
        aria-label="Syncing"
        className="size-2.5 motion-reduce:animate-none"
      />
      <span className="hidden sm:inline">Syncing</span>
    </span>
  )
}
