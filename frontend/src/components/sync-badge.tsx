import { Spinner } from "@/components/ui/spinner"
import { useSyncing } from "@/lib/sync"
import { cn } from "@/lib/utils"

// SyncBadge (cards #1089, #1231) is the small badge in the header's right
// hand row, just left of the meters strip, that shows this device syncing
// with a peer. It is an ordinary in-flow item, always in the page, drawn
// from opacity 0: it fades in when syncing starts and back to 0 when it
// ends, so there is nothing to shift when it appears. The host holds the
// flag a second past the last row, so a short sync still shows. With
// reduced motion it appears and goes without fading, and the spinner is
// still. It keeps its space while hidden (invisible, not removed), so the
// meters never shift when it fades in.
export function SyncBadge() {
  const syncing = useSyncing()
  return (
    <span
      data-slot="sync-badge"
      data-syncing={syncing}
      aria-hidden={!syncing}
      title="Syncing with your other devices"
      className={cn(
        "pointer-events-none inline-flex h-5 shrink-0 items-center gap-1 rounded-full bg-muted px-1.5 text-[10px] font-medium text-muted-foreground",
        "transition-[opacity,visibility] duration-300 ease-out motion-reduce:transition-none",
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
